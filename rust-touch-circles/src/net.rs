//! Wi-Fi, TCP and the HTTP client that talks to the irrigation controllers.
//!
//! Three things shape this module.
//!
//! **esp-radio needs a scheduler.** The Wi-Fi blobs require `esp-rtos`, so the
//! firmware is no longer strictly RTOS-free. The render loop still owns the main
//! thread; the scheduler exists for the radio's own tasks.
//!
//! **Requests are cooperative.** TCP connect, send, receive, Digest retry, and
//! multi-controller polling advance one step per UI-loop iteration. No network
//! timeout owns the main thread, so touch and animation remain responsive even
//! while a controller is offline.
//!
//! **The API is behind HTTP Digest.** The Axis device's own web server
//! authenticates every request to the ACAP, and Digest is the only scheme
//! offered - so this implements RFC 2617 MD5 challenge-response: an
//! unauthenticated probe, then a retry carrying the computed response.

extern crate alloc;

use core::fmt::Write as _;
use core::task::Poll;

use esp_radio::wifi::{Interface as WifiIface, WifiController, WifiRxToken, WifiTxToken};
use smoltcp::iface::{Config as IfConfig, Interface, SocketSet, SocketStorage};
use smoltcp::phy::{Checksum, Device, DeviceCapabilities, Medium, RxToken, TxToken};
use smoltcp::socket::{dhcpv4, tcp};
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, HardwareAddress, IpAddress, IpCidr, Ipv4Address};

use crate::model::{MAX_CONTROLLERS, MAX_ENTRIES, State, parse_dump};

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
    type RxToken<'a>
        = RxTok
    where
        Self: 'a;
    type TxToken<'a>
        = TxTok
    where
        Self: 'a;

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
///
/// Credentials and the Digest challenge live here rather than on `Net` because
/// controllers are configured from the panel now and need not share a login.
/// A shared challenge cache was also self-defeating with more than one host: A's
/// nonce is rejected by B, whose replacement is then rejected by A, so every
/// request paid for a re-challenge. Per host, each keeps its own valid nonce.
#[derive(Clone, Copy)]
pub struct Host {
    pub ip: Ipv4Address,
    user: crate::store::FixedStr<{ crate::store::MAX_USER }>,
    pass: crate::store::FixedStr<{ crate::store::MAX_SECRET }>,
    /// Realm and nonce from this host's last challenge, reused until it rejects
    /// them - re-probing on every request would double the round trips.
    realm: Buf<64>,
    nonce: Buf<64>,
    nc: u32,
}

const EMPTY_HOST: Host = Host {
    ip: Ipv4Address::UNSPECIFIED,
    user: crate::store::FixedStr::EMPTY,
    pass: crate::store::FixedStr::EMPTY,
    realm: Buf::new(),
    nonce: Buf::new(),
    nc: 0,
};

/// One access point seen by a scan. Fixed capacity: the picker shows a page of
/// networks, not an inventory, and the alternative is an allocation per scan.
#[derive(Clone, Copy)]
pub struct Network {
    pub ssid: crate::store::FixedStr<{ crate::store::MAX_SSID }>,
    pub rssi: i8,
    pub secure: bool,
}

pub const MAX_NETWORKS: usize = 14;

pub struct Networks {
    pub items: [Network; MAX_NETWORKS],
    pub n: usize,
    /// Whether a scan has completed at least once, so the picker can tell
    /// "nothing found" apart from "not looked yet".
    pub scanned: bool,
}

impl Networks {
    pub const EMPTY: Self = Self {
        items: [Network {
            ssid: crate::store::FixedStr::EMPTY,
            rssi: -128,
            secure: true,
        }; MAX_NETWORKS],
        n: 0,
        scanned: false,
    };
}

