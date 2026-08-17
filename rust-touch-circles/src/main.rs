#![no_std]
#![no_main]

use esp_backtrace as _;
#[cfg(feature = "profile")]
use esp_println::println;
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
const TILE_W: usize = 16;
const TILE_H: usize = STRIPE_ROWS;
const TILES_X: usize = W / TILE_W;
const TILES_Y: usize = H / TILE_H;
const MAX_CIRCLES: usize = 32;
const RIM_PIXELS: i16 = 3;
const TOUCH_ADDR: u8 = 0x5a;
const PMIC_ADDR: u8 = 0x34;
const FRAME_TICKS: u32 = 16_000_000 / 60;

const PALETTE: [(u8, u8, u8); 8] = [
    (255, 55, 125), (80, 210, 255), (255, 185, 45), (135, 80, 255),
    (45, 255, 170), (255, 90, 45), (70, 125, 255), (240, 70, 235),
];

#[derive(Clone, Copy)]
struct Circle {
    x: i32,
    y: i32,
    born_ticks: u32,
    full_radius_q4: u32,
    palette_index: u8,
    drawn_radius_q4: u32,
}

const EMPTY: Circle = Circle {
    x: 0, y: 0, born_ticks: 0, full_radius_q4: 0, palette_index: 0, drawn_radius_q4: 0,
};

struct Scene {
    circles: [Circle; MAX_CIRCLES],
    len: usize,
    next_color: usize,
}

#[derive(Clone, Copy)]
struct RenderCircle {
    center_x_q4: i32,
    center_y_q4: i32,
    radius_q4: u32,
    previous_radius_q4: u32,
    radius_squared_q8: u32,
    rgb565_pair: u32,
    rim565_pair: u32,
}

impl Scene {
    fn new() -> Self {
        Self {
            circles: [EMPTY; MAX_CIRCLES],
            len: 0,
            next_color: 0,
        }
    }

    fn press(&mut self, x: u16, y: u16, now_ticks: u32) -> bool {
        // CST9220 reports are pulses rather than stable down/up state. Reject
        // only reports belonging to a circle already alive at this location;
        // contacts elsewhere never share a timer or global lockout.
        const SAME_ORIGIN_RADIUS_SQUARED: u32 = 28 * 28;
        for circle in &self.circles[..self.len] {
            let dx = (x as i32 - circle.x).unsigned_abs();
            let dy = (y as i32 - circle.y).unsigned_abs();
            if dx * dx + dy * dy <= SAME_ORIGIN_RADIUS_SQUARED { return false; }
        }
        if self.len < MAX_CIRCLES {
            let x = (x as i32).clamp(0, (W - 1) as i32);
            let y = (y as i32).clamp(0, (H - 1) as i32);
            let dx = x.max((W - 1) as i32 - x) as u32;
            let dy = y.max((H - 1) as i32 - y) as u32;
            let far = isqrt(dx * dx + dy * dy) + 2;
            self.circles[self.len] = Circle {
                x, y, born_ticks: now_ticks,
                full_radius_q4: far << 4,
                palette_index: (self.next_color % PALETTE.len()) as u8,
                drawn_radius_q4: 0,
            };
            self.len += 1;
            self.next_color += 1;
            return true;
        }
        false
    }
}

#[inline(always)]
fn smoothstep_q15(elapsed: u32, duration: u32) -> u32 {
    let t = elapsed * 32_768 / duration;
    let squared = t * t >> 15;
    squared * (3 * 32_768 - 2 * t) >> 15
}

