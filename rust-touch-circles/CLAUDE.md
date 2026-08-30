# The panel

ESP32-C6 with a 480×480 AMOLED, a touch panel and no operating system. It drives
an irrigation controller (an Axis device running the Rainbird ACAP) and carries a
pile of small apps and games besides.

    cargo build --release
    espflash flash --chip esp32c6 --port COM4 --non-interactive \
        target/riscv32imac-unknown-none-elf/release/touch-circles
    espflash monitor --port COM4 --non-interactive     # prints a heartbeat a second

`wifi.txt` holds the Wi-Fi and controller credentials and is **gitignored** — keep
secrets there, never in source. `build.rs` bakes it in, along with the fonts and
the build stamp.

## Hardware on the I²C bus

    0x18  ES8311 audio codec
    0x34  AXP2101 PMIC (battery, external power)
    0x51  PCF85063 RTC - present, and not yet used by anything
    0x5a  CST9220 touch
    0x6b  6-axis IMU, QMI8658 by address - confirm WHO_AM_I before trusting it

The boot log prints what actually answers (`i2c:` line). Ask the hardware rather
than assuming; the board documentation has not always been to hand.

## Things that have bitten, more than once

**The scene holds a fixed number of primitives** (`MAX_PRIMS` in `gfx.rs`, 208).
Anything past the cap is silently dropped, so overflow looks like half a maze or a
stack missing its top rather than an error. Long runs of one colour are merged
into single pills — see `tetris.rs`, `snake.rs`, `pacman.rs`. The heartbeat prints
`prims=n/max`; check it after adding a busy page.

**Adding a `Screen` means wiring eight places**, and three of them have catch-all
arms so the compiler will not tell you:

    background()      accent()        is_menu()        scroll_slot()
    menu_total()      geom()          menu_target()    the draw dispatch

Miss `menu_total` and the page draws nothing. Miss `geom` and it uses another
page's row geometry. Miss `menu_target` and its rows open the settings pages.
All three happened. A grep for an existing neighbour screen finds them all.

**Never let two Wi-Fi connects be in flight.** The driver returns
`ESP_ERR_WIFI_CONN` (12295) and esp-radio's error table does not map it, so it
*panics* rather than returning it. Same for starting a scan while an association
is outstanding. If a panic ever says "Unknown error code", this is the family.

**Do not busy-wait on the radio.** A poll loop with no yield starves the driver
tasks the future is waiting for; it made scans fail and modem sleep impossible.
`block_on_deadline` yields between polls for this reason.

**Sizing a board from the screen and then pushing it below a header** puts it off
the bottom. Compute the geometry, then check the arithmetic before flashing —
2048, match-3, Simon and chess all shipped off-screen once.

**Fonts are baked at build time from per-font charsets.** A glyph not in the
charset draws as nothing, silently: the calculator's ×, ÷, √ and π were invisible,
and so was every accented letter in a foreign town name. Add characters to
`build.rs` and check the reported glyph count rises.

## Verify on the host where you can

The device is slow to flash and hard to observe. Pure logic can be checked on a
laptop in seconds, and it has repeatedly found real bugs:

- chess move generation against published **perft** counts — which caught
  promotion generating only queens;
- every opening-book line replayed through the generator — which caught eight
  plies with black castling written as `e1g8`;
- the tetromino tables rendered as text — 28 rotations, four cells each;
- Pac-Man's movement driven with jittered frame times and random reversals —
  which caught the ball passing through walls after a mid-cell turn.

`scratchpad/ct/` has the pattern: copy the module, stub `gfx`/`font`, drive it
with `rustc`.

## When something misbehaves on the device

Capture the serial log while it happens rather than reasoning about it. Three
rounds of plausible timeout fixes did not find the Wi-Fi crash; one capture named
it in a minute. The panic handler prints to the same link the heartbeat uses.

## Where things live

`ui.rs` owns every screen and the navigation stack; `net.rs` the Wi-Fi, the
controller client and the app fetches; `store.rs` the flash record — which is
versioned and **only ever appends**, so older records still read. `docs/` in the
*rainbird* repo (the controller) describes the API between the two, including
which sources the panel fetches directly because they are plain HTTP.
