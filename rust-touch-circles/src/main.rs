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

mod about;
mod asteroids;
mod audio;
mod breakout;
mod calc;
mod cat;
mod bubbles;
mod currency;
mod display;
mod fetch;
mod font;
mod g2048;
mod gfx;
mod invaders;
mod match3;
mod model;
mod net;
mod pomodoro;
mod rain;
mod pong;
mod simon;
mod snake;
mod spacewar;
mod store;
mod touch;
mod ui;
mod weather;

use esp_hal::{
    delay::Delay,
    dma_tx_buffer,
    gpio::{Level, Output, OutputConfig},
    i2c::master::{Config as I2cConfig, I2c},
    main,
    spi::{
        Mode,
        master::{Config as SpiConfig, Spi},
    },
    time::Rate,
    timer::systimer::{SystemTimer, Unit},
};

use display::Display;
use gfx::{STRIPE_BYTES, Scene};
use model::{Link, State};
use touch::Touch;
use ui::{Action, Ui};

const PMIC_ADDR: u8 = 0x34;

/// How often to re-read `/api/dump`. Only honoured while the UI is idle, so a
/// transfer can never interrupt an animation - see `net`.
const POLL_INTERVAL_MS: u32 = 2_000;
const INFO_POLL_INTERVAL_MS: u32 = 500;
/// While a relay or a queued sequence is running. The handover from one entry to
/// the next is only visible through a poll, and at two seconds that reads as the
/// progress bar sticking and then jumping.
const RUN_POLL_INTERVAL_MS: u32 = 500;
/// Untouched for this long: dim, and draw the quiet outlined look. Still polling,
/// because someone glancing at the panel should see the truth.
const DIM_AFTER_MS: u32 = 45_000;
/// Untouched for this long: stop talking to the network altogether. Nobody is
/// looking, so there is nothing to be current for, and the traffic was what kept
/// the radio awake.
const SLEEP_AFTER_MS: u32 = 180_000;
const DIM_BRIGHTNESS: u8 = 14;
/// Asleep: a glow, so the panel can still be found and the clock read in the dark.
const SLEEP_BRIGHTNESS: u8 = 4;
/// Backlight steps per loop pass, which at frame rate is a fade of about a
/// second either way - the register is free to write, so this costs nothing.
const BRIGHTNESS_STEP: u8 = 6;
/// How long the render thread blocks per pass while asleep. The cost is wake
/// latency on the first touch, which at this length is not perceptible.
const SLEEP_NAP_MS: u32 = 25;

/// Melodies are a handful of notes of differing counts, and there is no allocator
/// to hold them behind one type - so the sizes are spelled out and `as_ref` hands
/// out the slice.
enum Melody {
    One([audio::Tone; 1]),
    Two([audio::Tone; 2]),
    Three([audio::Tone; 3]),
    Four([audio::Tone; 4]),
}

impl AsRef<[audio::Tone]> for Melody {
    fn as_ref(&self) -> &[audio::Tone] {
        match self {
            Melody::One(tones) => tones,
            Melody::Two(tones) => tones,
            Melody::Three(tones) => tones,
            Melody::Four(tones) => tones,
        }
    }
}