// Center-out radial motion with cubic ease-out. Once the circle reaches the
// farthest corner, its opacity follows a smoothstep fade instead of a linear ramp.
#[inline(always)]
fn circle_state(c: Circle, now_ticks: u32) -> (u32, u8) {
    let age_ticks = now_ticks.wrapping_sub(c.born_ticks);
    // Convert once to bounded u32 milliseconds so RV32 uses its hardware DIVU;
    // u64 division otherwise calls the slow software __udivdi3 routine.
    let age_ms = age_ticks / 16_000;
    const GROW_MS: u32 = 700;
    const FADE_MS: u32 = 520;
    if age_ms < GROW_MS {
        // Q15 keeps every product within u32, avoiding RV32 multiword maths.
        let eased = smoothstep_q15(age_ms, GROW_MS);
        return (c.full_radius_q4 * eased >> 15, 255);
    }
    let fade_age = age_ms - GROW_MS;
    if fade_age >= FADE_MS { return (c.full_radius_q4, 0); }
    let smooth = smoothstep_q15(fade_age, FADE_MS);
    let alpha = ((32_768 - smooth) * 255 >> 15) as u8;
    (c.full_radius_q4, alpha)
}

fn prepare_render_circles(scene: &mut Scene, now_ticks: u32) -> ([RenderCircle; MAX_CIRCLES], usize) {
    const EMPTY_RENDER: RenderCircle = RenderCircle {
        center_x_q4: 0, center_y_q4: 0, radius_q4: 0, previous_radius_q4: 0,
        radius_squared_q8: 0, rgb565_pair: 0, rim565_pair: 0,
    };
    let mut rendered = [EMPTY_RENDER; MAX_CIRCLES];
    let mut alive = 0;
    let mut newest_full = None;

    for read in 0..scene.len {
        let circle = scene.circles[read];
        let (radius_q4, alpha) = circle_state(circle, now_ticks);
        if alpha == 0 { continue; }
        if radius_q4 >= circle.full_radius_q4 { newest_full = Some(alive); }
        let color = PALETTE[circle.palette_index as usize];
        let red = mul_255(color.0 as u16, alpha as u16) as u8;
        let green = mul_255(color.1 as u16, alpha as u16) as u8;
        let blue = mul_255(color.2 as u16, alpha as u16) as u8;
        let rgb565 = rgb888_to_565(red, green, blue);
        let rim565 = rgb888_to_565(
            mul_255(color.0 as u16 + ((255 - color.0 as u16) * 5 >> 3), alpha as u16) as u8,
            mul_255(color.1 as u16 + ((255 - color.1 as u16) * 5 >> 3), alpha as u16) as u8,
            mul_255(color.2 as u16 + ((255 - color.2 as u16) * 5 >> 3), alpha as u16) as u8,
        );
        let high = (rgb565 >> 8) as u8;
        let low = rgb565 as u8;
        let mut updated_circle = circle;
        updated_circle.drawn_radius_q4 = radius_q4;
        scene.circles[alive] = updated_circle;
        rendered[alive] = RenderCircle {
            center_x_q4: circle.x << 4,
            center_y_q4: circle.y << 4,
            radius_q4,
            previous_radius_q4: circle.drawn_radius_q4,
            radius_squared_q8: radius_q4 * radius_q4,
            rgb565_pair: u32::from_le_bytes([high, low, high, low]),
            rim565_pair: pack_rgb565_pair(rim565),
        };
        alive += 1;
    }
    scene.len = alive;

    if let Some(first_visible) = newest_full {
        let retained = alive - first_visible;
        scene.circles.copy_within(first_visible..alive, 0);
        scene.len = retained;
        rendered.copy_within(first_visible..alive, 0);
        alive = retained;
    }
    (rendered, alive)
}

#[inline(always)]
const fn rgb888_to_565(red: u8, green: u8, blue: u8) -> u16 {
    ((red as u16 & 0xf8) << 8) | ((green as u16 & 0xfc) << 3) | (blue as u16 >> 3)
}

