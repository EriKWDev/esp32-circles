#![no_std]
#![no_main]

use esp_backtrace as _;
esp_bootloader_esp_idf::esp_app_desc!();
use esp_hal::{
    delay::Delay,
    dma::DmaTxBuf,
    dma_tx_buffer,
    gpio::{Event, Input, InputConfig, Level, Output, OutputConfig, Pull},
    i2c::master::{Config as I2cConfig, I2c},
    main,
    spi::{
        Mode,
        master::{Address, Command, Config as SpiConfig, DataMode, Spi, SpiDma},
    },
    time::{Duration, Rate},
    timer::systimer::{SystemTimer, Unit},
};

const W: usize = 480;
const H: usize = 480;
// Two 32-row buffers keep GDMA busy while the CPU packs the following stripe.
const STRIPE_ROWS: usize = 32;
const STRIPE_BYTES: usize = W * STRIPE_ROWS * 2;
const FRAME_BYTES: usize = W * H * 3 / 2;
const TILE_W: usize = 16;
const TILE_H: usize = STRIPE_ROWS;
const TILES_X: usize = W / TILE_W;
const TILES_Y: usize = H / TILE_H;
const ALL_TILE_COLUMNS: u32 = (1u32 << TILES_X) - 1;
const MAX_CIRCLES: usize = 32;
const TOUCH_ADDR: u8 = 0x5a;
const PMIC_ADDR: u8 = 0x34;
const FRAME_TICKS: u32 = 16_000_000 / 60;

static FRAMEBUFFER: static_cell::StaticCell<[u8; FRAME_BYTES]> = static_cell::StaticCell::new();

const PALETTE: [(u8, u8, u8); 8] = [
    (255, 55, 125), (80, 210, 255), (255, 185, 45), (135, 80, 255),
    (45, 255, 170), (255, 90, 45), (70, 125, 255), (240, 70, 235),
];

#[derive(Clone, Copy)]
struct Circle {
    live: bool,
    x: i32,
    y: i32,
    born_ticks: u32,
    full_radius_q4: u32,
    palette_index: u8,
    drawn_radius_q4: u32,
    drawn_alpha: u8,
}

const EMPTY: Circle = Circle {
    live: false, x: 0, y: 0, born_ticks: 0, full_radius_q4: 0,
    palette_index: 0, drawn_radius_q4: 0, drawn_alpha: 0,
};

struct Scene {
    circles: [Circle; MAX_CIRCLES],
    next_color: usize,
    was_down: bool,
    release_started_ticks: u32,
    last_spawn_x: u16,
    last_spawn_y: u16,
    last_spawn_ticks: u32,
}

impl Scene {
    fn new() -> Self {
        Self {
            circles: [EMPTY; MAX_CIRCLES],
            next_color: 0,
            was_down: false,
            release_started_ticks: 0,
            last_spawn_x: u16::MAX,
            last_spawn_y: u16::MAX,
            last_spawn_ticks: 0,
        }
    }

    fn press(&mut self, x: u16, y: u16, now_ticks: u32) -> bool {
        // The interrupt is a report pulse, so suppress repeated packets by
        // position. A new contact elsewhere must not wait for global rearming.
        const SAME_TOUCH_RADIUS_SQUARED: u32 = 48 * 48;
        const SAME_TOUCH_TICKS: u32 = 16_000_000 * 180 / 1_000;
        if self.was_down && now_ticks.wrapping_sub(self.last_spawn_ticks) < SAME_TOUCH_TICKS {
            let dx = x.abs_diff(self.last_spawn_x) as u32;
            let dy = y.abs_diff(self.last_spawn_y) as u32;
            if dx * dx + dy * dy <= SAME_TOUCH_RADIUS_SQUARED { return false; }
        }
        self.was_down = true;
        if let Some(slot) = self.circles.iter_mut().find(|c| !c.live) {
            let x = (x as i32).clamp(0, (W - 1) as i32);
            let y = (y as i32).clamp(0, (H - 1) as i32);
            let dx = x.max((W - 1) as i32 - x) as u32;
            let dy = y.max((H - 1) as i32 - y) as u32;
            let far = isqrt(dx * dx + dy * dy) + 2;
            *slot = Circle {
                live: true, x, y, born_ticks: now_ticks,
                full_radius_q4: far << 4,
                palette_index: (self.next_color % PALETTE.len()) as u8,
                drawn_radius_q4: 0,
                drawn_alpha: 255,
            };
            self.next_color += 1;
            self.last_spawn_x = x as u16;
            self.last_spawn_y = y as u16;
            self.last_spawn_ticks = now_ticks;
            return true;
        }
        false
    }

