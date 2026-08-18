#![no_std]
#![no_main]

//! Rainbird irrigation panel for the Waveshare ESP32-C6-Touch-AMOLED-2.16.
//!
//! Bare metal: no RTOS, no allocator, no LVGL. The frame is composited straight
//! into DMA stripe buffers (see `gfx`/`display`) and the UI is a plain state
//! machine (see `ui`), which is what keeps transitions at panel rate.
//!
//! It talks to the irrigation controller's `/api/dump` and `/api/trigger`
//! endpoints - a line-oriented `key:value` format chosen so a device like this
//! can parse it with `sscanf`-grade code and no JSON (see `model`).

use esp_backtrace as _;
esp_bootloader_esp_idf::esp_app_desc!();

mod display;
mod font;
mod gfx;
mod model;
mod touch;
mod ui;

use esp_hal::{
    delay::Delay,
    dma_tx_buffer,
    gpio::{Level, Output, OutputConfig},
    i2c::master::{Config as I2cConfig, I2c},
    main,
    spi::{
        master::{Config as SpiConfig, Spi},
        Mode,
    },
    time::Rate,
    timer::systimer::{SystemTimer, Unit},
};

use display::Display;
use gfx::{Scene, STRIPE_BYTES};
use model::{Link, State};
use touch::Touch;
use ui::{Action, Ui};

const PMIC_ADDR: u8 = 0x34;

/// SYSTIMER runs at 16 MHz on the ESP32-C6.
#[inline]
fn now_ms() -> u32 {
    (SystemTimer::unit_value(Unit::Unit0) / 16_000) as u32
}

#[main]
fn main() -> ! {
    let p = esp_hal::init(
        esp_hal::Config::default().with_cpu_clock(esp_hal::clock::CpuClock::max()),
    );
    let delay = Delay::new();

    let mut i2c = I2c::new(
        p.I2C0,
        I2cConfig::default().with_frequency(Rate::from_khz(400)),
    )
    .unwrap()
    .with_sda(p.GPIO8)
    .with_scl(p.GPIO7);

    // AXP2101 ALDO3 is the AMOLED reset/power rail. Preserve unrelated bits.
    pmic_set_aldo3(&mut i2c, false);
    delay.delay_millis(10);
    pmic_set_aldo3(&mut i2c, true);
    delay.delay_millis(10);

    // CST9220 reset.
    let mut touch_rst = Output::new(p.GPIO11, Level::High, OutputConfig::default());
    delay.delay_millis(2);
    touch_rst.set_low();
    delay.delay_millis(10);
    touch_rst.set_high();
    delay.delay_millis(30);

    let transmit_buffer = dma_tx_buffer!(STRIPE_BYTES).unwrap();
    let packing_buffer = dma_tx_buffer!(STRIPE_BYTES).unwrap();
    let spi = Spi::new(
        p.SPI2,
        SpiConfig::default()
            .with_frequency(Rate::from_mhz(80))
            .with_mode(Mode::_0),
    )
    .unwrap()
    .with_sck(p.GPIO0)
    .with_sio0(p.GPIO1)
    .with_sio1(p.GPIO2)
    .with_sio2(p.GPIO3)
    .with_sio3(p.GPIO4)
    .with_cs(p.GPIO15)
    .with_dma(p.DMA_CH0);

    let mut lcd = Display::new(spi, transmit_buffer, packing_buffer);
    lcd.init(&delay);
    lcd.fill_black();

    let mut state = State::new();
    seed_mock(&mut state);

    let mut ui = Ui::new();
    let mut scene = Scene::new();
    let mut touch = Touch::new();

    let mut last_ms = now_ms();
    let mut last_drawn_second = u32::MAX;
    let mut last_touch_ms = 0u32;
    let mut last_beat_ms = now_ms();
    let mut frames = 0u32;
    let mut dirty = true;

    loop {
        let t = now_ms();
        let dt = t.wrapping_sub(last_ms);
        last_ms = t;

        state.tick(dt);

        // Touch is polled on a fixed cadence rather than every spin of the loop.
        // The demo could gate its reads on the CST9220 interrupt line because it
        // only cared that a contact happened; tracking a drag needs position
        // while the finger is held, which the interrupt does not give. Polling
        // unthrottled, though, means an I2C transaction every few microseconds
        // when the UI is idle - far more traffic than the controller expects, and
        // pointless besides. TOUCH_POLL_MS is comfortably faster than a finger.
        const TOUCH_POLL_MS: u32 = 8;
        let mut event = touch::Event::None;
        if t.wrapping_sub(last_touch_ms) >= TOUCH_POLL_MS {
            last_touch_ms = t;
            event = touch.poll(&mut i2c, t);
        }
        if event != touch::Event::None {
            dirty = true;
        }
        match ui.input(event, &state, t) {
            Action::Trigger { relay, seconds } => {
                // Until the network layer lands, apply locally so the countdown
                // and the single-active-relay behaviour can be exercised on the
                // bench exactly as they will behave against the controller.
                apply_local_trigger(&mut state, relay, seconds);
                dirty = true;
            }
            Action::Stop => {
                state.running = false;
                state.active = 0;
                state.left_s = 0;
                for r in state.relays.iter_mut() {
                    r.on = false;
                }
                dirty = true;
            }
            Action::None => {}
        }

        ui.update(&state, t);

        // Redraw when something is moving, when the visible clock or countdown
        // has ticked, or when input arrived. Idling without repainting keeps the
        // panel quiet and leaves the whole frame budget to animations.
        let second = state.ss as u32 + state.mm as u32 * 60 + state.hh as u32 * 3600;
        if second != last_drawn_second {
            last_drawn_second = second;
            dirty = true;
        }
        if ui.animating() {
            dirty = true;
        }

        if dirty {
            ui.build(&mut scene, &state, t);
            lcd.present(&scene);
            dirty = false;
            frames += 1;
        } else {
            // Nothing to repaint: yield a little rather than spinning flat out.
            delay.delay_micros(600);
        }

        // Heartbeat on the serial link. Cheap, once a second, and the fastest way
        // to tell a genuinely wedged loop from a merely static screen - a
        // distinction that is otherwise invisible from the outside.
        if t.wrapping_sub(last_beat_ms) >= 1_000 {
            last_beat_ms = t;
            esp_println::println!(
                "up={}s fps={} screen={} touch={} run={} left={}",
                t / 1000,
                frames,
                match ui.screen {
                    ui::Screen::Home => "home",
                    ui::Screen::Inspect => "inspect",
                    ui::Screen::Force => "force",
                    ui::Screen::Running => "run",
                },
                match touch.phase {
                    touch::Phase::Idle => "idle",
                    touch::Phase::Down { .. } => "down",
                },
                state.running as u8,
                state.left_s,
            );
            frames = 0;
        }
    }
}

