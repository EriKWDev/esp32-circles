//! About: who wrote it, what it is talking to, and the shape of the config.
//!
//! Three pages, advanced by tapping anywhere. Everything on them is read from
//! what is actually in use rather than restated here - the server names come from
//! the app modules that call them, the addresses from the resolver that resolved
//! them, and the build stamp from the build script, since the panel has no clock
//! of its own until a controller tells it the time.
//!
//! Labels are capped at MAX_TEXT, so the config schema is written to fit that in
//! one line each. It is the record layout of the controller's /api/dump, which is
//! also what POST /api/schedule accepts back.

use core::fmt::Write as _;

use crate::font::FontId;
use crate::gfx::{Align, Scene, TextBuf, W, muted, rgb};

pub const PAGES: usize = 3;

const INK: u16 = rgb(238, 245, 250);
const DIM: u16 = rgb(140, 152, 166);
const KEY: u16 = rgb(255, 190, 90);
pub const ACCENT: u16 = rgb(190, 170, 255);

const TOP: i32 = 96;
const LINE: i32 = 24;
const LEFT: i32 = 26;

/// What only main can see: the heap, the flash, and the address in use.
#[derive(Clone, Copy)]
pub struct Sys {
    pub ip: Option<[u8; 4]>,
    pub heap_free: u32,
    pub heap_used: u32,
    pub flash_bytes: u32,
    pub nvs_offset: u32,
    pub nvs_sector: u32,
    pub record_len: u32,
}

impl Sys {
    pub const EMPTY: Self = Self {
        ip: None,
        heap_free: 0,
        heap_used: 0,
        flash_bytes: 0,
        nvs_offset: 0,
        nvs_sector: 0,
        record_len: 0,
    };
}

