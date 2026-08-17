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
const TILE_H: usize = STRIPE_ROWS;
const TILES_Y: usize = H / TILE_H;
const MAX_CIRCLES: usize = 32;
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
}

const EMPTY: Circle = Circle {
    x: 0, y: 0, born_ticks: 0, full_radius_q4: 0, palette_index: 0,
};

struct Scene {
    circles: [Circle; MAX_CIRCLES],
    len: usize,
    next_color: usize,
}

#[derive(Clone, Copy)]
struct RenderCircle {
    x: i32,
    y: i32,
    radius_q4: u32,
    rgb565: u16,
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
fn circle_state(c: Circle, now_ticks: u32) -> (u32, u32, u8) {
    let age_ticks = now_ticks.wrapping_sub(c.born_ticks);
    // Convert once to bounded u32 milliseconds so RV32 uses its hardware DIVU;
    // u64 division otherwise calls the slow software __udivdi3 routine.
    let age_ms = age_ticks / 16_000;
    const GROW_MS: u32 = 700;
    const FADE_MS: u32 = 520;
    if age_ms < GROW_MS {
        // Q15 keeps every product within u32, avoiding RV32 multiword maths.
        let eased = smoothstep_q15(age_ms, GROW_MS);
        return (c.full_radius_q4 * eased >> 15, 16, 255);
    }
    let fade_age = age_ms - GROW_MS;
    if fade_age >= FADE_MS { return (c.full_radius_q4, 16, 0); }
    let smooth = smoothstep_q15(fade_age, FADE_MS);
    let alpha = ((32_768 - smooth) * 255 >> 15) as u8;
    (c.full_radius_q4, 16, alpha)
}

fn prepare_render_circles(scene: &mut Scene, now_ticks: u32) -> ([RenderCircle; MAX_CIRCLES], usize) {
    const EMPTY_RENDER: RenderCircle = RenderCircle { x: 0, y: 0, radius_q4: 0, rgb565: 0 };
    let mut rendered = [EMPTY_RENDER; MAX_CIRCLES];
    let mut count = 0;
    let mut newest_full = None;

    for index in 0..scene.len {
        let circle = scene.circles[index];
        let (radius_q4, _, alpha) = circle_state(circle, now_ticks);
        if alpha == 0 { continue; }
        if radius_q4 >= circle.full_radius_q4 { newest_full = Some(index); }
        let color = PALETTE[circle.palette_index as usize];
        let red = mul_255(color.0 as u16, alpha as u16) as u8;
        let green = mul_255(color.1 as u16, alpha as u16) as u8;
        let blue = mul_255(color.2 as u16, alpha as u16) as u8;
        rendered[count] = RenderCircle {
            x: circle.x,
            y: circle.y,
            radius_q4,
            rgb565: rgb888_to_565(red, green, blue),
        };
        count += 1;
    }

    if let Some(first_visible) = newest_full {
        let retained = scene.len - first_visible;
        scene.circles.copy_within(first_visible..scene.len, 0);
        scene.len = retained;
        rendered.copy_within(count - retained..count, 0);
        count = retained;
    } else {
        let mut write = 0;
        for read in 0..scene.len {
            if circle_state(scene.circles[read], now_ticks).2 != 0 {
                scene.circles[write] = scene.circles[read];
                write += 1;
            }
        }
        scene.len = write;
    }
    (rendered, count)
}

#[inline(always)]
const fn rgb888_to_565(red: u8, green: u8, blue: u8) -> u16 {
    ((red as u16 & 0xf8) << 8) | ((green as u16 & 0xfc) << 3) | (blue as u16 >> 3)
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
        let sole_full_circle_alpha = if scene.len == 1 {
            let circle = scene.circles[0];
            let (radius, _, alpha) = circle_state(circle, now_ticks);
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
            set_window(&mut lcd, 0, 0, (W - 1) as u16, (H - 1) as u16);
            stream_opaque_scene(&mut lcd, &render_circles[..render_count]);
            if hardware_fade_active { set_brightness(&mut lcd, 255); }
        }
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

fn stream_opaque_scene(lcd: &mut Lcd<'_>, circles: &[RenderCircle]) {
    let mut spi = lcd.spi.take().unwrap();
    let mut transmitting = lcd.tx.take().unwrap();
    let mut drawing = lcd.spare.take().unwrap();
    render_scene_stripe(circles, 0, transmitting.as_mut_slice());

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
            render_scene_stripe(circles, (stripe + 1) * STRIPE_ROWS, drawing.as_mut_slice());
        }
        (spi, transmitting) = transfer.wait();
        core::mem::swap(&mut transmitting, &mut drawing);
    }
    lcd.spi = Some(spi);
    lcd.tx = Some(drawing);
    lcd.spare = Some(transmitting);
}