    fn observe_touch_line(&mut self, is_low: bool, now_ticks: u32) {
        /*
            NOTE: CST9220 GPIO5 is a report pulse, not a stable contact level.
                  Global contact state re-arms after 160 ms without a report;
                  spatially distinct contacts bypass that wait in `press`.
        */
        const RELEASE_TICKS: u32 = 16_000_000 * 160 / 1_000;

        if is_low {
            self.release_started_ticks = 0;
        } else if self.was_down {
            if self.release_started_ticks == 0 {
                self.release_started_ticks = now_ticks;
            } else if now_ticks.wrapping_sub(self.release_started_ticks) >= RELEASE_TICKS {
                self.was_down = false;
                self.release_started_ticks = 0;
            }
        }
    }

}

// Center-out radial motion with cubic ease-out. Once the circle reaches the
// farthest corner, its opacity follows a smoothstep fade instead of a linear ramp.
#[inline(always)]
fn circle_state(c: Circle, now_ticks: u32) -> (u32, u32, u8) {
    let age_ticks = now_ticks.wrapping_sub(c.born_ticks);
    // Convert once to bounded u32 milliseconds so RV32 uses its hardware DIVU;
    // u64 division otherwise calls the slow software __udivdi3 routine.
    let age_ms = age_ticks / 16_000;
    const GROW_MS: u32 = 700;
    const FADE_MS: u32 = 520;
    if age_ms < GROW_MS {
        // Q15 keeps every product within u32, avoiding RV32 multiword maths.
        let t = age_ms * 32_768 / GROW_MS;
        let t2 = t * t >> 15;
        let eased = t2 * (3 * 32_768 - 2 * t) >> 15;
        return (c.full_radius_q4 * eased >> 15, 16, 255);
    }
    let fade_age = age_ms - GROW_MS;
    if fade_age >= FADE_MS { return (c.full_radius_q4, 16, 0); }
    let t = fade_age * 32_768 / FADE_MS;
    let t2 = t * t >> 15;
    let smooth = t2 * (3 * 32_768 - 2 * t) >> 15;
    let alpha = ((32_768 - smooth) * 255 >> 15) as u8;
    (c.full_radius_q4, 16, alpha)
}

