//! Plain HTTP GETs to hosts out on the internet, by name.
//!
//! The controller client next door cannot do this job: it talks to a fixed table
//! of numeric addresses behind Digest auth, and one request at a time. The apps
//! want a named host, no auth, and to be fetching in the background while a poll
//! of the controllers is in flight - so this has its own TCP socket and its own
//! buffer, and the two never wait on each other.
//!
//! Names are resolved through smoltcp's DNS socket against 1.1.1.1 rather than
//! whatever DHCP handed out: the panel lives on an irrigation network whose
//! router is not obliged to resolve anything beyond it.
//!
//! No TLS. That rules out every forecast API that redirects to https - met.no
//! among them - which is why the sources chosen are the ones still served over
//! port 80.

use core::fmt::Write as _;

use smoltcp::iface::{Interface, SocketHandle, SocketSet};
use smoltcp::socket::{dns, tcp};
use smoltcp::wire::{DnsQueryType, IpAddress, Ipv4Address};

use crate::net::Buf;

/// Enough for the largest reply an app asks for - the currency table, at about
/// 3 KiB - plus its headers.
const BODY: usize = 6144;
const TIMEOUT_MS: u32 = 12_000;

/// Public resolvers, tried in this order. The list is longer than one on purpose:
/// smoltcp caps it at DNS_MAX_SERVER_COUNT, which defaults to *one* and silently
/// truncates the rest - hence the dns-max-server-count-4 feature.
const PUBLIC_DNS: [IpAddress; 2] = [
    IpAddress::Ipv4(Ipv4Address::new(1, 1, 1, 1)),
    IpAddress::Ipv4(Ipv4Address::new(8, 8, 8, 8)),
];
/// Room for both of those plus what DHCP offers.
const MAX_DNS: usize = 4;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Resolving,
    Talking,
    /// A reply is sitting in the buffer, waiting to be taken.
    Ready,
    Failed,
}

pub struct Fetch {
    dns: SocketHandle,
    tcp: SocketHandle,
    phase: Phase,
    query: Option<dns::QueryHandle>,
    head: Buf<256>,
    body: [u8; BODY],
    got: usize,
    sent: bool,
    started_ms: u32,
    /// Bumped per request so consecutive connections do not reuse a port that
    /// the other end still has in TIME_WAIT.
    seq: u32,
}

impl Fetch {
    pub fn new(
        sockets: &mut SocketSet<'static>,
        rx_buf: &'static mut [u8],
        tx_buf: &'static mut [u8],
        queries: &'static mut [Option<dns::DnsQuery>],
    ) -> Self {
        let dns = sockets.add(dns::Socket::new(&PUBLIC_DNS, queries));
        let tcp = sockets.add(tcp::Socket::new(
            tcp::SocketBuffer::new(rx_buf),
            tcp::SocketBuffer::new(tx_buf),
        ));
        Self {
            dns,
            tcp,
            phase: Phase::Idle,
            query: None,
            head: Buf::new(),
            body: [0; BODY],
            got: 0,
            sent: false,
            started_ms: 0,
            seq: 0,
        }
    }

    /// Put the router's own resolver at the end of the list. A network that
    /// blocks outbound port 53 - which is common enough - can still be resolved
    /// through the resolver it handed out itself.
    pub fn adopt_dhcp_servers(&mut self, sockets: &mut SocketSet<'static>, offered: &[Ipv4Address]) {
        let mut servers = [IpAddress::Ipv4(Ipv4Address::UNSPECIFIED); MAX_DNS];
        let mut n = 0;
        for server in PUBLIC_DNS.iter().copied().chain(offered.iter().map(|ip| IpAddress::Ipv4(*ip))) {
            if n < MAX_DNS {
                servers[n] = server;
                n += 1;
            }
        }
        esp_println::println!("dns: {n} servers, last={}", servers[n - 1]);
        sockets
            .get_mut::<dns::Socket>(self.dns)
            .update_servers(&servers[..n]);
    }

    pub fn busy(&self) -> bool {
        matches!(self.phase, Phase::Resolving | Phase::Talking)
    }

