//! Sound: the ES8311 codec on this board, driven over I2S.
//!
//! The register sequence and the clock coefficients come from the vendor's
//! `esp_codec_dev` driver for this exact board profile (C6_AMOLED_2_16): I2S on
//! mclk 19, bclk 20, ws 22, dout 23, codec at I2C 0x18, no amplifier-enable pin,
//! and MCLK at 256x the sample rate - the simplest row in their table.
//!
//! Tones are played by blocking. A note is a few hundred milliseconds and the
//! alternative is circular DMA whose transfer object borrows the peripheral for
//! its whole life, which does not survive being held across frames. Blocking
//! costs a hitch in the animation while a note sounds, which on the two screens
//! that use it - a memory game and a cat - is not a screen anybody is watching
//! for smoothness. Nothing else on the panel plays sound, so nothing else pays.

use esp_hal::i2s::master::{Channels, Config, DataFormat, I2s};
use esp_hal::i2c::master::I2c;
use esp_hal::time::Rate;

const ADDR: u8 = 0x18;
pub const SAMPLE_RATE: u32 = 16_000;
/// Samples per DMA write. Small enough that a tone can be stopped promptly,
/// large enough that back-to-back writes leave no gap.
const CHUNK: usize = 512;

/// A note, as a quarter-wave step through the table below.
pub struct Tone {
    pub freq: u16,
    pub ms: u16,
}

/// One period of a soft waveform - a sine with a little second harmonic, which
/// sounds rounder than a square and much cuter than a raw sine.
const WAVE_LEN: usize = 64;
static WAVE: [i16; WAVE_LEN] = {
    let mut table = [0i16; WAVE_LEN];
    // Built by hand rather than at runtime: a const fn cannot call sin, and a
    // table this short is easier to read as numbers anyway.
    let quarter: [i16; 16] = [
        0, 1205, 2404, 3593, 4767, 5921, 7052, 8154, 9224, 10258, 11252, 12202, 13105, 13958,
        14757, 15500,
    ];
    let mut i = 0;
    while i < 16 {
        table[i] = quarter[i];
        table[31 - i] = quarter[i];
        table[32 + i] = -quarter[i];
        table[63 - i] = -quarter[i];
        i += 1;
    }
    table
};

pub struct Audio<'d> {
    tx: esp_hal::i2s::master::I2sTx<'d, esp_hal::Blocking>,
    buffer: &'static mut [u8],
    /// Phase accumulator, 16.16, so a frequency need not divide the rate.
    phase: u32,
    ready: bool,
}

impl<'d> Audio<'d> {
    pub fn new(
        i2s: esp_hal::peripherals::I2S0<'d>,
        channel: esp_hal::peripherals::DMA_CH1<'d>,
        mclk: esp_hal::gpio::AnyPin<'d>,
        bclk: esp_hal::gpio::AnyPin<'d>,
        ws: esp_hal::gpio::AnyPin<'d>,
        dout: esp_hal::gpio::AnyPin<'d>,
        descriptors: &'static mut [esp_hal::dma::DmaDescriptor],
        buffer: &'static mut [u8],
        i2c: &mut I2c<'_, esp_hal::Blocking>,
    ) -> Option<Self> {
        let config = Config::default()
            .with_sample_rate(Rate::from_hz(SAMPLE_RATE))
            .with_data_format(DataFormat::Data16Channel16)
            .with_channels(Channels::STEREO);
        let i2s = I2s::new(i2s, channel, config).ok()?;
        // MCLK belongs to the peripheral, the data pins to the transmitter.
        let tx = i2s
            .with_mclk(mclk)
            .i2s_tx
            .with_bclk(bclk)
            .with_ws(ws)
            .with_dout(dout)
            .build(descriptors);

        let ready = init_codec(i2c);
        Some(Self {
            tx,
            buffer,
            phase: 0,
            ready,
        })
    }