/// Set the station config and start the join.
///
/// Setting a station config is what starts the join; there is no separate connect
/// call in this release. `connect_async` performs the actual connect request on
/// its first poll and then only waits for the radio event, so it is polled once
/// and dropped: association continues in esp-radio's scheduler while the UI runs,
/// and progress is observed through `is_connected()`.
fn join(
    controller: &mut WifiController<'static>,
    settings: &crate::store::Settings,
) -> Result<(), &'static str> {
    if settings.ssid.is_empty() {
        return Err("no wi-fi network configured");
    }
    // The password setter wants an owned String; the heap exists for the radio's
    // benefit anyway, and this runs only when the network changes.
    let station_config = esp_radio::wifi::sta::StationConfig::default()
        .with_ssid(settings.ssid.as_str())
        .with_password(alloc::string::String::from(settings.psk.as_str()));
    let config = esp_radio::wifi::Config::Station(station_config);
    controller
        .set_config(&config)
        .map_err(|_| "wifi config rejected")?;
    if let Poll::Ready(Err(_)) = embassy_futures::poll_once(controller.connect_async()) {
        return Err("wifi association failed");
    }
    Ok(())
}

/// Drive a future to completion on this thread, giving up after `timeout_ms`.
///
/// Deliberately not `embassy_futures::block_on`, which spins forever: a radio
/// event that never arrives would take the whole panel with it. The waker is a
/// no-op because there is nothing to wake - this *is* the executor - and the
/// radio's own tasks are run by esp-rtos on other threads regardless.
fn block_on_deadline<F: core::future::Future>(future: F, timeout_ms: u32) -> Option<F::Output> {
    let mut future = core::pin::pin!(future);
    let mut cx = core::task::Context::from_waker(core::task::Waker::noop());
    let deadline =
        esp_hal::time::Instant::now() + esp_hal::time::Duration::from_millis(timeout_ms as u64);
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut cx) {
            return Some(value);
        }
        if esp_hal::time::Instant::now() > deadline {
            return None;
        }
    }
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
    body: [u8; RX_BODY],
    hosts: [Host; MAX_CONTROLLERS],
    n_hosts: usize,
    snapshots: [State; MAX_CONTROLLERS],
    snapshot_valid: [bool; MAX_CONTROLLERS],
    http: Option<Http>,
    job: Job,
    last_connect_attempt_ms: u32,
    /// When the UI last took charge of connecting. While that is recent, `step`
    /// keeps its hands off - see MANUAL_HOLD_MS.
    manual_from_ms: u32,
}

/// How long a user-driven join keeps the automatic reconnect out of the way.
///
/// Two connects must never be in flight at once: the second returns
/// ESP_ERR_WIFI_CONN, which esp-radio panics on rather than reporting. The UI
/// serialises its own attempts (its retry only appears once an attempt has been
/// given up on), so suppressing the automatic one for longer than the UI's own
/// timeout removes the last way the two could overlap.
const MANUAL_HOLD_MS: u32 = 25_000;

#[derive(Clone, Copy)]
enum HttpKind {
    Dump(usize),
    Trigger,
    Stop,
    /// Run a schedule on demand, or write one back. Neither returns anything the
    /// model needs: the next poll is authoritative either way, and treating the
    /// reply as authoritative would mean two sources of truth for the schedule.
    Schedule,
}

#[derive(Clone, Copy)]
struct Http {
    kind: HttpKind,
    host: usize,
    post: bool,
    path: Buf<96>,
    head: Buf<512>,
    started_ms: u32,
    sent: bool,
    got: usize,
    attempt: u8,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Job {
    Idle,
    Poll { next: usize },
    StopAll { next: usize },
}

/// Small fixed string, since there is no allocator budget to spare here.
#[derive(Clone, Copy)]
pub struct Buf<const N: usize> {
    bytes: [u8; N],
    len: usize,
}

impl<const N: usize> Buf<N> {
    pub const fn new() -> Self {
        Self {
            bytes: [0; N],
            len: 0,
        }
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
        settings: &crate::store::Settings,
        now_ms: u32,
    ) -> Result<Self, &'static str> {
        let mut controller = WifiController::new(wifi, Default::default())
            .map_err(|_| "wifi controller init failed")?;