#[inline(always)]
const fn pack_rgb565_pair(rgb565: u16) -> u32 {
    let high = (rgb565 >> 8) as u8;
    let low = rgb565 as u8;
    u32::from_le_bytes([high, low, high, low])
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
    let mut next_frame = fast_ticks();
    let mut scene_was_visible = false;
    let mut hardware_fade_active = false;
    let mut selective_scene_is_valid = true;
    #[cfg(feature = "profile")]
    let mut profile_frames = 0u32;

    // Establish a known black GRAM once. Subsequent growth is partial-update only.
    set_window(&mut lcd, 0, 0, (W - 1) as u16, (H - 1) as u16);
    lcd.buffer_mut()[..STRIPE_BYTES].fill(0);
    for stripe in 0..H / STRIPE_ROWS {
        lcd.send_prepared(0x32, if stripe == 0 { 0x2c } else { 0x3c }, DataMode::Quad, STRIPE_BYTES);
    }
    #[cfg(feature = "profile")]
    profile_renderer(&mut lcd, &delay);

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
        let sole_full_circle_alpha = if scene.len == 1 {
            let circle = scene.circles[0];
            let (radius, alpha) = circle_state(circle, now_ticks);
            (radius >= circle.full_radius_q4 && alpha != 0).then_some(alpha)
        } else {
            None
        };

        if let Some(alpha) = sole_full_circle_alpha {
            if !hardware_fade_active {
                let (render_circles, render_count) = prepare_render_circles(&mut scene, now_ticks);
                set_window(&mut lcd, 0, 0, (W - 1) as u16, (H - 1) as u16);
                stream_opaque_scene(&mut lcd, &render_circles[..render_count]);
                hardware_fade_active = true;
            }
            set_brightness(&mut lcd, alpha);
            scene_was_visible = true;
            continue;
        }

        let (render_circles, render_count) = prepare_render_circles(&mut scene, now_ticks);
        if render_count != 0 || scene_was_visible {
            if hardware_fade_active && render_count == 0 { set_brightness(&mut lcd, 0); }
            #[cfg(feature = "profile")]
            let frame_started = fast_ticks();
            if selective_scene_is_valid && render_count == 1 {
                if render_circles[0].radius_q4 > render_circles[0].previous_radius_q4 {
                    stream_single_circle_tiles(&mut lcd, render_circles[0]);
                }
            } else {
                set_window(&mut lcd, 0, 0, (W - 1) as u16, (H - 1) as u16);
                stream_opaque_scene(&mut lcd, &render_circles[..render_count]);
            }
            #[cfg(feature = "profile")]
            {
                let elapsed_ticks = fast_ticks().wrapping_sub(frame_started);
                profile_frames += 1;
                if profile_frames == 15 {
                    println!("circles={} frame_us={}", render_count, elapsed_ticks / 16);
                    profile_frames = 0;
                }
            }
            if hardware_fade_active { set_brightness(&mut lcd, 255); }
        }
        if render_count > 1 { selective_scene_is_valid = false; }
        if render_count == 0 { selective_scene_is_valid = true; }
        hardware_fade_active = false;
        scene_was_visible = render_count != 0;
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

#[inline]
fn set_brightness<S: LcdBus>(spi: &mut S, brightness: u8) {
    qspi(spi, 0x02, 0x51, DataMode::Single, &[brightness]);
}

#[cfg(feature = "profile")]
fn profile_renderer(lcd: &mut Lcd<'_>, delay: &Delay) {
    const EMPTY_RENDER: RenderCircle = RenderCircle {
        center_x_q4: 0, center_y_q4: 0, radius_q4: 0, previous_radius_q4: 0,
        radius_squared_q8: 0, rgb565_pair: 0, rim565_pair: 0,
    };
    let mut circles = [EMPTY_RENDER; MAX_CIRCLES];
    delay.delay_millis(5_000);
    for &count in &[1usize, 2, 4, 8, 16, 32] {
        for (index, circle) in circles[..count].iter_mut().enumerate() {
            let x = 48 + (index * 83 % 384) as i32;
            let y = 48 + (index * 137 % 384) as i32;
            let radius_q4 = ((96 + index % 6 * 24) as u32) << 4;
            let color = rgb888_to_565(
                48 + (index * 71 % 208) as u8,
                48 + (index * 43 % 208) as u8,
                48 + (index * 97 % 208) as u8,
            );
            let high = (color >> 8) as u8;
            let low = color as u8;
            *circle = RenderCircle {
                center_x_q4: x << 4,
                center_y_q4: y << 4,
                radius_q4,
                previous_radius_q4: 0,
                radius_squared_q8: radius_q4 * radius_q4,
                rgb565_pair: u32::from_le_bytes([high, low, high, low]),
                rim565_pair: u32::from_le_bytes([high, low, high, low]),
            };
        }
        set_window(lcd, 0, 0, (W - 1) as u16, (H - 1) as u16);
        let started = fast_ticks();
        stream_opaque_scene(lcd, &circles[..count]);
        println!("benchmark circles={} frame_us={}", count, fast_ticks().wrapping_sub(started) / 16);
    }
    let mut previous = 0u32;
    for &radius in &[48u32, 120, 240, 360] {
        let radius_q4 = radius << 4;
        let mut circle = circles[0];
        circle.center_x_q4 = 240 << 4;
        circle.center_y_q4 = 240 << 4;
        circle.previous_radius_q4 = previous;
        circle.radius_q4 = radius_q4;
        circle.radius_squared_q8 = radius_q4 * radius_q4;
        let started = fast_ticks();
        stream_single_circle_tiles(lcd, circle);
        println!("selective radius={} frame_us={}", radius, fast_ticks().wrapping_sub(started) / 16);
        previous = radius_q4;
    }
    delay.delay_millis(2_000);
}

#[inline(always)]
fn mul_255(a: u16, b: u16) -> u16 {
    let x = a * b + 128;
    (x + (x >> 8)) >> 8
}

#[inline(always)]
fn fast_ticks() -> u32 {
    // ESP32-C6 SYSTIMER runs at 16 MHz. Animation lifetimes are under two
    // seconds, so wrapping low-32-bit subtraction is exact for our intervals.
    SystemTimer::unit_value(Unit::Unit0) as u32
}

fn stream_single_circle_tiles(lcd: &mut Lcd<'_>, circle: RenderCircle) {
    let mut dirty_rows = [0u32; TILES_Y];
    let outer_radius = circle.radius_q4 + 8;
    let inner_radius = circle.previous_radius_q4.saturating_sub(8);
    let outer_squared = outer_radius * outer_radius;
    let inner_squared = inner_radius * inner_radius;
    let center_x = circle.center_x_q4 >> 4;
    let center_y = circle.center_y_q4 >> 4;
    for tile_y in 0..TILES_Y {
        let top = (tile_y * TILE_H) as i32;
        let bottom = top + TILE_H as i32 - 1;
        let near_y = if center_y < top { top - center_y } else if center_y > bottom { center_y - bottom } else { 0 };
        let far_y = (center_y - top).unsigned_abs().max((center_y - bottom).unsigned_abs());
        for tile_x in 0..TILES_X {
            let left = (tile_x * TILE_W) as i32;
            let right = left + TILE_W as i32 - 1;
            let near_x = if center_x < left { left - center_x } else if center_x > right { center_x - right } else { 0 };
            let far_x = (center_x - left).unsigned_abs().max((center_x - right).unsigned_abs());
            let nx = (near_x as u32) << 4;
            let ny = (near_y as u32) << 4;
            let fx = far_x << 4;
            let fy = far_y << 4;
            if nx * nx + ny * ny <= outer_squared && fx * fx + fy * fy >= inner_squared {
                dirty_rows[tile_y] |= 1 << tile_x;
            }
        }
    }
    let tile_count: usize = dirty_rows.iter().map(|row| row.count_ones() as usize).sum();
    let run_count: usize = dirty_rows.iter().map(|row| (row & !(row << 1)).count_ones() as usize).sum();
    if tile_count * TILE_W * TILE_H + run_count * 768 >= W * H {
        set_window(lcd, 0, 0, (W - 1) as u16, (H - 1) as u16);
        stream_opaque_scene(lcd, core::slice::from_ref(&circle));
        return;
    }
    for (tile_y, &row_mask) in dirty_rows.iter().enumerate() {
        let mut remaining = row_mask;
        while remaining != 0 {
            let first_tile = remaining.trailing_zeros() as usize;
            let shifted = remaining >> first_tile;
            let tiles = (!shifted).trailing_zeros().min((TILES_X - first_tile) as u32) as usize;
            let x0 = first_tile * TILE_W;
            let width = tiles * TILE_W;
            let y0 = tile_y * TILE_H;
            set_window(lcd, x0 as u16, y0 as u16, (x0 + width - 1) as u16, (y0 + TILE_H - 1) as u16);
            render_single_circle_rect(circle, x0, y0, width, TILE_H, lcd.buffer_mut());
            lcd.send_prepared(0x32, 0x2c, DataMode::Quad, width * TILE_H * 2);
            remaining &= !(((1u32 << tiles) - 1) << first_tile);
        }
    }
}

fn render_single_circle_rect(
    circle: RenderCircle, x0: usize, y0: usize, width: usize, height: usize, pixels: &mut [u8],
) {
    let x1 = (x0 + width - 1) as i32;
    for local_y in 0..height {
        let y = (y0 + local_y) as i32;
        let row = &mut pixels[local_y * width * 2..(local_y + 1) * width * 2];
        row.fill(0);
        let dy_q4 = ((y << 4) - circle.center_y_q4).unsigned_abs();
        if dy_q4 > circle.radius_q4 + 8 { continue; }
        let extent_q4 = isqrt(circle.radius_squared_q8.saturating_sub(dy_q4 * dy_q4)) as i32;
        let left_q4 = circle.center_x_q4 - extent_q4;
        let right_q4 = circle.center_x_q4 + extent_q4;
        let inner_left = ((left_q4 + 23) >> 4).clamp(x0 as i32, x1 + 1);
        let inner_right = ((right_q4 - 8) >> 4).clamp(x0 as i32 - 1, x1);
        if inner_left <= inner_right {
            fill_rgb565(row, (inner_left - x0 as i32) as i16, (inner_right - x0 as i32) as i16, circle.rgb565_pair);
            let left = (inner_left - x0 as i32) as i16;
            let right = (inner_right - x0 as i32) as i16;
            fill_rgb565(row, left, (left + RIM_PIXELS - 1).min(right), circle.rim565_pair);
            let rim_right = (right - RIM_PIXELS + 1).max(left);
            if rim_right > left + RIM_PIXELS - 1 {
                fill_rgb565(row, rim_right, right, circle.rim565_pair);
            }
        }
        let left_edge = inner_left - 1;
        if left_edge >= x0 as i32 {
            let coverage = (((left_edge << 4) - left_q4 + 8).clamp(0, 16)) as u8;
            if coverage > bayer4(left_edge as usize, y as usize) {
                let local_x = (left_edge - x0 as i32) as i16;
                fill_rgb565(row, local_x, local_x, circle.rim565_pair);
            }
        }
        let right_edge = inner_right + 1;
        if right_edge <= x1 && right_edge != left_edge {
            let coverage = ((right_q4 - (right_edge << 4) + 8).clamp(0, 16)) as u8;
            if coverage > bayer4(right_edge as usize, y as usize) {
                let local_x = (right_edge - x0 as i32) as i16;
                fill_rgb565(row, local_x, local_x, circle.rim565_pair);
            }
        }
    }
}

fn stream_opaque_scene(lcd: &mut Lcd<'_>, circles: &[RenderCircle]) {
    if circles.len() <= 8 {
        stream_scene(lcd, circles, None);
        return;
    }
    let mut x_extents = [[-1i16; MAX_CIRCLES]; H];
    build_x_extents(circles, &mut x_extents);
    stream_scene(lcd, circles, Some(&x_extents));
}

fn stream_scene(
    lcd: &mut Lcd<'_>, circles: &[RenderCircle],
    x_extents: Option<&[[i16; MAX_CIRCLES]; H]>,
) {
    let mut spi = lcd.spi.take().unwrap();
    let mut transmitting = lcd.tx.take().unwrap();
    let mut drawing = lcd.spare.take().unwrap();
    render_scene_stripe(circles, x_extents, 0, transmitting.as_mut_slice());

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
            render_scene_stripe(circles, x_extents, (stripe + 1) * STRIPE_ROWS, drawing.as_mut_slice());
        }
        (spi, transmitting) = transfer.wait();
        core::mem::swap(&mut transmitting, &mut drawing);
    }
    lcd.spi = Some(spi);
    lcd.tx = Some(drawing);
    lcd.spare = Some(transmitting);
}

