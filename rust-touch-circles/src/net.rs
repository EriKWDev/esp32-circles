//! Wi-Fi, TCP and the HTTP client that talks to the irrigation controllers.
//!
//! Three things shape this module.
//!
//! **esp-radio needs a scheduler.** The Wi-Fi blobs require `esp-rtos`, so the
//! firmware is no longer strictly RTOS-free. The render loop still owns the main
//! thread; the scheduler exists for the radio's own tasks.
//!
//! **Requests are made while the UI is idle.** A request is synchronous here,
//! pumping smoltcp in its own wait loop until the reply lands. That is far
//! simpler than a state machine spread across frames, and it costs nothing
//! visible because `poll_due` is only honoured when nothing is animating - so a
//! transfer can never interrupt a transition. `present()` already blocks for
//! ~20 ms per frame, which is longer than smoltcp likes to be ignored, so
//! interleaving would mean pumping the stack between DMA stripes; deliberately
//! not done yet.
//!
//! **The API is behind HTTP Digest.** The Axis device's own web server
//! authenticates every request to the ACAP, and Digest is the only scheme
//! offered - so this implements RFC 2617 MD5 challenge-response: an
//! unauthenticated probe, then a retry carrying the computed response.

extern crate alloc;

use core::fmt::Write as _;

use esp_radio::wifi::{Interface as WifiIface, WifiController, WifiRxToken, WifiTxToken};
use smoltcp::iface::{Config as IfConfig, Interface, SocketSet, SocketStorage};
use smoltcp::phy::{Checksum, Device, DeviceCapabilities, Medium, RxToken, TxToken};
use smoltcp::socket::{dhcpv4, tcp};
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, HardwareAddress, IpAddress, IpCidr, Ipv4Address};

use crate::model::{parse_dump, State};

include!(concat!(env!("OUT_DIR"), "/secrets.rs"));

/// Wi-Fi MTU as the driver reports it.
const MTU: usize = 1514;
/// One `/api/dump` is well under 1 KiB today; this leaves generous headroom for
/// more controllers' relays without risking a truncated parse.
const RX_BODY: usize = 4096;

/// smoltcp's `Device` over esp-radio's token pair.
///
/// esp-radio hands out tokens with `consume_token`, not smoltcp's `consume`, and
/// does not implement the trait itself in this release - hence the wrappers.
pub struct Phy {
    iface: WifiIface,
}

pub struct RxTok(WifiRxToken);
pub struct TxTok(WifiTxToken);

impl RxToken for RxTok {
    fn consume<R, F: FnOnce(&[u8]) -> R>(self, f: F) -> R {
        self.0.consume_token(|buf| f(buf))
    }
}

impl TxToken for TxTok {
    fn consume<R, F: FnOnce(&mut [u8]) -> R>(self, len: usize, f: F) -> R {
        self.0.consume_token(len, f)
    }
}

impl Device for Phy {
    type RxToken<'a> = RxTok where Self: 'a;
    type TxToken<'a> = TxTok where Self: 'a;

    fn receive(&mut self, _now: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        self.iface.receive().map(|(rx, tx)| (RxTok(rx), TxTok(tx)))
    }

    fn transmit(&mut self, _now: Instant) -> Option<Self::TxToken<'_>> {
        self.iface.transmit().map(TxTok)
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.max_transmission_unit = MTU;
        caps.medium = Medium::Ethernet;
        // The radio validates Ethernet frames itself; IP and TCP checksums are
        // ours to do.
        caps.checksum.ipv4 = Checksum::Both;
        caps.checksum.tcp = Checksum::Both;
        caps
    }
}

/// A controller we poll. Several will be deployed, and the UI is meant to
/// present their relays as one list, so the host is a table from the start
/// rather than a single constant.
#[derive(Clone, Copy)]
pub struct Host {
    pub ip: Ipv4Address,
    pub label: &'static str,
}

pub const HOSTS: &[Host] = &[Host {
    ip: parse_ip(RB_HOST),
    label: RB_HOST,
}];

/// `const`-evaluable dotted-quad parser, so HOSTS can be built at compile time
/// from the string baked in by build.rs.
const fn parse_ip(s: &str) -> Ipv4Address {
    let b = s.as_bytes();
    let mut octets = [0u8; 4];
    let mut index = 0;
    let mut value = 0u16;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'.' {
            if index < 4 {
                octets[index] = value as u8;
            }
            index += 1;
            value = 0;
        } else if c >= b'0' && c <= b'9' {
            value = value * 10 + (c - b'0') as u16;
        }
        i += 1;
    }
    if index < 4 {
        octets[index] = value as u8;
    }
    Ipv4Address::new(octets[0], octets[1], octets[2], octets[3])
}