/// Everything the page needs that lives elsewhere, borrowed for one draw.
pub struct Facts<'a> {
    pub sys: Sys,
    pub settings: &'a crate::store::Settings,
    /// Per controller, in settings order.
    pub online: &'a [bool],
    pub city: &'a str,
    pub lat: &'a str,
    pub lon: &'a str,
    pub geo: (&'a str, Option<[u8; 4]>),
    pub met: (&'a str, Option<[u8; 4]>),
    pub rates: (&'a str, Option<[u8; 4]>),
}

/// The /api/dump record layout. Each line is within MAX_TEXT, which is why the
/// angle brackets of the server's own documentation are dropped.
const SCHEMA: [&str; 15] = [
    "v:1",
    "relays:n",
    "r:id:port:enabled:on:name",
    "starts:n",
    "s:id:enabled:hh:mm:count",
    "e:start:relay:seconds",
    "time:HH:MM:SS",
    "run:0|1",
    "active:relay  0 idle -1 test",
    "left:seconds",
    "queued:n   qgap:0|1",
    "sched:start_id  0 manual",
    "maxrun:s   relaygap:s",
    "analogs:n",
    "a:port:level:name",
];

pub fn draw(scene: &mut Scene, page: usize, facts: &Facts, alpha: u8) {
    let mut row = TOP;

    match page {
        0 => {
            scene.label(
                W as i32 / 2,
                58,
                FontId::Body,
                INK,
                alpha,
                Align::Center,
                "RAINBIRD PANEL",
            );
            line(scene, &mut row, "BY", "ERIK GREN", ACCENT, alpha);
            line(scene, &mut row, "AND", "MARTIN GREN", ACCENT, alpha);
            row += 8;
            line(scene, &mut row, "BUILT", crate::net::BUILD_STAMP, INK, alpha);
            line(scene, &mut row, "CHIP", "ESP32-C6", INK, alpha);
            row += 8;

            let mut value = TextBuf::new();
            let _ = write!(value, "{} KB FREE", facts.sys.heap_free / 1024);
            line(scene, &mut row, "HEAP", value.as_str(), INK, alpha);
            let mut value = TextBuf::new();
            let _ = write!(value, "{} KB USED", facts.sys.heap_used / 1024);
            line(scene, &mut row, "", value.as_str(), DIM, alpha);
            let mut value = TextBuf::new();
            let _ = write!(value, "{} MB", facts.sys.flash_bytes / (1024 * 1024));
            line(scene, &mut row, "FLASH", value.as_str(), INK, alpha);
            let mut value = TextBuf::new();
            let _ = write!(
                value,
                "{} B AT {:#x}",
                facts.sys.nvs_sector, facts.sys.nvs_offset
            );
            line(scene, &mut row, "PARAMS", value.as_str(), INK, alpha);
            let mut value = TextBuf::new();
            let _ = write!(value, "{} B IN USE", facts.sys.record_len);
            line(scene, &mut row, "", value.as_str(), DIM, alpha);
        }
        1 => {
            scene.label(
                W as i32 / 2,
                58,
                FontId::Body,
                INK,
                alpha,
                Align::Center,
                "NETWORK",
            );
            line(scene, &mut row, "WI-FI", facts.settings.ssid.as_str(), INK, alpha);
            let mut value = TextBuf::new();
            match facts.sys.ip {
                Some(ip) => {
                    let _ = write!(value, "{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]);
                }
                None => {
                    let _ = write!(value, "NO ADDRESS");
                }
            }
            line(scene, &mut row, "THIS PANEL", value.as_str(), INK, alpha);
            row += 6;

            let n = facts.settings.n_controllers;
            for (index, controller) in facts.settings.controllers[..n].iter().enumerate() {
                let ip = controller.ip;
                let mut value = TextBuf::new();
                let _ = write!(
                    value,
                    "{}.{}.{}.{}:{}",
                    ip[0], ip[1], ip[2], ip[3], controller.port
                );
                let label = if controller.name.is_empty() {
                    "CONTROLLER"
                } else {
                    controller.name.as_str()
                };
                // Red when the last poll did not answer, which is the one thing
                // about a controller worth knowing at a glance.
                let online = facts.online.get(index).copied().unwrap_or(false);
                line(
                    scene,
                    &mut row,
                    label,
                    value.as_str(),
                    if online { INK } else { rgb(230, 110, 90) },
                    alpha,
                );
            }
            row += 6;

            for (what, (host, ip)) in [
                ("LOCATION", facts.geo),
                ("FORECAST", facts.met),
                ("RATES", facts.rates),
            ] {
                line(scene, &mut row, what, host, KEY, alpha);
                if let Some(ip) = ip {
                    let mut value = TextBuf::new();
                    let _ = write!(value, "{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]);
                    line(scene, &mut row, "", value.as_str(), DIM, alpha);
                }
            }
            row += 6;
            if !facts.city.is_empty() {
                line(scene, &mut row, "HERE", facts.city, INK, alpha);
                let mut value = TextBuf::new();
                let _ = write!(value, "{}, {}", facts.lat, facts.lon);
                line(scene, &mut row, "", value.as_str(), DIM, alpha);
            }
        }
        _ => {
            scene.label(
                W as i32 / 2,
                58,
                FontId::Body,
                INK,
                alpha,
                Align::Center,
                "CONFIG SCHEME",
            );
            scene.label(
                W as i32 / 2,
                84,
                FontId::Micro,
                DIM,
                alpha,
                Align::Center,
                "GET /api/dump, ONE PER LINE",
            );
            row = 112;
            for record in SCHEMA {
                scene.label(LEFT, row, FontId::Micro, KEY, alpha, Align::Left, record);
                row += 23;
            }
            scene.label(
                LEFT,
                row + 4,
                FontId::Micro,
                DIM,
                alpha,
                Align::Left,
                "s: AND e: POST BACK AS-IS",
            );
        }
    }

    for index in 0..PAGES {
        let x = W as i32 / 2 - (PAGES as i32 - 1) * 9 + index as i32 * 18;
        let here = index == page;
        scene.disc(
            x,
            460,
            if here { 6 } else { 4 },
            if here { ACCENT } else { muted(DIM) },
            alpha,
        );
    }
}

/// One key-and-value row, advancing the cursor.
#[allow(clippy::too_many_arguments)]
fn line(scene: &mut Scene, row: &mut i32, key: &str, value: &str, color: u16, alpha: u8) {
    if !key.is_empty() {
        scene.label(LEFT, *row, FontId::Micro, DIM, alpha, Align::Left, key);
    }
    if !value.is_empty() {
        scene.label(
            W as i32 - LEFT,
            *row,
            FontId::Micro,
            color,
            alpha,
            Align::Right,
            value,
        );
    }
    *row += LINE;
}