fn build_x_extents(circles: &[RenderCircle], extents: &mut [[i16; MAX_CIRCLES]; H]) {
    for (circle_index, circle) in circles.iter().enumerate() {
        let center_y = circle.center_y_q4 >> 4;
        let maximum_dy = ((circle.radius_q4 + 15) >> 4) as i32;
        let mut x = maximum_dy;
        for dy in 0..=maximum_dy {
            let dy_q4 = (dy as u32) << 4;
            while x >= 0 {
                let x_q4 = (x as u32) << 4;
                if x_q4 * x_q4 + dy_q4 * dy_q4 <= circle.radius_squared_q8 { break; }
                x -= 1;
            }
            if x < 0 { break; }
            let upper = center_y - dy;
            let lower = center_y + dy;
            if upper >= 0 && upper < H as i32 { extents[upper as usize][circle_index] = x as i16; }
            if lower >= 0 && lower < H as i32 { extents[lower as usize][circle_index] = x as i16; }
        }
    }
}

fn render_scene_stripe(
    circles: &[RenderCircle], x_extents: Option<&[[i16; MAX_CIRCLES]; H]>,
    y0: usize, pixels: &mut [u8],
) {
    for local_y in 0..STRIPE_ROWS {
        let y = (y0 + local_y) as i32;
        let row = &mut pixels[local_y * W * 2..(local_y + 1) * W * 2];
        let mut covered_left = [0i16; MAX_CIRCLES * 3];
        let mut covered_right = [0i16; MAX_CIRCLES * 3];
        let mut covered_count = 0;

        for circle_index in (0..circles.len()).rev() {
            let circle = &circles[circle_index];
            let is_active = circle_index + 1 == circles.len();
            if x_extents.is_none() {
                let dy_q4 = ((y << 4) - circle.center_y_q4).unsigned_abs();
                if dy_q4 > circle.radius_q4 + 8 { continue; }
                let extent_q4 = isqrt(circle.radius_squared_q8.saturating_sub(dy_q4 * dy_q4)) as i32;
                let left_q4 = circle.center_x_q4 - extent_q4;
                let right_q4 = circle.center_x_q4 + extent_q4;
                let inner_left = ((left_q4 + 23) >> 4).clamp(0, W as i32);
                let inner_right = ((right_q4 - 8) >> 4).clamp(-1, (W - 1) as i32);
                if inner_left <= inner_right {
                    if is_active {
                        paint_uncovered_rim(row, inner_left as i16, inner_right as i16,
                            circle.rgb565_pair, circle.rim565_pair,
                            &mut covered_left, &mut covered_right, &mut covered_count);
                    } else {
                        paint_uncovered(row, inner_left as i16, inner_right as i16, circle.rgb565_pair,
                            &mut covered_left, &mut covered_right, &mut covered_count);
                    }
                }
                let edge_color = if is_active { circle.rim565_pair } else { circle.rgb565_pair };
                let left_edge = inner_left - 1;
                if left_edge >= 0 {
                    let coverage = (((left_edge << 4) - left_q4 + 8).clamp(0, 16)) as u8;
                    if coverage > bayer4(left_edge as usize, y as usize) {
                        paint_uncovered(row, left_edge as i16, left_edge as i16, edge_color,
                            &mut covered_left, &mut covered_right, &mut covered_count);
                    }
                }
                let right_edge = inner_right + 1;
                if right_edge < W as i32 && right_edge != left_edge {
                    let coverage = ((right_q4 - (right_edge << 4) + 8).clamp(0, 16)) as u8;
                    if coverage > bayer4(right_edge as usize, y as usize) {
                        paint_uncovered(row, right_edge as i16, right_edge as i16, edge_color,
                            &mut covered_left, &mut covered_right, &mut covered_count);
                    }
                }
                continue;
            }
            let x_extent = x_extents.unwrap()[y as usize][circle_index] as i32;
            if x_extent < 0 { continue; }
            let center_x = circle.center_x_q4 >> 4;
            let inner_left = (center_x - x_extent + 1).clamp(0, W as i32);
            let inner_right = (center_x + x_extent - 1).clamp(-1, (W - 1) as i32);

            if inner_left <= inner_right {
                if is_active {
                    paint_uncovered_rim(
                        row, inner_left as i16, inner_right as i16,
                        circle.rgb565_pair, circle.rim565_pair,
                        &mut covered_left, &mut covered_right, &mut covered_count,
                    );
                } else {
                    paint_uncovered(
                        row, inner_left as i16, inner_right as i16, circle.rgb565_pair,
                        &mut covered_left, &mut covered_right, &mut covered_count,
                    );
                }
            }
            let edge_color = if is_active { circle.rim565_pair } else { circle.rgb565_pair };
            paint_dithered_edge(row, center_x - x_extent, y, circle, edge_color,
                &mut covered_left, &mut covered_right, &mut covered_count);
            if x_extent != 0 {
                paint_dithered_edge(row, center_x + x_extent, y, circle, edge_color,
                    &mut covered_left, &mut covered_right, &mut covered_count);
            }
        }
        paint_background(row, &covered_left, &covered_right, covered_count);
    }
}