        // Credentials come from the persisted settings, not straight from the
        // baked-in constants - those are only the first-boot default, and the
        // panel can be pointed at a different network from its own UI.
        join(&mut controller, settings)?;

        let station = WifiIface::station();
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

        let mut net = Self {
            controller,
            phy,
            iface,
            sockets,
            dhcp,
            tcp,
            ip: None,
            last_error: None,
            body: [0; RX_BODY],
            hosts: [EMPTY_HOST; MAX_CONTROLLERS],
            n_hosts: 0,
            snapshots: [const { State::new() }; MAX_CONTROLLERS],
            snapshot_valid: [false; MAX_CONTROLLERS],
            http: None,
            job: Job::Idle,
            last_connect_attempt_ms: now_ms,
            manual_from_ms: now_ms.wrapping_sub(MANUAL_HOLD_MS),
        };
        net.apply_hosts(settings);
        Ok(net)
    }

    /// Adopt the controller list from settings.
    ///
    /// Snapshots are dropped rather than reindexed: after an edit, slot 2 is not
    /// necessarily the controller it was, and presenting one host's relays under
    /// another's name would be worse than briefly showing none. The next poll
    /// refills them.
    pub fn apply_hosts(&mut self, settings: &crate::store::Settings) {
        self.hosts = [EMPTY_HOST; MAX_CONTROLLERS];
        self.n_hosts = 0;
        for controller in settings.controllers[..settings.n_controllers]
            .iter()
            .filter(|c| c.is_set())
        {
            if self.n_hosts >= MAX_CONTROLLERS {
                break;
            }
            let ip = controller.ip;
            self.hosts[self.n_hosts] = Host {
                ip: Ipv4Address::new(ip[0], ip[1], ip[2], ip[3]),
                user: controller.user,
                pass: controller.pass,
                ..EMPTY_HOST
            };
            self.n_hosts += 1;
        }
        self.snapshot_valid = [false; MAX_CONTROLLERS];
        // Any request in flight belongs to the old table's indices.
        self.sockets.get_mut::<tcp::Socket>(self.tcp).abort();
        self.http = None;
        self.job = Job::Idle;
    }