pub struct Net {
    controller: WifiController<'static>,
    phy: Phy,
    iface: Interface,
    sockets: SocketSet<'static>,
    dhcp: smoltcp::iface::SocketHandle,
    tcp: smoltcp::iface::SocketHandle,
    pub ip: Option<Ipv4Address>,
    pub last_error: Option<&'static str>,
    /// Realm and nonce from the last challenge, reused until the server rejects
    /// them - re-probing on every request would double the round trips.
    realm: Buf<64>,
    nonce: Buf<64>,
    nc: u32,
    body: [u8; RX_BODY],
}

/// Small fixed string, since there is no allocator budget to spare here.
#[derive(Clone, Copy)]
pub struct Buf<const N: usize> {
    bytes: [u8; N],
    len: usize,
}

impl<const N: usize> Buf<N> {
    pub const fn new() -> Self {
        Self { bytes: [0; N], len: 0 }
    }
    pub fn clear(&mut self) {
        self.len = 0;
    }
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len]).unwrap_or("")
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    fn push_str(&mut self, s: &str) {
        for &b in s.as_bytes() {
            if self.len < N {
                self.bytes[self.len] = b;
                self.len += 1;
            }
        }
    }
}

impl<const N: usize> core::fmt::Write for Buf<N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.push_str(s);
        Ok(())
    }
}