#[inline(always)]
fn paint_dithered_edge(
    row: &mut [u8], x: i32, y: i32, circle: &RenderCircle, color: u32,
    covered_left: &mut [i16; MAX_CIRCLES * 3],
    covered_right: &mut [i16; MAX_CIRCLES * 3],
    covered_count: &mut usize,
) {
    if x < 0 || x >= W as i32 { return; }
    let threshold = bayer4(x as usize, y as usize) as i32;
    let test_radius = (circle.radius_q4 as i32 + 8 - threshold).max(0) as u32;
    let dx_q4 = ((x << 4) - circle.center_x_q4).unsigned_abs();
    let dy_q4 = ((y << 4) - circle.center_y_q4).unsigned_abs();
    if dx_q4 * dx_q4 + dy_q4 * dy_q4 <= test_radius * test_radius {
        paint_uncovered(row, x as i16, x as i16, color,
            covered_left, covered_right, covered_count);
    }
}

// Color the already-owned span while preserving the compositor's single
// coverage insertion. Only fragments containing a geometric endpoint receive
// a rim pixel, so overlap clipping cannot create false internal outlines.
#[inline(always)]
fn paint_uncovered_rim(
    row: &mut [u8], left: i16, right: i16, body: u32, rim: u32,
    covered_left: &mut [i16; MAX_CIRCLES * 3],
    covered_right: &mut [i16; MAX_CIRCLES * 3],
    covered_count: &mut usize,
) {
    let mut cursor = left;
    for interval in 0..*covered_count {
        if covered_right[interval] < cursor { continue; }
        if covered_left[interval] > right { break; }
        if cursor < covered_left[interval] {
            fill_rimmed_span(row, cursor, (covered_left[interval] - 1).min(right), left, right, body, rim);
        }
        cursor = cursor.max(covered_right[interval] + 1);
        if cursor > right { break; }
    }
    if cursor <= right { fill_rimmed_span(row, cursor, right, left, right, body, rim); }
    insert_covered(left, right, covered_left, covered_right, covered_count);
}