fn render_scene_stripe(circles: &[RenderCircle], y0: usize, pixels: &mut [u8]) {
    let mut covered_left = [0i16; MAX_CIRCLES * 3];
    let mut covered_right = [0i16; MAX_CIRCLES * 3];

    for local_y in 0..STRIPE_ROWS {
        let y = (y0 + local_y) as i32;
        let row = &mut pixels[local_y * W * 2..(local_y + 1) * W * 2];
        let mut covered_count = 0;

        for circle in circles.iter().rev() {
            let dy_q4 = (y - circle.y).unsigned_abs() << 4;
            if dy_q4 > circle.radius_q4 + 8 { continue; }
            let radius_squared = circle.radius_q4 * circle.radius_q4;
            let x_extent_q4 = isqrt(radius_squared.saturating_sub(dy_q4 * dy_q4)) as i32;
            let left_q4 = (circle.x << 4) - x_extent_q4;
            let right_q4 = (circle.x << 4) + x_extent_q4;
            let inner_left = ((left_q4 + 23) >> 4).clamp(0, W as i32);
            let inner_right = ((right_q4 - 8) >> 4).clamp(-1, (W - 1) as i32);

            if inner_left <= inner_right {
                paint_uncovered(
                    row, inner_left as i16, inner_right as i16, circle.rgb565,
                    &mut covered_left, &mut covered_right, &mut covered_count,
                );
            }
            let left_edge = inner_left - 1;
            if left_edge >= 0 {
                let coverage = (((left_edge << 4) - left_q4 + 8).clamp(0, 16)) as u8;
                if coverage > bayer4(left_edge as usize, y as usize) {
                    paint_uncovered(row, left_edge as i16, left_edge as i16, circle.rgb565,
                        &mut covered_left, &mut covered_right, &mut covered_count);
                }
            }
            let right_edge = inner_right + 1;
            if right_edge < W as i32 && right_edge != left_edge {
                let coverage = ((right_q4 - (right_edge << 4) + 8).clamp(0, 16)) as u8;
                if coverage > bayer4(right_edge as usize, y as usize) {
                    paint_uncovered(row, right_edge as i16, right_edge as i16, circle.rgb565,
                        &mut covered_left, &mut covered_right, &mut covered_count);
                }
            }
        }
        paint_background(row, &covered_left, &covered_right, covered_count);
    }
}

#[inline(always)]
fn paint_uncovered(
    row: &mut [u8], left: i16, right: i16, color: u16,
    covered_left: &mut [i16; MAX_CIRCLES * 3],
    covered_right: &mut [i16; MAX_CIRCLES * 3],
    covered_count: &mut usize,
) {
    let mut cursor = left;
    for interval in 0..*covered_count {
        if covered_right[interval] < cursor { continue; }
        if covered_left[interval] > right { break; }
        if cursor < covered_left[interval] {
            fill_rgb565(row, cursor, (covered_left[interval] - 1).min(right), color);
        }
        cursor = cursor.max(covered_right[interval] + 1);
        if cursor > right { break; }
    }
    if cursor <= right { fill_rgb565(row, cursor, right, color); }
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
fn fill_rgb565(row: &mut [u8], left: i16, right: i16, color: u16) {
    let high = (color >> 8) as u8;
    let low = color as u8;
    let mut pixel = left as usize;
    let end = right as usize + 1;
    if pixel & 1 != 0 {
        let byte = pixel * 2;
        row[byte] = high;
        row[byte + 1] = low;
        pixel += 1;
    }
    let pair = u32::from_le_bytes([high, low, high, low]);
    while pixel + 1 < end {
        // `pixel` is even, so the DMA row address plus pixel*2 is word aligned.
        unsafe { (row.as_mut_ptr().add(pixel * 2) as *mut u32).write(pair) };
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