impl Net {
    /// Brings up the station and joins the configured network. The caller must
    /// already have started the scheduler and the heap - see main.
    pub fn new(
        wifi: esp_hal::peripherals::WIFI<'static>,
        sockets_storage: &'static mut [SocketStorage<'static>],
        rx_buf: &'static mut [u8],
        tx_buf: &'static mut [u8],
        now_ms: u32,
    ) -> Result<Self, &'static str> {
        let mut controller = WifiController::new(wifi, Default::default())
            .map_err(|_| "wifi controller init failed")?;

        // Setting a station config is what starts the join; there is no separate
        // connect call in this release.
        // The password setter wants an owned String; the heap exists for the
        // radio's benefit anyway, and this allocation happens once at startup.
        let station_config = esp_radio::wifi::sta::StationConfig::default()
            .with_ssid(WIFI_SSID)
            .with_password(alloc::string::String::from(WIFI_PASS));
        let config = esp_radio::wifi::Config::Station(station_config);
        controller.set_config(&config).map_err(|_| "wifi config rejected")?;

        let mut station = WifiIface::station();
        let mac = station.mac_address();
        let mut phy = Phy { iface: station };

        let mut if_config = IfConfig::new(HardwareAddress::Ethernet(EthernetAddress(mac)));
        if_config.random_seed = now_ms as u64 ^ 0x5eed_1234;
        let iface = Interface::new(if_config, &mut phy, Instant::from_millis(now_ms as i64));

        let mut sockets = SocketSet::new(sockets_storage);
        let dhcp = sockets.add(dhcpv4::Socket::new());
        let tcp_socket = tcp::Socket::new(
            tcp::SocketBuffer::new(rx_buf),
            tcp::SocketBuffer::new(tx_buf),
        );
        let tcp = sockets.add(tcp_socket);

        Ok(Self {
            controller,
            phy,
            iface,
            sockets,
            dhcp,
            tcp,
            ip: None,
            last_error: None,
            realm: Buf::new(),
            nonce: Buf::new(),
            nc: 0,
            body: [0; RX_BODY],
        })
    }

    pub fn is_connected(&self) -> bool {
        self.controller.is_connected()
    }

    /// Pump the stack once, and fold any DHCP result into our address.
    pub fn step(&mut self, now_ms: u32) {
        let now = Instant::from_millis(now_ms as i64);
        self.iface.poll(now, &mut self.phy, &mut self.sockets);

        // DHCP: adopt the lease when it arrives, drop the address when it is
        // lost, so the UI's link indicator reflects reality.
        let event = self.sockets.get_mut::<dhcpv4::Socket>(self.dhcp).poll();
        match event {
            Some(dhcpv4::Event::Configured(cfg)) => {
                self.ip = Some(cfg.address.address());
                self.iface.update_ip_addrs(|addrs| {
                    addrs.clear();
                    let _ = addrs.push(IpCidr::Ipv4(cfg.address));
                });
                if let Some(router) = cfg.router {
                    let _ = self.iface.routes_mut().add_default_ipv4_route(router);
                }
            }
            Some(dhcpv4::Event::Deconfigured) => {
                self.ip = None;
                self.iface.update_ip_addrs(|addrs| addrs.clear());
                self.iface.routes_mut().remove_default_ipv4_route();
            }
            None => {}
        }
    }

    /// GET `path` from `host`, following one Digest challenge if offered.
    ///
    /// Synchronous, with its own smoltcp pump and a wall-clock deadline. Callers
    /// only invoke it when the UI is idle - see the module note.
    fn request(&mut self, method: &str, host: Ipv4Address, path: &str, now_ms: u32) -> Result<usize, &'static str> {
        // First attempt reuses a cached challenge if we have one; otherwise it
        // goes out bare and we expect a 401 carrying the parameters.
        for attempt in 0..2 {
            let len = self.request_once(method, host, path, now_ms, attempt == 1)?;
            let status = status_code(&self.body[..len]);
            if status == 401 && attempt == 0 {
                if !self.absorb_challenge(len) {
                    return Err("401 without a usable Digest challenge");
                }
                continue;
            }
            if status == 200 {
                return Ok(len);
            }
            if status == 401 {
                return Err("authentication rejected");
            }
            return Err("unexpected HTTP status");
        }
        Err("authentication did not converge")
    }

    fn request_once(
        &mut self,
        method: &str,
        host: Ipv4Address,
        path: &str,
        now_ms: u32,
        authorize: bool,
    ) -> Result<usize, &'static str> {
        const PORT: u16 = 80;
        const TIMEOUT_MS: u32 = 4_000;

        // Build the request before touching the socket, so a formatting problem
        // cannot leave a half-open connection behind.
        let mut head = Buf::<512>::new();
        let _ = write!(head, "GET {path} HTTP/1.1\r\nHost: ");
        let _ = write!(head, "{host}");
        let _ = write!(head, "\r\nConnection: close\r\n");
        if authorize {
            self.nc += 1;
            let mut auth = Buf::<320>::new();
            build_digest(&mut auth, method, path, self.realm.as_str(), self.nonce.as_str(), self.nc);
            let _ = write!(head, "Authorization: {}\r\n", auth.as_str());
        }
        let _ = write!(head, "\r\n");

        {
            let socket = self.sockets.get_mut::<tcp::Socket>(self.tcp);
            socket.abort();
        }
        self.step(now_ms);
        {
            let socket = self.sockets.get_mut::<tcp::Socket>(self.tcp);
            let local_port = 49_152 + (now_ms % 16_000) as u16;
            socket
                .connect(self.iface.context(), (IpAddress::Ipv4(host), PORT), local_port)
                .map_err(|_| "connect failed")?;
        }

        let mut sent = false;
        let mut got = 0usize;
        let started = now_ms;
        loop {
            let t = crate::now_ms();
            if t.wrapping_sub(started) > TIMEOUT_MS {
                let socket = self.sockets.get_mut::<tcp::Socket>(self.tcp);
                socket.abort();
                return Err("request timed out");
            }
            self.step(t);

            let socket = self.sockets.get_mut::<tcp::Socket>(self.tcp);
            if !sent && socket.may_send() {
                socket.send_slice(head.as_str().as_bytes()).map_err(|_| "send failed")?;
                sent = true;
            }
            if socket.can_recv() {
                let body = &mut self.body;
                let read = socket
                    .recv(|data| {
                        let take = data.len().min(body.len() - got);
                        body[got..got + take].copy_from_slice(&data[..take]);
                        (take, take)
                    })
                    .map_err(|_| "receive failed")?;
                got += read;
            }
            if sent && !socket.is_active() {
                break;
            }
            if got >= RX_BODY {
                break;
            }
        }
        Ok(got)
    }

    /// Extract realm and nonce from a 401's WWW-Authenticate header.
    fn absorb_challenge(&mut self, len: usize) -> bool {
        let text = core::str::from_utf8(&self.body[..len]).unwrap_or("");
        let Some(line) = text
            .lines()
            .find(|l| l.to_ascii_lowercase().starts_with("www-authenticate:"))
        else {
            return false;
        };
        self.realm.clear();
        self.nonce.clear();
        if let Some(v) = quoted_param(line, "realm") {
            self.realm.push_str(v);
        }
        if let Some(v) = quoted_param(line, "nonce") {
            self.nonce.push_str(v);
        }
        self.nc = 0;
        !self.realm.is_empty() && !self.nonce.is_empty()
    }

    /// Fetch `/api/dump` from every configured controller and merge into `state`.
    pub fn poll_dump(&mut self, state: &mut State, now_ms: u32) {
        for host in HOSTS {
            match self.request("GET", host.ip, "/local/rainbird/app/api/dump", now_ms) {
                Ok(len) => {
                    let split = find_body(&self.body[..len]);
                    if let Ok(text) = core::str::from_utf8(&self.body[split..len]) {
                        parse_dump(text, state);
                        self.last_error = None;
                    } else {
                        self.last_error = Some("dump was not valid UTF-8");
                    }
                }
                Err(e) => self.last_error = Some(e),
            }
        }
    }

    /// Trigger a relay. Path is built into a fixed buffer; no allocation.
    pub fn trigger(&mut self, relay: u8, seconds: u32, now_ms: u32) {
        let mut path = Buf::<96>::new();
        let _ = write!(
            path,
            "/local/rainbird/app/api/trigger?relay={relay}&seconds={seconds}"
        );
        // One controller for now; when several are configured the relay id will
        // carry which one it belongs to.
        if let Some(host) = HOSTS.first() {
            if let Err(e) = self.request("GET", host.ip, path.as_str(), now_ms) {
                self.last_error = Some(e);
            }
        }
    }
}