    /// Begin a GET. Silently replaces anything already in flight, which is what
    /// the pages want: the newest request is the one whose answer is wanted.
    pub fn get(
        &mut self,
        sockets: &mut SocketSet<'static>,
        iface: &mut Interface,
        host: &str,
        path: &str,
        now_ms: u32,
    ) {
        sockets.get_mut::<tcp::Socket>(self.tcp).abort();
        self.head = Buf::new();
        let _ = write!(
            self.head,
            "GET {path} HTTP/1.0\r\nHost: {host}\r\nUser-Agent: rainbird-panel/1.0\r\nConnection: close\r\n\r\n"
        );
        self.got = 0;
        self.sent = false;
        self.started_ms = now_ms;
        self.seq = self.seq.wrapping_add(1);

        // A literal address needs no lookup, and asking 1.1.1.1 to resolve one
        // would fail.
        if let Some(ip) = parse_ipv4(host) {
            self.phase = Phase::Talking;
            self.connect(sockets, iface, IpAddress::Ipv4(ip), now_ms);
            return;
        }
        let query = sockets
            .get_mut::<dns::Socket>(self.dns)
            .start_query(iface.context(), host, DnsQueryType::A);
        match query {
            Ok(handle) => {
                self.query = Some(handle);
                self.phase = Phase::Resolving;
            }
            Err(_) => {
                esp_println::println!("fetch: cannot start lookup of {host}");
                self.phase = Phase::Failed;
            }
        }
    }

    fn connect(
        &mut self,
        sockets: &mut SocketSet<'static>,
        iface: &mut Interface,
        addr: IpAddress,
        now_ms: u32,
    ) {
        let port = 40_000 + (now_ms.wrapping_add(self.seq * 977) % 8_000) as u16;
        let socket = sockets.get_mut::<tcp::Socket>(self.tcp);
        socket.abort();
        if socket.connect(iface.context(), (addr, 80), port).is_err() {
            self.phase = Phase::Failed;
        }
    }

    /// One step of whatever is in flight. Cheap and safe to call every frame.
    pub fn step(&mut self, sockets: &mut SocketSet<'static>, iface: &mut Interface, now_ms: u32) {
        if !self.busy() {
            return;
        }
        if now_ms.wrapping_sub(self.started_ms) > TIMEOUT_MS {
            let state = sockets.get::<tcp::Socket>(self.tcp).state();
            esp_println::println!(
                "fetch: timed out ({}), tcp={state}, got={}",
                if self.phase == Phase::Resolving { "resolving" } else { "talking" },
                self.got
            );
            sockets.get_mut::<tcp::Socket>(self.tcp).abort();
            self.phase = Phase::Failed;
            return;
        }

        if self.phase == Phase::Resolving {
            let Some(handle) = self.query else {
                self.phase = Phase::Failed;
                return;
            };
            match sockets
                .get_mut::<dns::Socket>(self.dns)
                .get_query_result(handle)
            {
                Ok(addresses) => {
                    self.query = None;
                    match addresses.first() {
                        Some(&addr) => {
                            esp_println::println!("fetch: resolved to {addr}");
                            self.phase = Phase::Talking;
                            self.connect(sockets, iface, addr, now_ms);
                        }
                        None => self.phase = Phase::Failed,
                    }
                }
                Err(dns::GetQueryResultError::Pending) => {}
                Err(_) => {
                    self.query = None;
                    esp_println::println!("fetch: name lookup failed");
                    self.phase = Phase::Failed;
                }
            }
            return;
        }

        let socket = sockets.get_mut::<tcp::Socket>(self.tcp);
        if !self.sent && socket.may_send() {
            match socket.send_slice(self.head.as_str().as_bytes()) {
                Ok(_) => self.sent = true,
                Err(_) => {
                    socket.abort();
                    self.phase = Phase::Failed;
                    return;
                }
            }
        }
        // Drain everything available now rather than a slice per frame: the reply
        // is only complete once the buffer is empty and the peer has finished, and
        // that test would otherwise be taken against a buffer still holding data.
        while socket.can_recv() && self.got < BODY {
            let body = &mut self.body;
            let got = self.got;
            let Ok(read) = socket.recv(|data| {
                let take = data.len().min(BODY - got);
                body[got..got + take].copy_from_slice(&data[..take]);
                (take, take)
            }) else {
                break;
            };
            if read == 0 {
                break;
            }
            self.got += read;
        }
        // The server closing is what marks the end - the request asked for close,
        // and these replies are not all framed by a length header.
        //
        // `may_recv` is the test, not `is_active`: a peer that has sent its FIN
        // leaves the socket in CLOSE-WAIT, which counts as active, so the first
        // version of this waited out its whole timeout on top of a reply that had
        // arrived complete. `may_recv` goes false exactly when the remote is done
        // and the buffer is drained.
        if self.sent && !socket.may_recv() {
            socket.abort();
            let ok = self.got > 0 && status_ok(&self.body[..self.got]);
            esp_println::println!("fetch: {} bytes, ok={}", self.got, ok);
            self.phase = if ok { Phase::Ready } else { Phase::Failed };
        }
    }