#[main]
fn main() -> ! {
    let p = esp_hal::init(esp_hal::Config::default().with_cpu_clock(esp_hal::clock::CpuClock::max()));
    let delay = Delay::new();

    let mut i2c = I2c::new(p.I2C0, I2cConfig::default().with_frequency(Rate::from_khz(400)))
        .unwrap().with_sda(p.GPIO8).with_scl(p.GPIO7);

    // AXP2101 ALDO3 is the AMOLED reset/power rail. Preserve unrelated bits.
    pmic_set_aldo3(&mut i2c, false);
    delay.delay_millis(10);
    pmic_set_aldo3(&mut i2c, true);
    delay.delay_millis(10);

    // CST9220 reset. The vendor example waits 600 ms; 10/10/30 ms is enough in
    // practice and removes most boot latency. Increase the final delay if a board
    // revision proves marginal at cold temperature.
    let mut touch_rst = Output::new(p.GPIO11, Level::High, OutputConfig::default());
    delay.delay_millis(2);
    touch_rst.set_low();
    delay.delay_millis(10);
    touch_rst.set_high();
    delay.delay_millis(30);
    let mut touch_int = Input::new(p.GPIO5, InputConfig::default().with_pull(Pull::Up));
    touch_int.listen(Event::FallingEdge);

    let transmit_buffer = dma_tx_buffer!(STRIPE_BYTES).unwrap();
    let packing_buffer = dma_tx_buffer!(STRIPE_BYTES).unwrap();
    let spi = Spi::new(
        p.SPI2,
        SpiConfig::default().with_frequency(Rate::from_mhz(80)).with_mode(Mode::_0),
    ).unwrap()
        .with_sck(p.GPIO0)
        .with_sio0(p.GPIO1)
        .with_sio1(p.GPIO2)
        .with_sio2(p.GPIO3)
        .with_sio3(p.GPIO4)
        .with_cs(p.GPIO15)
        .with_dma(p.DMA_CH0);
    let mut lcd = Lcd::new(spi, transmit_buffer, packing_buffer);

    init_lcd(&mut lcd, &delay);
    let mut scene = Scene::new();
    let frame = FRAMEBUFFER.init_with(|| [0u8; FRAME_BYTES]);
    let mut dirty = [0u32; TILES_Y];
    let mut next_frame = fast_ticks();

    // Establish a known black GRAM once. Subsequent growth is partial-update only.
    set_window(&mut lcd, 0, 0, (W - 1) as u16, (H - 1) as u16);
    lcd.buffer_mut()[..STRIPE_BYTES].fill(0);
    for stripe in 0..TILES_Y {
        lcd.send_prepared(0x32, if stripe == 0 { 0x2c } else { 0x3c }, DataMode::Quad, STRIPE_BYTES);
    }

    loop {
        let now_ticks = fast_ticks();
        if sample_touch(&mut i2c, &mut touch_int, &mut scene, now_ticks) {
            next_frame = now_ticks;
        }
        if (now_ticks.wrapping_sub(next_frame) as i32) < 0 { continue; }
        next_frame = next_frame.wrapping_add(FRAME_TICKS);
        // If a frame ever overruns, resume from now instead of producing a burst
        // of obsolete catch-up frames.
        if now_ticks.wrapping_sub(next_frame) < 0x8000_0000 && now_ticks.wrapping_sub(next_frame) > FRAME_TICKS {
            next_frame = now_ticks.wrapping_add(FRAME_TICKS);
        }
        let fade = update_circles(&mut scene, now_ticks, frame, &mut dirty);
        flush_dirty(&mut lcd, frame, &mut dirty, fade.as_ref());
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

fn sample_touch(i2c: &mut I2c<'_, esp_hal::Blocking>, touch_int: &mut Input<'_>, scene: &mut Scene, now_ticks: u32) -> bool {
    let touch_line_is_low = touch_int.is_low();
    scene.observe_touch_line(touch_line_is_low, now_ticks);
    if !touch_line_is_low && !touch_int.is_interrupt_set() { return false; }
    touch_int.clear_interrupt();
    let mut d = [0u8; 10];
    if i2c.write_read(TOUCH_ADDR, &[0xd0, 0x00], &mut d).is_ok()
        && d[6] == 0xab && d[5] & 0x7f != 0 && d[0] & 0x0f == 0x06
    {
        let y = ((d[1] as u16) << 4) | ((d[3] as u16) >> 4);
        let raw_x = ((d[2] as u16) << 4) | ((d[3] as u16) & 0x0f);
        return scene.press(480u16.saturating_sub(raw_x), y, now_ticks);
    }
    false
}

fn init_lcd<S>(spi: &mut S, delay: &Delay)
where S: LcdBus {
    // CO5300 vendor sequence, with the data-sheet-mandated sleep-out wait.
    const INIT: &[(u8, &[u8], u32)] = &[
        (0x11, &[], 120), (0xfe, &[0x20], 0), (0x19, &[0x10], 0),
        (0x1c, &[0xa0], 0), (0xfe, &[0x00], 0), (0xc4, &[0x80], 0),
        (0x3a, &[0x55], 0), (0x35, &[0x00], 0), (0x36, &[0x30], 0),
        (0x53, &[0x20], 0), (0x51, &[0xff], 0), (0x63, &[0xff], 0),
        (0x2a, &[0x00, 0x00, 0x01, 0xdf], 0),
        (0x2b, &[0x00, 0x00, 0x01, 0xdf], 0), (0x29, &[], 20),
    ];
    for &(cmd, data, wait) in INIT {
        spi.send(0x02, cmd, DataMode::Single, data);
        if wait != 0 { delay.delay(Duration::from_millis(wait as u64)); }
    }
}

trait LcdBus {
    fn send(&mut self, opcode: u8, command: u8, mode: DataMode, bytes: &[u8]);
}

struct Lcd<'d> {
    spi: Option<SpiDma<'d, esp_hal::Blocking>>,
    tx: Option<DmaTxBuf>,
    spare: Option<DmaTxBuf>,
}

impl<'d> Lcd<'d> {
    fn new(spi: SpiDma<'d, esp_hal::Blocking>, tx: DmaTxBuf, spare: DmaTxBuf) -> Self {
        Self { spi: Some(spi), tx: Some(tx), spare: Some(spare) }
    }

    #[inline]
    fn buffer_mut(&mut self) -> &mut [u8] { self.tx.as_mut().unwrap().as_mut_slice() }

    #[inline]
    fn send_prepared(&mut self, opcode: u8, command: u8, mode: DataMode, len: usize) {
        let spi = self.spi.take().unwrap();
        let tx = self.tx.take().unwrap();
        let transfer = spi.half_duplex_write(
            mode,
            Command::_8Bit(opcode as u16, DataMode::Single),
            Address::_24Bit((command as u32) << 8, DataMode::Single),
            0,
            len,
            tx,
        ).unwrap_or_else(|_| panic!());
        let (spi, tx) = transfer.wait();
        self.spi = Some(spi);
        self.tx = Some(tx);
    }
}

impl LcdBus for Lcd<'_> {
    #[inline]
    fn send(&mut self, opcode: u8, command: u8, mode: DataMode, bytes: &[u8]) {
        self.buffer_mut()[..bytes.len()].copy_from_slice(bytes);
        self.send_prepared(opcode, command, mode, bytes.len());
    }
}

