//! Settings that survive a reboot, kept in the flash `nvs` partition.
//!
//! The board has no SD card, but it does have 16 MiB of SPI flash, and the
//! partition table the bootloader prints already reserves `nvs` at 0x9000 for
//! exactly this. Nothing else in the firmware touches it, so this owns it.
//!
//! Deliberately a single fixed-size record rather than a key-value store. The
//! whole settings blob is a few hundred bytes, it is rewritten as a unit, and a
//! flash sector is 4 KiB - so a full-record write is one erase and one program.
//! `sequential-storage` or a real NVS implementation would buy wear levelling
//! and partial updates that a config changed by hand a few times a year does not
//! need.
//!
//! Integrity: a magic number, a format version and a CRC over the payload. Any
//! of those failing means "no saved settings", and the build-time defaults from
//! `wifi.txt` are used instead - so a corrupt or first-boot sector degrades to
//! the compiled-in configuration rather than to a broken panel.

use embedded_storage::nor_flash::{NorFlash, ReadNorFlash};
use esp_storage::FlashStorage;

use crate::net::{RB_HOST, RB_PASS, RB_USER, WIFI_PASS, WIFI_SSID};

/// Offset of the `nvs` partition, from the partition table.
const NVS_OFFSET: u32 = 0x9000;
/// One flash sector: the erase granularity, and all we need.
const SECTOR: usize = 4096;

const MAGIC: u32 = 0x5242_4E31; // "RBN1"
const VERSION: u16 = 1;

pub const MAX_CONTROLLERS: usize = 6;
pub const MAX_SSID: usize = 32;
pub const MAX_SECRET: usize = 64;
pub const MAX_USER: usize = 32;

/// A fixed-capacity string stored inline, so the whole record is Copy and has a
/// stable on-flash layout without any serialization framework.
#[derive(Clone, Copy)]
pub struct FixedStr<const N: usize> {
    pub bytes: [u8; N],
    pub len: u8,
}

impl<const N: usize> FixedStr<N> {
    pub const EMPTY: Self = Self { bytes: [0; N], len: 0 };

    pub fn new(s: &str) -> Self {
        let mut out = Self::EMPTY;
        out.set(s);
        out
    }

    pub fn set(&mut self, s: &str) {
        // Truncate on a char boundary so a clipped value is still valid UTF-8.
        let mut take = 0;
        for (i, c) in s.char_indices() {
            if i + c.len_utf8() > N {
                break;
            }
            take = i + c.len_utf8();
        }
        self.bytes = [0; N];
        self.bytes[..take].copy_from_slice(&s.as_bytes()[..take]);
        self.len = take as u8;
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len as usize]).unwrap_or("")
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// One irrigation controller: where it is and how to authenticate to it.
#[derive(Clone, Copy)]
pub struct Controller {
    pub ip: [u8; 4],
    pub user: FixedStr<MAX_USER>,
    pub pass: FixedStr<MAX_SECRET>,
}

impl Controller {
    pub const EMPTY: Self = Self {
        ip: [0; 4],
        user: FixedStr::EMPTY,
        pass: FixedStr::EMPTY,
    };

    pub fn is_set(&self) -> bool {
        self.ip != [0, 0, 0, 0]
    }
}

#[derive(Clone, Copy)]
pub struct Settings {
    pub ssid: FixedStr<MAX_SSID>,
    pub psk: FixedStr<MAX_SECRET>,
    pub controllers: [Controller; MAX_CONTROLLERS],
    pub n_controllers: usize,
}

impl Settings {
    /// All-empty, for `const` initialisation before flash has been read.
    pub const EMPTY: Self = Self {
        ssid: FixedStr::EMPTY,
        psk: FixedStr::EMPTY,
        controllers: [Controller::EMPTY; MAX_CONTROLLERS],
        n_controllers: 0,
    };

    /// The compiled-in configuration from `wifi.txt`. Used on first boot, and
    /// whenever the saved record does not validate.
    pub fn defaults() -> Self {
        let mut out = Self {
            ssid: FixedStr::new(WIFI_SSID),
            psk: FixedStr::new(WIFI_PASS),
            controllers: [Controller::EMPTY; MAX_CONTROLLERS],
            n_controllers: 0,
        };
        if let Some(ip) = parse_ip(RB_HOST) {
            out.controllers[0] = Controller {
                ip,
                // The credentials baked in at build time become the default for
                // a controller added later too, which is what makes adding one
                // from the panel a matter of typing an address and nothing else.
                user: FixedStr::new(RB_USER),
                pass: FixedStr::new(RB_PASS),
            };
            out.n_controllers = 1;
        }
        out
    }

    pub fn add_controller(&mut self, ip: [u8; 4]) -> bool {
        if self.n_controllers >= MAX_CONTROLLERS {
            return false;
        }
        self.controllers[self.n_controllers] = Controller {
            ip,
            user: FixedStr::new(RB_USER),
            pass: FixedStr::new(RB_PASS),
        };
        self.n_controllers += 1;
        true
    }

    pub fn remove_controller(&mut self, index: usize) {
        if index >= self.n_controllers {
            return;
        }
        for i in index..self.n_controllers - 1 {
            self.controllers[i] = self.controllers[i + 1];
        }
        self.n_controllers -= 1;
        self.controllers[self.n_controllers] = Controller::EMPTY;
    }
}