    /// Re-join with the credentials now in settings. Used when the network is
    /// changed from the panel; the reconnect logic in `step` takes it from here.
    pub fn apply_wifi(
        &mut self,
        settings: &crate::store::Settings,
        now_ms: u32,
    ) -> Result<(), &'static str> {
        self.sockets.get_mut::<tcp::Socket>(self.tcp).abort();
        self.http = None;
        self.job = Job::Idle;
        self.ip = None;
        self.iface.update_ip_addrs(|addrs| addrs.clear());
        self.iface.routes_mut().remove_default_ipv4_route();
        // A DHCP socket that already holds a lease will not ask for a new one on
        // a different network, so it is reset along with the address.
        self.sockets.get_mut::<dhcpv4::Socket>(self.dhcp).reset();
        // Leave the old network *completely* before joining the new one. This one
        // is driven to completion rather than fired and forgotten: connecting
        // while a disconnect is still in flight leaves the driver's station
        // control block invalid, and the next connect then returns
        // ESP_ERR_WIFI_CONN - which esp-radio turns into a panic rather than an
        // error, because its error table does not map that code. Bounded, and
        // only reached when actually associated, so the pause is short and the UI
        // is already showing its connecting screen.
        if self.controller.is_connected() {
            let _ = block_on_deadline(self.controller.disconnect_async(), 1_000);
        }
        // Claim the connect slot: `step` must not fire a second connect on top of
        // the one below. Not doing this was exactly the panic described above.
        self.last_connect_attempt_ms = now_ms;
        self.manual_from_ms = now_ms;
        join(&mut self.controller, settings)
    }


    /// Scan for access points, blocking until the radio reports the scan done.
    ///
    /// This is the one deliberately blocking network call in the firmware. A scan
    /// takes a few hundred milliseconds and its result is the entire content of
    /// the screen that asked for it, so there is nothing to animate meanwhile;
    /// spreading it over UI iterations would mean holding a self-referential
    /// future across frames for no visible gain. The caller paints its "scanning"
    /// state first, and the deadline guarantees the loop resumes regardless.
    pub fn scan(&mut self, out: &mut Networks) {
        use esp_radio::wifi::scan::ScanConfig;

        let config = ScanConfig::default().with_max(MAX_NETWORKS);
        let found = match block_on_deadline(self.controller.scan_async(&config), 4_000) {
            Some(Ok(found)) => found,
            Some(Err(_)) => {
                self.last_error = Some("wifi scan failed");
                return;
            }
            None => {
                self.last_error = Some("wifi scan timed out");
                return;
            }
        };

        out.n = 0;
        for ap in found.iter() {
            let ssid = ap.ssid.as_str();
            if ssid.is_empty() {
                continue;
            }
            // The same network is seen once per band and per repeater. Keep the
            // strongest sighting of each name rather than a list of duplicates.
            if let Some(existing) = out.items[..out.n]
                .iter_mut()
                .find(|n| n.ssid.as_str() == ssid)
            {
                existing.rssi = existing.rssi.max(ap.signal_strength);
                continue;
            }
            if out.n >= MAX_NETWORKS {
                continue;
            }
            out.items[out.n] = Network {
                ssid: crate::store::FixedStr::new(ssid),
                rssi: ap.signal_strength,
                secure: !matches!(
                    ap.auth_method,
                    None | Some(esp_radio::wifi::AuthenticationMethod::None)
                ),
            };
            out.n += 1;
        }
        // Strongest first: the network you are standing next to is the one you
        // almost certainly mean.
        out.items[..out.n].sort_unstable_by(|a, b| b.rssi.cmp(&a.rssi));
        out.scanned = true;
    }

    pub fn is_connected(&self) -> bool {
        self.controller.is_connected()
    }

    /// Pump the stack once, and fold any DHCP result into our address.
    pub fn step(&mut self, now_ms: u32) {
        // A lost AP must never strand the panel or require a reboot. Retrying is
        // also one-shot/non-blocking for the same reason as initial association.
        //
        // The interval has to outlast a whole association attempt, not merely be
        // "often enough". Issuing a connect while one is already in flight makes
        // the driver return ESP_ERR_WIFI_CONN, and esp-radio panics on that code
        // instead of reporting it - so a retry that overlaps an attempt takes the
        // panel down. A wrong password takes about ten seconds to be rejected;
        // fifteen clears it with room to spare.
        const RECONNECT_MS: u32 = 15_000;
        if !self.controller.is_connected()
            && now_ms.wrapping_sub(self.last_connect_attempt_ms) >= RECONNECT_MS
            && now_ms.wrapping_sub(self.manual_from_ms) >= MANUAL_HOLD_MS
        {
            self.last_connect_attempt_ms = now_ms;
            if let Poll::Ready(Err(_)) = embassy_futures::poll_once(self.controller.connect_async())
            {
                self.last_error = Some("wifi association failed");
            }
        }
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

    /// Advance at most one small piece of HTTP work. This function never waits:
    /// TCP connect, send, receive, Digest retry, and multi-host traversal are
    /// spread across ordinary UI-loop iterations.
    pub fn service(&mut self, state: &mut State, now_ms: u32) -> bool {
        if self.http.is_none() {
            match self.job {
                Job::Poll { next } if next < self.n_hosts => {
                    let mut path = Buf::new();
                    path.push_str("/local/rainbird/app/api/dump");
                    if let Err(e) =
                        self.start_http(HttpKind::Dump(next), next, false, path, 0, now_ms)
                    {
                        self.last_error = Some(e);
                        state.controller_online[next] = false;
                        self.job = Job::Poll { next: next + 1 };
                    }
                    return false;
                }
                Job::Poll { .. } => {
                    self.job = Job::Idle;
                    self.rebuild_state(state);
                    return true;
                }
                Job::StopAll { next } if next < self.n_hosts => {
                    let mut path = Buf::new();
                    path.push_str("/local/rainbird/app/api/stop");
                    if let Err(e) = self.start_http(HttpKind::Stop, next, true, path, 0, now_ms) {
                        self.last_error = Some(e);
                        self.job = Job::StopAll { next: next + 1 };
                    }
                    return false;
                }
                Job::StopAll { .. } => {
                    self.job = Job::Idle;
                    return false;
                }
                Job::Idle => return false,
            }
        }

        let mut http = self.http.take().unwrap();
        if now_ms.wrapping_sub(http.started_ms) > 1_000 {
            self.sockets.get_mut::<tcp::Socket>(self.tcp).abort();
            self.finish_error(http, state, "request timed out");
            return true;
        }

        let (complete, closed) = {
            let socket = self.sockets.get_mut::<tcp::Socket>(self.tcp);
            if !http.sent && socket.may_send() {
                match socket.send_slice(http.head.as_str().as_bytes()) {
                    Ok(_) => http.sent = true,
                    Err(_) => {
                        socket.abort();
                        self.finish_error(http, state, "send failed");
                        return true;
                    }
                }
            }
            if socket.can_recv() && http.got < RX_BODY {
                let body = &mut self.body;
                let got = http.got;
                if let Ok(read) = socket.recv(|data| {
                    let take = data.len().min(body.len() - got);
                    body[got..got + take].copy_from_slice(&data[..take]);
                    (take, take)
                }) {
                    http.got += read;
                }
            }
            let complete = response_complete(&self.body[..http.got]);
            let closed = http.sent && !socket.is_active();
            if complete {
                socket.abort();
            }
            (complete, closed)
        };

        if !complete && !closed {
            self.http = Some(http);
            return false;
        }

        let status = status_code(&self.body[..http.got]);
        if status == 401 && http.attempt == 0 && self.absorb_challenge(http.host, http.got) {
            if let Err(e) = self.start_http(http.kind, http.host, http.post, http.path, 1, now_ms) {
                self.finish_error(http, state, e);
            }
            return false;
        }
        if status != 200 {
            self.finish_error(
                http,
                state,
                if status == 401 {
                    "authentication rejected"
                } else {
                    "unexpected HTTP status"
                },
            );
            return true;
        }
        self.finish_success(http, state);
        // A successful dump is published atomically when the complete
        // multi-controller poll is rebuilt. Trigger/stop already update the
        // model optimistically, so neither needs an otherwise wasted redraw.
        false
    }

    fn start_http(
        &mut self,
        kind: HttpKind,
        host: usize,
        post: bool,
        path: Buf<96>,
        attempt: u8,
        now_ms: u32,
    ) -> Result<(), &'static str> {
        let method = if post { "POST" } else { "GET" };
        let mut head = Buf::<512>::new();
        let _ = write!(
            head,
            "{method} {} HTTP/1.0\r\nHost: {}\r\nConnection: close\r\n",
            path.as_str(),
            self.hosts[host].ip
        );
        if post {
            let _ = write!(head, "Content-Length: 0\r\n");
        }
        if !self.hosts[host].realm.is_empty() || attempt > 0 {
            self.hosts[host].nc = self.hosts[host].nc.wrapping_add(1);
            let mut auth = Buf::<320>::new();
            build_digest(
                &mut auth,
                method,
                path.as_str(),
                &self.hosts[host],
            );
            let _ = write!(head, "Authorization: {}\r\n", auth.as_str());
        }
        let _ = write!(head, "\r\n");
        let nc = self.hosts[host].nc;
        let socket = self.sockets.get_mut::<tcp::Socket>(self.tcp);
        socket.abort();
        let local_port = 49_152 + (now_ms.wrapping_add(nc) % 16_000) as u16;
        socket
            .connect(
                self.iface.context(),
                (IpAddress::Ipv4(self.hosts[host].ip), 80),
                local_port,
            )
            .map_err(|_| "connect failed")?;
        self.http = Some(Http {
            kind,
            host,
            post,
            path,
            head,
            started_ms: now_ms,
            sent: false,
            got: 0,
            attempt,
        });
        Ok(())
    }

    fn finish_success(&mut self, http: Http, state: &mut State) {
        self.last_error = None;
        match http.kind {
            HttpKind::Dump(index) => {
                let split = find_body(&self.body[..http.got]);
                if let Ok(text) = core::str::from_utf8(&self.body[split..http.got]) {
                    let mut remote = State::new();
                    parse_dump(text, &mut remote);
                    self.snapshots[index] = remote;
                    self.snapshot_valid[index] = true;
                    state.controller_online[index] = true;
                } else {
                    state.controller_online[index] = false;
                    self.last_error = Some("dump was not valid UTF-8");
                }
                self.job = Job::Poll { next: index + 1 };
            }
            HttpKind::Stop => {
                if let Job::StopAll { next } = self.job {
                    self.job = Job::StopAll { next: next + 1 };
                }
            }
            // Neither carries anything the model needs on success: the next poll
            // is what makes the controller's decision visible.
            HttpKind::Trigger | HttpKind::Schedule => {}
        }
    }

    fn finish_error(&mut self, http: Http, state: &mut State, error: &'static str) {
        self.last_error = Some(error);
        match http.kind {
            HttpKind::Dump(index) => {
                state.controller_online[index] = false;
                self.job = Job::Poll { next: index + 1 };
            }
            HttpKind::Stop => {
                if let Job::StopAll { next } = self.job {
                    self.job = Job::StopAll { next: next + 1 };
                }
            }
            // Nothing to unwind for either: the request carried no model state,
            // and the failure is already recorded for the status line.
            HttpKind::Trigger | HttpKind::Schedule => {}
        }
    }

    fn rebuild_state(&self, state: &mut State) {
        let old_clock = (
            state.hh,
            state.mm,
            state.ss,
            state.clock_frac_ms,
            state.clock_valid,
        );
        state.local_ip = self.ip.map(|ip| ip.octets());
        state.n_controllers = self.n_hosts;
        // Keep the last good model while a poll is failing - but not when there
        // is nothing configured at all. With no hosts there is nothing to wait
        // for, and holding on would keep offering the zones of a controller the
        // user has just removed. Falling through clears the model instead.
        if self.n_hosts > 0
            && !self.snapshot_valid[..self.n_hosts]
                .iter()
                .any(|valid| *valid)
        {
            return;
        }
        state.n_relays = 0;
        state.n_starts = 0;
        state.n_analogs = 0;
        state.running = false;
        state.active = 0;
        state.queued = 0;
        state.max_run_s = u32::MAX;
        state.clock_valid = false;
        for index in 0..self.n_hosts {
            state.controller_ips[index] = self.hosts[index].ip.octets();
            if self.snapshot_valid[index] {
                let online = state.controller_online[index];
                merge_controller(state, &self.snapshots[index], index as u8, online);
            }
        }
        if !state.clock_valid {
            (
                state.hh,
                state.mm,
                state.ss,
                state.clock_frac_ms,
                state.clock_valid,
            ) = old_clock;
        }
    }

    /// Extract realm and nonce from a 401's WWW-Authenticate header, for `host`.
    fn absorb_challenge(&mut self, host: usize, len: usize) -> bool {
        let text = core::str::from_utf8(&self.body[..len]).unwrap_or("");
        let Some(line) = text
            .lines()
            .find(|l| l.to_ascii_lowercase().starts_with("www-authenticate:"))
        else {
            return false;
        };
        let entry = &mut self.hosts[host];
        entry.realm.clear();
        entry.nonce.clear();
        if let Some(v) = quoted_param(line, "realm") {
            entry.realm.push_str(v);
        }
        if let Some(v) = quoted_param(line, "nonce") {
            entry.nonce.push_str(v);
        }
        entry.nc = 0;
        !entry.realm.is_empty() && !entry.nonce.is_empty()
    }

    /// Fetch `/api/dump` from every configured controller and merge into `state`.
    pub fn poll_dump(&mut self, state: &mut State, now_ms: u32) {
        state.local_ip = self.ip.map(|ip| ip.octets());
        state.n_controllers = self.n_hosts;
        for index in 0..self.n_hosts {
            state.controller_ips[index] = self.hosts[index].ip.octets();
        }
        if self.job == Job::Idle && self.http.is_none() {
            self.job = Job::Poll { next: 0 };
        }
        let _ = now_ms;
    }

    /// Trigger a relay. Path is built into a fixed buffer; no allocation.
    pub fn trigger(&mut self, controller: u8, relay: u8, seconds: u32, now_ms: u32) {
        let mut path = Buf::<96>::new();
        let _ = write!(
            path,
            "/local/rainbird/app/api/trigger?relay={relay}&seconds={seconds}"
        );
        // One controller for now; when several are configured the relay id will
        // carry which one it belongs to.
        if controller as usize >= self.n_hosts {
            return;
        }
        self.sockets.get_mut::<tcp::Socket>(self.tcp).abort();
        self.http = None;
        self.job = Job::Idle;
        if let Err(e) = self.start_http(
            HttpKind::Trigger,
            controller as usize,
            false,
            path,
            0,
            now_ms,
        ) {
            self.last_error = Some(e);
        }
    }

    /// Run one schedule now. `start` is the id on that controller, not the
    /// merged one - see StartTime::remote_id.
    ///
    /// The controller supersedes whatever is running, so this needs no stop
    /// first; asking it to decide is also what keeps the single-relay rule its
    /// job rather than a race between the panel and the clock.
    pub fn run_schedule(&mut self, controller: u8, start: u8, now_ms: u32) {
        if controller as usize >= self.n_hosts {
            return;
        }
        let mut path = Buf::<96>::new();
        let _ = write!(path, "/local/rainbird/app/api/run-schedule?start={start}");
        self.sockets.get_mut::<tcp::Socket>(self.tcp).abort();
        self.http = None;
        self.job = Job::Idle;
        if let Err(e) = self.start_http(
            HttpKind::Schedule,
            controller as usize,
            false,
            path,
            0,
            now_ms,
        ) {
            self.last_error = Some(e);
        }
    }
}