fn pmic_set_aldo3(i2c: &mut I2c<'_, esp_hal::Blocking>, on: bool) {
    const LDO_ONOFF_CTRL0: u8 = 0x90;
    const LDO_VOL2_CTRL: u8 = 0x94;
    let mut v = [0u8];
    if on {
        // 3.3 V = (3300 - 500) / 100 = 28, retain ALDO4 bits.
        if i2c.write_read(PMIC_ADDR, &[LDO_VOL2_CTRL], &mut v).is_ok() {
            let _ = i2c.write(PMIC_ADDR, &[LDO_VOL2_CTRL, (v[0] & 0xe0) | 28]);
        }
    }
    if i2c.write_read(PMIC_ADDR, &[LDO_ONOFF_CTRL0], &mut v).is_ok() {
        let nv = if on { v[0] | 4 } else { v[0] & !4 };
        let _ = i2c.write(PMIC_ADDR, &[LDO_ONOFF_CTRL0, nv]);
    }
}

fn apply_local_trigger(state: &mut State, relay: u8, seconds: u32) {
    // Mirrors the controller's own rule: starting a relay drops whatever was
    // running, so at most one is ever on.
    for r in state.relays.iter_mut() {
        r.on = false;
    }
    if let Some(r) = state.relays[..state.n_relays].iter_mut().find(|r| r.id == relay) {
        r.on = true;
    }
    state.running = true;
    state.active = relay as i32;
    state.left_s = seconds.min(state.max_run_s);
}

/// Bench data mirroring the live controller's configuration, so layout and
/// legibility can be judged with real Swedish relay names (which is what
/// stresses the text: åäö, spaces, and names long enough to need truncation).
fn seed_mock(state: &mut State) {
    use gfx::Text;
    use model::{Entry, Relay, StartTime};

    const NAMES: [(&str, u8, u8); 5] = [
        ("dammen", 1, 1),
        ("gräsmatta", 2, 20),
        ("grannen", 3, 21),
        ("mot vägen", 4, 22),
        ("mot havet", 5, 23),
    ];
    for (index, (name, id, port)) in NAMES.iter().enumerate() {
        state.relays[index] = Relay {
            id: *id,
            port: *port,
            enabled: true,
            on: false,
            name: Text::new(name),
        };
    }
    state.n_relays = NAMES.len();

    let mut first = StartTime {
        id: 1,
        enabled: true,
        hh: 6,
        mm: 0,
        entries: [Entry { relay: 0, seconds: 0 }; model::MAX_ENTRIES],
        n_entries: 0,
    };
    for (index, seconds) in [300u16, 240, 180, 240, 300].iter().enumerate() {
        first.entries[index] = Entry { relay: index as u8 + 1, seconds: *seconds };
        first.n_entries += 1;
    }
    state.starts[0] = first;
    state.starts[1] = StartTime {
        id: 2,
        enabled: false,
        hh: 12,
        mm: 0,
        entries: [Entry { relay: 0, seconds: 0 }; model::MAX_ENTRIES],
        n_entries: 0,
    };
    state.starts[2] = StartTime {
        id: 3,
        enabled: true,
        hh: 21,
        mm: 30,
        entries: [Entry { relay: 2, seconds: 600 }; model::MAX_ENTRIES],
        n_entries: 2,
    };
    state.n_starts = 3;

    state.analogs[0] = model::Analog {
        port: 2,
        level: 1018,
        name: Text::new("I2"),
    };
    state.n_analogs = 1;

    state.hh = 14;
    state.mm = 32;
    state.ss = 0;
    state.clock_valid = true;
    state.link = Link::Connecting;
    state.max_run_s = 3600;
}