/// Parses a dotted quad. Returns None on anything malformed, so a typo cannot
/// silently become 0.0.0.0.
pub fn parse_ip(s: &str) -> Option<[u8; 4]> {
    let mut octets = [0u8; 4];
    let mut index = 0;
    let mut digits = 0;
    let mut value: u16 = 0;
    for c in s.chars() {
        match c {
            '0'..='9' => {
                value = value * 10 + (c as u8 - b'0') as u16;
                digits += 1;
                if value > 255 || digits > 3 {
                    return None;
                }
            }
            '.' => {
                if digits == 0 || index >= 3 {
                    return None;
                }
                octets[index] = value as u8;
                index += 1;
                value = 0;
                digits = 0;
            }
            _ => return None,
        }
    }
    if digits == 0 || index != 3 {
        return None;
    }
    octets[3] = value as u8;
    Some(octets)
}

/// CRC-32 (IEEE), computed bitwise. A table would be faster, but this runs twice
/// per settings change, not per frame.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// Byte layout, little-endian throughout:
///   0..4    magic
///   4..6    version
///   6..8    payload length
///   8..12   crc32 of payload
///   12..    payload
const HEADER: usize = 12;

fn encode(settings: &Settings, out: &mut [u8; SECTOR]) -> usize {
    let mut at = HEADER;
    let mut put = |bytes: &[u8], at: &mut usize| {
        out[*at..*at + bytes.len()].copy_from_slice(bytes);
        *at += bytes.len();
    };

    put(&[settings.ssid.len], &mut at);
    put(&settings.ssid.bytes, &mut at);
    put(&[settings.psk.len], &mut at);
    put(&settings.psk.bytes, &mut at);
    put(&[settings.n_controllers as u8], &mut at);
    for controller in &settings.controllers {
        put(&controller.ip, &mut at);
        put(&[controller.user.len], &mut at);
        put(&controller.user.bytes, &mut at);
        put(&[controller.pass.len], &mut at);
        put(&controller.pass.bytes, &mut at);
    }

    let payload_len = at - HEADER;
    let crc = crc32(&out[HEADER..at]);
    out[0..4].copy_from_slice(&MAGIC.to_le_bytes());
    out[4..6].copy_from_slice(&VERSION.to_le_bytes());
    out[6..8].copy_from_slice(&(payload_len as u16).to_le_bytes());
    out[8..12].copy_from_slice(&crc.to_le_bytes());
    at
}

fn decode(raw: &[u8; SECTOR]) -> Option<Settings> {
    if u32::from_le_bytes(raw[0..4].try_into().ok()?) != MAGIC {
        return None;
    }
    if u16::from_le_bytes(raw[4..6].try_into().ok()?) != VERSION {
        // A future version is not readable here. Falling back to defaults is
        // safer than guessing at a layout we do not know.
        return None;
    }
    let payload_len = u16::from_le_bytes(raw[6..8].try_into().ok()?) as usize;
    if payload_len == 0 || HEADER + payload_len > SECTOR {
        return None;
    }
    let expected = u32::from_le_bytes(raw[8..12].try_into().ok()?);
    if crc32(&raw[HEADER..HEADER + payload_len]) != expected {
        return None;
    }

    let mut at = HEADER;
    let take = |n: usize, at: &mut usize| -> &[u8] {
        let slice = &raw[*at..*at + n];
        *at += n;
        slice
    };

    let mut settings = Settings {
        ssid: FixedStr::EMPTY,
        psk: FixedStr::EMPTY,
        controllers: [Controller::EMPTY; MAX_CONTROLLERS],
        n_controllers: 0,
    };
    settings.ssid.len = take(1, &mut at)[0].min(MAX_SSID as u8);
    settings.ssid.bytes.copy_from_slice(take(MAX_SSID, &mut at));
    settings.psk.len = take(1, &mut at)[0].min(MAX_SECRET as u8);
    settings.psk.bytes.copy_from_slice(take(MAX_SECRET, &mut at));
    let count = take(1, &mut at)[0] as usize;
    for index in 0..MAX_CONTROLLERS {
        let mut controller = Controller::EMPTY;
        controller.ip.copy_from_slice(take(4, &mut at));
        controller.user.len = take(1, &mut at)[0].min(MAX_USER as u8);
        controller.user.bytes.copy_from_slice(take(MAX_USER, &mut at));
        controller.pass.len = take(1, &mut at)[0].min(MAX_SECRET as u8);
        controller.pass.bytes.copy_from_slice(take(MAX_SECRET, &mut at));
        settings.controllers[index] = controller;
    }
    settings.n_controllers = count.min(MAX_CONTROLLERS);
    Some(settings)
}

pub struct Store<'d> {
    flash: FlashStorage<'d>,
}

impl<'d> Store<'d> {
    pub fn new(flash: esp_hal::peripherals::FLASH<'d>) -> Self {
        // esp-storage's `Flash` is an alias for the HAL's FLASH peripheral.
        Self { flash: FlashStorage::new(flash) }
    }

    /// Saved settings, or the build-time defaults when nothing valid is stored.
    pub fn load(&mut self) -> (Settings, bool) {
        let mut raw = [0u8; SECTOR];
        if self.flash.read(NVS_OFFSET, &mut raw).is_ok() {
            if let Some(settings) = decode(&raw) {
                return (settings, true);
            }
        }
        (Settings::defaults(), false)
    }

    /// Erase the sector and write the record. One erase, one program.
    pub fn save(&mut self, settings: &Settings) -> Result<(), &'static str> {
        let mut raw = [0u8; SECTOR];
        let len = encode(settings, &mut raw);
        let _ = len;
        self.flash
            .erase(NVS_OFFSET, NVS_OFFSET + SECTOR as u32)
            .map_err(|_| "flash erase failed")?;
        self.flash
            .write(NVS_OFFSET, &raw)
            .map_err(|_| "flash write failed")?;
        Ok(())
    }
}