fn status_code(response: &[u8]) -> u16 {
    // "HTTP/1.1 200 OK"
    let text = core::str::from_utf8(response).unwrap_or("");
    let Some(first) = text.lines().next() else {
        return 0;
    };
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

fn response_complete(response: &[u8]) -> bool {
    let body = find_body(response);
    if body == 0 {
        return false;
    }
    let Ok(headers) = core::str::from_utf8(&response[..body]) else {
        return false;
    };
    let length = headers.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse::<usize>().ok())
            .flatten()
    });
    length.is_some_and(|length| response.len() >= body + length)
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

/// RFC 2617 MD5 digest, qop=auth, using `host`'s own credentials and challenge.
fn build_digest(out: &mut Buf<320>, method: &str, uri: &str, host: &Host) {
    let (realm, nonce, nc) = (host.realm.as_str(), host.nonce.as_str(), host.nc);
    let user = host.user.as_str();
    let mut ha1 = Buf::<32>::new();
    md5_hex(&[user, realm, host.pass.as_str()], &mut ha1);
    let mut ha2 = Buf::<32>::new();
    md5_hex(&[method, uri], &mut ha2);

    // A fixed client nonce is acceptable here because nc increments per request,
    // which is what actually makes each response unique.
    let cnonce = "0a4f113b";
    let mut nc_buf = Buf::<16>::new();
    let _ = write!(nc_buf, "{nc:08x}");

    let mut response = Buf::<32>::new();
    md5_hex(
        &[
            ha1.as_str(),
            nonce,
            nc_buf.as_str(),
            cnonce,
            "auth",
            ha2.as_str(),
        ],
        &mut response,
    );

    let _ = write!(
        out,
        "Digest username=\"{}\", realm=\"{}\", nonce=\"{}\", uri=\"{}\", \
         qop=auth, nc={}, cnonce=\"{}\", response=\"{}\"",
        user,
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
        self.sockets.get_mut::<tcp::Socket>(self.tcp).abort();
        self.http = None;
        self.job = Job::StopAll { next: 0 };
        let _ = now_ms;
    }
}