#[inline(always)]
fn fill_rimmed_span(
    row: &mut [u8], left: i16, right: i16, circle_left: i16, circle_right: i16,
    body: u32, rim: u32,
) {
    let left_rim_end = (circle_left + RIM_PIXELS - 1).min(circle_right);
    let right_rim_start = (circle_right - RIM_PIXELS + 1).max(circle_left);
    let a = left;
    let b = right.min(left_rim_end);
    if a <= b { fill_rgb565(row, a, b, rim); }
    let a = left.max(left_rim_end + 1);
    let b = right.min(right_rim_start - 1);
    if a <= b { fill_rgb565(row, a, b, body); }
    let a = left.max(right_rim_start).max(left_rim_end + 1);
    if a <= right { fill_rgb565(row, a, right, rim); }
}

#[inline(always)]
fn paint_uncovered(
    row: &mut [u8], left: i16, right: i16, rgb565_pair: u32,
    covered_left: &mut [i16; MAX_CIRCLES * 3],
    covered_right: &mut [i16; MAX_CIRCLES * 3],
    covered_count: &mut usize,
) {
    let mut cursor = left;
    for interval in 0..*covered_count {
        if covered_right[interval] < cursor { continue; }
        if covered_left[interval] > right { break; }
        if cursor < covered_left[interval] {
            fill_rgb565(row, cursor, (covered_left[interval] - 1).min(right), rgb565_pair);
        }
        cursor = cursor.max(covered_right[interval] + 1);
        if cursor > right { break; }
    }
    if cursor <= right { fill_rgb565(row, cursor, right, rgb565_pair); }
    insert_covered(left, right, covered_left, covered_right, covered_count);
}