    /// The body of a completed reply, once. Returns None until one is ready, and
    /// leaves the fetcher idle afterwards.
    pub fn take(&mut self) -> Option<&str> {
        if self.phase != Phase::Ready {
            return None;
        }
        self.phase = Phase::Idle;
        let start = body_start(&self.body[..self.got]);
        core::str::from_utf8(&self.body[start..self.got]).ok()
    }

    /// Whether the last request gave up, once.
    pub fn take_failure(&mut self) -> bool {
        if self.phase != Phase::Failed {
            return false;
        }
        self.phase = Phase::Idle;
        true
    }
}

fn status_ok(response: &[u8]) -> bool {
    let head = &response[..response.len().min(16)];
    core::str::from_utf8(head)
        .ok()
        .and_then(|text| text.split(' ').nth(1))
        .map(|code| code.starts_with("20"))
        .unwrap_or(false)
}

fn body_start(response: &[u8]) -> usize {
    response
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|at| at + 4)
        .unwrap_or(response.len())
}

fn parse_ipv4(host: &str) -> Option<Ipv4Address> {
    let mut octets = [0u8; 4];
    let mut parts = host.split('.');
    for octet in octets.iter_mut() {
        *octet = parts.next()?.parse().ok()?;
    }
    if parts.next().is_some() {
        return None;
    }
    Some(Ipv4Address::from(octets))
}

/// The JSON these APIs return is shallow and known, so the whole of it is not
/// worth parsing: find the key, read what follows.
///
/// Nothing here tracks nesting, so a key that occurs twice is a trap - and both
/// APIs set it. Open-meteo precedes `daily` with a `daily_units` object carrying
/// the same key names, so a flat search for `time` finds the string "iso8601"
/// instead of the array of dates. `scope` is the answer: narrow to the object
/// wanted first, then read keys out of that.

/// The text from just inside `"key":{` onwards, for reading keys within one
/// object rather than the whole document.
pub fn scope<'a>(text: &'a str, key: &str) -> &'a str {
    let mut needle = Buf::<32>::new();
    let _ = write!(needle, "\"{key}\":{{");
    match text.find(needle.as_str()) {
        Some(at) => &text[at + needle.len()..],
        None => text,
    }
}
pub fn json_str<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let after = field(text, key)?;
    let rest = after.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(&rest[..end])
}

/// A number as it was written, so the caller decides how to read it.
pub fn json_num<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let after = field(text, key)?;
    let end = after
        .find(|c: char| !(c.is_ascii_digit() || c == '-' || c == '.' || c == 'e' || c == '+'))
        .unwrap_or(after.len());
    Some(&after[..end]).filter(|s| !s.is_empty())
}

/// Values of a `"key":[...]` array, in order, unquoted.
pub fn json_array<'a>(text: &'a str, key: &str) -> impl Iterator<Item = &'a str> {
    field(text, key)
        .and_then(|after| after.strip_prefix('['))
        .and_then(|rest| rest.find(']').map(|end| &rest[..end]))
        .unwrap_or("")
        .split(',')
        .map(|item| item.trim().trim_matches('"'))
        .filter(|item| !item.is_empty())
}

/// What follows `"key":`, with whitespace skipped.
fn field<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let mut needle = Buf::<32>::new();
    let _ = write!(needle, "\"{key}\":");
    let at = text.find(needle.as_str())? + needle.len();
    Some(text[at..].trim_start())
}

/// A number written with a decimal point, as millionths.
///
/// Exchange rates need the places: a yen is 0.0645 kronor, and at tenths that
/// is nothing at all.
pub fn micros(text: &str) -> Option<i64> {
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let (whole, frac) = match digits.split_once('.') {
        Some((w, f)) => (w, f),
        None => (digits, ""),
    };
    let mut value: i64 = whole.parse().ok()?;
    value *= 1_000_000;
    let mut scale = 100_000i64;
    for byte in frac.bytes().take(6) {
        if !byte.is_ascii_digit() {
            return None;
        }
        value += (byte - b'0') as i64 * scale;
        scale /= 10;
    }
    Some(if negative { -value } else { value })
}

/// A number written with a decimal point, as tenths. The forecast and the rates
/// both arrive this way and both want one place, so this is the whole of the
/// float handling needed on the parsing side.
pub fn tenths(text: &str) -> Option<i32> {
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let (whole, frac) = match digits.split_once('.') {
        Some((w, f)) => (w, f),
        None => (digits, "0"),
    };
    let whole: i32 = whole.parse().ok()?;
    let first = frac.as_bytes().first().map(|b| (b - b'0') as i32)?;
    let rounded = match frac.as_bytes().get(1) {
        Some(&b) if b >= b'5' => first + 1,
        _ => first,
    };
    let value = whole * 10 + rounded.min(10);
    Some(if negative { -value } else { value })
}