#[inline]
fn qspi<S: LcdBus>(spi: &mut S, opcode: u8, cmd: u8, mode: DataMode, data: &[u8]) {
    spi.send(opcode, cmd, mode, data);
}

fn set_window<S: LcdBus>(spi: &mut S, x0: u16, y0: u16, x1: u16, y1: u16) {
    qspi(spi, 0x02, 0x2a, DataMode::Single, &[(x0 >> 8) as u8, x0 as u8, (x1 >> 8) as u8, x1 as u8]);
    qspi(spi, 0x02, 0x2b, DataMode::Single, &[(y0 >> 8) as u8, y0 as u8, (y1 >> 8) as u8, y1 as u8]);
}

fn update_circles(
    scene: &mut Scene,
    now_ticks: u32,
    frame: &mut [u8; FRAME_BYTES],
    dirty: &mut [u32; TILES_Y],
) -> Option<[[u8; 16]; 3]> {
    let mut combined = [[0u8; 16]; 3];
    for channel in &mut combined { for (i, v) in channel.iter_mut().enumerate() { *v = i as u8; } }
    let mut has_fade = false;
    for c in &mut scene.circles {
        if !c.live { continue; }
        let (radius, _, alpha) = circle_state(*c, now_ticks);
        if radius > c.drawn_radius_q4 {
            draw_annulus(frame, dirty, *c, c.drawn_radius_q4, radius);
            c.drawn_radius_q4 = radius;
        }
        if radius >= c.full_radius_q4 && alpha != c.drawn_alpha {
            let maps = fade_maps(PALETTE[c.palette_index as usize], c.drawn_alpha, alpha);
            for ch in 0..3 {
                for value in 0..16 {
                    combined[ch][value] = maps[ch][combined[ch][value] as usize];
                }
            }
            has_fade = true;
            c.drawn_alpha = alpha;
        }
        if alpha == 0 { c.live = false; }
    }
    if has_fade && !scene.circles.iter().any(|circle| circle.live) {
        frame.fill(0);
        dirty.fill(ALL_TILE_COLUMNS);
        None
    } else if has_fade {
        Some(combined)
    } else {
        None
    }
}