// smoltcp needs its storage to outlive the interface. There is no allocator
// budget to spare for this and no StaticCell dependency, so it is plain statics
// handed out exactly once during startup.
static mut SOCKETS: [smoltcp::iface::SocketStorage<'static>; 6] =
    [smoltcp::iface::SocketStorage::EMPTY; 6];
/// Second socket pair, for the apps' fetches: sharing the controller client's
/// would make a weather lookup and a poll wait on each other.
static mut FETCH_RX: [u8; 4096] = [0; 4096];
static mut FETCH_TX: [u8; 512] = [0; 512];
static mut DNS_QUERIES: [Option<smoltcp::socket::dns::DnsQuery>; 2] = [None, None];
static mut NET_RX: [u8; 5120] = [0; 5120];
static mut NET_TX: [u8; 2048] = [0; 2048];
// I2S needs its descriptors and its sample buffer to outlive the driver, same as
// smoltcp's storage above.
static mut I2S_DESC: [esp_hal::dma::DmaDescriptor; 4] =
    [esp_hal::dma::DmaDescriptor::EMPTY; 4];
static mut I2S_BUF: [u8; 2048] = [0; 2048];

/// SYSTIMER runs at 16 MHz on the ESP32-C6.
#[inline]
fn now_ms() -> u32 {
    (SystemTimer::unit_value(Unit::Unit0) / 16_000) as u32
}

#[main]
fn main() -> ! {
    let p =
        esp_hal::init(esp_hal::Config::default().with_cpu_clock(esp_hal::clock::CpuClock::max()));
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
    // Dark before the first pixels land, so boot is a fade-up rather than a
    // flash of half-drawn frame.
    lcd.set_brightness(0);
    lcd.fill_black();

    // Settings come off flash before the radio starts, so the credentials the
    // radio is handed are the saved ones rather than whatever was compiled in.
    let mut store = store::Store::new(p.FLASH);
    let (settings, restored) = store.load();
    // Asked once: the answer cannot change, and it costs an SPI round trip.
    let flash_bytes = store.flash_bytes();
    esp_println::println!(
        "store: restored={} ssid=\"{}\" controllers={}",
        restored as u8,
        settings.ssid.as_str(),
        settings.n_controllers
    );
    if !restored {
        // First boot, or an unreadable sector: persist the compiled-in defaults
        // so the panel has a record of its own from here on.
        match store.save(&settings) {
            Ok(()) => esp_println::println!("store: seeded defaults into flash"),
            Err(e) => esp_println::println!("store: save failed: {e}"),
        }
    }

    // The Wi-Fi blobs need a heap and a scheduler, and the scheduler MUST be
    // started before the radio is initialized. This is why the firmware is no
    // longer strictly RTOS-free: esp-radio requires esp-rtos. The render loop
    // still owns the main thread.
    esp_alloc::heap_allocator!(size: 72 * 1024);
    let timg0 = esp_hal::timer::timg::TimerGroup::new(p.TIMG0);
    let sw_int = esp_hal::interrupt::software::SoftwareInterruptControl::new(p.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_int.software_interrupt0);

    let mut state = State::new();
    seed_mock(&mut state);
    let _ = update_power_state(&mut i2c, &mut state);

    // Bring up the radio. A failure here is not fatal: the panel stays usable on
    // its bench model, with the link dot showing red, which is far better than a
    // dead screen in a shed.
    let mut net = match net::Net::new(
        p.WIFI,
        unsafe { &mut *core::ptr::addr_of_mut!(SOCKETS) },
        unsafe { &mut *core::ptr::addr_of_mut!(NET_RX) },
        unsafe { &mut *core::ptr::addr_of_mut!(NET_TX) },
        unsafe { &mut *core::ptr::addr_of_mut!(FETCH_RX) },
        unsafe { &mut *core::ptr::addr_of_mut!(FETCH_TX) },
        unsafe { &mut *core::ptr::addr_of_mut!(DNS_QUERIES) },
        &settings,
        now_ms(),
    ) {
        Ok(net) => {
            state.link = Link::Connecting;
            Some(net)
        }
        Err(e) => {
            esp_println::println!("wifi init failed: {e}");
            state.link = Link::Offline;
            None
        }
    };

    // Sound. A failure here is not fatal: the panel is an irrigation controller
    // first, and a mute cat is better than no panel.
    let mut audio = audio::Audio::new(
        p.I2S0,
        p.DMA_CH1,
        p.GPIO19.into(),
        p.GPIO20.into(),
        p.GPIO22.into(),
        p.GPIO23.into(),
        unsafe { &mut *core::ptr::addr_of_mut!(I2S_DESC) },
        unsafe { &mut *core::ptr::addr_of_mut!(I2S_BUF) },
        &mut i2c,
    );
    esp_println::println!("audio: {}", if audio.is_some() { "ready" } else { "unavailable" });

    let mut ui = Ui::new();
    // The UI reads persisted settings directly, so give it the loaded copy.
    ui.settings = settings;
    // A reboot must not lose an override in progress: without this the schedules
    // would stay disarmed and nothing would know to put them back.
    ui.rain.restore(
        settings.rain_active,
        settings.rain_armed_mask,
        settings.rain_manual,
    );
    let mut scene = Scene::new();
    let mut touch = Touch::new();

    // First frame while still dark, then ease the backlight up. This uses the
    // panel's brightness register, so it transfers no pixels at all - the fade
    // cannot stutter no matter what the renderer is doing, and it costs nothing.
    ui.build(&mut scene, &state, now_ms());
    lcd.present(&scene);
    {
        const BOOT_FADE_MS: u32 = 200;
        let start = now_ms();
        loop {
            let elapsed = now_ms().wrapping_sub(start);
            if elapsed >= BOOT_FADE_MS {
                break;
            }
            // Smoothstep, so it arrives gently instead of stopping dead.
            let t = elapsed * 32_768 / BOOT_FADE_MS;
            let eased = (((t * t) >> 15) * (3 * 32_768 - 2 * t)) >> 15;
            lcd.set_brightness((eased * 255 / 32_768) as u8);
        }
        lcd.set_brightness(255);
    }

    let mut last_ms = now_ms();
    let mut last_drawn_second = u32::MAX;
    let mut last_touch_ms = 0u32;
    let mut last_beat_ms = now_ms();
    let mut frames = 0u32;
    let mut build_us = 0u32;
    let mut present_us = 0u32;
    let mut last_poll_ms = 0u32;
    let mut last_power_poll_ms = now_ms();
    // Peak primitives in a frame. The scene silently ignores anything past its
    // capacity, which would present as a button that is simply not drawn, so the
    // headroom is worth being able to see.
    let mut peak_prims = 0usize;
    let mut last_input_ms = now_ms();
    let mut dim = false;
    let mut asleep = false;
    // Set when the moon is pressed, cleared when the finger is off the glass.
    // Without it the same contact both sleeps and wakes: the press asks for
    // sleep, and the drag reports that follow it - still the same touch - look
    // like fresh activity and wake the panel straight back up.
    let mut sleep_until_release = false;
    let mut brightness = 255u8;
    let mut dirty = true;

    loop {
        let t = now_ms();
        let dt = t.wrapping_sub(last_ms);
        last_ms = t;

        state.tick(dt);

        // PMIC state changes slowly. Two tiny I2C reads every two seconds keep
        // battery UI current without putting bus traffic in the frame path.
        if t.wrapping_sub(last_power_poll_ms) >= 2_000 {
            last_power_poll_ms = t;
            if update_power_state(&mut i2c, &mut state) {
                dirty = true;
            }
        }

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
        // The contact that asked for sleep stops counting as activity until it
        // ends. Cleared on a pass with no event at all, so the release itself -
        // which is still part of that contact - does not wake the panel either.
        if sleep_until_release && event == touch::Event::None && touch.phase == touch::Phase::Idle {
            sleep_until_release = false;
        }
        if event != touch::Event::None {
            dirty = true;
            if !sleep_until_release {
                last_input_ms = t;
            }
        }
        // The touch that wakes the panel is not a button press. Waking on the
        // press and then acting on it would mean a blind tap on a dim screen
        // could start watering.
        let event = if dim { touch::Event::None } else { event };
        match ui.input(event, &state, t) {
            Action::Trigger {
                relay,
                controller,
                remote_relay,
                seconds,
            } => {
                // Applied locally first so the countdown starts on the same frame
                // as the tap; the controller confirms it on the next poll, and its
                // reply is authoritative if the two ever disagree.
                apply_local_trigger(&mut state, relay, seconds);
                if let Some(net) = net.as_mut() {
                    net.trigger(controller, remote_relay, seconds, t);
                }
                dirty = true;
            }
            Action::Stop => {
                // Everything a run consists of, cleared together. Leaving any of
                // it behind is what produced ghosts: the queue depth kept the
                // panel believing watering was in progress, and a schedule still
                // flagged as running kept drawing its progress bars and claiming
                // the timer page - a stopped schedule haunting the next one until
                // a poll happened to correct it.
                clear_local_run(&mut state);
                if let Some(net) = net.as_mut() {
                    // Stopping is a trigger of zero length's opposite: the API
                    // exposes it as its own endpoint, so ask for the relay the UI
                    // just released.
                    net.stop(t);
                }
                dirty = true;
            }
            // The UI owns the settings and has already changed them; persisting
            // and reconfiguring belongs here, where the flash and the radio are.
            Action::SaveSettings => {
                persist(&mut store, &ui.settings);
                if let Some(net) = net.as_mut() {
                    net.apply_hosts(&ui.settings);
                }
                // Re-poll at the next opportunity rather than up to two seconds
                // later: until fresh data lands, the relay list still describes
                // the old controller table.
                last_poll_ms = t.wrapping_sub(POLL_INTERVAL_MS);
                dirty = true;
            }
            Action::ApplyWifi => {
                persist(&mut store, &ui.settings);
                if let Some(net) = net.as_mut() {
                    match net.apply_wifi(&ui.settings, t) {
                        Ok(()) => state.link = Link::Connecting,
                        Err(e) => {
                            esp_println::println!("wifi reconfigure failed: {e}");
                            state.link = Link::Offline;
                        }
                    }
                }
                dirty = true;
            }
            Action::RunSchedule { controller, start } => {
                // Applied locally first, for the same reason a manual trigger is:
                // the panel should show the schedule you just started, not the one
                // it superseded, on the same frame as the tap. The controller
                // always honours this request - it supersedes rather than refusing
                // - and the poll forced below confirms the details.
                apply_local_schedule(&mut state, controller, start);
                if let Some(net) = net.as_mut() {
                    net.run_schedule(controller, start, t);
                }
                last_poll_ms = t.wrapping_sub(RUN_POLL_INTERVAL_MS);
                dirty = true;
            }
            Action::SaveSchedule => {
                if let Some(net) = net.as_mut() {
                    net.post_schedule(&ui.draft, &state, t);
                }
                // The controller is authoritative on what was actually stored - it
                // clamps durations and skips records it cannot honour - so the
                // draft is not written into the model. A prompt poll shows what it
                // really kept.
                last_poll_ms = t.wrapping_sub(POLL_INTERVAL_MS);
                dirty = true;
            }
            Action::None => {}
        }

        // Pump the stack every pass. This is cheap - it only moves frames that are
        // already queued - and it is what keeps DHCP and TCP alive between the
        // comparatively rare dump requests.
        if let Some(n) = net.as_mut() {
            n.step(t);
            if n.service(&mut state, t) {
                dirty = true;
            }
            state.link = if !n.is_connected() {
                Link::Offline
            } else if n.ip.is_some() {
                Link::Online
            } else {
                Link::Connecting
            };

            // Requests hold off while the user is mid-interaction - a transition,
            // a drag, a tap's ripple - so a transfer cannot stutter something the
            // eye is following. Deliberately NOT gated on `animating()`, which is
            // also true for as long as a relay is running: that stopped the panel
            // polling for the whole of a multi-entry schedule, so the countdown
            // stuck at 00:00 and every progress bar froze after the first entry.
            // A running schedule is when fresh state matters most.
            let poll_interval = if ui.screen == ui::Screen::Info {
                INFO_POLL_INTERVAL_MS
            } else if state.running || state.queued > 0 {
                // Following a sequence: the interesting transitions are the
                // handovers between entries, which a two-second cadence renders
                // as a visible jump.
                RUN_POLL_INTERVAL_MS
            } else {
                POLL_INTERVAL_MS
            };
            // Nothing is fetched while asleep. Nobody is looking, so there is
            // nothing to be up to date for - and a request every so often is what
            // kept the radio awake, defeating the point. Waking polls at once.
            if !asleep
                && !ui.interaction_active()
                && n.ip.is_some()
                && t.wrapping_sub(last_poll_ms) >= poll_interval
            {
                last_poll_ms = t;
                n.poll_dump(&mut state, t);
                dirty = true;
            }
        }

        // Two stages, because they cost different things. Dimming is free to
        // reverse and keeps the panel truthful; going quiet saves the radio but
        // means the display is stale until touched. Any touch returns to awake.
        // The moon button asks for sleep now rather than in three minutes, by
        // backdating the last touch - so there is still exactly one thing that
        // decides the power state.
        if ui.take_sleep_request() {
            last_input_ms = t.wrapping_sub(SLEEP_AFTER_MS);
            sleep_until_release = true;
        }
        // Watering holds the panel awake. A run is the one thing worth watching
        // without touching anything, and a countdown that dims itself away is
        // worse than the battery it saves. The moon is the way out of that.
        let watching = state.running || state.queued > 0 || state.queue_gap;
        let untouched = t.wrapping_sub(last_input_ms);
        let want_dim = untouched >= DIM_AFTER_MS && !watching;
        let want_sleep = untouched >= SLEEP_AFTER_MS && !watching;
        if want_dim != dim || want_sleep != asleep {
            dim = want_dim;
            asleep = want_sleep;
            ui.idle = dim;
            if let Some(n) = net.as_mut() {
                n.set_power_saving(asleep);
            }
            if !asleep {
                // Waking is exactly when the panel is out of date, so ask at once.
                last_poll_ms = t.wrapping_sub(POLL_INTERVAL_MS);
            }
            dirty = true;
        }
        let target = if asleep {
            SLEEP_BRIGHTNESS
        } else if dim {
            DIM_BRIGHTNESS
        } else {
            255
        };
        if brightness != target {
            brightness = if brightness < target {
                brightness.saturating_add(BRIGHTNESS_STEP).min(target)
            } else {
                brightness.saturating_sub(BRIGHTNESS_STEP).max(target)
            };
            lcd.set_brightness(brightness);
        }

        // Sounds are played here rather than where they are decided: a note blocks
        // for its own length, which has no business inside input handling.
        if let Some(sound) = ui.take_sound(t)
            && let Some(audio) = audio.as_mut()
        {
            let melody = match sound {
                ui::Sound::Pad(pad) => Melody::One([audio::Tone {
                    freq: audio::NOTES[pad.min(3)],
                    ms: 190,
                }]),
                ui::Sound::Win => Melody::Four(audio::win()),
                ui::Sound::Lose => Melody::Three(audio::lose()),
                ui::Sound::Purr => Melody::Two(audio::purr()),
                ui::Sound::Meow => Melody::Two(audio::meow()),
            };

            // Unmuted only for the melody, and rendering carries on *inside* the
            // playback loop - the panel used to stall for the length of every note.
            audio.set_mute(&mut i2c, false);
            audio.play_while(melody.as_ref(), || {
                let now = now_ms();
                ui.update(&state, now);
                ui.build(&mut scene, &state, now);
                lcd.present(&scene);
            });
            audio.set_mute(&mut i2c, true);
            // The melody ate real time; do not let the frame clock blame the UI.
            last_ms = now_ms();
            dirty = true;
        }

        // The About page reports on the machine, which only this loop can see.
        // Refreshed while it is up rather than every frame: reading the heap is
        // cheap, but there is no reason to do it sixty times a second.
        if ui.screen == ui::Screen::About {
            ui.sys = about::Sys {
                ip: net.as_ref().and_then(|n| n.ip).map(|ip| ip.octets()),
                heap_free: esp_alloc::HEAP.free() as u32,
                heap_used: esp_alloc::HEAP.used() as u32,
                flash_bytes,
                nvs_offset: store::NVS_OFFSET,
                nvs_sector: store::SECTOR as u32,
                record_len: store.record_len as u32,
            };
        }

        ui.update(&state, t);

        // The rain override runs whatever page is up: it is the only thing here
        // that has to happen at a particular time of day.
        if let Some(n) = net.as_mut() {
            if ui.want_forecast {
                ui.want_forecast = false;
                ui.rain.begin(n, t);
            }
            ui.rain.step(n, &state, t);
            if ui.rain.dirty {
                ui.rain.dirty = false;
                let (active, mask, manual) = ui.rain.saved();
                ui.settings.rain_active = active;
                ui.settings.rain_armed_mask = mask;
                ui.settings.rain_manual = manual;
                persist(&mut store, &ui.settings);
                dirty = true;
            }
        }

        // The network-backed apps fetch for themselves, and only while their page
        // is up.
        if let Some(n) = net.as_mut() {
            if ui.step_apps(n, t) {
                dirty = true;
            }
        }

        // A scan blocks for a few hundred milliseconds, so it happens between
        // frames and only once the transition that asked for it has finished -
        // see Ui::take_scan_request. The "scanning" frame is painted first, so
        // the pause is explained rather than looking like a freeze.
        if ui.take_scan_request() {
            if let Some(n) = net.as_mut() {
                ui.scan_busy = true;
                ui.build(&mut scene, &state, t);
                lcd.present(&scene);
                n.scan(&mut ui.networks);
                ui.scan_busy = false;
                esp_println::println!("wifi scan: {} networks", ui.networks.n);
                // The blocking call consumed the loop's sense of time.
                last_ms = now_ms();
                dirty = true;
            }
        }

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
            // Measured in raw SYSTIMER ticks (16 MHz) so the numbers below are
            // real microseconds, not derived from the millisecond clock.
            let started = SystemTimer::unit_value(Unit::Unit0) as u32;
            ui.build(&mut scene, &state, t);
            let built = SystemTimer::unit_value(Unit::Unit0) as u32;
            lcd.present(&scene);
            let done = SystemTimer::unit_value(Unit::Unit0) as u32;
            peak_prims = peak_prims.max(scene.len);
            build_us += built.wrapping_sub(started) / 16;
            present_us += done.wrapping_sub(built) / 16;
            dirty = false;
            frames += 1;
        } else {
            // Nothing to repaint: *block* the thread rather than busy-waiting.
            //
            // This is the difference between an idle panel that spins the core at
            // full clock and one that spends its time halted. `delay_micros` is a
            // busy loop, so the render thread stayed permanently runnable, the
            // scheduler never reached its idle task - which is a plain
            // `wait_for_interrupt` - and the radio's own tasks only ever got the
            // CPU when the tick preempted us. Blocking here gates the core clock
            // between interrupts and hands the radio the time it needs, which is
            // also what makes Wi-Fi modem sleep viable at all.
            //
            // The sleep is bounded by what still has to feel immediate: touch
            // wakes the panel, so idle can afford a coarse period, while an
            // in-flight request wants the stack pumped promptly.
            let nap_ms = if net.as_ref().is_some_and(|n| n.busy()) {
                2
            } else if asleep {
                SLEEP_NAP_MS
            } else if dim {
                12
            } else {
                4
            };
            esp_rtos::CurrentThreadHandle::get()
                .delay(esp_hal::time::Duration::from_millis(nap_ms as u64));
        }

        // Heartbeat on the serial link. Cheap, once a second, and the fastest way
        // to tell a genuinely wedged loop from a merely static screen - a
        // distinction that is otherwise invisible from the outside.
        if t.wrapping_sub(last_beat_ms) >= 1_000 {
            last_beat_ms = t;
            // build_us is CPU compositing, present_us is compositing overlapped
            // with the DMA transfer. The panel's 80 MHz four-bit link cannot beat
            // ~11.5 ms for a full frame, so present_us at or near that means the
            // transfer is the limit and the CPU is keeping up.
            let n = frames.max(1);
            esp_println::println!(
                "up={}s fps={} build={}us present={}us prims={}/{} screen={} touch={} run={} left={} ip={:?} neterr={:?}",
                t / 1000,
                frames,
                build_us / n,
                present_us / n,
                peak_prims,
                gfx::MAX_PRIMS,
                match ui.screen {
                    ui::Screen::Home => "home",
                    ui::Screen::Inspect => "inspect",
                    ui::Screen::Force => "force",
                    ui::Screen::Running => "run",
                    ui::Screen::Detail => "detail",
                    ui::Screen::Info => "info",
                    ui::Screen::Extras => "extras",
                    ui::Screen::Config => "config",
                    ui::Screen::Wifi => "wifi",
                    ui::Screen::Controller => "controller",
                    ui::Screen::Keyboard => "keyboard",
                    ui::Screen::Connecting => "connecting",
                    ui::Screen::Confirm => "confirm",
                    ui::Screen::Bubbles => "bubbles",
                    ui::Screen::Pong => "pong",
                    ui::Screen::Breakout => "breakout",
                    ui::Screen::Invaders => "invaders",
                    ui::Screen::Asteroids => "asteroids",
                    ui::Screen::Apps => "apps",
                    ui::Screen::G2048 => "2048",
                    ui::Screen::Match3 => "match3",
                    ui::Screen::Pomodoro => "pomodoro",
                    ui::Screen::Simon => "simon",
                    ui::Screen::Cat => "cat",
                    ui::Screen::Calc => "calc",
                    ui::Screen::Weather => "weather",
                    ui::Screen::Currency => "currency",
                    ui::Screen::Snake => "snake",
                    ui::Screen::Spacewar => "spacewar",
                    ui::Screen::About => "about",
                    ui::Screen::Rain => "rain",
                },
                match touch.phase {
                    touch::Phase::Idle => "idle",
                    touch::Phase::Down { .. } => "down",
                },
                state.running as u8,
                state.left_s,
                state.local_ip,
                net.as_ref().and_then(|n| n.last_error),
            );
            frames = 0;
            build_us = 0;
            present_us = 0;
            peak_prims = 0;
        }
    }
}

