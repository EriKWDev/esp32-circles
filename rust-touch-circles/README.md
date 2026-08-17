# Touch Circles (bare-metal Rust)

Minimal, low-latency firmware for the Waveshare ESP32-C6-Touch-AMOLED-2.16.
It uses no RTOS, allocator, LVGL, Wi-Fi, or Bluetooth. Touch is polled at 400 kHz
once per frame, and the SH8601 runs at 80 MHz quad-SPI.

## Install and flash

```powershell
rustup target add riscv32imac-unknown-none-elf
cargo install espflash --locked
cd rust-touch-circles
cargo run --release
```

If more than one serial device is present, use:

```powershell
cargo espflash flash --release --monitor --port COMx
```

(`cargo run --release` uses the `espflash` runner and is normally simpler.) Hold
BOOT, tap RESET, then release BOOT if automatic download mode does not engage.

## Hardware mapping

| Function | Connection |
|---|---|
| SH8601 QSPI clock / D0..D3 / CS | GPIO 0 / 1..4 / 15 |
| CST9217 I2C SDA / SCL / reset | GPIO 8 / 7 / 11 |
| CST9217 address | `0x5A` |
| AXP2101 address / AMOLED rail | `0x34` / ALDO3 |
| Display | 480x480 RGB565 |

The 80 MHz four-bit display link has a theoretical full-frame floor of about
11.5 ms (roughly 86.8 frames/s before command overhead). The renderer uses a single
full-screen address window with DMA stripes and a scanline-span rasterizer. Up to
32 fixed-capacity circles are composited additively, with a
one-pixel antialiased edge, smooth fixed-time growth, and a fade after reaching
the farthest screen corner.

## Tuning

- `STRIPE_ROWS`: lower improves worst-case input latency; higher reduces command
  overhead. Eight is a good latency/throughput balance.
- Growth/fade timing is in `circle_state`.
- `PALETTE` controls circle colors.
- The shortened touch/panel startup waits favor boot speed. If touch is unreliable
  only during very cold power-on, raise the final CST reset delay from 30 to 50 ms.