fn status_code(response: &[u8]) -> u16 {
    // "HTTP/1.1 200 OK"
    let text = core::str::from_utf8(response).unwrap_or("");
    let Some(first) = text.lines().next() else { return 0 };
    let mut parts = first.split(' ');
    let _ = parts.next();
    parts.next().and_then(|s| s.parse().ok()).unwrap_or(0)
}

/// Byte offset of the body, just past the blank line ending the headers.
fn find_body(response: &[u8]) -> usize {
    response
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|p| p + 4)
        .unwrap_or(0)
}

fn quoted_param<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let start = line.find(key)?;
    let rest = &line[start + key.len()..];
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('=')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(&rest[..end])
}

fn hex(bytes: &[u8], out: &mut Buf<32>) {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    for &b in bytes {
        let pair = [DIGITS[(b >> 4) as usize], DIGITS[(b & 15) as usize]];
        out.push_str(core::str::from_utf8(&pair).unwrap_or(""));
    }
}

fn md5_hex(parts: &[&str], out: &mut Buf<32>) {
    use md5::{Digest, Md5};
    let mut hasher = Md5::new();
    for (index, part) in parts.iter().enumerate() {
        if index > 0 {
            hasher.update(b":");
        }
        hasher.update(part.as_bytes());
    }
    let digest = hasher.finalize();
    hex(&digest, out);
}

/// RFC 2617 MD5 digest, qop=auth.
fn build_digest(out: &mut Buf<320>, method: &str, uri: &str, realm: &str, nonce: &str, nc: u32) {
    let mut ha1 = Buf::<32>::new();
    md5_hex(&[RB_USER, realm, RB_PASS], &mut ha1);
    let mut ha2 = Buf::<32>::new();
    md5_hex(&[method, uri], &mut ha2);

    // A fixed client nonce is acceptable here because nc increments per request,
    // which is what actually makes each response unique.
    let cnonce = "0a4f113b";
    let mut nc_buf = Buf::<16>::new();
    let _ = write!(nc_buf, "{nc:08x}");

    let mut response = Buf::<32>::new();
    md5_hex(
        &[ha1.as_str(), nonce, nc_buf.as_str(), cnonce, "auth", ha2.as_str()],
        &mut response,
    );

    let _ = write!(
        out,
        "Digest username=\"{}\", realm=\"{}\", nonce=\"{}\", uri=\"{}\", \
         qop=auth, nc={}, cnonce=\"{}\", response=\"{}\"",
        RB_USER,
        realm,
        nonce,
        uri,
        nc_buf.as_str(),
        cnonce,
        response.as_str()
    );
}

impl Net {
    /// POST /api/stop. The controller cancels the whole run, queue included.
    pub fn stop(&mut self, now_ms: u32) {
        if let Some(host) = HOSTS.first() {
            // The endpoint is a POST with no body; a GET would be rejected, so
            // this reuses the digest machinery with the method overridden.
            if let Err(e) = self.request("POST", host.ip, "/local/rainbird/app/api/stop", now_ms) {
                self.last_error = Some(e);
            }
        }
    }
}