/// Write settings to flash. A failed write is reported and otherwise tolerated:
/// the change is already live in RAM, and refusing to apply it because it could
/// not be *remembered* would be the worse failure.
fn persist(store: &mut store::Store<'_>, settings: &store::Settings) {
    match store.save(settings) {
        Ok(()) => esp_println::println!(
            "store: saved ssid=\"{}\" controllers={}",
            settings.ssid.as_str(),
            settings.n_controllers
        ),
        Err(e) => esp_println::println!("store: save failed: {e}"),
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
    if i2c
        .write_read(PMIC_ADDR, &[LDO_ONOFF_CTRL0], &mut v)
        .is_ok()
    {
        let nv = if on { v[0] | 4 } else { v[0] & !4 };
        let _ = i2c.write(PMIC_ADDR, &[LDO_ONOFF_CTRL0, nv]);
    }
}

/// Read AXP2101 VBUS presence and its hardware fuel-gauge percentage.
/// Returns whether visible state changed; failed reads retain the last sample.
fn update_power_state(i2c: &mut I2c<'_, esp_hal::Blocking>, state: &mut State) -> bool {
    const STATUS1: u8 = 0x00;
    const BAT_PERCENT: u8 = 0xa4;
    let mut status = [0u8; 2];
    let mut percent = [0u8; 1];
    if i2c.write_read(PMIC_ADDR, &[STATUS1], &mut status).is_err()
        || i2c
            .write_read(PMIC_ADDR, &[BAT_PERCENT], &mut percent)
            .is_err()
    {
        return false;
    }
    // XPowersLib's AXP2101 definitions: VBUS is valid when STATUS1[5]
    // is set and STATUS2[3] is clear; STATUS1[3] reports battery presence.
    let external_power = status[0] & (1 << 5) != 0 && status[1] & (1 << 3) == 0;
    let battery_percent = (status[0] & (1 << 3) != 0 && percent[0] <= 100).then_some(percent[0]);
    let changed =
        state.external_power != external_power || state.battery_percent != battery_percent;
    state.external_power = external_power;
    state.battery_percent = battery_percent;
    changed
}

/// Forget everything about a run in progress.
///
/// One place, because the parts are not independent: `queued` alone keeps
/// `run_is_active` true, and a schedule left flagged as running keeps drawing
/// progress for a run that has ended.
fn clear_local_run(state: &mut State) {
    state.running = false;
    state.active = 0;
    state.left_s = 0;
    state.queued = 0;
    state.queue_gap = false;
    for r in state.relays.iter_mut() {
        r.on = false;
    }
    for s in state.starts.iter_mut() {
        s.running = false;
    }
}

/// Show a schedule as running the moment it is asked for.
///
/// Mirrors `apply_local_trigger`: the first entry becomes the active relay and
/// the rest become the queue, so the countdown and the progress bars are right
/// immediately rather than a poll later. It also transfers ownership away from a
/// schedule that was already running, which is what the controller does too -
/// without this, triggering a second schedule left the first one still claiming
/// the progress display until the next poll landed.
fn apply_local_schedule(state: &mut State, controller: u8, start: u8) {
    let Some(index) = state.starts[..state.n_starts]
        .iter()
        .position(|s| s.controller == controller && s.remote_id == start)
    else {
        return;
    };
    let schedule = state.starts[index];
    if schedule.n_entries == 0 {
        return;
    }
    // Order matters: apply_local_trigger clears schedule ownership, since a
    // manual run has none. Claim it afterwards, not before.
    let first = schedule.entries[0];
    apply_local_trigger(state, first.relay, first.seconds as u32);
    state.starts[index].running = true;
    state.queued = schedule.n_entries as u32 - 1;
}

fn apply_local_trigger(state: &mut State, relay: u8, seconds: u32) {
    // Mirrors the controller's own rule: starting a relay drops whatever was
    // running, so at most one is ever on - and a schedule that was running is no
    // longer the owner of anything.
    for r in state.relays.iter_mut() {
        r.on = false;
    }
    for s in state.starts.iter_mut() {
        s.running = false;
    }
    state.queued = 0;
    state.queue_gap = false;
    if let Some(r) = state.relays[..state.n_relays]
        .iter_mut()
        .find(|r| r.id == relay)
    {
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
            remote_id: *id,
            controller: 0,
            port: *port,
            enabled: true,
            on: false,
            name: Text::new(name),
        };
    }
    state.n_relays = NAMES.len();

    let mut first = StartTime {
        id: 1,
        remote_id: 1,
        controller: 0,
        running: false,
        enabled: true,
        hh: 6,
        mm: 0,
        entries: [Entry {
            relay: 0,
            seconds: 0,
        }; model::MAX_ENTRIES],
        n_entries: 0,
        gap_s: 0,
    };
    for (index, seconds) in [300u16, 240, 180, 240, 300].iter().enumerate() {
        first.entries[index] = Entry {
            relay: index as u8 + 1,
            seconds: *seconds,
        };
        first.n_entries += 1;
    }
    state.starts[0] = first;
    state.starts[1] = StartTime {
        id: 2,
        remote_id: 2,
        controller: 0,
        running: false,
        enabled: false,
        hh: 12,
        mm: 0,
        entries: [Entry {
            relay: 0,
            seconds: 0,
        }; model::MAX_ENTRIES],
        n_entries: 0,
        gap_s: 0,
    };
    state.starts[2] = StartTime {
        id: 3,
        remote_id: 3,
        controller: 0,
        running: false,
        enabled: true,
        hh: 21,
        mm: 30,
        entries: [Entry {
            relay: 2,
            seconds: 600,
        }; model::MAX_ENTRIES],
        n_entries: 2,
        gap_s: 0,
    };
    state.n_starts = 3;

    state.analogs[0] = model::Analog {
        // I1 is the input this installation actually has a sensor on, and it is
        // the first record the controller reports, so it is the default readout.
        port: 1,
        level: 1017,
        name: Text::new("I1"),
    };
    state.n_analogs = 1;

    // A plausible fake clock is worse than no clock. The first successful
    // /api/dump seeds the real controller time, then State::tick advances it.
    state.clock_valid = false;
    state.link = Link::Connecting;
    state.max_run_s = 3600;
}