fn draw_annulus(
    frame: &mut [u8; FRAME_BYTES],
    dirty: &mut [u32; TILES_Y],
    c: Circle,
    old_r: u32,
    new_r: u32,
) {
    let new2 = new_r * new_r;
    let old2 = old_r * old_r;
    let y0 = (c.y - ((new_r as i32 + 15) >> 4) - 1).clamp(0, (H - 1) as i32);
    let y1 = (c.y + ((new_r as i32 + 15) >> 4) + 1).clamp(0, (H - 1) as i32);
    for y in y0..=y1 {
        let dy = (y - c.y).unsigned_abs() << 4;
        if dy > new_r + 8 { continue; }
        let dy2 = dy * dy;
        let ne = isqrt(new2.saturating_sub(dy2)) as i32;
        let oe = if old_r != 0 && dy <= old_r + 8 { isqrt(old2.saturating_sub(dy2)) as i32 } else { -16 };
        let nl = (c.x << 4) - ne; let nr = (c.x << 4) + ne;
        let ol = (c.x << 4) - oe; let or = (c.x << 4) + oe;
        let x0 = ((nl - 8) >> 4).clamp(0, (W - 1) as i32);
        let x1 = ((nr + 8) >> 4).clamp(0, (W - 1) as i32);
        if oe < 0 {
            draw_annulus_range(frame, dirty, c, y, x0, x1, nl, nr, ol, or, false);
        } else {
            // Visit only the two changed side intervals. The already-filled
            // middle is neither tested nor touched.
            let left_end = ((ol + 8) >> 4).clamp(x0, x1);
            let right_start = ((or - 8) >> 4).clamp(x0, x1);
            if right_start <= left_end + 1 {
                draw_annulus_range(frame, dirty, c, y, x0, x1, nl, nr, ol, or, true);
            } else {
                draw_annulus_range(frame, dirty, c, y, x0, left_end, nl, nr, ol, or, true);
                draw_annulus_range(frame, dirty, c, y, right_start, x1, nl, nr, ol, or, true);
            }
        }
    }
}

#[inline(always)]
fn draw_annulus_range(
    frame: &mut [u8; FRAME_BYTES], dirty: &mut [u32; TILES_Y], c: Circle,
    y: i32, x0: i32, x1: i32, nl: i32, nr: i32, ol: i32, or: i32, has_old: bool,
) {
    let first_tile = x0 as usize / TILE_W;
    let last_tile = x1 as usize / TILE_W;
    let column_count = last_tile - first_tile + 1;
    dirty[y as usize / TILE_H] |= ((1u32 << column_count) - 1) << first_tile;
    let mut x = x0;
    while x <= x1 {
        let px = x << 4;
        let new_inside = (px - nl).min(nr - px);
        let old_inside = if has_old { (px - ol).min(or - px) } else { -16 };
        let nc = (new_inside + 8).clamp(0, 16) as u16;
        let oc = (old_inside + 8).clamp(0, 16) as u16;
        if nc <= oc {
            x += 1;
            continue;
        }
        if oc == 0 && nc == 16 && x & 1 == 0 && x < x1 {
            let next_px = (x + 1) << 4;
            let next_new_inside = (next_px - nl).min(nr - next_px);
            let next_old_inside = if has_old { (next_px - ol).min(or - next_px) } else { -16 };
            let next_new_coverage = (next_new_inside + 8).clamp(0, 16) as usize;
            let next_old_coverage = (next_old_inside + 8).clamp(0, 16) as usize;
            if next_old_coverage == 0 && next_new_coverage == 16 {
                blend_two_full_pixels(frame, x as usize, y as usize, c.palette_index as usize);
                x += 2;
                continue;
            }
        }

        blend_pixel(frame, x as usize, y as usize, c.palette_index as usize, oc as u8, nc as u8);
        x += 1;
    }
}

#[inline(always)]
fn blend_two_full_pixels(frame: &mut [u8], x: usize, y: usize, palette_index: usize) {
    let byte = (y * W + x) * 3 / 2;
    let first = ((frame[byte] as usize) << 4) | (frame[byte + 1] as usize >> 4);
    let second = (((frame[byte + 1] as usize) & 15) << 8) | frame[byte + 2] as usize;
    let transform = &PALETTE_SCREEN_TRANSFORMS[palette_index];
    let first = transform[first];
    let second = transform[second];

    frame[byte] = (first >> 4) as u8;
    frame[byte + 1] = ((first as u8 & 15) << 4) | ((second >> 8) as u8 & 15);
    frame[byte + 2] = second as u8;
}

#[inline(always)]
fn blend_pixel(
    frame: &mut [u8], x: usize, y: usize, palette_index: usize,
    old_coverage: u8, new_coverage: u8,
) {
    let pi = y * W + x;
    let old = get_rgb444(frame, pi);
    let color = PALETTE[palette_index];
    let nr = transition_component((old >> 8) & 15, color.0 >> 4, old_coverage, new_coverage);
    let ng = transition_component((old >> 4) & 15, color.1 >> 4, old_coverage, new_coverage);
    let nb = transition_component(old & 15, color.2 >> 4, old_coverage, new_coverage);
    set_rgb444(frame, pi, (nr << 8) | (ng << 4) | nb);
}

