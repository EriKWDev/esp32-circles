//! Sound: the ES8311 codec on this board, driven over I2S.
//!
//! The register sequence and the clock coefficients come from the vendor's
//! `esp_codec_dev` driver for this exact board profile (C6_AMOLED_2_16): I2S on
//! mclk 19, bclk 20, ws 22, dout 23, codec at I2C 0x18, no amplifier-enable pin,
//! and MCLK at 256x the sample rate - the simplest row in their table.
//!
//! Playing does not block the UI. A circular DMA transfer borrows the peripheral
//! for its whole life, so it cannot be stored between frames - which is why the
//! first version simply blocked, and why the panel stalled on every note. Control
//! is inverted instead: `play_while` keeps the transfer alive for the length of a
//! melody and calls back whenever the DMA is full, so the caller renders its
//! frames from inside the playback loop. Sound is continuous, screen keeps moving.

use esp_hal::i2c::master::I2c;
use esp_hal::i2s::master::{Channels, Config, DataFormat, I2s};
use esp_hal::time::Rate;

const ADDR: u8 = 0x18;
pub const SAMPLE_RATE: u32 = 16_000;
/// Bytes per stereo frame: two 16-bit samples.
const FRAME: usize = 4;
/// Samples of fade at each end of a note, so it neither clicks on nor off.
const RAMP: usize = 160;

pub struct Tone {
    pub freq: u16,
    pub ms: u16,
}

/// One period of a soft waveform. Rounder than a square, and cuter than a raw
/// sine because of a little second harmonic in the shoulders.
const WAVE_LEN: usize = 64;
static WAVE: [i16; WAVE_LEN] = {
    let mut table = [0i16; WAVE_LEN];
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
    ready: bool,
}

impl<'d> Audio<'d> {
    #[allow(clippy::too_many_arguments)]
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
        Some(Self { tx, buffer, ready })
    }

    /// The output is unmuted only while something plays. Left open it hisses: the
    /// DAC is powered and amplifying an input nothing is clocking, and at this
    /// volume that noise floor carries across a room.
    pub fn set_mute(&self, i2c: &mut I2c<'_, esp_hal::Blocking>, on: bool) {
        if self.ready {
            mute(i2c, on);
        }
    }

    /// Play a melody, calling `pump` whenever the DMA has no room.
    ///
    /// `pump` is where the caller renders. It must not touch this struct or the
    /// I2C bus the mute rides on - which is why `set_mute` is separate and called
    /// either side of this.
    pub fn play_while(&mut self, tones: &[Tone], mut pump: impl FnMut()) {
        if !self.ready {
            return;
        }
        // The buffer moves out for the duration: the transfer needs it, and it and
        // `tx` cannot both be borrowed out of `self` at once.
        let buffer = core::mem::replace(&mut self.buffer, &mut []);
        for byte in buffer.iter_mut() {
            *byte = 0;
        }
        let Ok(mut transfer) = self.tx.write_dma_circular(&buffer) else {
            self.buffer = buffer;
            return;
        };

        // Phase is a local, not a field: `self` is borrowed by the transfer for as
        // long as this runs.
        let mut phase: u32 = 0;
        for tone in tones {
            let total = SAMPLE_RATE as usize * tone.ms as usize / 1000;
            let step = ((tone.freq as u32 * WAVE_LEN as u32) << 16) / SAMPLE_RATE;
            let mut done = 0usize;
            while done < total {
                let remaining = (total - done) * FRAME;
                let from = done;
                let pushed = transfer
                    .push_with(|slot| {
                        let bytes = slot.len().min(remaining) / FRAME * FRAME;
                        for frame in 0..bytes / FRAME {
                            let index = from + frame;
                            let ramp = index.min(total - index).min(RAMP) as i32;
                            let wave = WAVE[(phase >> 16) as usize % WAVE_LEN] as i32;
                            let sample = (wave * ramp / RAMP as i32) as i16;
                            let pair = sample.to_le_bytes();
                            let at = frame * FRAME;
                            slot[at] = pair[0];
                            slot[at + 1] = pair[1];
                            slot[at + 2] = pair[0];
                            slot[at + 3] = pair[1];
                            phase = phase.wrapping_add(step);
                        }
                        bytes
                    })
                    .unwrap_or(0);
                done += pushed / FRAME;
                // Room or not, the caller gets its frame.
                pump();
            }
        }

        // Silence for one buffer's worth before stopping, or the DMA keeps
        // replaying whatever of the last note is still queued.
        let mut flushed = 0;
        while flushed < buffer.len() {
            let pushed = transfer
                .push_with(|slot| {
                    for byte in slot.iter_mut() {
                        *byte = 0;
                    }
                    slot.len()
                })
                .unwrap_or(0);
            if pushed == 0 {
                pump();
            }
            flushed += pushed;
        }
        let _ = transfer.stop();
        self.buffer = buffer;
    }
}

fn write(i2c: &mut I2c<'_, esp_hal::Blocking>, reg: u8, value: u8) -> bool {
    i2c.write(ADDR, &[reg, value]).is_ok()
}

/// The DAC's own mute bits, which silence the output without disturbing the
/// clocks or the volume setting.
fn mute(i2c: &mut I2c<'_, esp_hal::Blocking>, on: bool) {
    write(i2c, 0x31, if on { 0x60 } else { 0x00 });
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
        // Loud enough to hear across a room, short of where a small speaker starts
        // to rattle.
        (0x32, 0xB4),
    ] {
        ok &= write(i2c, reg, value);
    }
    // Silent until something asks for a note.
    mute(i2c, true);
    ok
}

/// A pentatonic set, so any order of them sounds deliberate rather than random:
/// C5 D5 E5 G5, plus A5 and top C for the flourishes.
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