#[inline(always)]
fn insert_covered(
    mut left: i16, mut right: i16,
    covered_left: &mut [i16; MAX_CIRCLES * 3],
    covered_right: &mut [i16; MAX_CIRCLES * 3],
    count: &mut usize,
) {
    let mut first = 0;
    while first < *count && covered_right[first] + 1 < left { first += 1; }
    let mut after = first;
    while after < *count && covered_left[after] <= right + 1 {
        left = left.min(covered_left[after]);
        right = right.max(covered_right[after]);
        after += 1;
    }
    let removed = after - first;
    if removed == 0 {
        let mut index = *count;
        while index > first {
            covered_left[index] = covered_left[index - 1];
            covered_right[index] = covered_right[index - 1];
            index -= 1;
        }
        *count += 1;
    } else if removed > 1 {
        let mut source = after;
        while source < *count {
            covered_left[source - removed + 1] = covered_left[source];
            covered_right[source - removed + 1] = covered_right[source];
            source += 1;
        }
        *count -= removed - 1;
    }
    covered_left[first] = left;
    covered_right[first] = right;
}

#[inline(always)]
fn fill_rgb565(row: &mut [u8], left: i16, right: i16, rgb565_pair: u32) {
    let high = rgb565_pair as u8;
    let low = (rgb565_pair >> 8) as u8;
    let mut pixel = left as usize;
    let end = right as usize + 1;
    if pixel & 1 != 0 {
        let byte = pixel * 2;
        row[byte] = high;
        row[byte + 1] = low;
        pixel += 1;
    }
    while pixel + 1 < end {
        // `pixel` is even, so the DMA row address plus pixel*2 is word aligned.
        unsafe { (row.as_mut_ptr().add(pixel * 2) as *mut u32).write(rgb565_pair) };
        pixel += 2;
    }
    if pixel < end {
        let byte = pixel * 2;
        row[byte] = high;
        row[byte + 1] = low;
    }
}

fn paint_background(row: &mut [u8], left: &[i16], right: &[i16], count: usize) {
    let mut cursor = 0i16;
    for interval in 0..count {
        if cursor < left[interval] { fill_rgb565(row, cursor, left[interval] - 1, 0); }
        cursor = right[interval] + 1;
    }
    if cursor < W as i16 { fill_rgb565(row, cursor, W as i16 - 1, 0); }
}

#[inline(always)]
const fn bayer4(x: usize, y: usize) -> u8 {
    const MATRIX: [u8; 16] = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5];
    MATRIX[(y & 3) * 4 + (x & 3)]
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