    /// Play one note, and return when it has finished.
    pub fn tone(&mut self, tone: &Tone) {
        if !self.ready {
            return;
        }
        let total = SAMPLE_RATE as usize * tone.ms as usize / 1000;
        let step = ((tone.freq as u32 * WAVE_LEN as u32) << 16) / SAMPLE_RATE;
        let mut done = 0;
        self.phase = 0;
        while done < total {
            let count = CHUNK.min(total - done);
            for index in 0..count {
                let slot = (self.phase >> 16) as usize % WAVE_LEN;
                // A short attack and release, so a note starts and stops without
                // the click a hard edge would make.
                let from_edge = index + done;
                let ramp = from_edge.min(total - from_edge).min(160) as i32;
                let sample = (WAVE[slot] as i32 * ramp / 160) as i16;
                let bytes = sample.to_le_bytes();
                let at = index * 4;
                self.buffer[at] = bytes[0];
                self.buffer[at + 1] = bytes[1];
                self.buffer[at + 2] = bytes[0];
                self.buffer[at + 3] = bytes[1];
                self.phase = self.phase.wrapping_add(step);
            }
            let slice = &self.buffer[..count * 4];
            if let Ok(transfer) = self.tx.write_dma(&slice) {
                let _ = transfer.wait();
            } else {
                return;
            }
            done += count;
        }
    }

    pub fn play(&mut self, tones: &[Tone]) {
        for tone in tones {
            self.tone(tone);
        }
    }
}

fn write(i2c: &mut I2c<'_, esp_hal::Blocking>, reg: u8, value: u8) -> bool {
    i2c.write(ADDR, &[reg, value]).is_ok()
}

/// The vendor's open-then-start sequence, with the clock registers filled in for
/// MCLK = 256 x 16 kHz and the codec as I2S slave.
fn init_codec(i2c: &mut I2c<'_, esp_hal::Blocking>) -> bool {
    // Written twice on purpose: the vendor notes that the first I2C write to this
    // part occasionally does not take.
    let mut ok = write(i2c, 0x44, 0x08);
    ok &= write(i2c, 0x44, 0x08);

    for (reg, value) in [
        (0x01u8, 0x30u8),
        (0x02, 0x00),
        (0x03, 0x10),
        (0x16, 0x24),
        (0x04, 0x10),
        (0x05, 0x00),
        (0x0B, 0x00),
        (0x0C, 0x00),
        (0x10, 0x1F),
        (0x11, 0x7F),
        // Slave mode: the ESP32 is master, so bit 6 stays clear.
        (0x00, 0x80),
    ] {
        ok &= write(i2c, reg, value);
    }

    // Clocks, from the {4096000, 16000} row: every divider is one, the frame is
    // 0x00ff long, and BCLK divides by four.
    for (reg, value) in [
        (0x02u8, 0x00u8),
        (0x05, 0x00),
        (0x03, 0x10),
        (0x04, 0x20),
        (0x07, 0x00),
        (0x08, 0xFF),
        (0x06, 0x03),
    ] {
        ok &= write(i2c, reg, value);
    }

    // Sixteen-bit I2S in and out.
    ok &= write(i2c, 0x09, 0x0C);
    ok &= write(i2c, 0x0A, 0x0C);

    // Power up the DAC and open the output.
    for (reg, value) in [
        (0x17u8, 0xBFu8),
        (0x0E, 0x02),
        (0x12, 0x00),
        (0x14, 0x1A),
        (0x0D, 0x01),
        (0x15, 0x40),
        (0x37, 0x08),
        (0x45, 0x00),
        (0x01, 0x3F),
        // Volume. Loud enough to hear across a room, short of the point where a
        // small speaker starts to rattle.
        (0x32, 0xB4),
    ] {
        ok &= write(i2c, reg, value);
    }
    ok
}

/// A pentatonic set, so any order of them sounds deliberate. C5 D5 E5 G5, then A5
/// and C6 for the flourishes.
pub const NOTES: [u16; 6] = [523, 587, 659, 784, 880, 1047];

pub fn win() -> [Tone; 4] {
    [
        Tone { freq: NOTES[0], ms: 90 },
        Tone { freq: NOTES[2], ms: 90 },
        Tone { freq: NOTES[3], ms: 90 },
        Tone { freq: NOTES[5], ms: 180 },
    ]
}

pub fn lose() -> [Tone; 3] {
    [
        Tone { freq: 392, ms: 150 },
        Tone { freq: 330, ms: 150 },
        Tone { freq: 262, ms: 320 },
    ]
}

pub fn purr() -> [Tone; 2] {
    [
        Tone { freq: 220, ms: 70 },
        Tone { freq: 196, ms: 90 },
    ]
}

pub fn meow() -> [Tone; 2] {
    [
        Tone { freq: 659, ms: 80 },
        Tone { freq: 880, ms: 130 },
    ]
}