/// Merge one controller into a flat model while retaining private routing ids.
fn merge_controller(state: &mut State, remote: &State, controller: u8, online: bool) {
    let relay_base = state.n_relays;
    for source in remote.relays[..remote.n_relays].iter() {
        if state.n_relays >= state.relays.len() {
            break;
        }
        let mut relay = *source;
        relay.remote_id = source.id;
        relay.controller = controller;
        relay.id = (state.n_relays + 1) as u8;
        // Keep names/config from the last good snapshot, but never present a
        // stale energized state for a controller that is currently offline.
        relay.on = online && source.on;
        state.relays[state.n_relays] = relay;
        state.n_relays += 1;
    }

    let global_id = |remote_id: u8| -> u8 {
        remote.relays[..remote.n_relays]
            .iter()
            .position(|r| r.id == remote_id)
            .map(|i| (relay_base + i + 1) as u8)
            .unwrap_or(0)
    };
    for source in remote.starts[..remote.n_starts].iter() {
        if state.n_starts >= state.starts.len() {
            break;
        }
        let mut start = *source;
        for entry in start.entries[..start.n_entries.min(MAX_ENTRIES)].iter_mut() {
            entry.relay = global_id(entry.relay);
        }
        // Keep the controller's own id and which controller it came from: every
        // controller numbers its schedules 1..3, so the merged id cannot be sent
        // back to one of them. Same reason relays keep `remote_id`.
        start.remote_id = source.id;
        start.controller = controller;
        start.id = (state.n_starts + 1) as u8;
        // The controller names the schedule it is running; a schedule on an
        // offline controller is never shown as running, since that state is as
        // stale as the rest of its snapshot.
        start.running = online && remote.active_start != 0 && remote.active_start == source.id;
        state.starts[state.n_starts] = start;
        state.n_starts += 1;
    }
    for analog in remote.analogs[..remote.n_analogs].iter() {
        if state.n_analogs >= state.analogs.len() {
            break;
        }
        state.analogs[state.n_analogs] = *analog;
        state.n_analogs += 1;
    }
    if online && remote.clock_valid && !state.clock_valid {
        state.hh = remote.hh;
        state.mm = remote.mm;
        state.ss = remote.ss;
        state.clock_frac_ms = 0;
        state.clock_valid = true;
    }
    if online && remote.running {
        state.running = true;
        state.active = global_id(remote.active as u8) as i32;
        state.left_s = remote.left_s;
    }
    if online {
        state.queued = state.queued.saturating_add(remote.queued);
    }
    state.max_run_s = state.max_run_s.min(remote.max_run_s);
    if online && remote.err.len > 0 {
        state.err = remote.err;
    }
}
