# Touch Circles (bare-metal Rust)

Minimal, low-latency firmware for the Waveshare ESP32-C6-Touch-AMOLED-2.16.
It uses no RTOS, allocator, LVGL, Wi-Fi, or Bluetooth. Touch is read over a
400 kHz I2C link, and the CO5300 panel runs at 80 MHz quad-SPI using SPI2/GDMA.

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
| CO5300 QSPI clock / D0..D3 / CS | GPIO 0 / 1..4 / 15 |
| CST9220 I2C SDA / SCL / reset / interrupt | GPIO 8 / 7 / 11 / 5 |
| CST9220 address | `0x5A` |
| AXP2101 address / AMOLED rail | `0x34` / ALDO3 |
| Display | 480x480 RGB565 |

The 80 MHz four-bit display link has a theoretical full-frame floor of about
11.5 ms (roughly 86.8 frames/s before command and packing overhead). The panel
cannot accept RGB332 over its QSPI interface, so RGB565 is the minimum transfer
format. A sole growing circle updates only 16x32 tiles crossed by its annulus;
untouched pixels remain in CO5300 GRAM. Fragmented or layered scenes switch to a
full-screen address window with ping-ponged DMA stripes, while sole full-screen
fades use the panel brightness command and transfer no pixels. Up to 32
fixed-capacity circles are composited, with a
three-pixel luminous same-hue front on the newest bubble, a dithered antialiased
edge, smooth fixed-time growth, and a fade after reaching the farthest screen
corner. The front reuses the existing scanline extent—there is no second square
root, geometry pass, alpha buffer, or additional display traffic.

## Tuning

- `STRIPE_ROWS`: higher reduces transaction overhead but consumes twice its size
  in SRAM for ping-pong DMA. The current 32-row setting leaves about 34 KiB stack.
- Growth/fade timing is in `circle_state`.
- `PALETTE` controls circle colors.
- The shortened touch/panel startup waits favor boot speed. If touch is unreliable
  only during very cold power-on, raise the final CST reset delay from 30 to 50 ms.