#[inline(always)]
fn transition_component(shown: u16, source: u8, old_coverage: u8, new_coverage: u8) -> u16 {
    let old_source = (source as usize * old_coverage as usize + 8) >> 4;
    let new_source = (source as usize * new_coverage as usize + 8) >> 4;
    COMPONENT_TRANSITIONS[old_source][new_source][shown as usize] as u16
}

const fn make_component_transitions() -> [[[u8; 16]; 16]; 16] {
    let mut table = [[[0u8; 16]; 16]; 16];
    let mut old_source = 0;
    while old_source < 16 {
        let mut new_source = 0;
        while new_source < 16 {
            let mut shown = 0;
            while shown < 16 {
                let mut underlying = 0;
                let mut best_error = 16;
                let mut candidate = 0;
                while candidate < 16 {
                    let reconstructed = SCREEN4[old_source * 16 + candidate] as usize;
                    let error = reconstructed.abs_diff(shown);
                    if error < best_error { best_error = error; underlying = candidate; }
                    candidate += 1;
                }
                table[old_source][new_source][shown] = SCREEN4[new_source * 16 + underlying];
                shown += 1;
            }
            new_source += 1;
        }
        old_source += 1;
    }
    table
}

const COMPONENT_TRANSITIONS: [[[u8; 16]; 16]; 16] = make_component_transitions();

fn fade_maps(color: (u8, u8, u8), old_a: u8, new_a: u8) -> [[u8; 16]; 3] {
    [fade_map(color.0, old_a, new_a), fade_map(color.1, old_a, new_a), fade_map(color.2, old_a, new_a)]
}

fn fade_map(channel: u8, old_a: u8, new_a: u8) -> [u8; 16] {
    let os = (mul_255(channel as u16, old_a as u16) >> 4) as usize;
    let ns = (mul_255(channel as u16, new_a as u16) >> 4) as usize;
    let mut map = [0u8; 16];
    for shown in 0..16 {
        let mut base = 0usize; let mut best = 16usize;
        for candidate in 0..16 {
            let v = SCREEN4[os * 16 + candidate] as usize;
            let err = v.abs_diff(shown);
            if err < best { best = err; base = candidate; }
        }
        map[shown] = SCREEN4[ns * 16 + base];
    }
    map
}

#[inline(always)]
fn mul_255(a: u16, b: u16) -> u16 {
    let x = a * b + 128;
    (x + (x >> 8)) >> 8
}

#[inline(always)]
const fn make_screen4() -> [u8; 256] {
    let mut t = [0u8; 256]; let mut src = 0usize;
    while src < 16 { let mut old = 0usize; while old < 16 {
        t[src * 16 + old] = (old + src - old * src / 15) as u8; old += 1;
    } src += 1; } t
}

const SCREEN4: [u8; 256] = make_screen4();

const fn make_palette_screen_transforms() -> [[u16; 4096]; PALETTE.len()] {
    let mut transforms = [[0u16; 4096]; PALETTE.len()];
    let mut palette_index = 0;

    while palette_index < PALETTE.len() {
        let color = PALETTE[palette_index];
        let source_red = (color.0 >> 4) as usize;
        let source_green = (color.1 >> 4) as usize;
        let source_blue = (color.2 >> 4) as usize;
        let mut destination = 0;

        while destination < 4096 {
            let red = SCREEN4[source_red * 16 + ((destination >> 8) & 15)] as u16;
            let green = SCREEN4[source_green * 16 + ((destination >> 4) & 15)] as u16;
            let blue = SCREEN4[source_blue * 16 + (destination & 15)] as u16;
            transforms[palette_index][destination] = (red << 8) | (green << 4) | blue;
            destination += 1;
        }
        palette_index += 1;
    }

    transforms
}

const PALETTE_SCREEN_TRANSFORMS: [[u16; 4096]; PALETTE.len()] = make_palette_screen_transforms();

#[inline(always)]
fn fast_ticks() -> u32 {
    // ESP32-C6 SYSTIMER runs at 16 MHz. Animation lifetimes are under two
    // seconds, so wrapping low-32-bit subtraction is exact for our intervals.
    SystemTimer::unit_value(Unit::Unit0) as u32
}

