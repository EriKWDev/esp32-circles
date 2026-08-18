//! CO5300 AMOLED over 80 MHz quad-SPI with GDMA ping-pong stripes.
//!
//! Lifted essentially unchanged from the circles demo - the init sequence,
//! stripe size and ping-pong scheme were already tuned against this panel, and
//! the UI only needed a way to hand it a full frame.

use esp_hal::{
    delay::Delay,
    dma::DmaTxBuf,
    spi::master::{Address, Command, DataMode, SpiDma},
    time::Duration,
};

use crate::gfx::{self, H, STRIPE_BYTES, STRIPE_ROWS, Scene, W};

pub struct Display<'d> {
    spi: Option<SpiDma<'d, esp_hal::Blocking>>,
    tx: Option<DmaTxBuf>,
    spare: Option<DmaTxBuf>,
}

impl<'d> Display<'d> {
    pub fn new(spi: SpiDma<'d, esp_hal::Blocking>, tx: DmaTxBuf, spare: DmaTxBuf) -> Self {
        Self {
            spi: Some(spi),
            tx: Some(tx),
            spare: Some(spare),
        }
    }

    #[inline]
    fn buffer_mut(&mut self) -> &mut [u8] {
        self.tx.as_mut().unwrap().as_mut_slice()
    }

    #[inline]
    fn send_prepared(&mut self, opcode: u8, command: u8, mode: DataMode, len: usize) {
        let spi = self.spi.take().unwrap();
        let tx = self.tx.take().unwrap();
        let transfer = spi
            .half_duplex_write(
                mode,
                Command::_8Bit(opcode as u16, DataMode::Single),
                Address::_24Bit((command as u32) << 8, DataMode::Single),
                0,
                len,
                tx,
            )
            .unwrap_or_else(|_| panic!());
        let (spi, tx) = transfer.wait();
        self.spi = Some(spi);
        self.tx = Some(tx);
    }

    #[inline]
    fn cmd(&mut self, opcode: u8, command: u8, mode: DataMode, bytes: &[u8]) {
        self.buffer_mut()[..bytes.len()].copy_from_slice(bytes);
        self.send_prepared(opcode, command, mode, bytes.len());
    }

    pub fn init(&mut self, delay: &Delay) {
        // CO5300 vendor sequence, with the data-sheet-mandated sleep-out wait.
        const INIT: &[(u8, &[u8], u32)] = &[
            (0x11, &[], 120),
            (0xfe, &[0x20], 0),
            (0x19, &[0x10], 0),
            (0x1c, &[0xa0], 0),
            (0xfe, &[0x00], 0),
            (0xc4, &[0x80], 0),
            (0x3a, &[0x55], 0),
            (0x35, &[0x00], 0),
            (0x36, &[0x30], 0),
            (0x53, &[0x20], 0),
            (0x51, &[0xff], 0),
            (0x63, &[0xff], 0),
            (0x2a, &[0x00, 0x00, 0x01, 0xdf], 0),
            (0x2b, &[0x00, 0x00, 0x01, 0xdf], 0),
            (0x29, &[], 20),
        ];
        for &(cmd, data, wait) in INIT {
            self.cmd(0x02, cmd, DataMode::Single, data);
            if wait != 0 {
                delay.delay(Duration::from_millis(wait as u64));
            }
        }
        self.set_window(0, 0, (W - 1) as u16, (H - 1) as u16);
    }

    pub fn set_window(&mut self, x0: u16, y0: u16, x1: u16, y1: u16) {
        self.cmd(
            0x02,
            0x2a,
            DataMode::Single,
            &[(x0 >> 8) as u8, x0 as u8, (x1 >> 8) as u8, x1 as u8],
        );
        self.cmd(
            0x02,
            0x2b,
            DataMode::Single,
            &[(y0 >> 8) as u8, y0 as u8, (y1 >> 8) as u8, y1 as u8],
        );
    }

    #[inline]
    pub fn set_brightness(&mut self, brightness: u8) {
        self.cmd(0x02, 0x51, DataMode::Single, &[brightness]);
    }

    pub fn fill_black(&mut self) {
        self.set_window(0, 0, (W - 1) as u16, (H - 1) as u16);
        self.buffer_mut()[..STRIPE_BYTES].fill(0);
        for stripe in 0..H / STRIPE_ROWS {
            self.send_prepared(
                0x32,
                if stripe == 0 { 0x2c } else { 0x3c },
                DataMode::Quad,
                STRIPE_BYTES,
            );
        }
    }

    /// Composite and stream a whole frame.
    ///
    /// The CPU packs stripe N+1 while GDMA is still transmitting stripe N, so
    /// the frame costs roughly max(render, transfer) rather than their sum. The
    /// panel's 80 MHz four-bit link puts the transfer floor near 11.5 ms, which
    /// is what sets the ceiling on frame rate here.
    pub fn present(&mut self, scene: &Scene) {
        self.set_window(0, 0, (W - 1) as u16, (H - 1) as u16);
        let mut spi = self.spi.take().unwrap();
        let mut transmitting = self.tx.take().unwrap();
        let mut drawing = self.spare.take().unwrap();

        gfx::render_stripe(scene, 0, transmitting.as_mut_slice());
        for stripe in 0..H / STRIPE_ROWS {
            let command = if stripe == 0 { 0x2c } else { 0x3c };
            let transfer = spi
                .half_duplex_write(
                    DataMode::Quad,
                    Command::_8Bit(0x32, DataMode::Single),
                    Address::_24Bit((command as u32) << 8, DataMode::Single),
                    0,
                    STRIPE_BYTES,
                    transmitting,
                )
                .unwrap_or_else(|_| panic!());
            if stripe + 1 < H / STRIPE_ROWS {
                gfx::render_stripe(scene, (stripe + 1) * STRIPE_ROWS, drawing.as_mut_slice());
            }
            (spi, transmitting) = transfer.wait();
            core::mem::swap(&mut transmitting, &mut drawing);
        }
        self.spi = Some(spi);
        self.tx = Some(drawing);
        self.spare = Some(transmitting);
    }
}