const fn make_rgb444_to_565() -> [u16; 4096] {
    let mut t = [0u16; 4096]; let mut c = 0usize;
    while c < 4096 {
        let r4 = ((c >> 8) & 15) as u16; let g4 = ((c >> 4) & 15) as u16; let b4 = (c & 15) as u16;
        t[c] = (((r4 << 1) | (r4 >> 3)) << 11)
            | (((g4 << 2) | (g4 >> 2)) << 5)
            | ((b4 << 1) | (b4 >> 3));
        c += 1;
    }
    t
}

const RGB444_TO_565: [u16; 4096] = make_rgb444_to_565();

#[inline(always)]
fn get_rgb444(buf: &[u8], pixel: usize) -> u16 {
    let i = (pixel >> 1) * 3;
    if pixel & 1 == 0 {
        ((buf[i] as u16) << 4) | ((buf[i + 1] as u16) >> 4)
    } else {
        (((buf[i + 1] as u16) & 15) << 8) | buf[i + 2] as u16
    }
}

#[inline(always)]
fn set_rgb444(buf: &mut [u8], pixel: usize, color: u16) {
    let i = (pixel >> 1) * 3;
    if pixel & 1 == 0 {
        buf[i] = (color >> 4) as u8;
        buf[i + 1] = (buf[i + 1] & 15) | ((color as u8 & 15) << 4);
    } else {
        buf[i + 1] = (buf[i + 1] & 0xf0) | ((color >> 8) as u8 & 15);
        buf[i + 2] = color as u8;
    }
}

fn flush_dirty(
    lcd: &mut Lcd<'_>,
    frame: &mut [u8; FRAME_BYTES],
    dirty: &mut [u32; TILES_Y],
    fade: Option<&[[u8; 16]; 3]>,
) {
    // A fade necessarily changes the whole image. Transform packed RGB444 and
    // emit RGB565 in the same cache pass, instead of walking 230,400 pixels
    // twice. The counter rectangle is deliberately excluded from scene fades.
    if let Some(maps) = fade {
        set_window(lcd, 0, 0, (W - 1) as u16, (H - 1) as u16);
        stream_full_screen(lcd, frame, Some(maps));
        dirty.fill(0);
        return;
    }
    let dirty_tile_count: usize = dirty.iter().map(|row| row.count_ones() as usize).sum();
    let dirty_run_count: usize = dirty
        .iter()
        .map(|row| (row & !(row << 1)).count_ones() as usize)
        .sum();

    /*
        NOTE: A partial run costs two address-window DMA transactions plus its
              pixel transaction. Once sparse traffic reaches this threshold,
              one pipelined full-screen stream finishes sooner despite sending
              unchanged pixels. This also bounds multi-circle frame time.
    */
    let partial_pixel_cost = dirty_tile_count * TILE_W * TILE_H;
    let transaction_cost_in_pixels = dirty_run_count * 768;
    let full_screen_is_cheaper = partial_pixel_cost + transaction_cost_in_pixels >= W * H;

    if full_screen_is_cheaper {
        set_window(lcd, 0, 0, (W - 1) as u16, (H - 1) as u16);
        stream_full_screen(lcd, frame, None);
        dirty.fill(0);
        return;
    }
    for ty in 0..TILES_Y {
        let mut columns = dirty[ty];
        while columns != 0 {
            let start = columns.trailing_zeros() as usize;
            let shifted = columns >> start;
            let width_tiles = (!shifted).trailing_zeros().min((TILES_X - start) as u32) as usize;
            let end = start + width_tiles;
            let x0 = start * TILE_W; let width = (end - start) * TILE_W;
            let y0 = ty * TILE_H;
            let n = pack_rect(frame, x0, y0, width, lcd.buffer_mut());
            set_window(lcd, x0 as u16, y0 as u16, (x0 + width - 1) as u16, (y0 + TILE_H - 1) as u16);
            lcd.send_prepared(0x32, 0x2c, DataMode::Quad, n);
            let run_mask = ((1u32 << width_tiles) - 1) << start;
            columns &= !run_mask;
        }
    }
    dirty.fill(0);
}

fn stream_full_screen(
    lcd: &mut Lcd<'_>,
    frame: &mut [u8; FRAME_BYTES],
    fade_maps: Option<&[[u8; 16]; 3]>,
) {
    let mut spi = lcd.spi.take().unwrap();
    let mut transmitting = lcd.tx.take().unwrap();
    let mut packing = lcd.spare.take().unwrap();

    pack_screen_stripe(frame, 0, fade_maps, transmitting.as_mut_slice());

    for stripe in 0..H / STRIPE_ROWS {
        let command = if stripe == 0 { 0x2c } else { 0x3c };
        let transfer = spi.half_duplex_write(
            DataMode::Quad,
            Command::_8Bit(0x32, DataMode::Single),
            Address::_24Bit((command as u32) << 8, DataMode::Single),
            0,
            STRIPE_BYTES,
            transmitting,
        ).unwrap_or_else(|_| panic!());

        if stripe + 1 < H / STRIPE_ROWS {
            pack_screen_stripe(
                frame,
                (stripe + 1) * STRIPE_ROWS,
                fade_maps,
                packing.as_mut_slice(),
            );
        }

        (spi, transmitting) = transfer.wait();
        core::mem::swap(&mut transmitting, &mut packing);
    }

    lcd.spi = Some(spi);
    lcd.tx = Some(packing);
    lcd.spare = Some(transmitting);
}

#[inline]
fn pack_screen_stripe(
    frame: &mut [u8; FRAME_BYTES],
    y0: usize,
    fade_maps: Option<&[[u8; 16]; 3]>,
    pixels: &mut [u8],
) {
    if let Some(maps) = fade_maps {
        pack_mapped_stripe(frame, y0, maps, pixels);
    } else {
        pack_rect(frame, 0, y0, W, pixels);
    }
}

#[inline(always)]
fn map_rgb444(c: u16, maps: &[[u8; 16]; 3]) -> u16 {
    ((maps[0][((c >> 8) & 15) as usize] as u16) << 8)
        | ((maps[1][((c >> 4) & 15) as usize] as u16) << 4)
        | maps[2][(c & 15) as usize] as u16
}

#[inline]
fn pack_mapped_stripe(
    frame: &mut [u8; FRAME_BYTES], y0: usize, maps: &[[u8; 16]; 3], pixels: &mut [u8],
) {
    let mut n = 0;
    for y in y0..y0 + STRIPE_ROWS {
        let mut bi = y * W * 3 / 2;
        for _ in (0..W).step_by(2) {
            let b0 = frame[bi]; let b1 = frame[bi + 1]; let b2 = frame[bi + 2];
            let mut c0 = ((b0 as u16) << 4) | (b1 as u16 >> 4);
            let mut c1 = (((b1 as u16) & 15) << 8) | b2 as u16;
            c0 = map_rgb444(c0, maps);
            c1 = map_rgb444(c1, maps);
            frame[bi] = (c0 >> 4) as u8;
            frame[bi + 1] = ((c0 as u8 & 15) << 4) | ((c1 >> 8) as u8 & 15);
            frame[bi + 2] = c1 as u8;
            let p0 = RGB444_TO_565[c0 as usize]; let p1 = RGB444_TO_565[c1 as usize];
            pixels[n] = (p0 >> 8) as u8; pixels[n + 1] = p0 as u8;
            pixels[n + 2] = (p1 >> 8) as u8; pixels[n + 3] = p1 as u8;
            n += 4; bi += 3;
        }
    }
}

#[inline]
fn pack_rect(frame: &[u8; FRAME_BYTES], x0: usize, y0: usize, width: usize, pixels: &mut [u8]) -> usize {
    let mut n = 0;
    for y in y0..y0 + TILE_H {
        let mut bi = (y * W + x0) * 3 / 2;
        for _ in (0..width).step_by(2) {
            let b0 = frame[bi]; let b1 = frame[bi + 1]; let b2 = frame[bi + 2];
            let c0 = ((b0 as usize) << 4) | (b1 as usize >> 4);
            let c1 = (((b1 as usize) & 15) << 8) | b2 as usize;
            let p0 = RGB444_TO_565[c0]; let p1 = RGB444_TO_565[c1];
            pixels[n] = (p0 >> 8) as u8; pixels[n + 1] = p0 as u8;
            pixels[n + 2] = (p1 >> 8) as u8; pixels[n + 3] = p1 as u8;
            n += 4; bi += 3;
        }
    }
    n
}

#[inline]
fn isqrt(mut n: u32) -> u32 {
    let mut res = 0u32; let mut bit = 1u32 << 30;
    while bit > n { bit >>= 2; }
    while bit != 0 {
        if n >= res + bit { n -= res + bit; res = (res >> 1) + bit; }
        else { res >>= 1; }
        bit >>= 2;
    }
    res
}
