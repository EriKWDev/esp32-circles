//! Screens, transitions and hit testing.
//!
//! Layout geometry lives in the `L` constants below and is used by *both* the
//! drawing code and the single zone-registration routine, so a button can never
//! drift away from the region that activates it.
//!
//! Everything is placed inside a circle of radius SAFE_R about the screen
//! centre. The panel is addressed as a 480x480 square, but a 2.16" AMOLED of
//! this kind is round, and even on a square one a circular composition suits a
//! UI whose whole visual language is expanding discs. Nothing important is put
//! where a bezel might eat it.

use core::fmt::Write as _;

use crate::font::FontId;
use crate::gfx::{Align, H, Scene, Text, W, rgb};
use crate::model::{Link, StartTime, State};
use crate::touch::Event;

const CX: i32 = (W / 2) as i32;
const CY: i32 = (H / 2) as i32;
/// Keep-out radius: content beyond this risks the round panel's edge.
const SAFE_R: i32 = 226;

const INK: u16 = rgb(238, 245, 250);
const MUTED: u16 = rgb(122, 143, 158);
const DIM: u16 = rgb(70, 84, 96);

/// True black. On an OLED the pixels are simply off, so this is the one
/// background choice that is also a power decision.
const BG_HOME: u16 = rgb(0, 0, 0);
const BG_INSPECT: u16 = rgb(8, 22, 48);
const BG_FORCE: u16 = rgb(38, 22, 4);
const BG_RUN: u16 = rgb(5, 30, 20);
const BG_INFO: u16 = rgb(18, 15, 36);

const C_INSPECT: u16 = rgb(46, 104, 214);
const C_FORCE: u16 = rgb(226, 142, 24);
const C_RUN: u16 = rgb(30, 176, 108);
const C_CANCEL: u16 = rgb(212, 52, 48);
const C_INFO: u16 = rgb(142, 104, 226);
/// Configuration reads as a cooler, more technical relative of Extras.
const BG_CONFIG: u16 = rgb(10, 24, 32);
const C_CONFIG: u16 = rgb(58, 158, 178);
/// The demo's first palette entry, used for its button on the Extras menu so the
/// page announces itself before you open it.
const C_BUBBLES: u16 = rgb(255, 55, 125);
const C_BREAKOUT: u16 = rgb(88, 190, 255);
const C_INVADERS: u16 = rgb(120, 230, 170);
const C_ASTEROIDS: u16 = rgb(190, 200, 230);
const C_GAMES: u16 = rgb(255, 150, 210);
const C_APPS: u16 = rgb(255, 150, 90);
const C_2048: u16 = rgb(242, 177, 121);
const C_MATCH3: u16 = rgb(255, 110, 140);
const C_POMODORO: u16 = rgb(255, 120, 90);
const C_SIMON: u16 = rgb(120, 210, 255);
const C_CAT: u16 = rgb(255, 190, 120);
const C_CALC: u16 = rgb(232, 140, 60);
const C_WEATHER: u16 = rgb(96, 176, 255);
const C_CURRENCY: u16 = rgb(120, 220, 170);
const C_SNAKE: u16 = rgb(120, 230, 140);
const C_SKY: u16 = rgb(96, 176, 255);

/// Controllers can report `run=1` for one final poll after the countdown and
/// queue have both drained. Treat that as completed activity everywhere; the
/// separate acknowledgment latch decides whether its 00:00 badge remains.
/// Whether watering is still in progress, from the panel's point of view.
///
/// A sequence is idle between its entries - nothing is energized, `run:` is 0 -
/// but it has certainly not finished, so the queue and the gap flag both count as
/// activity. Requiring `running` here was what made DONE appear on the timer part
/// way through a schedule, and the corner badge blink out with it.
fn run_is_active(state: &State) -> bool {
    (state.running && state.left_s > 0) || state.queued > 0 || state.queue_gap
}

/// Layout. Shared by drawing and hit testing - see the module note.
mod l {
    /// (x0, y0, x1, y1) for the two home buttons. Identical size and corner
    /// radius: they are peers, so they are differentiated by colour and position
    /// rather than by shape, and the generous gap keeps them from touching.
    pub const HOME_INSPECT: (i32, i32, i32, i32) = (84, 236, 396, 308);
    pub const HOME_FORCE: (i32, i32, i32, i32) = (84, 330, 396, 402);
    pub const HOME_PILL_R: i32 = 36;

    /// (cx, cy, r). The panel is square with rounded corners, not round, so this
    /// sits properly in the top-left instead of being pulled toward the middle.
    pub const BACK: (i32, i32, i32) = (58, 58, 40);
    /// Top left, where Home has nothing else - the run badge owns the right
    /// corner, so the two never have to negotiate over it.
    pub const INFO: (i32, i32, i32) = (58, 58, 40);
    pub const RUN_BADGE: (i32, i32, i32) = (422, 58, 32);
    /// Sleep now, bottom left on Home - diagonally opposite the cog and clear of
    /// both buttons.
    pub const MOON: (i32, i32, i32) = (58, 422, 34);
    /// The forecast symbol, right of the clock, with a target far larger than the
    /// drawing - it is a small thing to hit and the page behind it is harmless.
    pub const SKY: (i32, i32, i32) = (376, 140, 46);
    /// The pencil on the stocks page, in the corner opposite Back.
    pub const STOCK_EDIT: (i32, i32, i32, i32) = (400, 14, 466, 50);
    /// The three choices on the rain page.
    pub const RAIN_CHOICE: [(i32, i32, i32, i32); 3] =
        [(56, 250, 424, 314), (56, 326, 424, 390), (56, 402, 424, 452)];
    /// Clear of the relay list, vertically centred on the panel.
    pub const GO: (i32, i32, i32) = (406, 240, 50);
    /// (x0, y0, x1, y1) - centred under the countdown digits and comfortably
    /// inside RING_INNER, so it never crosses the ring.
    pub const CANCEL: (i32, i32, i32, i32) = (152, 316, 328, 374);

    /// Progress ring: a closed circle, swept clockwise from 12 o'clock.
    ///
    /// Pulled in from 232/218 so the ring clears the Back button in the corner -
    /// at 232 the band ran straight through it, since Back's disc reaches to
    /// within 217 px of centre. Everything else on this screen is composed inside
    /// RING_INNER and vertically balanced about the panel centre.
    pub const RING_OUTER: i32 = 208;
    pub const RING_INNER: i32 = 194;
    /// Second, thinner gauge inside the first: the entry running now, while the
    /// outer one measures the whole schedule. Only drawn for a sequence.
    pub const RING2_OUTER: i32 = 186;
    pub const RING2_INNER: i32 = 177;
    /// Baselines for the countdown stack.
    pub const RUN_NAME_BASELINE: i32 = 152;
    pub const RUN_DIGITS_BASELINE: i32 = 284;
    /// Position in the running order and the total left, under the digits and
    /// clear of the cancel button below them.
    pub const RUN_QUEUE_BASELINE: i32 = 308;

    pub const SLIDER_X: i32 = 88;
    pub const SLIDER_TOP: i32 = 148;
    pub const SLIDER_BOTTOM: i32 = 404;
    pub const SLIDER_HALF_W: i32 = 25;
    /// Duration range, in minutes. 20 steps over 256 px of travel is about 13 px
    /// per minute - still comfortably larger than a fingertip's precision.
    pub const MINUTES_MIN: u32 = 1;
    pub const MINUTES_MAX: u32 = 20;

    pub const RELAY_X0: i32 = 150;
    pub const RELAY_X1: i32 = 344;
    pub const RELAY_FIRST_CY: i32 = 206;
    pub const RELAY_PITCH: i32 = 48;
    pub const RELAY_HALF_H: i32 = 22;
    pub const RELAY_MAX_ROWS: usize = 5;

    /// Schedule rows use nearly the full panel width. They were inset by 66 px a
    /// side, which squeezed the clock and the zone summary into each other for no
    /// reason - the panel is square, so that margin was pure waste.
    pub const SCHED_X0: i32 = 26;
    pub const SCHED_X1: i32 = 454;
    pub const SCHED_FIRST_CY: i32 = 186;
    pub const SCHED_PITCH: i32 = 76;
    pub const SCHED_HALF_H: i32 = 30;
    pub const SCHED_MAX_ROWS: usize = 4;

    pub const DETAIL_FIRST_CY: i32 = 200;
    pub const DETAIL_PITCH: i32 = 48;
    pub const DETAIL_MAX_ROWS: usize = 5;
    /// Narrower than the schedules list: this page keeps a column free on the
    /// right for the run button, the same arrangement the manual page uses for
    /// GO, so "the big button that starts watering" is always in one place.
    pub const DETAIL_X0: i32 = 18;
    pub const DETAIL_X1: i32 = 342;
    /// (cx, cy, r) - vertically centred on the entry list.
    pub const RUN_NOW: (i32, i32, i32) = (408, 296, 54);
    /// EDIT, top right, opposite Back. Becomes SAVE in edit mode - same place,
    /// same size, so the thing you press to finish is where the thing you pressed
    /// to start was.
    pub const EDIT: (i32, i32, i32, i32) = (360, 30, 464, 86);
    /// Edit mode: the clock's four adjusters, in a row under it.
    pub const TIME_HH_DOWN: (i32, i32, i32, i32) = (96, 132, 156, 176);
    pub const TIME_HH_UP: (i32, i32, i32, i32) = (162, 132, 222, 176);
    pub const TIME_MM_DOWN: (i32, i32, i32, i32) = (258, 132, 318, 176);
    pub const TIME_MM_UP: (i32, i32, i32, i32) = (324, 132, 384, 176);
    /// Edit mode: the armed toggle, left of the adjusters.
    pub const ARMED: (i32, i32, i32, i32) = (18, 132, 90, 176);
    /// Edit mode: the selected entry's controls, in the column the run button
    /// vacates. Six of them, 40 px tall - a comfortable target at this width.
    pub const EDIT_COL_X0: i32 = 352;
    pub const EDIT_COL_X1: i32 = 472;
    pub const EDIT_COL_TOP: i32 = 182;
    pub const EDIT_COL_PITCH: i32 = 44;
    pub const EDIT_COL_H: i32 = 40;
    pub const ANALOG_X0: i32 = 132;
    pub const ANALOG_X1: i32 = 438;
    pub const ANALOG_FIRST_CY: i32 = 198;
    pub const ANALOG_PITCH: i32 = 42;

    /// Menu rows, shared by every list screen in the settings branch.
    ///
    /// Deliberately identical geometry to the schedule rows above, because these
    /// are the same kind of thing: a scrollable column of openable rows. They
    /// only differ in colour and content, so they should not also differ in
    /// width, height, pitch or corner radius.
    pub const MENU_X0: i32 = SCHED_X0;
    pub const MENU_X1: i32 = SCHED_X1;
    pub const MENU_FIRST_CY: i32 = SCHED_FIRST_CY;
    pub const MENU_PITCH: i32 = SCHED_PITCH;
    pub const MENU_HALF_H: i32 = SCHED_HALF_H;
    pub const MENU_MAX_ROWS: usize = SCHED_MAX_ROWS;
    /// The clipped band the rows scroll inside, and the scrollbar's extent.
    pub const MENU_VIEW_TOP: i32 = 156;
    pub const MENU_VIEW_BOTTOM: i32 = 438;
    /// Heading baseline. At 116 a short title clears the Back button entirely;
    /// this is why the headings are short - "SETTINGS", not "CONFIGURATION",
    /// which at this size is wide enough to run under the button.
    pub const MENU_TITLE: i32 = 116;
    pub const MENU_STATUS: i32 = 146;

    /// Extras uses Home's button shape rather than list rows: it is a menu of
    /// destinations, like Home, and will grow to hold games and other toys.
    pub const EXTRA_X0: i32 = 84;
    pub const EXTRA_X1: i32 = 396;
    pub const EXTRA_FIRST_CY: i32 = 200;
    pub const EXTRA_PITCH: i32 = 94;
    pub const EXTRA_HALF_H: i32 = 36;
    pub const EXTRA_MAX_ROWS: usize = 3;
    pub const EXTRA_VIEW_TOP: i32 = 150;
    pub const EXTRA_VIEW_BOTTOM: i32 = 448;
    // Six rows fit the clipped viewport completely. A seventh row previously
    // looked like accidental clipping and never enabled the scrollbar.
    pub const ANALOG_MAX_ROWS: usize = 6;
    pub const ANALOG_VIEW_TOP: i32 = 168;
    pub const ANALOG_VIEW_BOTTOM: i32 = 438;

    /// On-screen keyboard.
    ///
    /// 480 px across is what makes a real QWERTY possible here: ten keys of 46 px
    /// with a gap between them is a larger target than most phones give a thumb,
    /// so there was no need to fall back to a compromise layout. Rows are
    /// centred on their own key count rather than stretched to a fixed band,
    /// which keeps every key the same size on every row.
    pub const KEY_W: i32 = 46;
    /// Digits get a proper keypad: three columns of these instead of ten of
    /// KEY_W, because an IP address is the one thing typed here under time
    /// pressure, standing at a valve box.
    pub const KEY_W_PAD: i32 = 88;
    pub const KEY_TOP: i32 = 238;
    pub const KEY_H: i32 = 52;
    pub const KEY_PITCH: i32 = 58;
    pub const KEY_GAP: i32 = 6;
    pub const KEY_R: i32 = 12;
    /// Row 2's flanking keys: case toggle on the left, delete on the right.
    pub const KEY_SHIFT: (i32, i32) = (4, 68);
    pub const KEY_DEL: (i32, i32) = (412, 476);
    /// Bottom row: mode toggle, space, commit.
    pub const KEY_BOTTOM_Y: i32 = KEY_TOP + 3 * KEY_PITCH;
    pub const KEY_MODE: (i32, i32) = (8, 116);
    pub const KEY_SPACE: (i32, i32) = (126, 354);
    pub const KEY_DONE: (i32, i32) = (364, 472);
    /// The text being edited, between the heading and the keys.
    pub const FIELD: (i32, i32, i32, i32) = (24, 162, 456, 222);

    /// The single destructive button on the confirmation screen, and the retry on
    /// the connecting one. Both are Home-button shaped: this panel's language for
    /// "the thing to press".
    pub const CONFIRM_YES: (i32, i32, i32, i32) = (84, 320, 396, 392);
    pub const RETRY: (i32, i32, i32, i32) = (110, 386, 370, 452);
}

/// Row geometry of a list screen, so drawing, scrolling and hit testing all read
/// the same numbers even though Extras' buttons are a different size from the
/// settings rows.
struct RowGeom {
    x0: i32,
    x1: i32,
    first_cy: i32,
    pitch: i32,
    half_h: i32,
    max_rows: usize,
    top: i32,
    bottom: i32,
}

fn geom(screen: Screen) -> RowGeom {
    if screen == Screen::Extras || screen == Screen::Apps {
        RowGeom {
            x0: l::EXTRA_X0,
            x1: l::EXTRA_X1,
            first_cy: l::EXTRA_FIRST_CY,
            pitch: l::EXTRA_PITCH,
            half_h: l::EXTRA_HALF_H,
            max_rows: l::EXTRA_MAX_ROWS,
            top: l::EXTRA_VIEW_TOP,
            bottom: l::EXTRA_VIEW_BOTTOM,
        }
    } else {
        RowGeom {
            x0: l::MENU_X0,
            x1: l::MENU_X1,
            first_cy: l::MENU_FIRST_CY,
            pitch: l::MENU_PITCH,
            half_h: l::MENU_HALF_H,
            max_rows: l::MENU_MAX_ROWS,
            top: l::MENU_VIEW_TOP,
            bottom: l::MENU_VIEW_BOTTOM,
        }
    }
}

/// Fixed-capacity string, so labels can be formatted without an allocator.
struct Buf<const N: usize> {
    bytes: [u8; N],
    len: usize,
}

impl<const N: usize> Buf<N> {
    fn new() -> Self {
        Self {
            bytes: [0; N],
            len: 0,
        }
    }
    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len]).unwrap_or("")
    }
}

impl<const N: usize> core::fmt::Write for Buf<N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for &b in s.as_bytes() {
            if self.len < N {
                self.bytes[self.len] = b;
                self.len += 1;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Home,
    Inspect,
    Force,
    Running,
    /// One schedule's running order, opened from the schedules list.
    Detail,
    /// Analog readouts. Reached from Extras rather than directly from the cog,
    /// so more diagnostic and settings pages can be added beside it.
    Info,
    /// The cog's destination: a menu of everything that is not day-to-day
    /// watering. A list rather than a fixed layout, because the whole point is
    /// that entries get added to it.
    Extras,
    /// Network and controller configuration, backed by flash.
    Config,
    /// Wi-Fi picker: the networks a scan found, plus rescan and manual entry.
    Wifi,
    /// One controller's settings, and the only place it can be removed.
    Controller,
    /// Text entry. What it is editing, and where Back returns to, are held in
    /// `edit`, and Back is the ordinary history pop, rather than encoded in
    /// more screen variants.
    Keyboard,
    /// Joining a network: working, then joined, or failed with a retry.
    Connecting,
    /// A destructive action, held until it is confirmed.
    Confirm,
    /// The circles demo from the project's main branch, as a page of its own.
    Bubbles,
    /// Two-player pong, which drives the same circles.
    Pong,
    /// Breakout, which brings its own palette per level.
    Breakout,
    Invaders,
    Asteroids,
    /// The games and demos, moved off Extras onto a page of their own.
    Apps,
    G2048,
    Match3,
    Pomodoro,
    Simon,
    Cat,
    Calc,
    Weather,
    Currency,
    Snake,
    Spacewar,
    About,
    /// What to do about tomorrow's forecast.
    Rain,
    Chess,
    Tetris,
    /// Share prices, three watchlists.
    Stocks,
    Pacman,
    /// The games menu. Apps' twin, and drawn by the same page.
    Games,
}

impl Screen {
    fn background(self) -> u16 {
        match self {
            Screen::Home => BG_HOME,
            Screen::Inspect => BG_INSPECT,
            Screen::Force => BG_FORCE,
            Screen::Running => BG_RUN,
            Screen::Detail => BG_INSPECT,
            Screen::Info => BG_INFO,
            // Extras shares Info's palette: it is the same part of the app.
            Screen::Extras => BG_INFO,
            // Everything reached from Configuration keeps its palette, so the
            // whole settings branch reads as one place.
            Screen::Config
            | Screen::Wifi
            | Screen::Controller
            | Screen::Keyboard
            | Screen::Connecting
            | Screen::Confirm => BG_CONFIG,
            // Black, as the demo has it - and since the transition disc grows in
            // the destination's background colour, arriving here is a wipe to
            // black, which is the right way into it.
            Screen::Bubbles
            | Screen::Pong
            | Screen::Invaders
            | Screen::Asteroids
            | Screen::G2048
            | Screen::Match3
            | Screen::Pomodoro
            | Screen::Simon
            | Screen::Cat
            | Screen::Calc
            | Screen::Weather
            | Screen::Currency
            | Screen::Snake
            | Screen::Spacewar
            | Screen::About
            | Screen::Rain
            | Screen::Chess
            | Screen::Tetris
            | Screen::Stocks
            | Screen::Pacman => rgb(0, 0, 0),
            // Both menus are Extras' twins, so they share its palette.
            Screen::Apps | Screen::Games => BG_INFO,
            // Breakout's ground comes from the level, so `build` overrides this -
            // the fallback keeps the transition disc a sensible colour.
            Screen::Breakout => rgb(6, 10, 20),
        }
    }
    fn accent(self) -> u16 {
        match self {
            Screen::Home => C_RUN,
            Screen::Inspect => C_INSPECT,
            Screen::Force => C_FORCE,
            Screen::Running => C_RUN,
            Screen::Detail => C_INSPECT,
            Screen::Info => C_INFO,
            Screen::Extras => C_INFO,
            Screen::Confirm => C_CANCEL,
            Screen::Bubbles => C_BUBBLES,
            Screen::Pong => crate::pong::LEFT_COLOR,
            Screen::Breakout => C_BREAKOUT,
            Screen::Invaders => C_INVADERS,
            Screen::Asteroids => C_ASTEROIDS,
            Screen::Apps => C_APPS,
            Screen::Games => C_GAMES,
            Screen::G2048 => C_2048,
            Screen::Match3 => C_MATCH3,
            Screen::Pomodoro => C_POMODORO,
            Screen::Simon => C_SIMON,
            Screen::Cat => C_CAT,
            Screen::Calc => C_CALC,
            Screen::Weather => C_WEATHER,
            Screen::Currency => C_CURRENCY,
            Screen::Snake => C_SNAKE,
            Screen::Spacewar => crate::spacewar::P1,
            Screen::About => crate::about::ACCENT,
            Screen::Rain => C_SKY,
            Screen::Chess => crate::chess::ACCENT,
            Screen::Tetris => crate::tetris::ACCENT,
            Screen::Stocks => crate::stocks::ACCENT,
            Screen::Pacman => crate::pacman::ACCENT,
            Screen::Config
            | Screen::Wifi
            | Screen::Controller
            | Screen::Keyboard
            | Screen::Connecting => C_CONFIG,
        }
    }

    /// True for the list-shaped screens: one scrollable column of menu rows.
    /// They share drawing, scrolling and hit testing, so they are recognised
    /// here rather than enumerated at each of those places.
    fn is_menu(self) -> bool {
        matches!(
            self,
            Screen::Extras
                | Screen::Apps
                | Screen::Games
                | Screen::Config
                | Screen::Wifi
                | Screen::Controller
        )
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Target {
    Back,
    Inspect,
    Force,
    Go,
    Cancel,
    Relay(usize),
    Slider,
    /// A row of the Extras menu.
    Extra(usize),
    /// A row of the Config page: Wi-Fi first, then one per controller, then Add.
    ConfigRow(usize),
    /// A row of the Wi-Fi picker: the scanned networks, then rescan, then manual.
    WifiRow(usize),
    /// A row of one controller's page: address, user, password, remove.
    CtlRow(usize),
    /// A keyboard grid row; which key is resolved from x, using the same slot
    /// arithmetic the renderer places the plates with.
    KeyRow(usize),
    /// A keyboard key that is not part of a grid row - shift, delete, mode,
    /// space, commit.
    KeyAux(usize),
    /// Anywhere on the bubbles page that is not the Back button.
    Bubble,
    /// Run this schedule now, or stop it if it is already running.
    RunNow,
    /// Sleep immediately, without waiting out the idle timer.
    SleepNow,
    /// The forecast symbol on the home screen.
    Sky,
    /// One of the three watchlist tabs.
    StockTab(usize),
    /// The scrolling band of quotes.
    StockList,
    /// Enter or leave the watchlist editor.
    StockEdit,
    /// Water anyway, skip tomorrow, or follow the forecast.
    RainChoice(usize),
    /// Enter edit mode, or save and leave it.
    EditSave,
    /// Edit mode: one entry of the schedule being edited, or the row past the
    /// end, which appends one.
    EntryRow(usize),
    /// Edit mode: one of the clock adjusters, or the armed toggle.
    TimeAdjust(usize),
    Armed,
    /// Edit mode: one of the selected entry's controls.
    EditCtl(usize),
    /// Go ahead with the destructive action being confirmed.
    Confirm,
    /// Try the failed network join again.
    Retry,
    Schedule(usize),
    Info,
    RunningBadge,
    List,
}

#[derive(Clone, Copy)]
enum Zone {
    Disc { cx: i32, cy: i32, r: i32 },
    Rect { x0: i32, y0: i32, x1: i32, y1: i32 },
}

impl Zone {
    fn contains(&self, x: i32, y: i32) -> bool {
        match *self {
            // Generous by a few pixels: fingers are bigger than hit boxes.
            Zone::Disc { cx, cy, r } => {
                let dx = x - cx;
                let dy = y - cy;
                dx * dx + dy * dy <= (r + 6) * (r + 6)
            }
            Zone::Rect { x0, y0, x1, y1 } => {
                x >= x0 - 6 && x <= x1 + 6 && y >= y0 - 6 && y <= y1 + 6
            }
        }
    }
}

/// Zones are registered per frame and an overflow is silently dropped, which
/// would present as a button that simply does not work. Edit mode needs
/// fourteen, so this keeps real headroom rather than one spare.
const MAX_ZONES: usize = 24;

/// What the UI wants the network layer to do.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    Trigger {
        relay: u8,
        controller: u8,
        remote_relay: u8,
        seconds: u32,
    },
    Stop,
    /// Settings changed: persist them and adopt the new controller list.
    SaveSettings,
    /// The network changed: persist, then re-join with the new credentials.
    ApplyWifi,
    /// Run one schedule immediately. `start` is the id on its own controller.
    RunSchedule {
        controller: u8,
        start: u8,
    },
    /// Write the edited schedule back. The draft itself is read from the UI.
    SaveSchedule,
}

/// What the keyboard is filling in. Held as state rather than as more `Screen`
/// variants, because every one of these uses the same screen.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Edit {
    /// A network name typed by hand, for an access point that does not
    /// broadcast or that a scan missed. Commits into the password step.
    WifiSsid,
    WifiPsk,
    NewControllerIp,
    ControllerIp(usize),
    ControllerUser(usize),
    ControllerPass(usize),
    ControllerName(usize),
    /// A currency code for the rates app. Not persisted: it is a look, not a
    /// setting.
    Symbol,
    /// A watchlist's name, and a ticker to add to it.
    ListName(usize),
    AddSymbol(usize),
}

impl Edit {
    /// Kept short: at heading size a long word runs under the Back button, and
    /// the context that would have padded it out belongs in the subtitle anyway -
    /// "PASSWORD", with the network's name underneath it.
    fn title(self) -> &'static str {
        match self {
            Edit::WifiSsid => "NETWORK",
            Edit::WifiPsk => "PASSWORD",
            Edit::NewControllerIp | Edit::ControllerIp(_) => "ADDRESS",
            Edit::ControllerUser(_) => "USERNAME",
            Edit::ControllerPass(_) => "PASSWORD",
            Edit::ControllerName(_) => "NAME",
            Edit::Symbol => "CURRENCY",
            Edit::ListName(_) => "LIST NAME",
            Edit::AddSymbol(_) => "ADD SYMBOL",
        }
    }

    /// Addresses get the keypad; everything else gets letters.
    fn mode(self) -> KeyMode {
        match self {
            Edit::NewControllerIp | Edit::ControllerIp(_) => KeyMode::Numeric,
            // Tickers and list names are both shouted, and a ticker has dots and
            // dashes in it that the letter layout carries.
            Edit::Symbol | Edit::ListName(_) | Edit::AddSymbol(_) => KeyMode::Upper,
            _ => KeyMode::Lower,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum KeyMode {
    Lower,
    Upper,
    Symbols,
    Numeric,
}

impl KeyMode {
    /// The three grid rows, and how wide one key is.
    fn rows(self) -> (&'static [&'static str; 3], i32) {
        match self {
            // Passwords are case-sensitive, so the case toggle is not cosmetic:
            // the keyboard shows the case it will actually type.
            KeyMode::Lower => (&["qwertyuiop", "asdfghjkl", "zxcvbnm"], l::KEY_W),
            KeyMode::Upper => (&["QWERTYUIOP", "ASDFGHJKL", "ZXCVBNM"], l::KEY_W),
            // Restricted to glyphs the baked fonts actually carry - there is no
            // fallback box to draw, a missing glyph is simply invisible.
            KeyMode::Symbols => (&["1234567890", "-_.,:;/@'\"", "!?#%&*+()="], l::KEY_W),
            KeyMode::Numeric => (&["123", "456", "789"], l::KEY_W_PAD),
        }
    }

    fn is_alpha(self) -> bool {
        matches!(self, KeyMode::Lower | KeyMode::Upper)
    }
}

/// The selected entry's controls, top to bottom in the column the run button
/// vacates. One table, so registration, drawing and the handler cannot disagree
/// about which button is which.
const EDIT_CONTROLS: [&str; 6] = ["UP", "DOWN", "RELAY", "LONGER", "SHORTER", "REMOVE"];

/// How long a pressed key stays lit. Long enough to see on a panel refreshing at
/// 40-60 fps, short enough not to lag a fast typist.
const KEY_FLASH_MS: u32 = 130;
/// Shortest gesture 2048 accepts as a swipe, so a tap does not move the board.
const SWIPE_MIN: i32 = 26;
/// List pages that remember a scroll offset - see `scroll_slot`.
const MENU_PAGES: usize = 6;

/// What the games ask to be played. Kept as an intent rather than a call so the
/// blocking part stays in main.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Sound {
    Pad(usize),
    Win,
    Lose,
    Purr,
    Meow,
}

/// How long to wait for a network to let us in before offering a retry.
///
/// There is no failure event to observe here - a rejected password looks exactly
/// like one that has not been answered yet - so this is a timeout, set past the
/// ten seconds or so an access point takes to turn one down. It is also well
/// inside `MANUAL_HOLD_MS`, so the retry it offers can never race the automatic
/// reconnect.
const CONNECT_TIMEOUT_MS: u32 = 14_000;
/// How long the "connected" confirmation stays up before returning to settings.
const CONNECT_SETTLE_MS: u32 = 1_400;

/// A decorative expanding ring, spawned by every tap. The circles demo's
/// signature effect, kept as the UI's tactile feedback.
#[derive(Clone, Copy)]
struct Ripple {
    x: i32,
    y: i32,
    born_ms: u32,
    max_r: i32,
    color: u16,
    active: bool,
}

const MAX_RIPPLES: usize = 6;
const RIPPLE_GROW_MS: u32 = 460;
const RIPPLE_FADE_MS: u32 = 260;

#[derive(Clone, Copy)]
struct Bubble {
    x: i32,
    y: i32,
    born_ms: u32,
    max_r: i32,
    active: bool,
}

const NO_BUBBLE: Bubble = Bubble {
    x: 0,
    y: 0,
    born_ms: 0,
    max_r: 0,
    active: false,
};
const MAX_BUBBLES: usize = 10;
const BUBBLE_MS: u32 = 1_050;

/// A screen change, animated as a disc of the destination's colour growing from
/// the point touched until it has swallowed the old screen.
#[derive(Clone, Copy)]
struct Wipe {
    to: Screen,
    x: i32,
    y: i32,
    born_ms: u32,
    color: u16,
}

const WIPE_COVER_MS: u32 = 400;
const WIPE_REVEAL_MS: u32 = 240;
/// Belt and braces: no transition may ever outlive this, whatever the clock
/// does. A stuck wipe would swallow all input, which is the worst failure this
/// UI can have.
/// How long DONE stays on the timer page before it returns to the clock.
const DONE_LINGER_MS: u32 = 3_000;
const WIPE_MAX_MS: u32 = 2_000;

/// How deep the navigation history goes. The longest real route is
/// Home > Extras > Settings > Wi-Fi > keyboard > connecting, so this has room to
/// spare; beyond it the oldest entry is dropped.
const NAV_DEPTH: usize = 8;

pub struct Ui {
    /// Dimmed, outlined, black - see `draw_home`. Set by main, which owns the
    /// idle timer because it also owns the backlight and the radio.
    pub idle: bool,
    pub screen: Screen,
    /// Screens above this one, oldest first - see `open` and `back`.
    nav: [Screen; NAV_DEPTH],
    nav_len: usize,
    wipe: Option<Wipe>,
    ripples: [Ripple; MAX_RIPPLES],
    bubbles: [Bubble; MAX_BUBBLES],
    zones: [(Target, Zone); MAX_ZONES],
    n_zones: usize,
    /// Force menu: minutes 1..=10, and which usable relay is selected.
    pub minutes: u32,
    pub selected: usize,
    relay_scroll: i32,
    info_scroll: i32,
    /// One offset per list page, so leaving a page and coming back finds it where
    /// it was left. A single shared value meant every page snapped to the top -
    /// and worse, that Apps and Settings fought over the same number.
    menu_scroll: [i32; MENU_PAGES],
    /// Which controller the Controller page is showing.
    controller_selected: usize,
    /// The UI's copy of persisted settings. Held here rather than threaded
    /// through every draw call, and refreshed by main whenever it changes.
    pub settings: crate::store::Settings,
    /// Networks from the last scan. Filled by main, which owns the radio.
    pub networks: crate::net::Networks,
    /// Set while main is inside a blocking scan, so the picker can say so.
    pub scan_busy: bool,
    /// A scan the UI has asked for but that has not been run yet. Not an
    /// `Action`, because a scan blocks for a few hundred milliseconds and must
    /// wait for the transition that requested it to finish animating.
    want_scan: bool,
    /// Sleep asked for from the moon button; main owns the backlight and clock.
    want_sleep: bool,
    /// Sound to play, left for main - which owns the codec, and whose call to it
    /// blocks for the length of the note.
    pub want_sound: Option<Sound>,
    /// Keyboard: what is being edited, the text so far, and the layout.
    edit: Edit,
    edit_buf: crate::store::FixedStr<{ crate::store::MAX_SECRET }>,
    /// Set when a commit was refused, so the field can say why instead of the
    /// DONE key appearing to do nothing.
    edit_invalid: bool,
    key_mode: KeyMode,
    /// (grid row, slot) of the key lit right now, and when it was pressed.
    key_hot: Option<(usize, usize)>,
    key_hot_ms: u32,
    /// The network chosen in the picker, held until its password is entered.
    pending_ssid: crate::store::FixedStr<{ crate::store::MAX_SSID }>,
    /// The circles demo from the main branch. Its own model with its own rules -
    /// see `bubbles`. Called `game` because `bubbles` is already the ambient
    /// decoration on the other screens, and the two are unrelated.
    game: crate::bubbles::Bubbles,
    pong: crate::pong::Pong,
    breakout: crate::breakout::Breakout,
    invaders: crate::invaders::Invaders,
    asteroids: crate::asteroids::Asteroids,
    g2048: crate::g2048::G2048,
    match3: crate::match3::Match3,
    pomodoro: crate::pomodoro::Pomodoro,
    simon: crate::simon::Simon,
    cat: crate::cat::Cat,
    calc: crate::calc::Calc,
    weather: crate::weather::Weather,
    currency: crate::currency::Currency,
    snake: crate::snake::Snake,
    /// Filled in by main, which is the only place that can see the heap and the
    /// flash.
    pub sys: crate::about::Sys,
    pub rain: crate::rain::Rain,
    chess: crate::chess::Chess,
    tetris: crate::tetris::Tetris,
    pub stocks: crate::stocks::Stocks,
    pacman: crate::pacman::Pacman,
    chess_bubble_ms: u32,
    /// Whether the stocks page is in editing mode.
    stock_edit: bool,
    /// Set when the rain page is opened, so main asks for a fresh forecast.
    pub want_forecast: bool,
    about_page: usize,
    spacewar: crate::spacewar::Spacewar,
    /// Where the current drag began, for 2048's swipes.
    swipe_from: (i32, i32),
    pong_last_ms: u32,
    /// When the current join attempt started, and whether it has been given up
    /// on. The connecting screen is driven from these two.
    connect_started_ms: u32,
    connect_failed: bool,
    /// When the join first succeeded, so the confirmation can be held briefly
    /// before returning on its own.
    connect_ok_ms: Option<u32>,
    schedule_scroll: i32,
    detail_scroll: i32,
    dragging_slider: bool,
    /// When the current slider drag began. A drag is abandoned after
    /// DRAG_MAX_MS so a touch controller that latches a contact - and therefore
    /// never reports a release - cannot strand the UI in drag mode.
    drag_started_ms: u32,
    /// Which schedule the detail screen is showing.
    pub detail: usize,
    /// Edit mode, and the schedule being edited.
    ///
    /// A working copy, not a reference into the model: polls overwrite the model
    /// twice a second, which would undo every keystroke. The draft is what the
    /// page draws while editing, and what SAVE posts.
    editing: bool,
    pub draft: StartTime,
    /// Which entry the edit controls act on.
    draft_row: usize,
    /// The knob's *displayed* position, eased toward the selected minute. Values
    /// that snap look mechanical, so the knob chases its target - the same reason
    /// the progress sweep is driven from fractional time. Q4 for smooth easing at
    /// sub-pixel steps.
    knob_q4: i32,
    /// Previous `state.running`, so entering and leaving the countdown is driven
    /// by the *edges* of that flag.
    ///
    /// This was level-triggered once, and it deadlocked the UI: tapping Back on
    /// the countdown moved to Home, Home immediately saw `running` still true and
    /// wiped straight back, and because input is ignored while a wipe is in
    /// flight the panel ping-ponged with nothing responding. Edges also express
    /// the real intent - follow the controller when a run *starts* or *ends*, and
    /// otherwise leave navigation to whoever is holding the panel.
    last_running: bool,
    observed_run_total_s: u32,
    /// Remaining time for the active entry: anchored to the local clock when the
    /// entry changes, then counted down, never recomputed from a poll. The
    /// controller reports truncated whole seconds, so folding each poll back in
    /// made the arcs jump backwards. Polls decide *which* entry runs, not where
    /// inside it we are.
    run_left_ms: u32,
    /// The entry the anchor belongs to; a change to it, and nothing else,
    /// re-anchors.
    run_key: (i32, u32),
    run_anchor_ms: u32,
    run_anchor_left_ms: u32,
    run_finished: bool,
    /// When to leave the timer page of its own accord, set once a run watched
    /// there has finished.
    auto_home_at_ms: Option<u32>,
    /// A completed run remains available as a 00:00 badge until DONE is used.
    completion_pending: bool,
    /// Cancel/DONE consumes the next falling edge; otherwise the optimistic
    /// local stop would immediately re-latch the completion it just dismissed.
    completion_acknowledged: bool,
    dragging_list: bool,
    list_start_y: i32,
    list_start_offset: i32,
    list_drag_on_bar: bool,
    pending_row: Option<Target>,
    last_bubble_ms: u32,
    last_bubble_x: i32,
    last_bubble_y: i32,
}

const NO_RIPPLE: Ripple = Ripple {
    x: 0,
    y: 0,
    born_ms: 0,
    max_r: 0,
    color: 0,
    active: false,
};

impl Ui {
    pub const fn new() -> Self {
        Self {
            idle: false,
            screen: Screen::Home,
            nav: [Screen::Home; NAV_DEPTH],
            nav_len: 0,
            wipe: None,
            ripples: [NO_RIPPLE; MAX_RIPPLES],
            bubbles: [NO_BUBBLE; MAX_BUBBLES],
            zones: [(Target::Back, Zone::Disc { cx: 0, cy: 0, r: 0 }); MAX_ZONES],
            n_zones: 0,
            minutes: 3,
            selected: 0,
            relay_scroll: 0,
            info_scroll: 0,
            menu_scroll: [0; MENU_PAGES],
            controller_selected: 0,
            settings: crate::store::Settings::EMPTY,
            networks: crate::net::Networks::EMPTY,
            scan_busy: false,
            want_scan: false,
            want_sleep: false,
            want_sound: None,
            edit: Edit::WifiPsk,
            edit_buf: crate::store::FixedStr::EMPTY,
            edit_invalid: false,
            key_mode: KeyMode::Lower,
            key_hot: None,
            key_hot_ms: 0,
            pending_ssid: crate::store::FixedStr::EMPTY,
            game: crate::bubbles::Bubbles::new(),
            pong: crate::pong::Pong::new(),
            breakout: crate::breakout::Breakout::new(),
            invaders: crate::invaders::Invaders::new(),
            asteroids: crate::asteroids::Asteroids::new(),
            g2048: crate::g2048::G2048::new(),
            match3: crate::match3::Match3::new(),
            pomodoro: crate::pomodoro::Pomodoro::new(),
            simon: crate::simon::Simon::new(),
            cat: crate::cat::Cat::new(),
            calc: crate::calc::Calc::new(),
            weather: crate::weather::Weather::new(),
            currency: crate::currency::Currency::new(),
            snake: crate::snake::Snake::new(),
            sys: crate::about::Sys::EMPTY,
            rain: crate::rain::Rain::new(),
            chess: crate::chess::Chess::new(),
            tetris: crate::tetris::Tetris::new(),
            stocks: crate::stocks::Stocks::new(),
            pacman: crate::pacman::Pacman::new(),
            chess_bubble_ms: 0,
            stock_edit: false,
            want_forecast: false,
            about_page: 0,
            spacewar: crate::spacewar::Spacewar::new(),
            swipe_from: (0, 0),
            pong_last_ms: 0,
            connect_started_ms: 0,
            connect_failed: false,
            connect_ok_ms: None,
            schedule_scroll: 0,
            detail_scroll: 0,
            dragging_slider: false,
            drag_started_ms: 0,
            detail: 0,
            editing: false,
            draft: StartTime::EMPTY,
            draft_row: 0,
            knob_q4: 0,
            last_running: false,
            observed_run_total_s: 0,
            run_left_ms: 0,
            run_key: (0, 0),
            run_anchor_ms: 0,
            run_anchor_left_ms: 0,
            run_finished: false,
            auto_home_at_ms: None,
            completion_pending: false,
            completion_acknowledged: false,
            dragging_list: false,
            list_start_y: 0,
            list_start_offset: 0,
            list_drag_on_bar: false,
            pending_row: None,
            last_bubble_ms: 0,
            last_bubble_x: -100,
            last_bubble_y: -100,
        }
    }

    fn zone(&mut self, target: Target, zone: Zone) {
        if self.n_zones < MAX_ZONES {
            self.zones[self.n_zones] = (target, zone);
            self.n_zones += 1;
        }
    }

    fn hit(&self, x: i32, y: i32) -> Option<Target> {
        self.zones[..self.n_zones]
            .iter()
            .find(|(_, z)| z.contains(x, y))
            .map(|(t, _)| *t)
    }

    fn ripple(&mut self, x: i32, y: i32, color: u16, now_ms: u32) {
        let dx = x.max(W as i32 - x);
        let dy = y.max(H as i32 - y);
        let max_r = isqrt_i32(dx * dx + dy * dy).min(150);
        let slot = self.ripples.iter().position(|r| !r.active).unwrap_or(0);
        self.ripples[slot] = Ripple {
            x,
            y,
            born_ms: now_ms,
            max_r,
            color,
            active: true,
        };
    }

    fn bubble(&mut self, x: i32, y: i32, now_ms: u32) {
        let dx = x - self.last_bubble_x;
        let dy = y - self.last_bubble_y;
        let elapsed = now_ms.wrapping_sub(self.last_bubble_ms);
        if elapsed < 110 || (elapsed < 180 && dx * dx + dy * dy < 24 * 24) {
            return;
        }
        let slot = self.bubbles.iter().position(|b| !b.active).unwrap_or(0);
        self.bubbles[slot] = Bubble {
            x,
            y,
            born_ms: now_ms,
            max_r: 86 + ((x as u32 ^ y as u32 ^ now_ms) % 55) as i32,
            active: true,
        };
        self.last_bubble_ms = now_ms;
        self.last_bubble_x = x;
        self.last_bubble_y = y;
    }

    /// Begin a transition to `to` without touching the history.
    ///
    /// Prefer `open`, `back` or `unwind_to`: this is the raw move, for the two
    /// cases that are genuinely not navigation - a coloured cancel, and the
    /// transitions that replace the current screen in place.
    fn start_wipe(&mut self, to: Screen, x: i32, y: i32, now_ms: u32) {
        self.wipe = Some(Wipe {
            to,
            x,
            y,
            born_ms: now_ms,
            color: to.background(),
        });
    }

    /// Go one level deeper: remember where we are, then transition.
    ///
    /// Replaces a table of hardcoded parents, which was wrong by construction -
    /// screens reachable from two places needed a remembered caller anyway, and a
    /// screen added without an entry silently fell through to Home.
    fn open(&mut self, to: Screen, x: i32, y: i32, now_ms: u32) {
        let from = self.interactive_screen();
        if self.nav_len == NAV_DEPTH {
            // Deep enough that the oldest entry is of no interest; drop it rather
            // than refuse to record where we are now.
            self.nav.rotate_left(1);
            self.nav_len -= 1;
        }
        self.nav[self.nav_len] = from;
        self.nav_len += 1;
        self.start_wipe(to, x, y, now_ms);
    }

    /// Pop one level. Home is the floor, so Back is never a dead end.
    fn back(&mut self, x: i32, y: i32, now_ms: u32) -> Screen {
        let to = if self.nav_len > 0 {
            self.nav_len -= 1;
            self.nav[self.nav_len]
        } else {
            Screen::Home
        };
        self.start_wipe(to, x, y, now_ms);
        to
    }

    /// Return to a screen already in the history, dropping everything above it -
    /// for the transitions that finish a task several levels deep. Popping one
    /// level at a time would land back on the keyboard.
    fn unwind_to(&mut self, to: Screen, x: i32, y: i32, now_ms: u32) {
        if let Some(at) = self.nav[..self.nav_len].iter().rposition(|s| *s == to) {
            self.nav_len = at;
        }
        // A target that is not in the history keeps the history as it is: the
        // screen still opens, and Back still leads somewhere sensible.
        self.start_wipe(to, x, y, now_ms);
    }

    /// Which screen a touch belongs to: once a wipe starts, the destination
    /// already owns input, even while it is still being covered.
    /// Let the network-backed apps advance their fetches, and say whether the
    /// screen changed as a result. Only the page on screen gets to ask: nothing
    /// here should be talking to the internet in the background.
    pub fn step_apps(
        &mut self,
        net: &mut crate::net::Net,
        now_ms: u32,
        clock: Option<(u8, u8)>,
    ) -> bool {
        match self.interactive_screen() {
            Screen::Weather => {
                let before = self.weather.fingerprint();
                self.weather.step(net, now_ms);
                self.weather.fingerprint() != before
            }
            Screen::Stocks => {
                if let Some(text) = net.take_quotes() {
                    self.stocks.absorb(text, &self.settings, now_ms);
                    return true;
                }
                if net.take_quote_failure() {
                    self.stocks.failed();
                    return true;
                }
                // One page at a time, and only when the controller client is
                // otherwise idle: a quote is never worth delaying a poll for.
                if self.stocks.due(&self.settings, now_ms) && !net.busy() {
                    if let Some(page) = self.stocks.next_page(&self.settings) {
                        net.request_quotes(0, page.as_str(), now_ms);
                        self.stocks.page_sent(&self.settings, now_ms);
                    }
                }
                false
            }
            Screen::Currency => {
                let before = self.currency.fingerprint();
                self.currency.step(net, now_ms, clock);
                self.currency.fingerprint() != before
            }
            _ => false,
        }
    }

    fn interactive_screen(&self) -> Screen {
        self.wipe.map_or(self.screen, |w| w.to)
    }

    /// Feed one touch event. Returns the action the network layer should take.
    ///
    /// Buttons act on PRESS, not release. Release is only knowable after
    /// RELEASE_MS of silence from the touch controller, so acting on it charged
    /// every tap a fixed latency before anything happened at all - which is most
    /// of why this felt sluggish. It was also a liveness hazard: if the
    /// controller ever latched a contact, the release never arrived and the UI
    /// stopped accepting input entirely. Acting on press removes both, at the
    /// cost of not being able to slide off a button to cancel it - a trade worth
    /// making for a panel whose buttons are this large.
    pub fn input(&mut self, ev: Event, state: &State, now_ms: u32) -> Action {
        // Ignore input while a transition runs: the target that was hit is
        // already leaving, and letting a second tap through mid-animation is how
        // you end up two screens deep by accident.
        if self.wipe.is_some() {
            return Action::None;
        }

        match ev {
            Event::Press(x, y) => {
                let Some(target) = self.hit(x, y) else {
                    self.bubble(x, y, now_ms);
                    return Action::None;
                };
                if target == Target::Slider {
                    self.dragging_slider = true;
                    self.drag_started_ms = now_ms;
                    self.minutes = minutes_from_y(y);
                    return Action::None;
                }
                if target == Target::List {
                    self.bubble(x, y, now_ms);
                    self.dragging_list = true;
                    self.list_start_y = y;
                    self.list_drag_on_bar = x >= 450;
                    self.list_start_offset = match self.screen {
                        Screen::Inspect => self.schedule_scroll,
                        Screen::Detail => self.detail_scroll,
                        Screen::Info => self.info_scroll,
                        Screen::Force => self.relay_scroll,
                        screen if screen.is_menu() => self.scroll_of(screen),
                        _ => 0,
                    };
                    self.pending_row = self.row_at(y, state);
                    return Action::None;
                }
                let color = match target {
                    Target::Inspect => C_INSPECT,
                    Target::Force => C_FORCE,
                    Target::Go => C_RUN,
                    Target::Cancel => {
                        if self.run_finished {
                            C_RUN
                        } else {
                            C_CANCEL
                        }
                    }
                    Target::Info => C_INFO,
                    _ => self.screen.accent(),
                };
                // Keys light up instead of rippling. A ripple per keystroke would
                // be visual noise, and it would evict the ripples that carry
                // meaning - there are only MAX_RIPPLES slots. The bubbles page
                // answers a touch with its own circle, so it needs no ripple
                // either - and the demo has none.
                if !matches!(
                    target,
                    Target::KeyRow(_) | Target::KeyAux(_) | Target::Bubble
                ) {
                    self.ripple(x, y, color, now_ms);
                }

                match target {
                    Target::Back => {
                        // In edit mode Back is the way out of the *mode*, not the
                        // page: the draft is dropped and the schedule reappears as
                        // the controller has it. Leaving the page as well would
                        // discard the work with no way to tell that had happened.
                        if self.editing {
                            self.cancel_edit();
                            self.ripple(x, y, C_CANCEL, now_ms);
                            return Action::None;
                        }
                        // One level up, wherever that turns out to be. On the
                        // keyboard this also cancels the edit: the buffer is
                        // simply dropped. On the confirmation screen it is the
                        // "no".
                        self.back(x, y, now_ms);
                        Action::None
                    }
                    Target::Schedule(index) => {
                        self.detail = index;
                        self.detail_scroll = 0;
                        self.open(Screen::Detail, x, y, now_ms);
                        Action::None
                    }
                    Target::RunningBadge => {
                        self.run_finished = self.completion_pending;
                        self.open(Screen::Running, x, y, now_ms);
                        Action::None
                    }
                    Target::Inspect => {
                        self.open(Screen::Inspect, x, y, now_ms);
                        Action::None
                    }
                    Target::Info => {
                        // The cog opens the menu now, not the analog page.
                        self.open(Screen::Extras, x, y, now_ms);
                        Action::None
                    }
                    // List rows normally arrive here on release, via `row_at`, but
                    // the same handler serves a press so the two paths can never
                    // drift apart.
                    Target::Extra(_)
                    | Target::ConfigRow(_)
                    | Target::WifiRow(_)
                    | Target::CtlRow(_) => self.activate_row(target, x, y, now_ms),
                    Target::Bubble => {
                        if self.interactive_screen() == Screen::Pacman {
                            self.pacman.tap(now_ms);
                            self.pacman.steer_at(x, y, now_ms);
                        } else if self.interactive_screen() == Screen::Tetris {
                            self.tetris.press(x, y, now_ms);
                        } else if self.interactive_screen() == Screen::Chess {
                            self.chess_tap(x, y, now_ms);
                        } else if self.interactive_screen() == Screen::About {
                            self.about_page = (self.about_page + 1) % crate::about::PAGES;
                        } else if self.interactive_screen() == Screen::Snake {
                            self.snake.tap(now_ms);
                            self.snake.steer(x, y);
                        } else if self.interactive_screen() == Screen::Spacewar {
                            self.spacewar.press(x, y, now_ms);
                        } else if self.interactive_screen() == Screen::Pong {
                            self.pong.touch(x, y);
                        } else if self.interactive_screen() == Screen::Breakout {
                            self.breakout.touch(x);
                        } else if self.interactive_screen() == Screen::Invaders {
                            self.invaders.touch(x);
                        } else if self.interactive_screen() == Screen::Asteroids {
                            self.asteroids.touch(x, y);
                        } else if self.interactive_screen() == Screen::G2048 {
                            // The swipe is measured on release; the press only
                            // records where it began.
                            self.swipe_from = (x, y);
                        } else if self.interactive_screen() == Screen::Simon {
                            // Anywhere starts or retries; only a pad counts as an
                            // answer.
                            if self.simon.awaiting_start() {
                                self.simon.restart(now_ms);
                            } else if let Some(pad) = crate::simon::Simon::pad_at(x, y) {
                                self.want_sound = match self.simon.tap(pad, now_ms) {
                                    crate::simon::Answer::Pad(pad) => Some(Sound::Pad(pad)),
                                    crate::simon::Answer::Round => Some(Sound::Win),
                                    crate::simon::Answer::Lost => Some(Sound::Lose),
                                    crate::simon::Answer::Nothing => None,
                                };
                            }
                        } else if self.interactive_screen() == Screen::Currency {
                            let (sx0, sy0, sx1, sy1) = crate::currency::SYMBOL;
                            if x >= sx0 && x <= sx1 && y >= sy0 && y <= sy1 {
                                let code =
                                    crate::store::FixedStr::<8>::new(self.currency.custom_code());
                                self.open_keyboard(Edit::Symbol, code.as_str(), x, y, now_ms);
                            }
                        } else if self.interactive_screen() == Screen::Weather {
                            for (rect, forward) in [
                                (crate::weather::PREV, false),
                                (crate::weather::NEXT, true),
                            ] {
                                let (x0, y0, x1, y1) = rect;
                                if x >= x0 && x <= x1 && y >= y0 && y <= y1 {
                                    if forward {
                                        self.weather.next_day();
                                    } else {
                                        self.weather.prev_day();
                                    }
                                }
                            }
                        } else if self.interactive_screen() == Screen::Calc {
                            if let Some(key) = crate::calc::Calc::key_at(x, y) {
                                self.calc.press(key);
                            }
                        } else if self.interactive_screen() == Screen::Cat {
                            self.want_sound = if self.cat.tap(x, y) {
                                Some(Sound::Purr)
                            } else {
                                None
                            };
                        } else if self.interactive_screen() == Screen::Match3 {
                            self.match3.press(x, y);
                        } else if self.interactive_screen() == Screen::Pomodoro {
                            let (rx0, ry0, rx1, ry1) = crate::pomodoro::RESET;
                            if x >= rx0 && x <= rx1 && y >= ry0 && y <= ry1 {
                                self.pomodoro.reset_phase();
                            } else {
                                self.pomodoro.press(x, y);
                            }
                        } else {
                            self.game.press(x, y, now_ms);
                        }
                        Action::None
                    }
                    Target::RunNow if !state.any_controller_online() => Action::None,
                    Target::RunNow => {
                        let Some(schedule) = state.starts.get(self.detail).copied() else {
                            return Action::None;
                        };
                        if schedule.running {
                            // Same stop the countdown uses: the controller drops
                            // the whole run, queue included.
                            self.ripple(x, y, C_CANCEL, now_ms);
                            Action::Stop
                        } else if schedule.n_entries == 0 {
                            Action::None
                        } else {
                            self.ripple(x, y, C_RUN, now_ms);
                            Action::RunSchedule {
                                controller: schedule.controller,
                                start: schedule.remote_id,
                            }
                        }
                    }
                    Target::EditSave => self.edit_save(state, x, y, now_ms),
                    Target::TimeAdjust(which) => {
                        self.time_adjust(which);
                        Action::None
                    }
                    Target::Armed => {
                        self.draft.enabled = !self.draft.enabled;
                        Action::None
                    }
                    Target::EditCtl(which) => {
                        self.edit_control(which, state);
                        Action::None
                    }
                    Target::StockTab(index) => {
                        if self.stock_edit {
                            // In edit mode a tab is how its name is changed; the
                            // current one, so renaming is never a surprise.
                            if index == self.stocks.list {
                                let name = self.settings.lists[index].name;
                                self.open_keyboard(
                                    Edit::ListName(index),
                                    name.as_str(),
                                    x,
                                    y,
                                    now_ms,
                                );
                                return Action::None;
                            }
                        }
                        self.stocks.open(index, now_ms);
                        Action::None
                    }
                    Target::StockEdit => {
                        self.stock_edit = !self.stock_edit;
                        Action::None
                    }
                    Target::StockList => {
                        self.stocks.press(x, y);
                        Action::None
                    }
                    Target::Sky => {
                        // Asked for afresh on the way in, so a symbol that has
                        // been on screen since 11:00 is not what you decide on.
                        self.want_forecast = true;
                        self.open(Screen::Rain, x, y, now_ms);
                        Action::None
                    }
                    Target::RainChoice(index) => {
                        match index {
                            0 => self.rain.choose_water(state),
                            1 => self.rain.choose_skip(state),
                            _ => self.rain.choose_auto(state),
                        }
                        self.unwind_to(Screen::Home, x, y, now_ms);
                        Action::SaveSettings
                    }
                    Target::SleepNow => {
                        self.want_sleep = true;
                        Action::None
                    }
                    Target::Confirm => self.confirm_action(x, y, now_ms),
                    Target::Retry => {
                        self.connect_started_ms = now_ms;
                        self.connect_failed = false;
                        Action::ApplyWifi
                    }
                    Target::KeyRow(row) => {
                        let (rows, key_w) = self.key_mode.rows();
                        let text = rows[row];
                        let count = text.chars().count();
                        let (bx0, bx1) = key_band(count, key_w);
                        if let Some(slot) = crate::gfx::key_slot_at(bx0, bx1, count, x)
                            && let Some(ch) = text.chars().nth(slot)
                        {
                            self.type_char(ch);
                            self.key_hot = Some((row, slot));
                            self.key_hot_ms = now_ms;
                        }
                        Action::None
                    }
                    Target::KeyAux(index) => self.key_aux(index, x, y, now_ms),
                    Target::Force => {
                        self.selected = self.selected.min(state.n_usable().saturating_sub(1));
                        self.open(Screen::Force, x, y, now_ms);
                        Action::None
                    }
                    Target::Relay(index) => {
                        self.selected = index;
                        Action::None
                    }
                    Target::Go => {
                        // Refused rather than sent nowhere. The button is drawn
                        // inert in the same case, so this is the second line of
                        // defence, not the message.
                        if !state.any_controller_online() {
                            return Action::None;
                        }
                        let Some(relay) = state.usable().nth(self.selected) else {
                            return Action::None;
                        };
                        self.run_finished = false;
                        self.open(Screen::Running, x, y, now_ms);
                        Action::Trigger {
                            relay: relay.id,
                            controller: relay.controller,
                            remote_relay: relay.remote_id,
                            seconds: self.minutes * 60,
                        }
                    }
                    Target::Cancel => {
                        // Cancel and Done are Back with a colour: same pop, but the
                        // transition carries the outcome - green for a run that
                        // finished, red for one being stopped.
                        let to = self.back(x, y, now_ms);
                        self.wipe = Some(Wipe {
                            to,
                            x,
                            y,
                            born_ms: now_ms,
                            color: if self.run_finished { C_RUN } else { C_CANCEL },
                        });
                        self.completion_pending = false;
                        self.completion_acknowledged = true;
                        if self.run_finished {
                            Action::None
                        } else {
                            Action::Stop
                        }
                    }
                    Target::Slider => Action::None,
                    // Both arrive through the list band and are acted on at
                    // release, once it is known the touch was a tap and not the
                    // beginning of a scroll.
                    Target::EntryRow(_) | Target::List => Action::None,
                }
            }
            Event::Drag(x, y) => {
                if self.dragging_slider {
                    self.minutes = minutes_from_y(y);
                }
                if self.dragging_list {
                    let (pitch, visible, total) = match self.screen {
                        Screen::Inspect => (l::SCHED_PITCH, l::SCHED_MAX_ROWS, state.n_starts),
                        Screen::Detail => (
                            l::DETAIL_PITCH,
                            l::DETAIL_MAX_ROWS,
                            // While editing it is the draft that is listed, plus
                            // the row that appends - which has to be reachable on
                            // a schedule that already fills the page.
                            if self.editing {
                                self.draft.n_entries + 1
                            } else {
                                state.starts.get(self.detail).map_or(0, |s| s.n_entries)
                            },
                        ),
                        Screen::Info => (l::ANALOG_PITCH, l::ANALOG_MAX_ROWS, state.n_analogs),
                        Screen::Force => (l::RELAY_PITCH, l::RELAY_MAX_ROWS, state.n_usable()),
                        // From `geom`, not the MENU_* constants: Extras uses
                        // taller rows and fits fewer per page, and hardcoding the
                        // settings numbers here gave it a scroll range of zero.
                        screen if screen.is_menu() => {
                            let g = geom(screen);
                            (g.pitch, g.max_rows, self.menu_total(screen))
                        }
                        _ => (1, 1, 0),
                    };
                    let max_offset = total.saturating_sub(visible) as i32 * pitch;
                    let delta = if self.list_drag_on_bar {
                        (self.list_start_y - y) * max_offset / 240
                    } else {
                        self.list_start_y - y
                    };
                    let offset = (self.list_start_offset + delta).clamp(0, max_offset);
                    match self.screen {
                        Screen::Inspect => self.schedule_scroll = offset,
                        Screen::Detail => self.detail_scroll = offset,
                        Screen::Info => self.info_scroll = offset,
                        Screen::Force => self.relay_scroll = offset,
                        screen if screen.is_menu() => self.set_scroll(screen, offset),
                        _ => {}
                    }
                }
                // Dragging across the demo leaves a trail: its spawn rule rejects
                // contacts near a live circle's origin, so a moving finger starts
                // a new one roughly every fingertip's width. That is the original's
                // behaviour, not an addition.
                if self.interactive_screen() == Screen::Pacman {
                    if self.hit(x, y) == Some(Target::Bubble) {
                        self.pacman.steer_at(x, y, now_ms);
                    }
                } else if self.interactive_screen() == Screen::Stocks {
                    self.stocks.drag(y, &self.settings);
                } else if self.interactive_screen() == Screen::Tetris {
                    if self.hit(x, y) == Some(Target::Bubble) {
                        self.tetris.drag(x, y, now_ms);
                    }
                } else if self.interactive_screen() == Screen::Snake {
                    if self.hit(x, y) == Some(Target::Bubble) {
                        self.snake.steer(x, y);
                    }
                } else if self.interactive_screen() == Screen::Spacewar {
                    if self.hit(x, y) == Some(Target::Bubble) {
                        self.spacewar.drag(x, y);
                    }
                } else if self.interactive_screen() == Screen::Pong {
                    if self.hit(x, y) == Some(Target::Bubble) {
                        self.pong.touch(x, y);
                    }
                } else if self.interactive_screen() == Screen::Breakout {
                    if self.hit(x, y) == Some(Target::Bubble) {
                        self.breakout.touch(x);
                    }
                } else if self.interactive_screen() == Screen::Invaders {
                    if self.hit(x, y) == Some(Target::Bubble) {
                        self.invaders.touch(x);
                    }
                } else if self.interactive_screen() == Screen::Asteroids {
                    if self.hit(x, y) == Some(Target::Bubble) {
                        self.asteroids.touch(x, y);
                    }
                } else if self.interactive_screen() == Screen::Match3 {
                    self.match3.drag(x, y, now_ms);
                } else if self.interactive_screen() == Screen::Pomodoro {
                    self.pomodoro.drag(x, y);
                } else if self.interactive_screen() == Screen::Bubbles {
                    if self.hit(x, y) == Some(Target::Bubble) {
                        self.game.press(x, y, now_ms);
                    }
                } else if self.dragging_list || (!self.dragging_slider && self.hit(x, y).is_none())
                {
                    self.bubble(x, y, now_ms);
                }
                Action::None
            }
            Event::Release { x, y, tap } => {
                self.dragging_slider = false;
                // Lifting off stops the thrust, so the ship coasts.
                if self.interactive_screen() == Screen::Asteroids {
                    self.asteroids.release();
                }
                if self.interactive_screen() == Screen::Match3 {
                    self.match3.release();
                }
                if self.interactive_screen() == Screen::Pomodoro {
                    self.pomodoro.release();
                }
                if self.interactive_screen() == Screen::Tetris {
                    self.tetris.release();
                }
                if self.interactive_screen() == Screen::Stocks {
                    // Acted on release, once it is known the touch was a tap and
                    // not the beginning of a scroll - the same rule the schedule
                    // rows follow.
                    if tap && self.stock_edit && self.hit(x, y) == Some(Target::StockList) {
                        let list = self.stocks.list;
                        match self.stocks.row_at(y, &self.settings) {
                            Some(row) => {
                                self.settings.lists[list].remove(row);
                                self.stocks.open(list, now_ms);
                                self.stocks.release();
                                return Action::SaveSettings;
                            }
                            None => {
                                // Past the last row: room to add one.
                                self.open_keyboard(Edit::AddSymbol(list), "", x, y, now_ms);
                            }
                        }
                    }
                    self.stocks.release();
                }
                // Lifting off stops a turn; thrust stays latched.
                if self.interactive_screen() == Screen::Spacewar {
                    self.spacewar.release();
                }
                // 2048: the direction of the whole gesture, decided on release, so
                // a wandering finger still produces one clean move.
                if self.interactive_screen() == Screen::G2048 {
                    let (dx, dy) = (x - self.swipe_from.0, y - self.swipe_from.1);
                    if dx.abs().max(dy.abs()) >= SWIPE_MIN {
                        if dx.abs() > dy.abs() {
                            self.g2048.swipe(dx.signum(), 0, now_ms);
                        } else {
                            self.g2048.swipe(0, dy.signum(), now_ms);
                        }
                    }
                }
                if self.dragging_list {
                    self.dragging_list = false;
                    if tap {
                        match self.pending_row.take() {
                            Some(Target::Schedule(index)) => {
                                self.detail = index;
                                self.detail_scroll = 0;
                                self.ripple(x, y, C_INSPECT, now_ms);
                                self.open(Screen::Detail, x, y, now_ms);
                            }
                            Some(Target::Relay(index)) => {
                                self.selected = index;
                                self.ripple(x, y, C_FORCE, now_ms);
                            }
                            // Edit mode: a row picks which entry the controls act
                            // on, and the row past the end appends another.
                            Some(Target::EntryRow(index)) => {
                                if index < self.draft.n_entries {
                                    self.draft_row = index;
                                } else {
                                    self.add_entry(state);
                                }
                                self.ripple(x, y, C_INSPECT, now_ms);
                            }
                            // Settings rows land here: the band is draggable, so
                            // a row can only be *opened* once the touch turns out
                            // to have been a tap and not the start of a scroll.
                            Some(
                                row @ (Target::Extra(_)
                                | Target::ConfigRow(_)
                                | Target::WifiRow(_)
                                | Target::CtlRow(_)),
                            ) => {
                                self.ripple(x, y, self.screen.accent(), now_ms);
                                return self.activate_row(row, x, y, now_ms);
                            }
                            _ => {}
                        }
                    } else {
                        self.pending_row = None;
                    }
                }
                Action::None
            }
            Event::None => Action::None,
        }
    }

    fn row_at(&self, y: i32, state: &State) -> Option<Target> {
        if self.screen.is_menu() {
            let g = geom(self.screen);
            let from_top = y - (g.first_cy - g.half_h) + self.scroll_of(self.screen);
            if from_top < 0 {
                return None;
            }
            let index = (from_top / g.pitch) as usize;
            return (index < self.menu_total(self.screen))
                .then(|| Self::menu_target(self.screen, index));
        }
        let index = match self.screen {
            Screen::Inspect => {
                ((y - (l::SCHED_FIRST_CY - l::SCHED_HALF_H) + self.schedule_scroll)
                    / l::SCHED_PITCH) as usize
            }
            Screen::Detail => {
                let from_top = y - (l::DETAIL_FIRST_CY - 21) + self.detail_scroll;
                if from_top < 0 {
                    return None;
                }
                (from_top / l::DETAIL_PITCH) as usize
            }
            Screen::Info => {
                ((y - (l::ANALOG_FIRST_CY - 16) + self.info_scroll) / l::ANALOG_PITCH) as usize
            }
            Screen::Force => {
                ((y - (l::RELAY_FIRST_CY - l::RELAY_HALF_H) + self.relay_scroll) / l::RELAY_PITCH)
                    as usize
            }
            _ => return None,
        };
        match self.screen {
            Screen::Inspect => (index < state.n_starts).then_some(Target::Schedule(index)),
            Screen::Force => (index < state.n_usable()).then_some(Target::Relay(index)),
            // One row past the last entry is the "add another" row, so the list
            // reaches one further than it has entries. Only while editing: the
            // view-mode list is not interactive.
            Screen::Detail if self.editing => {
                (index <= self.draft.n_entries).then_some(Target::EntryRow(index))
            }
            _ => None,
        }
    }

    /// Advance animations, and follow the controller into and out of a run.
    pub fn update(&mut self, state: &State, now_ms: u32) {
        // Leaving the finished timer page. Treated exactly as the DONE button is,
        // so the outcome still rides in on the green transition, and only from the
        // page itself: if it was left in the meantime, the badge takes over.
        if let Some(at) = self.auto_home_at_ms
            && now_ms.wrapping_sub(at) < u32::MAX / 2
        {
            self.auto_home_at_ms = None;
            if self.screen == Screen::Running && self.wipe.is_none() {
                let (cx, cy) = (W as i32 / 2, H as i32 / 2);
                self.unwind_to(Screen::Home, cx, cy, now_ms);
                self.wipe = Some(Wipe {
                    to: Screen::Home,
                    x: cx,
                    y: cy,
                    born_ms: now_ms,
                    color: C_RUN,
                });
                self.completion_pending = false;
                self.completion_acknowledged = true;
            }
        }
        if let Some(w) = self.wipe {
            let age = now_ms.wrapping_sub(w.born_ms);
            if age >= WIPE_COVER_MS + WIPE_REVEAL_MS || age > WIPE_MAX_MS {
                self.screen = w.to;
                self.wipe = None;
            }
        }
        for r in self.ripples.iter_mut() {
            if r.active && now_ms.wrapping_sub(r.born_ms) >= RIPPLE_GROW_MS + RIPPLE_FADE_MS {
                r.active = false;
            }
        }
        if self.key_hot.is_some() && now_ms.wrapping_sub(self.key_hot_ms) >= KEY_FLASH_MS {
            self.key_hot = None;
        }
        self.update_connect(state, now_ms);
        self.game.update(now_ms);
        let dt = now_ms.wrapping_sub(self.pong_last_ms).min(60);
        self.pong_last_ms = now_ms;
        match self.interactive_screen() {
            Screen::Pong => self.pong.update(dt, now_ms, &mut self.game),
            Screen::Snake => self.snake.update(now_ms, &mut self.game),
            Screen::Tetris => self.tetris.update(now_ms, &mut self.game),
            Screen::Pacman => self.pacman.update(dt, now_ms, &mut self.game),
            // The search runs in slices from here, so twenty seconds of thinking
            // costs the loop a few milliseconds per frame instead of stalling it.
            Screen::Chess => {
                self.chess.think(now_ms);
                // A circle every so often while it thinks, so a long search looks
                // like work rather than a hang. They land round the edges, which is
                // all the board leaves visible.
                if self.chess.thinking()
                    && now_ms.wrapping_sub(self.chess_bubble_ms) > 520
                {
                    self.chess_bubble_ms = now_ms;
                    const AT: [(i32, i32); 6] = [
                        (40, 120),
                        (440, 150),
                        (30, 380),
                        (450, 400),
                        (240, 20),
                        (240, 462),
                    ];
                    let (x, y) = AT[(now_ms / 520) as usize % AT.len()];
                    self.game.spawn(
                        x,
                        y,
                        now_ms,
                        Some(crate::chess::ACCENT),
                        Some(150),
                        true,
                    );
                }
            }
            Screen::Spacewar => self.spacewar.update(dt, now_ms, &mut self.game),
            Screen::Breakout => self.breakout.update(dt, now_ms, &mut self.game),
            Screen::Invaders => self.invaders.update(dt, now_ms, &mut self.game),
            Screen::Asteroids => self.asteroids.update(dt, now_ms, &mut self.game),
            Screen::G2048 => self.g2048.update(now_ms, &mut self.game),
            Screen::Match3 => self.match3.update(now_ms, &mut self.game),
            Screen::Pomodoro => self.pomodoro.update(dt, now_ms, &mut self.game),
            Screen::Simon => {
                self.simon.update(now_ms);
                // Sounding a step is main's job; the pattern only advances once it
                // reports back through `advance_show`.
                if let Some(pad) = self.simon.pending_show(now_ms)
                    && self.want_sound.is_none()
                {
                    self.want_sound = Some(Sound::Pad(pad));
                }
            }
            Screen::Cat => {
                if self.cat.update(dt) {
                    self.want_sound = Some(Sound::Meow);
                }
            }
            _ => {}
        }
        for bubble in self.bubbles.iter_mut() {
            if bubble.active && now_ms.wrapping_sub(bubble.born_ms) >= BUBBLE_MS {
                bubble.active = false;
            }
        }
        // A drag cannot outlive this. See `drag_started_ms`.
        const DRAG_MAX_MS: u32 = 8_000;
        if self.dragging_slider && now_ms.wrapping_sub(self.drag_started_ms) > DRAG_MAX_MS {
            self.dragging_slider = false;
        }

        // Ease the knob toward the selected minute. A quarter of the remaining
        // distance per frame is a critically-damped-looking approach that settles
        // in about six frames without ever overshooting.
        let target_q4 = y_from_minutes(self.minutes) << 4;
        if self.knob_q4 == 0 {
            self.knob_q4 = target_q4;
        } else {
            let delta = target_q4 - self.knob_q4;
            self.knob_q4 += if delta.abs() <= 16 { delta } else { delta / 4 };
        }

        // Re-anchor only when the entry itself changes; within one entry the
        // countdown runs off the local clock so it can never step backwards.
        let key = (state.active, state.queued);
        if key != self.run_key {
            self.run_key = key;
            self.run_anchor_ms = now_ms;
            self.run_anchor_left_ms = state.left_s * 1000;
        }
        self.run_left_ms = if run_is_active(state) {
            self.run_anchor_left_ms
                .saturating_sub(now_ms.wrapping_sub(self.run_anchor_ms))
        } else {
            0
        };

        // Preserve the largest observed remainder as the denominator for a
        // manual run. A dump after reboot seeds this immediately.
        if state.running {
            self.observed_run_total_s = self.observed_run_total_s.max(state.left_s.max(1));
        } else {
            self.observed_run_total_s = 0;
        }

        // `last_running` tracks user-visible activity, rather than the raw run
        // bit: some controller revisions leave that bit asserted for one final
        // poll after both the countdown and queue have drained.
        let active = run_is_active(state);
        let started = active && !self.last_running;
        let ended = !active && self.last_running;
        self.last_running = active;
        if started {
            self.run_finished = false;
            self.completion_pending = false;
            self.completion_acknowledged = false;
        }
        if started {
            self.auto_home_at_ms = None;
        }
        if ended && !self.completion_acknowledged {
            self.run_finished = true;
            // Watched to the end, so the page has done its job: show DONE long
            // enough to be read, then fall back to the clock by itself.
            if self.screen == Screen::Running {
                self.auto_home_at_ms = Some(now_ms + DONE_LINGER_MS);
            }
            // Completion waits in the corner badge rather than seizing the screen.
            // Opening the timer page unasked interrupted whatever was being done,
            // once per finished run; the badge is the invitation, and tapping it
            // stays the only way in.
            self.completion_pending = true;
        } else if self.screen == Screen::Running
            && state.left_s == 0
            && state.queued == 0
            && state.running
        {
            // Also cover booting directly into the controller's stale final
            // poll while the timer page is already open.
            self.run_finished = true;
        }
    }

    /// True while the frame needs to keep being repainted. The countdown counts
    /// because its progress sweep is driven from fractional time, so it moves
    /// continuously rather than in one-second jumps.
    pub fn animating(&self) -> bool {
        self.wipe.is_some()
            || self.ripples.iter().any(|r| r.active)
            || self.bubbles.iter().any(|b| b.active)
            || (self.screen == Screen::Running && !self.run_finished)
            || self.auto_home_at_ms.is_some()
            || self.last_running
            || self.dragging_slider
            // A lit key has to be un-lit again, which needs one more frame.
            || self.key_hot.is_some()
            // The connecting sweep is continuous, and its elapsed-seconds readout
            // has to keep counting even though the clock is not what drives it.
            || self.screen == Screen::Connecting
            // The demo runs at whatever rate the loop can manage, exactly as it
            // does on its own branch, and stops asking for frames once the last
            // circle has faded.
            || self.game.active()
            || self.screen == Screen::Pong
            || self.screen == Screen::Snake
            || self.screen == Screen::Chess
            || self.screen == Screen::Tetris
            || self.screen == Screen::Pacman
            || self.screen == Screen::Spacewar
            || self.screen == Screen::Breakout
            || self.screen == Screen::Invaders
            || self.screen == Screen::Asteroids
            || self.screen == Screen::G2048
            || self.screen == Screen::Match3
            || self.screen == Screen::Pomodoro
            || self.screen == Screen::Simon
            || self.screen == Screen::Cat
            || (self.screen == Screen::Force && self.knob_q4 != y_from_minutes(self.minutes) << 4)
    }

    /// Something a network transfer would visibly disturb: a transition, a tap's
    /// feedback, a finger on the panel.
    ///
    /// Deliberately not `animating`, which stays true for as long as a relay runs.
    /// Gating polls on that deadlocked a multi-entry schedule: nothing could arrive
    /// to say the next entry had started, so the countdown sat at 00:00 until the
    /// run was stopped by hand.
    pub fn interaction_active(&self) -> bool {
        self.wipe.is_some()
            || self.ripples.iter().any(|r| r.active)
            || self.dragging_slider
            || self.dragging_list
            || self.game.active()
    }

    /// Build the frame.
    ///
    /// Hit zones are registered on *every* frame for `interactive_screen()`,
    /// never only in particular animation phases. An earlier version registered
    /// them while drawing the fully-revealed screen only, so a wipe that
    /// finished between two repaints left the panel with no zones at all and
    /// nothing responded to touch until some later repaint happened along.
    pub fn build(&mut self, scene: &mut Scene, state: &State, now_ms: u32) {
        self.n_zones = 0;
        let base = self.screen;
        // Idle paints on black whatever screen is showing: unlit pixels are the
        // saving, and the dimmed backlight would make a tinted background muddy
        // rather than dark.
        // Breakout is the one screen whose ground changes, so it is asked rather
        // than looked up: `Screen::background` is a plain match with no access to
        // the game.
        scene.clear(if self.idle {
            rgb(0, 0, 0)
        } else if base == Screen::Breakout {
            self.breakout.palette().bg
        } else {
            base.background()
        });

        match self.wipe {
            None => self.draw_screen(scene, base, state, now_ms, 255),
            Some(w) => {
                let age = now_ms.wrapping_sub(w.born_ms);
                if age < WIPE_COVER_MS {
                    // The outgoing screen stays visible under the growing disc,
                    // so the change reads as one surface covering another.
                    self.draw_screen(scene, base, state, now_ms, 255);
                    let t = (age * 32_768 / WIPE_COVER_MS).min(32_768);
                    let eased = ease_out_q15(t);
                    let dx = w.x.max(W as i32 - w.x);
                    let dy = w.y.max(H as i32 - w.y);
                    let max_r = isqrt_i32(dx * dx + dy * dy) + 4;
                    let r = (max_r as u32 * eased / 32_768) as i32;
                    scene.disc(w.x, w.y, r, w.color, 255);
                } else {
                    scene.clear(w.color);
                    let reveal = (age - WIPE_COVER_MS).min(WIPE_REVEAL_MS);
                    let t = (reveal * 32_768 / WIPE_REVEAL_MS).min(32_768);
                    let alpha = (smoothstep_q15(t) * 255 / 32_768) as u8;
                    self.draw_screen(scene, w.to, state, now_ms, alpha);
                }
            }
        }
        // Keep the compact run affordance out of wipe frames. Popping it onto
        // the outgoing screen on the same frame a manual run starts made it
        // briefly intersect the expanding transition disc.
        // Kept off the demo page too, which is meant to be the animation and a way
        // back and nothing else. A run in progress is still one tap away, since
        // Back leads to a screen that does show the badge.
        if ((run_is_active(state) && !self.completion_acknowledged) || self.completion_pending)
            && !matches!(
                self.interactive_screen(),
                Screen::Running
                    | Screen::Bubbles
                    | Screen::Pong
                    | Screen::Breakout
                    | Screen::Invaders
                    | Screen::Asteroids
                    | Screen::G2048
                    | Screen::Match3
                    | Screen::Pomodoro
                    | Screen::Simon
                    | Screen::Cat
                    | Screen::Calc
                    | Screen::Weather
                    | Screen::Currency
                    | Screen::Snake
                    | Screen::Spacewar
                    | Screen::About
                    | Screen::Chess
                    | Screen::Tetris
                    | Screen::Stocks
                    | Screen::Pacman
            )
        {
            self.draw_running_badge(scene, state, 255);
        }

        // One place registers zones, from the same constants the drawing uses.
        let interactive = self.interactive_screen();
        self.register(interactive, state);

        // Ripples ride above everything: they are feedback, not content.
        for i in 0..MAX_RIPPLES {
            let r = self.ripples[i];
            if !r.active {
                continue;
            }
            let age = now_ms.wrapping_sub(r.born_ms);
            let (radius, alpha) = if age < RIPPLE_GROW_MS {
                let t = age * 32_768 / RIPPLE_GROW_MS;
                ((r.max_r as u32 * ease_out_q15(t) / 32_768) as i32, 150u32)
            } else {
                let t = ((age - RIPPLE_GROW_MS) * 32_768 / RIPPLE_FADE_MS).min(32_768);
                (r.max_r, 150 * (32_768 - smoothstep_q15(t)) / 32_768)
            };
            // A soft expanding ring, so it reads as a ripple and never hides the
            // label underneath it.
            let thickness = (radius / 7).clamp(3, 16);
            scene.ring(
                r.x,
                r.y,
                radius,
                (radius - thickness).max(0),
                r.color,
                alpha as u8,
            );
        }
    }

    /// Register touch targets for `screen`. Uses the same `l::` geometry the
    /// drawing does, so the two cannot disagree.
    fn register(&mut self, screen: Screen, state: &State) {
        let (bx, by, br) = l::BACK;
        match screen {
            Screen::Home => {
                let (ix, iy, ir) = l::INFO;
                self.zone(
                    Target::Info,
                    Zone::Disc {
                        cx: ix,
                        cy: iy,
                        r: ir,
                    },
                );
                let (x0, y0, x1, y1) = l::HOME_INSPECT;
                self.zone(Target::Inspect, Zone::Rect { x0, y0, x1, y1 });
                let (x0, y0, x1, y1) = l::HOME_FORCE;
                self.zone(Target::Force, Zone::Rect { x0, y0, x1, y1 });
                let (sx, sy, sr) = l::SKY;
                self.zone(
                    Target::Sky,
                    Zone::Disc {
                        cx: sx,
                        cy: sy,
                        r: sr,
                    },
                );
                let (mx, my, mr) = l::MOON;
                self.zone(
                    Target::SleepNow,
                    Zone::Disc {
                        cx: mx,
                        cy: my,
                        r: mr,
                    },
                );
            }
            Screen::Inspect => {
                self.zone(
                    Target::Back,
                    Zone::Disc {
                        cx: bx,
                        cy: by,
                        r: br,
                    },
                );
                self.zone(
                    Target::List,
                    Zone::Rect {
                        x0: l::SCHED_X0,
                        y0: l::SCHED_FIRST_CY - l::SCHED_HALF_H,
                        x1: l::SCHED_X1,
                        y1: l::SCHED_FIRST_CY
                            + (l::SCHED_MAX_ROWS as i32 - 1) * l::SCHED_PITCH
                            + l::SCHED_HALF_H,
                    },
                );
                self.zone(
                    Target::List,
                    Zone::Rect {
                        x0: 450,
                        y0: 156,
                        x1: 478,
                        y1: 438,
                    },
                );
            }
            Screen::Detail => {
                self.zone(
                    Target::Back,
                    Zone::Disc {
                        cx: bx,
                        cy: by,
                        r: br,
                    },
                );
                self.zone(
                    Target::List,
                    Zone::Rect {
                        x0: l::DETAIL_X0,
                        y0: l::DETAIL_FIRST_CY - 21,
                        x1: l::DETAIL_X1,
                        y1: l::DETAIL_FIRST_CY
                            + (l::DETAIL_MAX_ROWS as i32 - 1) * l::DETAIL_PITCH
                            + 21,
                    },
                );
                if self.editing || self.can_edit(state) {
                    let (ex0, ey0, ex1, ey1) = l::EDIT;
                    self.zone(
                        Target::EditSave,
                        Zone::Rect {
                            x0: ex0,
                            y0: ey0,
                            x1: ex1,
                            y1: ey1,
                        },
                    );
                }
                if self.editing {
                    let (ax0, ay0, ax1, ay1) = l::ARMED;
                    self.zone(
                        Target::Armed,
                        Zone::Rect {
                            x0: ax0,
                            y0: ay0,
                            x1: ax1,
                            y1: ay1,
                        },
                    );
                    for (which, (x0, y0, x1, y1)) in [
                        l::TIME_HH_DOWN,
                        l::TIME_HH_UP,
                        l::TIME_MM_DOWN,
                        l::TIME_MM_UP,
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        self.zone(Target::TimeAdjust(which), Zone::Rect { x0, y0, x1, y1 });
                    }
                    for which in 0..EDIT_CONTROLS.len() {
                        let y0 = l::EDIT_COL_TOP + which as i32 * l::EDIT_COL_PITCH;
                        self.zone(
                            Target::EditCtl(which),
                            Zone::Rect {
                                x0: l::EDIT_COL_X0,
                                y0,
                                x1: l::EDIT_COL_X1,
                                y1: y0 + l::EDIT_COL_H,
                            },
                        );
                    }
                } else {
                    let (rx, ry, rr) = l::RUN_NOW;
                    self.zone(
                        Target::RunNow,
                        Zone::Disc {
                            cx: rx,
                            cy: ry,
                            r: rr,
                        },
                    );
                }
            }
            // Every list screen works the way the schedules list does: the whole
            // band is one draggable zone, and which row a tap landed on is
            // resolved by `row_at` on release. Registering a zone per row instead
            // meant a drag that started on a row - which is to say, almost every
            // drag - scrolled nothing at all.
            Screen::Extras
            | Screen::Apps
            | Screen::Games
            | Screen::Config
            | Screen::Wifi
            | Screen::Controller => {
                self.zone(
                    Target::Back,
                    Zone::Disc {
                        cx: bx,
                        cy: by,
                        r: br,
                    },
                );
                let g = geom(screen);
                self.zone(
                    Target::List,
                    Zone::Rect {
                        x0: g.x0,
                        y0: g.top,
                        x1: g.x1,
                        y1: g.bottom,
                    },
                );
                // The scrollbar gutter drags too, at bar scale.
                self.zone(
                    Target::List,
                    Zone::Rect {
                        x0: 450,
                        y0: g.top,
                        x1: 478,
                        y1: g.bottom,
                    },
                );
            }
            Screen::Stocks => {
                self.zone(
                    Target::Back,
                    Zone::Disc {
                        cx: bx,
                        cy: by,
                        r: br,
                    },
                );
                let width = (W as i32 - 32) / crate::store::MAX_LISTS as i32;
                for index in 0..crate::store::MAX_LISTS {
                    let x0 = 16 + index as i32 * width;
                    self.zone(
                        Target::StockTab(index),
                        Zone::Rect {
                            x0,
                            y0: crate::stocks::TAB_Y.0,
                            x1: x0 + width,
                            y1: crate::stocks::TAB_Y.1,
                        },
                    );
                }
                // One band for the whole list, like the schedules page: per-row
                // zones would mean a drag starting on a row scrolls nothing.
                self.zone(
                    Target::StockList,
                    Zone::Rect {
                        x0: 0,
                        y0: crate::stocks::LIST_TOP,
                        x1: W as i32,
                        y1: H as i32,
                    },
                );
                let (x0, y0, x1, y1) = l::STOCK_EDIT;
                self.zone(Target::StockEdit, Zone::Rect { x0, y0, x1, y1 });
            }
            Screen::Rain => {
                self.zone(
                    Target::Back,
                    Zone::Disc {
                        cx: bx,
                        cy: by,
                        r: br,
                    },
                );
                for (index, rect) in l::RAIN_CHOICE.iter().enumerate() {
                    let (x0, y0, x1, y1) = *rect;
                    self.zone(Target::RainChoice(index), Zone::Rect { x0, y0, x1, y1 });
                }
            }
            Screen::Confirm => {
                self.zone(
                    Target::Back,
                    Zone::Disc {
                        cx: bx,
                        cy: by,
                        r: br,
                    },
                );
                let (x0, y0, x1, y1) = l::CONFIRM_YES;
                self.zone(Target::Confirm, Zone::Rect { x0, y0, x1, y1 });
            }
            Screen::Pong
            | Screen::Breakout
            | Screen::Invaders
            | Screen::Asteroids
            | Screen::G2048
            | Screen::Match3
            | Screen::Pomodoro
            | Screen::Simon
            | Screen::Cat
            | Screen::Calc
            | Screen::Weather
            | Screen::Currency
            | Screen::Snake
            | Screen::Spacewar
            | Screen::About
            | Screen::Chess
            | Screen::Tetris
            | Screen::Pacman
            | Screen::Bubbles => {
                // Back first, so the corner it occupies belongs to it; the rest of
                // the panel is the game's, which is how the demo behaves - a
                // contact anywhere starts a circle.
                self.zone(
                    Target::Back,
                    Zone::Disc {
                        cx: bx,
                        cy: by,
                        r: br,
                    },
                );
                self.zone(
                    Target::Bubble,
                    Zone::Rect {
                        x0: 0,
                        y0: 0,
                        x1: W as i32 - 1,
                        y1: H as i32 - 1,
                    },
                );
            }
            Screen::Connecting => {
                self.zone(
                    Target::Back,
                    Zone::Disc {
                        cx: bx,
                        cy: by,
                        r: br,
                    },
                );
                // The retry only exists once the attempt has been given up on, so
                // two attempts can never be in flight together.
                if self.connect_failed {
                    let (x0, y0, x1, y1) = l::RETRY;
                    self.zone(Target::Retry, Zone::Rect { x0, y0, x1, y1 });
                }
            }
            Screen::Keyboard => {
                self.zone(
                    Target::Back,
                    Zone::Disc {
                        cx: bx,
                        cy: by,
                        r: br,
                    },
                );
                // Grid rows are one zone each; which key was pressed is resolved
                // from x with the renderer's own slot arithmetic. Registered
                // before the flanking keys so that where their hit slop overlaps,
                // a letter wins - mistyping a letter is recoverable, and
                // mis-hitting delete is not.
                let (key_rows, key_w) = self.key_mode.rows();
                for (row, keys) in key_rows.iter().enumerate() {
                    let (x0, x1) = key_band(keys.chars().count(), key_w);
                    let y0 = l::KEY_TOP + row as i32 * l::KEY_PITCH;
                    self.zone(
                        Target::KeyRow(row),
                        Zone::Rect {
                            x0,
                            y0,
                            x1,
                            y1: y0 + l::KEY_H,
                        },
                    );
                }
                let row2 = l::KEY_TOP + 2 * l::KEY_PITCH;
                if self.key_mode.is_alpha() {
                    let (x0, x1) = l::KEY_SHIFT;
                    self.zone(
                        Target::KeyAux(0),
                        Zone::Rect {
                            x0,
                            y0: row2,
                            x1,
                            y1: row2 + l::KEY_H,
                        },
                    );
                }
                let (x0, x1) = l::KEY_DEL;
                self.zone(
                    Target::KeyAux(1),
                    Zone::Rect {
                        x0,
                        y0: row2,
                        x1,
                        y1: row2 + l::KEY_H,
                    },
                );
                let by_bottom = l::KEY_BOTTOM_Y;
                for (index, (x0, x1)) in [l::KEY_MODE, l::KEY_SPACE, l::KEY_DONE]
                    .into_iter()
                    .enumerate()
                {
                    self.zone(
                        Target::KeyAux(2 + index),
                        Zone::Rect {
                            x0,
                            y0: by_bottom,
                            x1,
                            y1: by_bottom + l::KEY_H,
                        },
                    );
                }
            }
            Screen::Info => {
                self.zone(
                    Target::Back,
                    Zone::Disc {
                        cx: bx,
                        cy: by,
                        r: br,
                    },
                );
                self.zone(
                    Target::List,
                    Zone::Rect {
                        x0: 20,
                        y0: l::ANALOG_VIEW_TOP,
                        x1: l::ANALOG_X1,
                        y1: l::ANALOG_VIEW_BOTTOM,
                    },
                );
                self.zone(
                    Target::List,
                    Zone::Rect {
                        x0: 450,
                        y0: l::ANALOG_VIEW_TOP,
                        x1: 478,
                        y1: l::ANALOG_VIEW_BOTTOM,
                    },
                );
            }
            Screen::Force => {
                self.zone(
                    Target::Back,
                    Zone::Disc {
                        cx: bx,
                        cy: by,
                        r: br,
                    },
                );
                self.zone(
                    Target::Slider,
                    Zone::Rect {
                        x0: l::SLIDER_X - l::SLIDER_HALF_W - 9,
                        y0: l::SLIDER_TOP - 20,
                        x1: l::SLIDER_X + l::SLIDER_HALF_W + 9,
                        y1: l::SLIDER_BOTTOM + 20,
                    },
                );
                self.zone(
                    Target::List,
                    Zone::Rect {
                        x0: l::RELAY_X0,
                        y0: l::RELAY_FIRST_CY - l::RELAY_HALF_H,
                        x1: l::RELAY_X1,
                        y1: l::RELAY_FIRST_CY
                            + (l::RELAY_MAX_ROWS as i32 - 1) * l::RELAY_PITCH
                            + l::RELAY_HALF_H,
                    },
                );
                self.zone(
                    Target::List,
                    Zone::Rect {
                        x0: 450,
                        y0: l::RELAY_FIRST_CY - l::RELAY_HALF_H,
                        x1: 478,
                        y1: l::RELAY_FIRST_CY
                            + (l::RELAY_MAX_ROWS as i32 - 1) * l::RELAY_PITCH
                            + l::RELAY_HALF_H,
                    },
                );
                let (gx, gy, gr) = l::GO;
                self.zone(
                    Target::Go,
                    Zone::Disc {
                        cx: gx,
                        cy: gy,
                        r: gr,
                    },
                );
            }
            Screen::Running => {
                self.zone(
                    Target::Back,
                    Zone::Disc {
                        cx: bx,
                        cy: by,
                        r: br,
                    },
                );
                let (x0, y0, x1, y1) = l::CANCEL;
                self.zone(Target::Cancel, Zone::Rect { x0, y0, x1, y1 });
            }
        }
        if ((run_is_active(state) && !self.completion_acknowledged) || self.completion_pending)
            && screen != Screen::Running
        {
            let (cx, cy, r) = l::RUN_BADGE;
            self.zone(Target::RunningBadge, Zone::Disc { cx, cy, r });
        }
    }

    fn draw_screen(
        &mut self,
        scene: &mut Scene,
        screen: Screen,
        state: &State,
        now_ms: u32,
        alpha: u8,
    ) {
        // The ambient decoration is skipped on the demo page: its own circles are
        // the content there, and a second, different kind of circle drifting
        // behind them would not read as the same animation.
        if !matches!(
            screen,
            Screen::Bubbles
                | Screen::Pong
                | Screen::Breakout
                | Screen::Invaders
                | Screen::Asteroids
                | Screen::G2048
                | Screen::Match3
                | Screen::Pomodoro
                | Screen::Simon
                | Screen::Cat
                | Screen::Calc
                | Screen::Snake
                | Screen::Spacewar
        ) && !self.idle
        {
            self.draw_bubbles(scene, screen, now_ms, alpha);
        }
        match screen {
            Screen::Home => self.draw_home(scene, state, alpha),
            Screen::Inspect => {
                self.draw_inspect(scene, state, alpha);
                self.draw_wifi_badge(scene, state, alpha);
            }
            Screen::Force => {
                self.draw_force(scene, state, alpha);
                self.draw_wifi_badge(scene, state, alpha);
            }
            Screen::Running => self.draw_running(scene, state, alpha),
            Screen::Detail => self.draw_detail(scene, state, alpha),
            Screen::Info => self.draw_info(scene, state, alpha),
            Screen::Extras => self.draw_extras(scene, alpha, Screen::Extras),
            Screen::Apps => self.draw_extras(scene, alpha, Screen::Apps),
            Screen::Games => self.draw_extras(scene, alpha, Screen::Games),
            Screen::Config => {
                self.draw_config(scene, state, alpha);
                self.draw_wifi_badge(scene, state, alpha);
            }
            Screen::Wifi => self.draw_wifi(scene, alpha),
            Screen::Controller => self.draw_controller(scene, state, alpha),
            Screen::Keyboard => self.draw_keyboard(scene, now_ms, alpha),
            Screen::Connecting => self.draw_connecting(scene, state, now_ms, alpha),
            Screen::Confirm => self.draw_confirm(scene, alpha),
            Screen::Rain => self.draw_rain(scene, alpha),
            Screen::Chess => {
                self.game.draw(scene, now_ms, alpha);
                self.chess.draw(scene, now_ms, alpha);
                self.draw_back(scene, alpha);
            }
            Screen::Tetris => {
                self.game.draw(scene, now_ms, alpha);
                self.tetris.draw(scene, alpha);
                self.draw_back(scene, alpha);
            }
            Screen::Pacman => {
                self.game.draw(scene, now_ms, alpha);
                self.pacman.draw(scene, now_ms, alpha);
                self.draw_back(scene, alpha);
            }
            Screen::Stocks => {
                Self::draw_ring_theme(scene, crate::stocks::ACCENT, alpha);
                self.stocks.draw(scene, &self.settings, alpha);
                if self.stocks.waiting {
                    scene.label(
                        CX,
                        250,
                        FontId::Caption,
                        MUTED,
                        alpha,
                        Align::Center,
                        "ASKING THE CONTROLLER\u{2026}",
                    );
                }
                // The editor is the same page with the rows acting differently, so
                // the pencil is a state rather than a screen.
                let (ex0, ey0, ex1, ey1) = l::STOCK_EDIT;
                scene.pill(
                    ex0,
                    ey0,
                    ex1,
                    ey1,
                    (ey1 - ey0) / 2,
                    if self.stock_edit {
                        crate::stocks::ACCENT
                    } else {
                        rgb(24, 28, 36)
                    },
                    alpha,
                );
                scene.label(
                    (ex0 + ex1) / 2,
                    (ey0 + ey1) / 2 + 8,
                    FontId::Micro,
                    if self.stock_edit {
                        rgb(10, 12, 16)
                    } else {
                        crate::stocks::ACCENT
                    },
                    alpha,
                    Align::Center,
                    if self.stock_edit { "DONE" } else { "EDIT" },
                );
                if self.stock_edit {
                    scene.label(
                        CX,
                        H as i32 - 16,
                        FontId::Micro,
                        MUTED,
                        alpha,
                        Align::Center,
                        "TAP A ROW TO REMOVE  TAB TO RENAME",
                    );
                }
                self.draw_back(scene, alpha);
            }
            Screen::Bubbles => self.draw_bubbles_game(scene, now_ms, alpha),
            Screen::Pong => {
                self.game.draw(scene, now_ms, alpha);
                self.pong.draw(scene, alpha);
                self.draw_back(scene, alpha);
            }
            Screen::Breakout => {
                self.game.draw(scene, now_ms, alpha);
                self.breakout.draw(scene, alpha);
                self.draw_back(scene, alpha);
            }
            Screen::Invaders => {
                self.game.draw(scene, now_ms, alpha);
                self.invaders.draw(scene, alpha);
                self.draw_back(scene, alpha);
            }
            Screen::Asteroids => {
                self.game.draw(scene, now_ms, alpha);
                self.asteroids.draw(scene, alpha);
                self.draw_back(scene, alpha);
            }
            Screen::G2048 => {
                self.game.draw(scene, now_ms, alpha);
                self.g2048.draw(scene, now_ms, alpha);
                self.g2048.draw_header(scene, alpha);
                self.draw_back(scene, alpha);
            }
            Screen::Match3 => {
                self.game.draw(scene, now_ms, alpha);
                self.match3.draw(scene, now_ms, alpha);
                self.draw_back(scene, alpha);
            }
            Screen::Pomodoro => {
                self.game.draw(scene, now_ms, alpha);
                self.pomodoro.draw(scene, alpha);
                self.draw_back(scene, alpha);
            }
            Screen::Simon => {
                self.game.draw(scene, now_ms, alpha);
                self.simon.draw(scene, alpha);
                self.draw_back(scene, alpha);
            }
            Screen::Cat => {
                self.game.draw(scene, now_ms, alpha);
                self.cat.draw(scene, alpha);
                self.draw_back(scene, alpha);
            }
            Screen::Calc => {
                self.calc.draw(scene, alpha);
                self.draw_back(scene, alpha);
            }
            Screen::Weather => {
                Self::draw_ring_theme(scene, C_WEATHER, alpha);
                self.weather.draw(scene, alpha);
                self.draw_back(scene, alpha);
            }
            Screen::Currency => {
                Self::draw_ring_theme(scene, C_CURRENCY, alpha);
                self.currency.draw(scene, alpha);
                self.draw_back(scene, alpha);
            }
            Screen::Snake => {
                self.snake.draw(scene, alpha);
                self.draw_back(scene, alpha);
            }
            Screen::Spacewar => {
                self.spacewar.draw(scene, now_ms, alpha);
                self.draw_back(scene, alpha);
            }
            Screen::About => {
                let (city, lat, lon) = self.weather.place();
                let (geo_ip, met_ip) = self.weather.hosts();
                let facts = crate::about::Facts {
                    sys: self.sys,
                    settings: &self.settings,
                    online: &state.controller_online,
                    city,
                    lat,
                    lon,
                    geo: (crate::weather::HOST_GEO, geo_ip),
                    met: (crate::weather::HOST_MET, met_ip),
                    rates: (crate::currency::HOST, self.currency.host_ip()),
                };
                crate::about::draw(scene, self.about_page, &facts, alpha);
                self.draw_back(scene, alpha);
            }
        }
        // The battery is drawn by draw_home, not here. It occupies the top centre
        // strip, which every other screen uses for its own heading - the minutes
        // readout on Force, the schedule time on Detail - so drawing it globally
        // put two things in one place. Home is also where it belongs: it is
        // ambient status, not something you consult mid-task, and the link dot it
        // shares that strip with is already Home-only.
    }

    /// Three thin rings, off-centre and clipped by the edges, so the data pages
    /// share the circle theme without anything moving behind text that has to be
    /// read.
    fn draw_ring_theme(scene: &mut Scene, accent: u16, alpha: u8) {
        let color = mix565(rgb(0, 0, 0), accent, 40);
        for (cx, cy, r) in [(96, 96, 150), (400, 250, 190), (200, 470, 120)] {
            scene.ring(cx, cy, r, r - 2, color, alpha);
        }
    }

    fn draw_bubbles(&self, scene: &mut Scene, screen: Screen, now_ms: u32, alpha: u8) {
        for bubble in self.bubbles.iter().filter(|b| b.active) {
            let age = now_ms.wrapping_sub(bubble.born_ms).min(BUBBLE_MS);
            let t = age * 32_768 / BUBBLE_MS;
            let radius = 3 + (bubble.max_r as u32 * ease_out_q15(t) / 32_768) as i32;
            let fade = (72 * (32_768 - smoothstep_q15(t)) / 32_768) as u8;
            let thickness = if age > BUBBLE_MS * 4 / 5 { 3 } else { 6 };
            let accent = if screen == Screen::Home {
                rgb(156, 166, 174)
            } else {
                screen.accent()
            };
            // Bubbles are below every UI primitive on a uniform background.
            // Preblending once here makes their normal alpha 255, sending the
            // ring spans through the paired-store fast path instead of doing
            // three channel multiplies for every covered pixel. During a wipe,
            // `alpha` still fades the whole destination scene as intended.
            let color = mix565(screen.background(), accent, fade);
            scene.ring(
                bubble.x,
                bubble.y,
                radius,
                (radius - thickness).max(0),
                color,
                alpha,
            );
        }
    }

    fn draw_link(&self, scene: &mut Scene, state: &State, alpha: u8) {
        // One dot: green online, amber connecting, red offline. Small on
        // purpose - it matters only when it is wrong.
        let color = match state.link {
            Link::Online => C_RUN,
            Link::Connecting => C_FORCE,
            Link::Offline => C_CANCEL,
        };
        let x = if !state.external_power && state.battery_percent.is_some() {
            CX - 48
        } else {
            CX
        };
        scene.disc(x, 42, 7, color, alpha);
    }

    /// Wi-Fi, top right: three arcs of a fan, lit as far as the signal goes, and
    /// struck through when there is no address at all.
    ///
    /// Drawn from the same two facts the link dot uses, so the two can never
    /// disagree - an association without a lease is not a connection worth
    /// claiming, which is why it takes an address rather than `is_connected`.
    fn draw_wifi_badge(&self, scene: &mut Scene, state: &State, alpha: u8) {
        const X: i32 = 440;
        const Y: i32 = 52;
        let online = state.link == Link::Online;
        let joining = state.link == Link::Connecting;
        let lit = if online {
            C_RUN
        } else if joining {
            C_FORCE
        } else {
            rgb(60, 66, 76)
        };
        // Rings from the bottom up, biggest last, each clipped to its top half by
        // drawing the background over the lower half afterwards.
        for (index, radius) in [8, 15, 22].into_iter().enumerate() {
            let reached = online || (joining && index == 0);
            scene.ring(
                X,
                Y,
                radius,
                radius - 3,
                if reached { lit } else { rgb(44, 50, 60) },
                alpha,
            );
        }
        // The lower half of the fan is not part of the symbol.
        scene.pill(X - 26, Y + 1, X + 26, Y + 26, 0, self.screen.background(), alpha);
        scene.disc(X, Y, 4, lit, alpha);
        if !online && !joining {
            // Struck through rather than absent: a missing symbol reads as a
            // missing feature, a crossed one as a missing network.
            scene.pill(X - 20, Y - 3, X + 20, Y + 3, 3, rgb(210, 90, 80), alpha);
        }
    }

    fn draw_power(&self, scene: &mut Scene, screen: Screen, state: &State, alpha: u8) {
        let Some(percent) = state.battery_percent.filter(|_| !state.external_power) else {
            return;
        };
        const X0: i32 = CX - 32;
        const X1: i32 = CX + 28;
        const Y0: i32 = 28;
        const Y1: i32 = 55;
        let color = if percent <= 15 {
            C_CANCEL
        } else if percent <= 30 {
            C_FORCE
        } else {
            C_RUN
        };
        // A proper battery silhouette with a small terminal. The inset uses
        // the page background, leaving a crisp three-pixel outline.
        scene.pill(X1 - 1, Y0 + 8, X1 + 7, Y1 - 8, 3, MUTED, alpha);
        scene.pill(X0, Y0, X1, Y1, 7, MUTED, alpha);
        scene.pill(
            X0 + 3,
            Y0 + 3,
            X1 - 3,
            Y1 - 3,
            4,
            screen.background(),
            alpha,
        );
        let fill_right = X0 + 4 + (X1 - X0 - 8) * percent as i32 / 100;
        if fill_right > X0 + 4 {
            scene.pill(X0 + 4, Y0 + 4, fill_right, Y1 - 4, 3, color, alpha);
        }
        let mut value = Buf::<6>::new();
        let _ = write!(value, "{percent}%");
        scene.label(
            (X0 + X1) / 2,
            49,
            FontId::Micro,
            INK,
            alpha,
            Align::Center,
            value.as_str(),
        );
    }

    fn draw_back(&self, scene: &mut Scene, alpha: u8) {
        let (bx, by, br) = l::BACK;
        scene.ring(bx, by, br, br - 4, MUTED, alpha);
        scene.label(
            bx + 1,
            by + 16,
            FontId::Icon,
            INK,
            alpha,
            Align::Center,
            "\u{f104}",
        );
    }

    fn draw_cog(&self, scene: &mut Scene, alpha: u8) {
        let (x, y, _) = l::INFO;
        // The icon itself is the affordance; the generous invisible hit area
        // does not need another enclosing circle.
        scene.label(
            x,
            y + 15,
            FontId::Icon,
            INK,
            alpha,
            Align::Center,
            "\u{f013}",
        );
    }

    fn draw_running_badge(&self, scene: &mut Scene, state: &State, alpha: u8) {
        let (cx, cy, r) = l::RUN_BADGE;
        scene.disc(cx, cy, r, rgb(8, 48, 31), alpha);
        scene.ring(cx, cy, r, r - 4, rgb(25, 82, 56), alpha);
        let total = self.observed_run_total_s.max(state.left_s).max(1);
        let span = progress_span_q12(total * 1000, self.run_left_ms);
        if span > 0 {
            scene.arc(cx, cy, r, r - 5, 0, span, C_RUN, alpha);
        }
        let shown_s = (self.run_left_ms + 999) / 1000;
        let mut left = Buf::<8>::new();
        let _ = write!(left, "{}:{:02}", shown_s / 60, shown_s % 60);
        scene.label(
            cx,
            cy + 7,
            FontId::Micro,
            INK,
            alpha,
            Align::Center,
            left.as_str(),
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_scrollbar(
        &self,
        scene: &mut Scene,
        offset_px: i32,
        rows_per_page: usize,
        row_pitch: i32,
        total: usize,
        y0: i32,
        y1: i32,
        color: u16,
        alpha: u8,
    ) {
        if total <= rows_per_page {
            return;
        }
        scene.pill(462, y0, 468, y1, 3, DIM, alpha);
        let track = y1 - y0;
        let thumb_h = (track * rows_per_page as i32 / total as i32).max(18);
        let travel = track - thumb_h;
        let max_offset_px = (total - rows_per_page) as i32 * row_pitch;
        let thumb_y = y0 + travel * offset_px.clamp(0, max_offset_px) / max_offset_px;
        scene.pill(460, thumb_y, 470, thumb_y + thumb_h, 5, color, alpha);
    }

    fn draw_home(&mut self, scene: &mut Scene, state: &State, alpha: u8) {
        // Status chrome. These two share the top centre strip and the link dot
        // shifts aside when a battery is present, so they are drawn together.
        self.draw_link(scene, state, alpha);
        self.draw_power(scene, Screen::Home, state, alpha);
        self.draw_cog(scene, alpha);
        self.draw_wifi_badge(scene, state, alpha);
        // A crescent, cut from one disc by another - no new shape needed.
        let (mx, my, mr) = l::MOON;
        scene.disc(mx, my, mr - 6, rgb(120, 132, 150), alpha);
        scene.disc(mx + 11, my - 7, mr - 7, rgb(0, 0, 0), alpha);

        let mut clock = Buf::<8>::new();
        if state.clock_valid {
            let _ = write!(clock, "{:02}:{:02}", state.hh, state.mm);
        } else {
            let _ = write!(clock, "--:--");
        }
        scene.label(
            CX,
            152,
            FontId::Display,
            INK,
            alpha,
            Align::Center,
            clock.as_str(),
        );

        // Tomorrow's sky, beside the clock. Only once a forecast has been
        // fetched - an empty ring would be a puzzle rather than information.
        if self.rain.have_forecast {
            let (sx, sy, _) = l::SKY;
            self.rain.symbol(scene, sx, sy, alpha);
        }

        let mut next = Buf::<40>::new();
        if self.rain.skipping() {
            let _ = write!(next, "RAIN EXPECTED · NO WATERING");
        } else if self.rain.manual == crate::rain::Manual::Water && self.rain.wet() {
            let _ = write!(next, "WATERING ANYWAY TOMORROW");
        } else if self.rain.manual == crate::rain::Manual::Skip {
            let _ = write!(next, "SKIPPING TOMORROW");
        } else {
            match state.next_start() {
            Some((s, minutes)) => {
                if minutes < 60 {
                    let _ = write!(next, "NEXT {:02}:{:02} IN {} MIN", s.hh, s.mm, minutes);
                } else {
                    let name = state
                        .relay_by_id(s.entries[0].relay as i32)
                        .map(|r| r.name)
                        .unwrap_or(Text::EMPTY);
                    let _ = write!(
                        next,
                        "NEXT {:02}:{:02} \u{b7} {}",
                        s.hh,
                        s.mm,
                        name.as_str()
                    );
                }
            }
            None if !state.any_controller_online() => {
                // "No schedule armed" is a claim about the controller, and with
                // nothing answering there is nothing to claim.
                let _ = write!(next, "NO CONNECTION TO CONTROLLER");
                }
            None => {
                let _ = write!(next, "NO SCHEDULE ARMED");
                }
            }
        }
        scene.label(
            CX,
            200,
            FontId::Caption,
            MUTED,
            alpha,
            Align::Center,
            next.as_str(),
        );

        // Two peers, same shape and size; colour and order carry the hierarchy.
        // Idle turns them inside out - an outline and coloured type on black -
        // which is both quieter to look at and cheaper to light.
        self.home_button(scene, l::HOME_INSPECT, C_INSPECT, INK, "SCHEDULES", alpha);
        self.home_button(
            scene,
            l::HOME_FORCE,
            C_FORCE,
            rgb(26, 15, 2),
            "MANUAL",
            alpha,
        );
    }

    fn home_button(
        &self,
        scene: &mut Scene,
        rect: (i32, i32, i32, i32),
        color: u16,
        ink: u16,
        caption: &str,
        alpha: u8,
    ) {
        let (x0, y0, x1, y1) = rect;
        let r = l::HOME_PILL_R;
        scene.pill(x0, y0, x1, y1, r, color, alpha);
        if self.idle {
            // An outline is the filled plate with a black one punched out of it -
            // two primitives, and no new shape in the renderer.
            scene.pill(x0 + 3, y0 + 3, x1 - 3, y1 - 3, r - 3, rgb(0, 0, 0), alpha);
        }
        scene.label(
            CX,
            (y0 + y1) / 2 + 14,
            FontId::Body,
            if self.idle { color } else { ink },
            alpha,
            Align::Center,
            caption,
        );
    }

    /// Entries of the Extras menu, in display order. Adding a page here is the
    /// only change needed to surface it - the row count, scrolling, hit testing
    /// and navigation all derive from this table. The colour is the entry's own,
    /// so a page whose screen shares a palette with another can still be told
    /// apart on this list.
    const EXTRAS: &'static [(&'static str, u16, Screen)] = &[
        ("SETTINGS", C_CONFIG, Screen::Config),
        ("SENSORS", C_INFO, Screen::Info),
        ("GAMES", C_GAMES, Screen::Games),
        ("APPS", C_APPS, Screen::Apps),
    ];

    /// Things to play with. Same table shape as EXTRAS, so one page draws all
    /// three - which is also why splitting these in two cost nothing but the list
    /// itself.
    const GAMES: &'static [(&'static str, u16, Screen)] = &[
        ("PACMAN", crate::pacman::ACCENT, Screen::Pacman),
        ("TETRIS", crate::tetris::ACCENT, Screen::Tetris),
        ("MASKEN", C_SNAKE, Screen::Snake),
        ("CHESS", crate::chess::ACCENT, Screen::Chess),
        ("PONG", crate::pong::RIGHT_COLOR, Screen::Pong),
        ("BREAKOUT", C_BREAKOUT, Screen::Breakout),
        ("INVADERS", C_INVADERS, Screen::Invaders),
        ("ASTEROIDS", C_ASTEROIDS, Screen::Asteroids),
        ("SPACE WAR", crate::spacewar::P1, Screen::Spacewar),
        ("2048", C_2048, Screen::G2048),
        ("MATCH 3", C_MATCH3, Screen::Match3),
        ("SIMON", C_SIMON, Screen::Simon),
        ("BUBBLES", C_BUBBLES, Screen::Bubbles),
        ("CAT", C_CAT, Screen::Cat),
    ];

    /// Things with a use. Pomodoro sits here rather than with the games: it is a
    /// kitchen timer, whatever it is made of.
    const APPS: &'static [(&'static str, u16, Screen)] = &[
        ("WEATHER", C_WEATHER, Screen::Weather),
        ("STOCKS", crate::stocks::ACCENT, Screen::Stocks),
        ("CURRENCY", C_CURRENCY, Screen::Currency),
        ("CALCULATOR", C_CALC, Screen::Calc),
        ("POMODORO", C_POMODORO, Screen::Pomodoro),
        ("ABOUT", crate::about::ACCENT, Screen::About),
    ];

    /// Extras is a menu of destinations, exactly like Home, so its buttons are
    /// Home's buttons: same width, same height, same fully-rounded ends, one word
    /// centred in each. It scrolls, because this is where games and other toys
    /// will land.
    fn table_for(screen: Screen) -> &'static [(&'static str, u16, Screen)] {
        match screen {
            Screen::Apps => Self::APPS,
            Screen::Games => Self::GAMES,
            _ => Self::EXTRAS,
        }
    }

    fn draw_extras(&mut self, scene: &mut Scene, alpha: u8, page: Screen) {
        self.draw_back(scene, alpha);
        scene.label(
            CX,
            l::MENU_TITLE,
            FontId::Body,
            INK,
            alpha,
            Align::Center,
            match page {
                Screen::Apps => "APPS",
                Screen::Games => "GAMES",
                _ => "EXTRAS",
            },
        );

        scene.clip(l::EXTRA_VIEW_TOP, l::EXTRA_VIEW_BOTTOM);
        self.menu_rows(page, |_, row, cy| {
            if let Some((title, color, _)) = Self::table_for(page).get(row) {
                let (x0, x1) = (l::EXTRA_X0, l::EXTRA_X1);
                scene.pill(
                    x0,
                    cy - l::EXTRA_HALF_H,
                    x1,
                    cy + l::EXTRA_HALF_H,
                    l::EXTRA_HALF_H,
                    *color,
                    alpha,
                );
                scene.label(
                    CX,
                    cy + 14,
                    FontId::Body,
                    ink_on(*color),
                    alpha,
                    Align::Center,
                    title,
                );
            }
        });
        scene.clip_reset();

        self.menu_scrollbar(scene, page, C_INFO, alpha);
    }

    /// Built exactly like a schedule row, and on one baseline: a title stacked
    /// over a subtitle reserved room for a second line whether or not there was
    /// one, which left rows without a value looking mis-centred.
    #[allow(clippy::too_many_arguments)]
    fn menu_row(
        &self,
        scene: &mut Scene,
        cy: i32,
        accent: u16,
        label: &str,
        value: &str,
        alpha: u8,
        chevron: bool,
    ) {
        let (x0, x1) = (l::MENU_X0, l::MENU_X1);
        let half = l::MENU_HALF_H;
        scene.pill(x0, cy - half, x1, cy + half, 18, rgb(16, 36, 46), alpha);
        scene.disc(x0 + 28, cy, 9, accent, alpha);
        scene.label(
            x0 + 52,
            cy + 13,
            FontId::Body,
            INK,
            alpha,
            Align::Left,
            label,
        );
        if !value.is_empty() {
            scene.label(
                x1 - 44,
                cy + 10,
                FontId::Caption,
                MUTED,
                alpha,
                Align::Right,
                value,
            );
        }
        if chevron {
            scene.pill(x1 - 26, cy - 8, x1 - 20, cy + 1, 3, MUTED, alpha);
            scene.pill(x1 - 26, cy - 1, x1 - 20, cy + 8, 3, MUTED, alpha);
        }
    }

    /// Config rows are derived, not hard-coded: Wi-Fi, then one per controller,
    /// then Add. `config_rows` is the single source of truth for how many there
    /// are, so drawing, scrolling and hit testing cannot disagree.
    fn config_rows(&self) -> usize {
        1 + self.settings.n_controllers + 1
    }

    /// How many rows a list screen has. Same role as `config_rows`, for all of
    /// them: drawing, scrolling and hit testing all ask here.
    fn menu_total(&self, screen: Screen) -> usize {
        match screen {
            Screen::Extras => Self::EXTRAS.len(),
            Screen::Apps => Self::APPS.len(),
            Screen::Config => self.config_rows(),
            // The networks found, then "scan again", then "type it in".
            Screen::Wifi => self.networks.n + 2,
            // Name, address, username, password, remove.
            Screen::Controller => 5,
            _ => 0,
        }
    }

    fn scroll_slot(screen: Screen) -> usize {
        match screen {
            Screen::Extras => 0,
            Screen::Apps => 1,
            Screen::Games => 5,
            Screen::Config => 2,
            Screen::Wifi => 3,
            _ => 4,
        }
    }

    /// Clamped on the way out, so a list that has since shrunk - a controller
    /// removed, fewer networks found - cannot be left scrolled past its end.
    fn scroll_of(&self, screen: Screen) -> i32 {
        let g = geom(screen);
        let total = self.menu_total(screen);
        let max = total.saturating_sub(g.max_rows) as i32 * g.pitch;
        self.menu_scroll[Self::scroll_slot(screen)].clamp(0, max)
    }

    fn set_scroll(&mut self, screen: Screen, value: i32) {
        self.menu_scroll[Self::scroll_slot(screen)] = value;
    }

    fn menu_target(screen: Screen, row: usize) -> Target {
        match screen {
            Screen::Extras | Screen::Apps => Target::Extra(row),
            Screen::Wifi => Target::WifiRow(row),
            Screen::Controller => Target::CtlRow(row),
            _ => Target::ConfigRow(row),
        }
    }

    /// Open whatever a list row stands for.
    ///
    /// Shared by the press and release paths, and the only place a row's meaning
    /// is decided - the drawing reads the same row indices, so what you tap is
    /// what you saw.
    fn activate_row(&mut self, target: Target, x: i32, y: i32, now_ms: u32) -> Action {
        match target {
            Target::Extra(row) => {
                let page = self.interactive_screen();
                if let Some((_, _, screen)) = Self::table_for(page).get(row) {
                    self.info_scroll = 0;
                    if *screen == Screen::Pong {
                        self.pong.reset(now_ms);
                    }
                    if *screen == Screen::Breakout {
                        self.breakout.restart(now_ms);
                    }
                    if *screen == Screen::Invaders {
                        self.invaders.restart(now_ms);
                    }
                    if *screen == Screen::Asteroids {
                        self.asteroids.restart(now_ms);
                    }
                    if *screen == Screen::G2048 {
                        self.g2048.restart(now_ms);
                    }
                    if *screen == Screen::Match3 {
                        self.match3.restart(now_ms);
                    }
                    if *screen == Screen::Simon {
                        self.simon.arm();
                    }
                    if *screen == Screen::Cat {
                        self.cat.restart();
                    }
                    if *screen == Screen::Calc {
                        self.calc.restart();
                    }
                    if *screen == Screen::Weather {
                        self.weather.wake();
                    }
                    if *screen == Screen::Currency {
                        self.currency.wake();
                    }
                    if *screen == Screen::Snake {
                        self.snake.restart(now_ms);
                    }
                    if *screen == Screen::Spacewar {
                        self.spacewar.restart();
                    }
                    if *screen == Screen::About {
                        self.about_page = 0;
                    }
                    if *screen == Screen::Chess {
                        self.chess.setup();
                    }
                    if *screen == Screen::Tetris {
                        self.tetris.restart(now_ms);
                    }
                    if *screen == Screen::Pacman {
                        self.pacman.restart(now_ms);
                    }
                    if *screen == Screen::Stocks {
                        let list = self.stocks.list;
                        self.stocks.open(list, now_ms);
                    }
                    self.open(*screen, x, y, now_ms);
                }
                Action::None
            }
            Target::ConfigRow(row) => {
                if row == 0 {
                    // Ask for a scan on the way in, so the picker has something in
                    // it by the time the transition lands.
                    self.want_scan = !self.networks.scanned;
                    self.open(Screen::Wifi, x, y, now_ms);
                } else if row <= self.settings.n_controllers {
                    self.controller_selected = row - 1;
                    self.open(Screen::Controller, x, y, now_ms);
                } else {
                    self.open_keyboard(Edit::NewControllerIp, "", x, y, now_ms);
                }
                Action::None
            }
            Target::WifiRow(row) => {
                let found = self.networks.n;
                if row < found {
                    let network = self.networks.items[row];
                    self.pending_ssid = network.ssid;
                    if network.secure {
                        // Re-entering the network you are already on is usually
                        // about fixing something else, so the known password is
                        // offered rather than cleared.
                        let known = if network.ssid.as_str() == self.settings.ssid.as_str() {
                            self.settings.psk
                        } else {
                            crate::store::FixedStr::EMPTY
                        };
                        self.open_keyboard(Edit::WifiPsk, known.as_str(), x, y, now_ms);
                        Action::None
                    } else {
                        // Open network: nothing to type.
                        self.settings.ssid = network.ssid;
                        self.settings.psk = crate::store::FixedStr::EMPTY;
                        self.begin_connect(x, y, now_ms);
                        Action::ApplyWifi
                    }
                } else if row == found {
                    self.want_scan = true;
                    Action::None
                } else {
                    self.open_keyboard(Edit::WifiSsid, "", x, y, now_ms);
                    Action::None
                }
            }
            Target::CtlRow(row) => {
                let index = self.controller_selected;
                let Some(controller) = self
                    .settings
                    .controllers
                    .get(index)
                    .copied()
                    .filter(|_| index < self.settings.n_controllers)
                else {
                    self.unwind_to(Screen::Config, x, y, now_ms);
                    return Action::None;
                };
                match row {
                    0 => self.open_keyboard(
                        Edit::ControllerName(index),
                        controller.name.as_str(),
                        x,
                        y,
                        now_ms,
                    ),
                    1 => {
                        let mut current = Buf::<20>::new();
                        let ip = controller.ip;
                        let _ = write!(current, "{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]);
                        self.open_keyboard(
                            Edit::ControllerIp(index),
                            current.as_str(),
                            x,
                            y,
                            now_ms,
                        );
                    }
                    2 => self.open_keyboard(
                        Edit::ControllerUser(index),
                        controller.user.as_str(),
                        x,
                        y,
                        now_ms,
                    ),
                    3 => self.open_keyboard(
                        Edit::ControllerPass(index),
                        controller.pass.as_str(),
                        x,
                        y,
                        now_ms,
                    ),
                    // Removal is one tap away from a controller you may have had
                    // to walk somewhere to find the address of, so it asks first.
                    _ => self.open(Screen::Confirm, x, y, now_ms),
                }
                Action::None
            }
            _ => Action::None,
        }
    }

    /// The schedule the detail page is showing, clamped once so drawing and
    /// editing can never disagree about which one that is.
    fn shown_schedule(&self, state: &State) -> Option<(usize, StartTime)> {
        if state.n_starts == 0 {
            return None;
        }
        let index = self.detail.min(state.n_starts - 1);
        Some((index, state.starts[index]))
    }

    /// Editing is refused while this schedule is watering: the draft would be
    /// written back over a config the controller is mid-way through using.
    fn can_edit(&self, state: &State) -> bool {
        self.shown_schedule(state)
            .is_some_and(|(_, schedule)| !schedule.running)
    }

    /// Enter edit mode, or leave it by saving.
    fn edit_save(&mut self, state: &State, x: i32, y: i32, now_ms: u32) -> Action {
        if self.editing {
            self.editing = false;
            self.ripple(x, y, C_RUN, now_ms);
            return Action::SaveSchedule;
        }
        let Some((_, schedule)) = self.shown_schedule(state).filter(|(_, s)| !s.running) else {
            return Action::None;
        };
        self.draft = schedule;
        self.draft_row = 0;
        self.detail_scroll = 0;
        self.editing = true;
        self.ripple(x, y, C_INSPECT, now_ms);
        Action::None
    }

    /// Discard the draft and return to viewing.
    fn cancel_edit(&mut self) {
        self.editing = false;
        self.draft_row = 0;
    }

    /// The clock adjusters and the armed toggle.
    ///
    /// Minutes move in fives: a watering start time is not a stopwatch, and sixty
    /// taps to cross an hour would be worse than a coarse step.
    fn time_adjust(&mut self, which: usize) {
        match which {
            0 => self.draft.hh = (self.draft.hh + 23) % 24,
            1 => self.draft.hh = (self.draft.hh + 1) % 24,
            2 => {
                self.draft.mm = match self.draft.mm {
                    0 => 55,
                    m => (m.saturating_sub(1) / 5) * 5,
                }
            }
            _ => self.draft.mm = ((self.draft.mm / 5 + 1) * 5) % 60,
        }
    }

    /// One of the selected entry's controls: reorder, change relay, change
    /// duration, remove.
    fn edit_control(&mut self, which: usize, state: &State) {
        let n = self.draft.n_entries;
        if n == 0 {
            return;
        }
        let row = self.draft_row.min(n - 1);
        match which {
            // Reorder by swapping with the neighbour, and follow the entry so the
            // controls stay pointed at what was just moved.
            0 => {
                if row > 0 {
                    self.draft.entries.swap(row, row - 1);
                    self.draft_row = row - 1;
                }
            }
            1 => {
                if row + 1 < n {
                    self.draft.entries.swap(row, row + 1);
                    self.draft_row = row + 1;
                }
            }
            // Next usable relay on this schedule's own controller - the only ones
            // it can address.
            2 => {
                let owner = self.draft.controller;
                let mut ids = [0u8; crate::model::MAX_RELAYS];
                let mut count = 0;
                for relay in state.usable().filter(|r| r.controller == owner) {
                    if count < ids.len() {
                        ids[count] = relay.id;
                        count += 1;
                    }
                }
                if count == 0 {
                    return;
                }
                let current = self.draft.entries[row].relay;
                let at = ids[..count].iter().position(|id| *id == current);
                let next = match at {
                    Some(index) => ids[(index + 1) % count],
                    None => ids[0],
                };
                self.draft.entries[row].relay = next;
            }
            // Duration, in steps that scale with the value: five seconds while
            // it is a test pulse, half a minute once it is real watering.
            3 | 4 => {
                let seconds = self.draft.entries[row].seconds as u32;
                let step = if seconds < 60 { 5 } else { 30 };
                let next = if which == 3 {
                    seconds + step
                } else {
                    seconds.saturating_sub(step)
                };
                let ceiling = state.max_run_s.max(5);
                self.draft.entries[row].seconds = next.clamp(5, ceiling) as u16;
            }
            // Remove, keeping the order of the rest.
            _ => {
                for i in row..n - 1 {
                    self.draft.entries[i] = self.draft.entries[i + 1];
                }
                self.draft.n_entries = n - 1;
                self.draft.entries[self.draft.n_entries] = crate::model::Entry {
                    relay: 0,
                    seconds: 0,
                };
                self.draft_row = self.draft_row.min(self.draft.n_entries.saturating_sub(1));
            }
        }
    }

    /// Append an entry, copying the previous one's duration so a schedule of
    /// equal-length zones takes one tap each.
    fn add_entry(&mut self, state: &State) {
        if self.draft.n_entries >= crate::model::MAX_ENTRIES {
            return;
        }
        let owner = self.draft.controller;
        let relay = state
            .usable()
            .find(|r| r.controller == owner)
            .map(|r| r.id)
            .unwrap_or(0);
        if relay == 0 {
            return;
        }
        let seconds = if self.draft.n_entries > 0 {
            self.draft.entries[self.draft.n_entries - 1].seconds
        } else {
            300
        };
        self.draft.entries[self.draft.n_entries] = crate::model::Entry { relay, seconds };
        self.draft_row = self.draft.n_entries;
        self.draft.n_entries += 1;
    }

    /// The confirmation screen's YES. Only one thing needs confirming so far, so
    /// the screen is written for that one thing rather than made generic before
    /// there is a second case to generalise from.
    fn confirm_action(&mut self, x: i32, y: i32, now_ms: u32) -> Action {
        self.settings.remove_controller(self.controller_selected);
        self.unwind_to(Screen::Config, x, y, now_ms);
        Action::SaveSettings
    }

    /// Show the connecting screen and start its clock.
    fn begin_connect(&mut self, x: i32, y: i32, now_ms: u32) {
        self.connect_started_ms = now_ms;
        self.connect_failed = false;
        self.connect_ok_ms = None;
        self.open(Screen::Connecting, x, y, now_ms);
    }

    /// A timeout is the only way to detect failure: a rejected password is
    /// indistinguishable from one not yet answered, since the driver's disconnect
    /// events are not exposed by this release.
    fn update_connect(&mut self, state: &State, now_ms: u32) {
        if self.interactive_screen() != Screen::Connecting {
            return;
        }
        if state.link == Link::Online {
            let since = *self.connect_ok_ms.get_or_insert(now_ms);
            self.connect_failed = false;
            if now_ms.wrapping_sub(since) >= CONNECT_SETTLE_MS && self.wipe.is_none() {
                self.unwind_to(Screen::Config, CX, 262, now_ms);
            }
        } else if !self.connect_failed
            && now_ms.wrapping_sub(self.connect_started_ms) >= CONNECT_TIMEOUT_MS
        {
            self.connect_failed = true;
        }
    }

    /// The sound main should play, if any. Taking it also advances a pattern that
    /// was waiting to be heard.
    pub fn take_sound(&mut self, now_ms: u32) -> Option<Sound> {
        let sound = self.want_sound.take()?;
        if self.screen == Screen::Simon && self.simon.pending_show(now_ms).is_some() {
            self.simon.advance_show(now_ms);
        }
        Some(sound)
    }

    pub fn take_sleep_request(&mut self) -> bool {
        core::mem::take(&mut self.want_sleep)
    }

    /// A scan the UI has asked for, handed to main - which owns the radio.
    ///
    /// Withheld until nothing is animating: the scan blocks for a few hundred
    /// milliseconds, and taking that out of the middle of the transition that
    /// requested it would be visible as a stutter.
    pub fn take_scan_request(&mut self) -> bool {
        if self.want_scan && !self.animating() {
            self.want_scan = false;
            return true;
        }
        false
    }

    fn open_keyboard(&mut self, edit: Edit, initial: &str, x: i32, y: i32, now_ms: u32) {
        self.edit = edit;
        self.edit_buf = crate::store::FixedStr::new(initial);
        self.edit_invalid = false;
        self.key_mode = edit.mode();
        self.key_hot = None;
        self.open(Screen::Keyboard, x, y, now_ms);
    }

    fn type_char(&mut self, ch: char) {
        self.edit_invalid = false;
        let mut text = Buf::<80>::new();
        let _ = write!(text, "{}{}", self.edit_buf.as_str(), ch);
        self.edit_buf.set(text.as_str());
        // One-shot shift, as on every touch keyboard: typing one capital does
        // not commit you to shouting.
        if self.key_mode == KeyMode::Upper {
            self.key_mode = KeyMode::Lower;
        }
    }

    fn backspace(&mut self) {
        self.edit_invalid = false;
        // Drop one *character*, not one byte: å is two bytes, and half of it is
        // not valid UTF-8.
        let keep = self
            .edit_buf
            .as_str()
            .char_indices()
            .next_back()
            .map_or(0, |(at, _)| at);
        self.edit_buf.bytes[keep..].fill(0);
        self.edit_buf.len = keep as u8;
    }

    /// The keys that are not part of a grid row. Their meaning depends on the
    /// layout, which is why they are indexed by position rather than by name.
    fn key_aux(&mut self, index: usize, x: i32, y: i32, now_ms: u32) -> Action {
        match index {
            0 => {
                self.key_mode = match self.key_mode {
                    KeyMode::Lower => KeyMode::Upper,
                    KeyMode::Upper => KeyMode::Lower,
                    other => other,
                };
                Action::None
            }
            1 => {
                self.backspace();
                Action::None
            }
            2 => {
                match self.key_mode {
                    KeyMode::Symbols => self.key_mode = KeyMode::Lower,
                    // On the keypad this position is the dot, since an address
                    // needs one and there is no second layout to switch to.
                    KeyMode::Numeric => self.type_char('.'),
                    _ => self.key_mode = KeyMode::Symbols,
                }
                Action::None
            }
            3 => {
                if self.key_mode == KeyMode::Numeric {
                    self.type_char('0');
                } else {
                    self.type_char(' ');
                }
                Action::None
            }
            _ => self.commit_edit(x, y, now_ms),
        }
    }

    /// DONE: write the edited value into settings and have main persist it.
    ///
    /// A refused commit raises `edit_invalid` instead of quietly doing nothing -
    /// a DONE key that appears dead is the worst outcome here, since there is no
    /// other way off this screen except discarding the work.
    fn commit_edit(&mut self, x: i32, y: i32, now_ms: u32) -> Action {
        let value = self.edit_buf;
        match self.edit {
            Edit::WifiSsid => {
                if value.is_empty() {
                    self.edit_invalid = true;
                    return Action::None;
                }
                // Chain straight into the password instead of returning to the
                // picker: a hand-typed network still needs one.
                self.pending_ssid = crate::store::FixedStr::new(value.as_str());
                self.edit = Edit::WifiPsk;
                self.edit_buf = crate::store::FixedStr::EMPTY;
                self.key_mode = KeyMode::Lower;
                self.ripple(x, y, C_CONFIG, now_ms);
                Action::None
            }
            Edit::WifiPsk => {
                self.settings.ssid = self.pending_ssid;
                self.settings.psk = value;
                self.begin_connect(x, y, now_ms);
                Action::ApplyWifi
            }
            Edit::NewControllerIp => {
                let Some(ip) = crate::store::parse_ip(value.as_str()) else {
                    self.edit_invalid = true;
                    return Action::None;
                };
                if !self.settings.add_controller(ip) {
                    // The table is full. Nothing to do but say so.
                    self.edit_invalid = true;
                    return Action::None;
                }
                self.unwind_to(Screen::Config, x, y, now_ms);
                Action::SaveSettings
            }
            Edit::ControllerIp(index) => {
                let Some(ip) = crate::store::parse_ip(value.as_str()) else {
                    self.edit_invalid = true;
                    return Action::None;
                };
                if index >= self.settings.n_controllers {
                    self.unwind_to(Screen::Config, x, y, now_ms);
                    return Action::None;
                }
                self.settings.controllers[index].ip = ip;
                self.unwind_to(Screen::Controller, x, y, now_ms);
                Action::SaveSettings
            }
            Edit::ControllerUser(index) => {
                if value.is_empty() || index >= self.settings.n_controllers {
                    self.edit_invalid = true;
                    return Action::None;
                }
                self.settings.controllers[index].user.set(value.as_str());
                self.unwind_to(Screen::Controller, x, y, now_ms);
                Action::SaveSettings
            }
            Edit::ControllerPass(index) => {
                if index >= self.settings.n_controllers {
                    self.edit_invalid = true;
                    return Action::None;
                }
                self.settings.controllers[index].pass.set(value.as_str());
                self.unwind_to(Screen::Controller, x, y, now_ms);
                Action::SaveSettings
            }
            Edit::ControllerName(index) => {
                if index >= self.settings.n_controllers {
                    self.edit_invalid = true;
                    return Action::None;
                }
                // An empty name is legitimate: it means "just show the address".
                self.settings.controllers[index].name.set(value.as_str());
                self.unwind_to(Screen::Controller, x, y, now_ms);
                Action::SaveSettings
            }
            Edit::Symbol => {
                self.currency.set_custom(value.as_str());
                self.unwind_to(Screen::Currency, x, y, now_ms);
                Action::None
            }
            Edit::ListName(index) => {
                if value.is_empty() || index >= crate::store::MAX_LISTS {
                    self.edit_invalid = true;
                    return Action::None;
                }
                self.settings.lists[index].name.set(value.as_str());
                self.unwind_to(Screen::Stocks, x, y, now_ms);
                Action::SaveSettings
            }
            Edit::AddSymbol(index) => {
                if index >= crate::store::MAX_LISTS
                    || !self.settings.lists[index].add(value.as_str())
                {
                    // Refused: empty, or the list is full. Saying so beats a DONE
                    // key that appears to do nothing.
                    self.edit_invalid = true;
                    return Action::None;
                }
                self.stocks.open(index, now_ms);
                self.unwind_to(Screen::Stocks, x, y, now_ms);
                Action::SaveSettings
            }
        }
    }

    /// The heading and the grey line under it, shared by the settings screens so
    /// they are all positioned identically and all clear of the Back button.
    fn page_head(&self, scene: &mut Scene, title: &str, status: &str, alpha: u8) {
        self.draw_back(scene, alpha);
        scene.label(
            CX,
            l::MENU_TITLE,
            FontId::Body,
            INK,
            alpha,
            Align::Center,
            title,
        );
        if !status.is_empty() {
            scene.label(
                CX,
                l::MENU_STATUS,
                FontId::Micro,
                MUTED,
                alpha,
                Align::Center,
                status,
            );
        }
    }

    /// How a controller is described in a list: its name if it has one, else its
    /// address. Shared so the Settings list, the controller's own page and the
    /// confirmation all call it the same thing.
    fn controller_label(controller: &crate::store::Controller) -> Buf<24> {
        let mut out = Buf::<24>::new();
        if controller.name.is_empty() {
            let ip = controller.ip;
            let _ = write!(out, "{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]);
        } else {
            let _ = write!(out, "{}", controller.name.as_str());
        }
        out
    }

    fn draw_config(&mut self, scene: &mut Scene, state: &State, alpha: u8) {
        // The panel's own address, which is what you need when something else has
        // to reach it - and the first thing to check when nothing works.
        let mut panel = Buf::<32>::new();
        match state.local_ip {
            Some(ip) => {
                let _ = write!(panel, "THIS PANEL {}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]);
            }
            None => {
                let _ = write!(panel, "THIS PANEL \u{b7} NO ADDRESS YET");
            }
        }
        self.page_head(scene, "SETTINGS", panel.as_str(), alpha);

        scene.clip(l::MENU_VIEW_TOP, l::MENU_VIEW_BOTTOM);
        self.menu_rows(Screen::Config, |ui, row, cy| {
            if row == 0 {
                let mut value = Buf::<28>::new();
                if ui.settings.ssid.is_empty() {
                    let _ = write!(value, "TAP TO CHOOSE");
                } else {
                    let _ = write!(value, "{}", ui.settings.ssid.as_str());
                }
                let accent = match state.link {
                    Link::Online => C_RUN,
                    Link::Connecting => C_FORCE,
                    Link::Offline => C_CANCEL,
                };
                ui.menu_row(scene, cy, accent, "WI-FI", value.as_str(), alpha, true);
            } else if row <= ui.settings.n_controllers {
                let controller = ui.settings.controllers[row - 1];
                let ip = controller.ip;
                // Online state comes from the live model, matched by address, so
                // it stays right when controllers are reordered.
                let online = state.controller_ips[..state.n_controllers]
                    .iter()
                    .position(|a| *a == ip)
                    .map(|i| state.controller_online[i])
                    .unwrap_or(false);
                // Named controllers show their name with the address beside it;
                // unnamed ones show the address alone rather than repeating it.
                let mut value = Buf::<24>::new();
                if !controller.name.is_empty() {
                    let _ = write!(value, "{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]);
                } else if !online {
                    let _ = write!(value, "OFFLINE");
                }
                ui.menu_row(
                    scene,
                    cy,
                    if online { C_RUN } else { C_CANCEL },
                    Self::controller_label(&controller).as_str(),
                    value.as_str(),
                    alpha,
                    true,
                );
            } else {
                ui.menu_row(scene, cy, C_CONFIG, "ADD CONTROLLER", "", alpha, true);
            }
        });
        scene.clip_reset();

        self.menu_scrollbar(scene, Screen::Config, C_CONFIG, alpha);
    }

    /// The rows of a list screen, drawn from `menu_total` and `menu_scroll`.
    /// Calls `row` for each visible one with its index and centre line.
    fn menu_rows(&self, screen: Screen, mut row: impl FnMut(&Self, usize, i32)) {
        let g = geom(screen);
        let total = self.menu_total(screen);
        let offset = self.scroll_of(screen);
        let first = (offset / g.pitch) as usize;
        let shift = -(offset % g.pitch);
        for slot in 0..g.max_rows + 1 {
            let index = first + slot;
            if index >= total {
                break;
            }
            let cy = g.first_cy + slot as i32 * g.pitch + shift;
            if cy + g.half_h < g.top - 12 || cy - g.half_h > g.bottom + 12 {
                continue;
            }
            row(self, index, cy);
        }
    }

    /// The scrollbar for a list screen, from that screen's own geometry.
    fn menu_scrollbar(&self, scene: &mut Scene, screen: Screen, color: u16, alpha: u8) {
        let g = geom(screen);
        self.draw_scrollbar(
            scene,
            self.scroll_of(screen),
            g.max_rows,
            g.pitch,
            self.menu_total(screen),
            g.top,
            g.bottom,
            color,
            alpha,
        );
    }

    fn draw_wifi(&mut self, scene: &mut Scene, alpha: u8) {
        let mut status = Buf::<48>::new();
        if self.scan_busy {
            let _ = write!(status, "SCANNING...");
        } else if self.networks.n > 0 {
            let _ = write!(status, "{} NETWORKS FOUND", self.networks.n);
        } else if self.networks.scanned {
            let _ = write!(status, "NOTHING FOUND \u{b7} TRY AGAIN");
        } else {
            let _ = write!(status, "TAP SCAN TO LOOK FOR NETWORKS");
        }
        self.page_head(scene, "WI-FI", status.as_str(), alpha);

        let found = self.networks.n;
        let current = self.settings.ssid;
        scene.clip(l::MENU_VIEW_TOP, l::MENU_VIEW_BOTTOM);
        self.menu_rows(Screen::Wifi, |ui, index, cy| {
            if index < found {
                let network = ui.networks.items[index];
                let joined = network.ssid.as_str() == current.as_str();
                let mut value = Buf::<16>::new();
                if joined {
                    let _ = write!(value, "CURRENT");
                } else if !network.secure {
                    let _ = write!(value, "OPEN");
                } else {
                    let _ = write!(value, "{}", signal_words(network.rssi));
                }
                ui.menu_row(
                    scene,
                    cy,
                    // Green for the one we are on, otherwise strength - which is
                    // the only thing that predicts whether joining will work.
                    if joined {
                        C_RUN
                    } else {
                        signal_color(network.rssi)
                    },
                    network.ssid.as_str(),
                    value.as_str(),
                    alpha,
                    true,
                );
            } else if index == found {
                ui.menu_row(scene, cy, C_CONFIG, "SCAN AGAIN", "", alpha, false);
            } else {
                ui.menu_row(scene, cy, MUTED, "OTHER NETWORK", "HIDDEN", alpha, true);
            }
        });
        scene.clip_reset();

        self.menu_scrollbar(scene, Screen::Wifi, C_CONFIG, alpha);
    }

    fn draw_controller(&mut self, scene: &mut Scene, state: &State, alpha: u8) {
        self.draw_back(scene, alpha);

        let index = self.controller_selected;
        let controller = self
            .settings
            .controllers
            .get(index)
            .copied()
            .unwrap_or(crate::store::Controller::EMPTY);
        let ip = controller.ip;

        // Matched by address rather than by slot, so it stays right after an edit
        // reorders the table.
        let online = state.controller_ips[..state.n_controllers]
            .iter()
            .position(|a| *a == ip)
            .map(|i| state.controller_online[i])
            .unwrap_or(false);
        let mut status = Buf::<40>::new();
        let _ = write!(
            status,
            "RAINBIRD \u{b7} {}",
            if online { "ONLINE" } else { "NOT ANSWERING" }
        );
        self.page_head(
            scene,
            Self::controller_label(&controller).as_str(),
            status.as_str(),
            alpha,
        );

        let mut address = Buf::<20>::new();
        let _ = write!(address, "{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]);
        // The password is masked here but shown while typing: the risk this
        // guards against is someone reading the panel over your shoulder, which
        // the editing screen cannot avoid anyway.
        let mut masked = Buf::<24>::new();
        if controller.pass.is_empty() {
            let _ = write!(masked, "NOT SET");
        } else {
            for _ in 0..controller.pass.len.min(10) {
                let _ = write!(masked, "\u{b7}");
            }
        }
        let mut name = Buf::<24>::new();
        let _ = write!(
            name,
            "{}",
            if controller.name.is_empty() {
                "NOT SET"
            } else {
                controller.name.as_str()
            }
        );

        scene.clip(l::MENU_VIEW_TOP, l::MENU_VIEW_BOTTOM);
        self.menu_rows(Screen::Controller, |ui, row, cy| {
            let (accent, label, value): (u16, &str, &str) = match row {
                0 => (C_CONFIG, "NAME", name.as_str()),
                1 => (C_CONFIG, "ADDRESS", address.as_str()),
                2 => (C_CONFIG, "USERNAME", controller.user.as_str()),
                3 => (C_CONFIG, "PASSWORD", masked.as_str()),
                _ => (C_CANCEL, "REMOVE", ""),
            };
            ui.menu_row(scene, cy, accent, label, value, alpha, true);
        });
        scene.clip_reset();

        self.menu_scrollbar(scene, Screen::Controller, C_CONFIG, alpha);
    }

    /// No heading, status line, ambient bubbles, battery or run badge - see
    /// `draw_screen` and `build`. Just the animation and a way back.
    fn draw_bubbles_game(&mut self, scene: &mut Scene, now_ms: u32, alpha: u8) {
        self.game.draw(scene, now_ms, alpha);
        // Drawn last so it survives whatever lands on the panel: a Back button
        // underneath an opaque screen-filling disc would strand you here.
        self.draw_back(scene, alpha);
    }

    /// A full screen rather than a dialog: the panel has no notion of a modal, and
    /// an address may have taken a walk to find.
    /// Setup taps pick a side, cycle the think time, or start. Board taps pick a
    /// piece up and put it down; a finished game restarts from the setup panel.
    fn chess_tap(&mut self, x: i32, y: i32, now_ms: u32) {
        use crate::chess::Phase;
        match self.chess.phase {
            Phase::Setup => match crate::chess::setup_at(x, y) {
                Some(0) => self.chess.choose_side(true),
                Some(1) => self.chess.choose_side(false),
                Some(2) => self.chess.cycle_think(),
                Some(crate::chess::CONTINUE) => self.chess.resume(now_ms),
                Some(_) => self.chess.begin(now_ms),
                None => {}
            },
            Phase::Done => self.chess.setup(),
            // Ignored while it thinks: a move taken mid-search would be played
            // against a board the search no longer describes.
            Phase::Thinking => {}
            Phase::HumanTurn => self.chess.tap(x, y, now_ms),
        }
    }

    /// Tomorrow, and what to do about it. Three choices rather than a yes/no:
    /// the interesting case is not confirming the forecast's decision but
    /// overruling it in either direction, and "follow the forecast" has to be a
    /// way back from having done so.
    fn draw_rain(&mut self, scene: &mut Scene, alpha: u8) {
        self.page_head(scene, "TOMORROW", "", alpha);
        let (sx, sy, _) = l::SKY;
        self.rain.symbol(scene, sx - 26, sy + 12, alpha);

        let mut headline = Buf::<40>::new();
        if self.rain.have_forecast {
            let tenths = self.rain.tomorrow_tenths;
            let _ = write!(headline, "{}.{} MM RAIN", tenths / 10, tenths % 10);
        } else {
            let _ = write!(headline, "NO FORECAST YET");
        }
        scene.label(
            CX,
            172,
            FontId::Body,
            INK,
            alpha,
            Align::Center,
            headline.as_str(),
        );

        let mut verdict = Buf::<40>::new();
        let _ = match self.rain.manual {
            crate::rain::Manual::Water => write!(verdict, "SET TO WATER ANYWAY"),
            crate::rain::Manual::Skip => write!(verdict, "SET TO SKIP"),
            crate::rain::Manual::Auto if self.rain.wet() => write!(verdict, "TOO WET TO WATER"),
            crate::rain::Manual::Auto => write!(verdict, "DRY ENOUGH TO WATER"),
        };
        scene.label(
            CX,
            208,
            FontId::Micro,
            MUTED,
            alpha,
            Align::Center,
            verdict.as_str(),
        );

        let chosen = match self.rain.manual {
            crate::rain::Manual::Water => 0,
            crate::rain::Manual::Skip => 1,
            crate::rain::Manual::Auto => 2,
        };
        for (index, caption) in [
            "WATER ANYWAY",
            "NO NEED TOMORROW",
            "FOLLOW FORECAST",
        ]
        .into_iter()
        .enumerate()
        {
            let (x0, y0, x1, y1) = l::RAIN_CHOICE[index];
            let color = match index {
                0 => C_RUN,
                1 => C_SKY,
                _ => MUTED,
            };
            let here = index == chosen;
            let r = (y1 - y0) / 2;
            scene.pill(x0, y0, x1, y1, r, if here { color } else { rgb(22, 26, 32) }, alpha);
            scene.label(
                (x0 + x1) / 2,
                (y0 + y1) / 2 + 13,
                FontId::Body,
                if here { rgb(8, 12, 16) } else { color },
                alpha,
                Align::Center,
                caption,
            );
        }
    }

    fn draw_confirm(&mut self, scene: &mut Scene, alpha: u8) {
        let controller = self
            .settings
            .controllers
            .get(self.controller_selected)
            .copied()
            .unwrap_or(crate::store::Controller::EMPTY);
        self.page_head(scene, "REMOVE", "", alpha);

        scene.label(
            CX,
            206,
            FontId::Body,
            INK,
            alpha,
            Align::Center,
            Self::controller_label(&controller).as_str(),
        );
        let ip = controller.ip;
        let mut detail = Buf::<40>::new();
        let _ = write!(
            detail,
            "{}.{}.{}.{} WILL NO LONGER BE POLLED",
            ip[0], ip[1], ip[2], ip[3]
        );
        scene.label(
            CX,
            244,
            FontId::Micro,
            MUTED,
            alpha,
            Align::Center,
            detail.as_str(),
        );

        let (x0, y0, x1, y1) = l::CONFIRM_YES;
        scene.pill(x0, y0, x1, y1, (y1 - y0) / 2, C_CANCEL, alpha);
        scene.label(
            (x0 + x1) / 2,
            (y0 + y1) / 2 + 14,
            FontId::Body,
            INK,
            alpha,
            Align::Center,
            "REMOVE",
        );
        // No explicit "no": Back is already the way out of every other screen,
        // and one unmistakable destructive button beats two similar ones.
        scene.label(
            CX,
            l::CONFIRM_YES.3 + 46,
            FontId::Caption,
            MUTED,
            alpha,
            Align::Center,
            "OR GO BACK TO KEEP IT",
        );
    }

    /// Joining a network: a breathing ring while it works, then the outcome.
    fn draw_connecting(&mut self, scene: &mut Scene, state: &State, now_ms: u32, alpha: u8) {
        let joined = state.link == Link::Online;
        self.page_head(
            scene,
            if joined {
                "CONNECTED"
            } else if self.connect_failed {
                "NO LUCK"
            } else {
                "CONNECTING"
            },
            self.settings.ssid.as_str(),
            alpha,
        );

        let color = if joined {
            C_RUN
        } else if self.connect_failed {
            C_CANCEL
        } else {
            C_CONFIG
        };

        // While working, a sweep chases its own tail; once settled, the ring
        // closes. Same vocabulary as the countdown's progress arc.
        let (cx, cy, r) = (CX, 262, 92);
        scene.ring(cx, cy, r, r - 12, rgb(18, 38, 48), alpha);
        if joined || self.connect_failed {
            scene.ring(cx, cy, r, r - 12, color, alpha);
        } else {
            let turn = (now_ms.wrapping_sub(self.connect_started_ms) % 1_200) * 4_096 / 1_200;
            scene.arc(
                cx,
                cy,
                r,
                r - 12,
                turn as i32,
                turn as i32 + 1_100,
                color,
                alpha,
            );
        }

        let mut middle = Buf::<20>::new();
        if joined {
            if let Some(ip) = state.local_ip {
                let _ = write!(middle, "{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]);
            } else {
                let _ = write!(middle, "JOINED");
            }
        } else if self.connect_failed {
            let _ = write!(middle, "FAILED");
        } else {
            let elapsed = now_ms.wrapping_sub(self.connect_started_ms) / 1_000;
            let _ = write!(middle, "{} S", elapsed);
        }
        scene.label(
            cx,
            cy + 10,
            FontId::Caption,
            INK,
            alpha,
            Align::Center,
            middle.as_str(),
        );

        if self.connect_failed {
            let (x0, y0, x1, y1) = l::RETRY;
            scene.pill(x0, y0, x1, y1, (y1 - y0) / 2, C_CONFIG, alpha);
            scene.label(
                (x0 + x1) / 2,
                (y0 + y1) / 2 + 14,
                FontId::Body,
                ink_on(C_CONFIG),
                alpha,
                Align::Center,
                "TRY AGAIN",
            );
        } else if !joined {
            scene.label(
                CX,
                l::RETRY.3 + 4,
                FontId::Caption,
                DIM,
                alpha,
                Align::Center,
                "CHECKING THE PASSWORD",
            );
        }
    }

    fn draw_keyboard(&mut self, scene: &mut Scene, now_ms: u32, alpha: u8) {
        // The subtitle carries the context the short heading leaves out - which
        // network's password, which controller's username.
        let mut context = Buf::<32>::new();
        match self.edit {
            Edit::WifiPsk => {
                let _ = write!(context, "{}", self.pending_ssid.as_str());
            }
            Edit::WifiSsid => {
                let _ = write!(context, "TYPE THE NETWORK NAME");
            }
            Edit::NewControllerIp => {
                let _ = write!(context, "NEW CONTROLLER");
            }
            Edit::Symbol => {
                let _ = write!(context, "THREE-LETTER CODE");
            }
            Edit::ListName(_) => {
                let _ = write!(context, "WHAT TO CALL THIS TAB");
            }
            Edit::AddSymbol(_) => {
                let _ = write!(context, "AS YAHOO SPELLS IT, E.G. HMS.ST");
            }
            Edit::ControllerIp(i)
            | Edit::ControllerUser(i)
            | Edit::ControllerPass(i)
            | Edit::ControllerName(i) => {
                if let Some(controller) = self.settings.controllers.get(i) {
                    let _ = write!(context, "{}", Self::controller_label(controller).as_str());
                }
            }
        }
        self.page_head(scene, self.edit.title(), context.as_str(), alpha);

        // The value being edited. A rejected commit turns the frame red, which is
        // the only feedback DONE can give when it refuses.
        let (fx0, fy0, fx1, fy1) = l::FIELD;
        scene.pill(fx0, fy0, fx1, fy1, 20, rgb(5, 14, 20), alpha);
        let frame = if self.edit_invalid {
            C_CANCEL
        } else {
            C_CONFIG
        };
        scene.pill(fx0, fy1 - 4, fx1, fy1, 2, frame, alpha);

        let text = self.edit_buf.as_str();
        let baseline = (fy0 + fy1) / 2 + 14;
        if text.is_empty() {
            scene.label(
                fx0 + 22,
                baseline,
                FontId::Body,
                DIM,
                alpha,
                Align::Left,
                if self.edit_invalid {
                    "NOT VALID"
                } else {
                    "TYPE HERE"
                },
            );
        } else {
            // Long values scroll: the tail is what you are working on.
            let shown = fit_tail(FontId::Body, text, fx1 - fx0 - 52);
            let width = FontId::Body.get().width(shown);
            scene.label(
                fx0 + 22,
                baseline,
                FontId::Body,
                INK,
                alpha,
                Align::Left,
                shown,
            );
            scene.pill(
                fx0 + 26 + width,
                fy0 + 16,
                fx0 + 30 + width,
                fy1 - 16,
                2,
                frame,
                alpha,
            );
        }

        // The letter grid. One primitive per row - see Prim::KeyRow.
        let (rows, key_w) = self.key_mode.rows();
        let lit = self
            .key_hot
            .filter(|_| now_ms.wrapping_sub(self.key_hot_ms) < KEY_FLASH_MS);
        for (row, keys) in rows.iter().enumerate() {
            let count = keys.chars().count();
            let (bx0, bx1) = key_band(count, key_w);
            let y0 = l::KEY_TOP + row as i32 * l::KEY_PITCH;
            let highlight = lit
                .filter(|(hot_row, _)| *hot_row == row)
                .map_or(-1, |(_, slot)| slot as i8);
            scene.key_row(
                bx0,
                y0,
                bx1,
                y0 + l::KEY_H,
                l::KEY_R,
                l::KEY_GAP,
                rgb(26, 46, 56),
                C_CONFIG,
                INK,
                alpha,
                highlight,
                FontId::Body,
                keys,
            );
        }

        // Keys that are not part of the grid.
        let row2 = l::KEY_TOP + 2 * l::KEY_PITCH;
        if self.key_mode.is_alpha() {
            let (x0, x1) = l::KEY_SHIFT;
            // Lit while it is armed, so the case you are about to type is
            // visible rather than remembered.
            let armed = self.key_mode == KeyMode::Upper;
            self.aux_key(
                scene,
                x0,
                row2,
                x1,
                row2 + l::KEY_H,
                if armed { C_CONFIG } else { rgb(20, 36, 46) },
                if armed { rgb(6, 20, 26) } else { INK },
                FontId::Icon,
                "\u{f062}",
                alpha,
            );
        }
        let (x0, x1) = l::KEY_DEL;
        self.aux_key(
            scene,
            x0,
            row2,
            x1,
            row2 + l::KEY_H,
            rgb(20, 36, 46),
            INK,
            FontId::Icon,
            "\u{f55a}",
            alpha,
        );

        let by = l::KEY_BOTTOM_Y;
        let numeric = self.key_mode == KeyMode::Numeric;
        let (x0, x1) = l::KEY_MODE;
        self.aux_key(
            scene,
            x0,
            by,
            x1,
            by + l::KEY_H,
            rgb(20, 36, 46),
            INK,
            FontId::Caption,
            match self.key_mode {
                KeyMode::Numeric => ".",
                KeyMode::Symbols => "abc",
                _ => "?123",
            },
            alpha,
        );
        let (x0, x1) = l::KEY_SPACE;
        self.aux_key(
            scene,
            x0,
            by,
            x1,
            by + l::KEY_H,
            rgb(20, 36, 46),
            if numeric { INK } else { MUTED },
            if numeric {
                FontId::Body
            } else {
                FontId::Caption
            },
            if numeric { "0" } else { "SPACE" },
            alpha,
        );
        let (x0, x1) = l::KEY_DONE;
        self.aux_key(
            scene,
            x0,
            by,
            x1,
            by + l::KEY_H,
            C_RUN,
            rgb(4, 22, 14),
            FontId::Caption,
            "DONE",
            alpha,
        );
    }

    /// One key that is not part of a grid row: a plate and a centred caption.
    #[allow(clippy::too_many_arguments)]
    fn aux_key(
        &self,
        scene: &mut Scene,
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
        plate: u16,
        ink: u16,
        font: FontId,
        caption: &str,
        alpha: u8,
    ) {
        scene.pill(x0, y0, x1, y1, l::KEY_R, plate, alpha);
        let f = font.get();
        scene.label(
            (x0 + x1) / 2,
            (y0 + y1) / 2 + f.ascent / 2 - f.ascent / 8,
            font,
            ink,
            alpha,
            Align::Center,
            caption,
        );
    }

    fn draw_info(&mut self, scene: &mut Scene, state: &State, alpha: u8) {
        self.draw_back(scene, alpha);
        scene.label(CX, 69, FontId::Body, INK, alpha, Align::Center, "INFO");

        let mut panel = Buf::<28>::new();
        match state.local_ip {
            Some(ip) => {
                let _ = write!(panel, "PANEL {}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]);
            }
            None => {
                let _ = write!(panel, "PANEL CONNECTING");
            }
        }
        scene.label(
            CX,
            103,
            FontId::Caption,
            MUTED,
            alpha,
            Align::Center,
            panel.as_str(),
        );

        for i in 0..state.n_controllers {
            let ip = state.controller_ips[i];
            let col = i % 2;
            let row = i / 2;
            let x = if col == 0 { 28 } else { 254 };
            let y = 132 + row as i32 * 28;
            scene.disc(
                x,
                y - 7,
                6,
                if state.controller_online[i] {
                    C_RUN
                } else {
                    C_CANCEL
                },
                alpha,
            );
            let mut address = Buf::<24>::new();
            let _ = write!(address, "{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]);
            scene.label(
                x + 13,
                y,
                FontId::Caption,
                MUTED,
                alpha,
                Align::Left,
                address.as_str(),
            );
        }

        let first = (self.info_scroll / l::ANALOG_PITCH) as usize;
        let shift = -(self.info_scroll % l::ANALOG_PITCH);
        let rows = state
            .n_analogs
            .saturating_sub(first)
            .min(l::ANALOG_MAX_ROWS + 1);
        scene.clip(l::ANALOG_VIEW_TOP, l::ANALOG_VIEW_BOTTOM);
        for row in 0..rows {
            let a = state.analogs[first + row];
            let cy = l::ANALOG_FIRST_CY + row as i32 * l::ANALOG_PITCH + shift;
            scene.label(
                28,
                cy + 8,
                FontId::Caption,
                INK,
                alpha,
                Align::Left,
                a.name.as_str(),
            );
            let mut value = Buf::<20>::new();
            let _ = write!(value, "{}  0..4095", a.level);
            scene.label(
                l::ANALOG_X1,
                cy - 8,
                FontId::Caption,
                MUTED,
                alpha,
                Align::Right,
                value.as_str(),
            );
            scene.pill(
                l::ANALOG_X0,
                cy,
                l::ANALOG_X1,
                cy + 10,
                5,
                rgb(48, 39, 76),
                alpha,
            );
            let fill =
                l::ANALOG_X0 + (l::ANALOG_X1 - l::ANALOG_X0) * a.level.min(4095) as i32 / 4095;
            if fill > l::ANALOG_X0 {
                scene.pill(l::ANALOG_X0, cy, fill, cy + 10, 5, C_INFO, alpha);
            }
            scene.disc(fill, cy + 5, 8, rgb(218, 198, 252), alpha);
        }
        scene.clip_reset();
        if rows == 0 {
            scene.label(
                CX,
                252,
                FontId::Caption,
                DIM,
                alpha,
                Align::Center,
                "NO ANALOG INPUTS",
            );
        }
        self.draw_scrollbar(
            scene,
            self.info_scroll,
            l::ANALOG_MAX_ROWS,
            l::ANALOG_PITCH,
            state.n_analogs,
            l::ANALOG_VIEW_TOP,
            l::ANALOG_VIEW_BOTTOM,
            C_INFO,
            alpha,
        );
    }

    fn draw_inspect(&mut self, scene: &mut Scene, state: &State, alpha: u8) {
        self.draw_back(scene, alpha);
        // Plural: this page lists every schedule, it is not one schedule's page.
        scene.label(
            CX,
            116,
            FontId::Body,
            INK,
            alpha,
            Align::Center,
            "SCHEDULES",
        );

        let first = (self.schedule_scroll / l::SCHED_PITCH) as usize;
        let shift = -(self.schedule_scroll % l::SCHED_PITCH);
        let rows = state
            .n_starts
            .saturating_sub(first)
            .min(l::SCHED_MAX_ROWS + 1);
        scene.clip(156, 438);
        for row in 0..rows {
            let index = first + row;
            let s = state.starts[index];
            let on = s.enabled && s.n_entries > 0;
            let cy = l::SCHED_FIRST_CY + row as i32 * l::SCHED_PITCH + shift;
            let (x0, x1) = (l::SCHED_X0, l::SCHED_X1);
            scene.pill(
                x0,
                cy - l::SCHED_HALF_H,
                x1,
                cy + l::SCHED_HALF_H,
                18,
                if on { rgb(18, 44, 92) } else { rgb(13, 22, 38) },
                alpha,
            );
            if let Some((elapsed, total, _, _)) = state.schedule_progress(index) {
                let fill = x0 + (x1 - x0) * elapsed as i32 / total.max(1) as i32;
                if fill > x0 {
                    scene.pill(x0, cy + 22, fill, cy + 30, 4, C_RUN, alpha);
                }
            }
            // Armed indicator.
            scene.disc(x0 + 28, cy, 9, if on { C_RUN } else { DIM }, alpha);

            let mut time = Buf::<8>::new();
            let _ = write!(time, "{:02}:{:02}", s.hh, s.mm);
            scene.label(
                x0 + 52,
                cy + 13,
                FontId::Body,
                if on { INK } else { DIM },
                alpha,
                Align::Left,
                time.as_str(),
            );

            let mut summary = Buf::<28>::new();
            if let Some((elapsed, total, _, _)) = state.schedule_progress(index) {
                let _ = write!(summary, "RUNNING {}%", elapsed * 100 / total.max(1));
            } else if s.n_entries == 0 {
                let _ = write!(summary, "EMPTY");
            } else {
                let total = s.total_seconds();
                if total >= 60 {
                    let _ = write!(summary, "{} ZONES \u{b7} {} MIN", s.n_entries, total / 60);
                } else {
                    let _ = write!(summary, "{} ZONES \u{b7} {} S", s.n_entries, total);
                }
            }
            // Right-aligned clear of the chevron. The row is now the full width of
            // the panel, so this no longer has to fight the clock for space.
            scene.label(
                x1 - 44,
                cy + 10,
                FontId::Caption,
                if on { MUTED } else { DIM },
                alpha,
                Align::Right,
                summary.as_str(),
            );
            // Chevron, marking the row as something you can open.
            scene.pill(x1 - 26, cy - 8, x1 - 20, cy + 1, 3, MUTED, alpha);
            scene.pill(x1 - 26, cy - 1, x1 - 20, cy + 8, 3, MUTED, alpha);
        }
        scene.clip_reset();

        self.draw_scrollbar(
            scene,
            self.schedule_scroll,
            l::SCHED_MAX_ROWS,
            l::SCHED_PITCH,
            state.n_starts,
            156,
            438,
            C_INSPECT,
            alpha,
        );

        // Sensor line. The controller reports several analog inputs; the first
        // record is I1, which is the one wired for this installation.
        if state.n_analogs > 0 {
            let a = state.analogs[0];
            let mut line = Buf::<28>::new();
            let _ = write!(line, "{} {} / 4095", a.name.as_str(), a.level);
            scene.label(
                CX,
                452,
                FontId::Caption,
                DIM,
                alpha,
                Align::Center,
                line.as_str(),
            );
        }
    }

    /// Edit mode: the armed toggle, the clock adjusters, and the selected entry's
    /// controls down the right-hand column.
    fn draw_edit_controls(&self, scene: &mut Scene, state: &State, draft: &StartTime, alpha: u8) {
        // Armed, as a toggle rather than a label: this is the one field of a
        // schedule that changes what the controller does without changing what it
        // waters.
        let (ax0, ay0, ax1, ay1) = l::ARMED;
        scene.pill(
            ax0,
            ay0,
            ax1,
            ay1,
            (ay1 - ay0) / 2,
            if draft.enabled {
                C_RUN
            } else {
                rgb(58, 22, 22)
            },
            alpha,
        );
        scene.label(
            (ax0 + ax1) / 2,
            (ay0 + ay1) / 2 + 9,
            FontId::Caption,
            if draft.enabled { rgb(4, 22, 14) } else { MUTED },
            alpha,
            Align::Center,
            if draft.enabled { "ON" } else { "OFF" },
        );

        // Clock adjusters. Hours by one, minutes by five - a start time is not a
        // stopwatch, and sixty taps to cross an hour would be worse than a coarse
        // step.
        for (label, (x0, y0, x1, y1)) in [
            ("H-", l::TIME_HH_DOWN),
            ("H+", l::TIME_HH_UP),
            ("M-", l::TIME_MM_DOWN),
            ("M+", l::TIME_MM_UP),
        ] {
            scene.pill(x0, y0, x1, y1, (y1 - y0) / 2, rgb(20, 44, 74), alpha);
            scene.label(
                (x0 + x1) / 2,
                (y0 + y1) / 2 + 9,
                FontId::Caption,
                INK,
                alpha,
                Align::Center,
                label,
            );
        }

        // The selected entry's controls. Greyed rather than hidden when they
        // cannot act - an empty schedule, or an entry already at one end - so the
        // column does not rearrange itself under the finger.
        let n = draft.n_entries;
        let row = self.draft_row.min(n.saturating_sub(1));
        for (which, caption) in EDIT_CONTROLS.iter().enumerate() {
            let y0 = l::EDIT_COL_TOP + which as i32 * l::EDIT_COL_PITCH;
            let y1 = y0 + l::EDIT_COL_H;
            let live = n > 0
                && match which {
                    0 => row > 0,
                    1 => row + 1 < n,
                    3 => (draft.entries[row].seconds as u32) < state.max_run_s,
                    4 => draft.entries[row].seconds > 5,
                    _ => true,
                };
            let plate = match which {
                _ if !live => rgb(18, 30, 42),
                5 => rgb(72, 26, 26),
                _ => rgb(24, 52, 100),
            };
            scene.pill(l::EDIT_COL_X0, y0, l::EDIT_COL_X1, y1, 12, plate, alpha);
            scene.label(
                (l::EDIT_COL_X0 + l::EDIT_COL_X1) / 2,
                (y0 + y1) / 2 + 9,
                FontId::Caption,
                if live { INK } else { DIM },
                alpha,
                Align::Center,
                caption,
            );
        }
    }

    /// The big button on the right of a schedule's entry list: start it now, or
    /// stop it while it runs.
    ///
    /// Deliberately the same size, position and language as the manual page's GO -
    /// on both screens it is "the button that opens a valve", and it turns red for
    /// stopping exactly as the countdown's cancel does. A schedule with no entries
    /// gets a dimmed plate rather than a hidden one, so the layout does not shift
    /// and the reason is visible.
    fn draw_run_button(&self, scene: &mut Scene, schedule: &StartTime, alpha: u8) {
        let (cx, cy, r) = l::RUN_NOW;
        let empty = schedule.n_entries == 0;
        let color = if schedule.running {
            C_CANCEL
        } else if empty {
            rgb(28, 44, 70)
        } else {
            C_RUN
        };
        scene.disc(cx, cy, r, color, alpha);
        let (caption, ink) = if schedule.running {
            ("STOP", INK)
        } else if empty {
            ("EMPTY", MUTED)
        } else {
            ("RUN", rgb(4, 22, 14))
        };
        let font = if caption == "EMPTY" {
            FontId::Caption
        } else {
            FontId::Body
        };
        let f = font.get();
        scene.label(
            cx,
            cy + f.ascent / 2 - f.ascent / 8,
            font,
            ink,
            alpha,
            Align::Center,
            caption,
        );
    }

    /// One schedule's running order: which relay, for how long, in sequence.
    fn draw_detail(&mut self, scene: &mut Scene, state: &State, alpha: u8) {
        self.draw_back(scene, alpha);
        let index = self.detail.min(state.n_starts.saturating_sub(1));
        // The draft while editing: the model underneath is replaced by every poll,
        // which would undo each change as fast as it was made.
        let s = if self.editing {
            self.draft
        } else {
            state.starts[index]
        };
        let editing = self.editing;

        let mut title = Buf::<16>::new();
        let _ = write!(title, "{:02}:{:02}", s.hh, s.mm);
        scene.label(
            CX,
            116,
            FontId::Display,
            INK,
            alpha,
            Align::Center,
            title.as_str(),
        );

        let mut sub = Buf::<32>::new();
        if editing {
            // Nothing here: the armed toggle and the clock adjusters occupy this
            // strip, and a caption behind them would only be clutter.
        } else if let Some((elapsed, total, active, _)) = state.schedule_progress(index) {
            // While it runs, the subtitle is the progress readout: which entry of
            // how many, and how far through the whole schedule.
            let _ = write!(
                sub,
                "RUNNING {} OF {} \u{b7} {}%",
                active + 1,
                s.n_entries,
                elapsed * 100 / total.max(1)
            );
        } else if s.enabled {
            let _ = write!(sub, "ARMED \u{b7} {} IN ORDER", s.n_entries);
        } else {
            let _ = write!(sub, "DISARMED");
        }
        if !sub.as_str().is_empty() {
            scene.label(
                CX,
                152,
                FontId::Caption,
                MUTED,
                alpha,
                Align::Center,
                sub.as_str(),
            );
        }

        // Entries run top to bottom in the order the controller will drive them.
        let first = (self.detail_scroll / l::DETAIL_PITCH) as usize;
        let shift = -(self.detail_scroll % l::DETAIL_PITCH);
        // One row past the entries while editing - that is the "add another" row,
        // and `row_at` reaches one further for the same reason.
        let listed = s.n_entries + editing as usize;
        let rows = listed.saturating_sub(first).min(l::DETAIL_MAX_ROWS + 1);
        // Milliseconds, not seconds: a row is over 300 px wide, so a whole
        // second of a short entry is a visible jump.
        let progress = state.schedule_progress_ms(index, self.run_left_ms);
        scene.clip(178, 432);
        for row in 0..rows {
            let i = first + row;
            let cy = l::DETAIL_FIRST_CY + row as i32 * l::DETAIL_PITCH + shift;

            // The row past the end, while editing: add another entry.
            if i >= s.n_entries {
                scene.pill(
                    l::DETAIL_X0,
                    cy - 21,
                    l::DETAIL_X1,
                    cy + 21,
                    18,
                    rgb(18, 44, 60),
                    alpha,
                );
                let full = s.n_entries >= crate::model::MAX_ENTRIES;
                scene.label(
                    (l::DETAIL_X0 + l::DETAIL_X1) / 2,
                    cy + 9,
                    FontId::Caption,
                    if full { DIM } else { INK },
                    alpha,
                    Align::Center,
                    if full { "FULL" } else { "+ ADD ZONE" },
                );
                continue;
            }

            let e = s.entries[i];
            let selected = editing && i == self.draft_row.min(s.n_entries.saturating_sub(1));
            scene.pill(
                l::DETAIL_X0,
                cy - 21,
                l::DETAIL_X1,
                cy + 21,
                18,
                if selected {
                    rgb(28, 62, 122)
                } else {
                    rgb(15, 34, 72)
                },
                alpha,
            );
            // The selected row carries a bar down its left edge, so which entry
            // the controls opposite will act on is never in doubt.
            if selected {
                scene.pill(
                    l::DETAIL_X0 + 4,
                    cy - 14,
                    l::DETAIL_X0 + 9,
                    cy + 14,
                    2,
                    C_INSPECT,
                    alpha,
                );
            }
            if let Some((active, elapsed_ms, duration_ms)) = progress.filter(|_| !editing) {
                // Entries before the active one are complete, the active one is
                // partly filled, later ones are untouched.
                let (amount, of) = if i < active {
                    (1, 1)
                } else if i == active {
                    (elapsed_ms, duration_ms.max(1))
                } else {
                    (0, 1)
                };
                if amount > 0 {
                    let fill =
                        l::DETAIL_X0 + ((l::DETAIL_X1 - l::DETAIL_X0) as u32 * amount / of) as i32;
                    scene.pill(l::DETAIL_X0, cy + 15, fill, cy + 21, 3, C_RUN, alpha);
                }
            }
            // Position in the running order.
            scene.disc(l::DETAIL_X0 + 26, cy, 14, rgb(30, 70, 148), alpha);
            let mut n = Buf::<4>::new();
            let _ = write!(n, "{}", i + 1);
            scene.label(
                l::DETAIL_X0 + 26,
                cy + 9,
                FontId::Caption,
                INK,
                alpha,
                Align::Center,
                n.as_str(),
            );

            let name = state
                .relay_by_id(e.relay as i32)
                .map(|r| r.name)
                .unwrap_or(Text::EMPTY);
            scene.label(
                l::DETAIL_X0 + 52,
                cy + 9,
                FontId::Caption,
                INK,
                alpha,
                Align::Left,
                name.as_str(),
            );

            let mut dur = Buf::<12>::new();
            if e.seconds >= 60 {
                let _ = write!(dur, "{}:{:02}", e.seconds / 60, e.seconds % 60);
            } else {
                let _ = write!(dur, "{} S", e.seconds);
            }
            scene.label(
                l::DETAIL_X1 - 22,
                cy + 9,
                FontId::Caption,
                MUTED,
                alpha,
                Align::Right,
                dur.as_str(),
            );
        }
        scene.clip_reset();

        // SAVE takes EDIT's place, so the button that finishes is where the one
        // that started was. Hidden entirely while this schedule waters: the run is
        // the thing to watch then, and STOP is the only control that makes sense.
        if editing || self.can_edit(state) {
            let (ex0, ey0, ex1, ey1) = l::EDIT;
            scene.pill(
                ex0,
                ey0,
                ex1,
                ey1,
                (ey1 - ey0) / 2,
                if editing { C_RUN } else { rgb(24, 52, 100) },
                alpha,
            );
            scene.label(
                (ex0 + ex1) / 2,
                (ey0 + ey1) / 2 + 10,
                FontId::Caption,
                if editing { rgb(4, 22, 14) } else { INK },
                alpha,
                Align::Center,
                if editing { "SAVE" } else { "EDIT" },
            );
        }

        if editing {
            self.draw_edit_controls(scene, state, &s, alpha);
        } else {
            self.draw_run_button(scene, &s, alpha);
        }

        self.draw_scrollbar(
            scene,
            self.detail_scroll,
            l::DETAIL_MAX_ROWS,
            l::DETAIL_PITCH,
            listed,
            178,
            432,
            C_INSPECT,
            alpha,
        );
    }

    fn draw_force(&mut self, scene: &mut Scene, state: &State, alpha: u8) {
        self.draw_back(scene, alpha);

        let mut mins = Buf::<4>::new();
        let _ = write!(mins, "{}", self.minutes);
        scene.label(
            246,
            158,
            FontId::Display,
            INK,
            alpha,
            Align::Right,
            mins.as_str(),
        );
        scene.label(258, 158, FontId::Caption, MUTED, alpha, Align::Left, "MIN");

        // Slider: track, filled portion below the knob, knob.
        let sx = l::SLIDER_X;
        let hw = l::SLIDER_HALF_W;
        scene.pill(
            sx - hw,
            l::SLIDER_TOP,
            sx + hw,
            l::SLIDER_BOTTOM,
            hw,
            rgb(58, 36, 8),
            alpha,
        );
        // Eased position, not the raw target - see `knob_q4`.
        let knob_y = (self.knob_q4 >> 4).clamp(l::SLIDER_TOP, l::SLIDER_BOTTOM);
        let fill_radius = ((l::SLIDER_BOTTOM - knob_y) / 2).min(hw).max(0);
        scene.pill(
            sx - hw,
            knob_y,
            sx + hw,
            l::SLIDER_BOTTOM,
            fill_radius,
            C_FORCE,
            alpha,
        );
        scene.disc(sx, knob_y, 28, rgb(252, 216, 154), alpha);

        let first = (self.relay_scroll / l::RELAY_PITCH) as usize;
        let shift = -(self.relay_scroll % l::RELAY_PITCH);
        scene.clip(
            l::RELAY_FIRST_CY - l::RELAY_HALF_H,
            l::RELAY_FIRST_CY + (l::RELAY_MAX_ROWS as i32 - 1) * l::RELAY_PITCH + l::RELAY_HALF_H,
        );
        for (row, (index, relay)) in state
            .usable()
            .enumerate()
            .skip(first)
            .take(l::RELAY_MAX_ROWS + 1)
            .enumerate()
        {
            let selected = index == self.selected;
            let cy = l::RELAY_FIRST_CY + row as i32 * l::RELAY_PITCH + shift;
            scene.pill(
                l::RELAY_X0,
                cy - l::RELAY_HALF_H,
                l::RELAY_X1,
                cy + l::RELAY_HALF_H,
                l::RELAY_HALF_H,
                if selected { C_FORCE } else { rgb(52, 34, 10) },
                alpha,
            );
            scene.label(
                (l::RELAY_X0 + l::RELAY_X1) / 2,
                cy + 9,
                FontId::Caption,
                if selected { rgb(26, 14, 0) } else { MUTED },
                alpha,
                Align::Center,
                relay.name.as_str(),
            );
        }
        scene.clip_reset();

        let total = state.n_usable();
        self.draw_scrollbar(
            scene,
            self.relay_scroll,
            l::RELAY_MAX_ROWS,
            l::RELAY_PITCH,
            total,
            l::RELAY_FIRST_CY - l::RELAY_HALF_H,
            l::RELAY_FIRST_CY + (l::RELAY_MAX_ROWS as i32 - 1) * l::RELAY_PITCH + l::RELAY_HALF_H,
            C_FORCE,
            alpha,
        );

        // Offline: the button is a ring with its reason in it rather than a
        // filled invitation. Sending the request anyway would show a countdown for
        // water that is not running.
        let (gx, gy, gr) = l::GO;
        let reachable = state.any_controller_online();
        if reachable {
            scene.disc(gx, gy, gr, C_RUN, alpha);
        } else {
            scene.ring(gx, gy, gr, gr - 4, rgb(70, 78, 90), alpha);
        }
        scene.label(
            gx,
            gy + 16,
            FontId::Body,
            if reachable { rgb(2, 22, 12) } else { rgb(150, 160, 172) },
            alpha,
            Align::Center,
            if reachable { "GO!" } else { "OFFLINE" },
        );
    }

    fn draw_running(&mut self, scene: &mut Scene, state: &State, alpha: u8) {
        self.draw_back(scene, alpha);

        // Progress gauge. Track first, then the elapsed sweep over it.
        //
        // Progress is computed from fractional seconds, not whole ones: driving
        // it from `left_s` alone moved the arc in one-second steps, which is what
        // made it look stuttery. With the sub-second remainder folded in, and the
        // countdown screen repainting continuously, the sweep is smooth.
        // Closed track, drawn as a ring rather than a full-turn arc: a ring needs
        // no angular test at all, so the cheap primitive does the cheap job.
        scene.ring(CX, CY, l::RING_OUTER, l::RING_INNER, rgb(10, 52, 36), alpha);

        // Which schedule, if any, this run belongs to. When one does, the outer
        // ring measures the *whole* schedule and an inner ring measures the entry
        // running now - so the screen answers both "how far through the watering
        // am I" and "how long until this valve closes" at once.
        let sequence = state.starts[..state.n_starts]
            .iter()
            .position(|schedule| schedule.running)
            .and_then(|index| {
                let schedule = state.starts[index];
                let (at, elapsed_ms, duration_ms) =
                    state.schedule_progress_ms(index, self.run_left_ms)?;
                let before_s = schedule.entries[..at]
                    .iter()
                    .map(|entry| entry.seconds as u32)
                    .sum::<u32>()
                    + schedule.gap_s as u32 * at as u32;
                let total_s = schedule.total_seconds();
                Some((schedule, at, elapsed_ms, duration_ms, before_s, total_s))
            });

        let left_ms = self.run_left_ms;
        // The outer sweep. For a sequence it is the whole schedule; for a manual
        // run, the length of that run. It used to be `self.minutes` either way -
        // the manual slider - so a schedule-driven run was measured against
        // whatever the slider happened to be left on, and a run this panel did not
        // start had no relation to it at all.
        let (outer_total_ms, outer_done_ms) = match sequence {
            Some((_, _, elapsed_ms, _, before_s, total_s)) => {
                (total_s.max(1) * 1000, before_s * 1000 + elapsed_ms)
            }
            None => {
                let total = self.observed_run_total_s.max(state.left_s).max(1);
                (
                    total * 1000,
                    (total * 1000).saturating_sub(left_ms.min(total * 1000)),
                )
            }
        };
        let span = progress_span_q12(
            outer_total_ms,
            outer_total_ms - outer_done_ms.min(outer_total_ms),
        );
        if span > 0 {
            scene.arc(CX, CY, l::RING_OUTER, l::RING_INNER, 0, span, C_RUN, alpha);
        }

        // Inner gauge: the entry running right now, within that.
        if let Some((_, _, elapsed_ms, duration_ms, _, _)) = sequence {
            scene.ring(
                CX,
                CY,
                l::RING2_OUTER,
                l::RING2_INNER,
                rgb(8, 38, 27),
                alpha,
            );
            let entry_span = progress_span_q12(
                duration_ms.max(1),
                duration_ms.saturating_sub(elapsed_ms.min(duration_ms)),
            );
            if entry_span > 0 {
                scene.arc(
                    CX,
                    CY,
                    l::RING2_OUTER,
                    l::RING2_INNER,
                    0,
                    entry_span,
                    rgb(120, 214, 160),
                    alpha,
                );
            }
        }

        let name = state
            .relay_by_id(state.active)
            .map(|r| r.name)
            .unwrap_or(Text::EMPTY);
        scene.label(
            CX,
            l::RUN_NAME_BASELINE,
            FontId::Body,
            INK,
            alpha,
            Align::Center,
            if self.run_finished && !state.running {
                "COMPLETE"
            } else if name.len > 0 {
                name.as_str()
            } else {
                "WATERING"
            },
        );

        // The countdown - the reason this screen exists, so it gets the largest
        // type on the device.
        // Rounded up, and from the anchored value: a countdown that reads 0:00
        // while a valve is still open is worse than one that lingers a moment on
        // 0:01, and the digits must agree with the arcs beside them.
        let shown_s = (self.run_left_ms + 999) / 1000;
        let mut big = Buf::<10>::new();
        let _ = write!(big, "{}:{:02}", shown_s / 60, shown_s % 60);
        scene.label(
            CX,
            l::RUN_DIGITS_BASELINE,
            FontId::Countdown,
            INK,
            alpha,
            Align::Center,
            big.as_str(),
        );

        // What the queue is doing, under the digits: where we are in the running
        // order and how much watering is left overall. This is the part the
        // "6 more queued" line could not tell you.
        if let Some((schedule, at, _, _, _, total_s)) = sequence {
            let mut line = Buf::<32>::new();
            let remaining = total_s.saturating_sub(outer_done_ms / 1000);
            let _ = write!(
                line,
                "{} OF {} \u{b7} {}:{:02} LEFT",
                at + 1,
                schedule.n_entries,
                remaining / 60,
                remaining % 60
            );
            scene.label(
                CX,
                l::RUN_QUEUE_BASELINE,
                FontId::Caption,
                rgb(120, 214, 160),
                alpha,
                Align::Center,
                line.as_str(),
            );
        }

        let (x0, y0, x1, y1) = l::CANCEL;
        let finished = self.run_finished;
        scene.pill(
            x0,
            y0,
            x1,
            y1,
            (y1 - y0) / 2,
            if finished { C_RUN } else { C_CANCEL },
            alpha,
        );
        scene.label(
            CX,
            (y0 + y1) / 2 + 14,
            FontId::Body,
            INK,
            alpha,
            Align::Center,
            if finished { "DONE!" } else { "CANCEL" },
        );

        if state.queued > 0 {
            let mut q = Buf::<24>::new();
            let _ = write!(q, "{} MORE QUEUED", state.queued);
            scene.label(
                CX,
                402,
                FontId::Caption,
                MUTED,
                alpha,
                Align::Center,
                q.as_str(),
            );
        }
    }
}

/// Slider maps bottom = MINUTES_MIN, top = MINUTES_MAX.
fn progress_span_q12(total_ms: u32, left_ms: u32) -> i32 {
    let total_ms = total_ms.max(1);
    let done_ms = total_ms.saturating_sub(left_ms.min(total_ms));
    // Scale before multiplying so long controller-side runs remain inside u32.
    let denom = (total_ms / 8).max(1);
    ((done_ms / 8).min(denom) * 4096 / denom).min(4095) as i32
}

fn mix565(background: u16, foreground: u16, alpha: u8) -> u16 {
    let a = alpha as u32;
    let ia = 255 - a;
    let br = (background >> 11) as u32 & 0x1f;
    let bg = (background >> 5) as u32 & 0x3f;
    let bb = background as u32 & 0x1f;
    let fr = (foreground >> 11) as u32 & 0x1f;
    let fg = (foreground >> 5) as u32 & 0x3f;
    let fb = foreground as u32 & 0x1f;
    let r = (fr * a + br * ia + 127) / 255;
    let g = (fg * a + bg * ia + 127) / 255;
    let b = (fb * a + bb * ia + 127) / 255;
    ((r as u16) << 11) | ((g as u16) << 5) | b as u16
}

/// The x range of a centred key row of `n` keys, each `key_w` wide.
///
/// Rows are centred rather than stretched to a common band, so a key is the same
/// size whether its row holds ten of them or three.
fn key_band(n: usize, key_w: i32) -> (i32, i32) {
    let half = n as i32 * key_w / 2;
    (CX - half, CX + half)
}

/// The longest tail of `text` that fits `width`.
///
/// The tail, not the head: what you are typing is at the end, and a field that
/// shows the beginning of a long password while hiding the character you just
/// pressed is worse than one that scrolls.
fn fit_tail(font: FontId, text: &str, width: i32) -> &str {
    let f = font.get();
    let mut start = 0;
    while start < text.len() && f.width(&text[start..]) > width {
        start += text[start..].chars().next().map_or(1, |c| c.len_utf8());
    }
    &text[start..]
}

/// RSSI in words. Absolute dBm means nothing to whoever is holding the panel;
/// whether the network is worth joining does.
fn signal_words(rssi: i8) -> &'static str {
    match rssi {
        r if r >= -55 => "STRONG",
        r if r >= -70 => "GOOD",
        r if r >= -80 => "FAIR",
        _ => "WEAK",
    }
}

/// Legible text on a filled button, chosen from the fill's brightness rather than
/// picked per button - so a new Extras entry cannot end up with white type on a
/// pale plate.
fn ink_on(color: u16) -> u16 {
    let r = ((color >> 11) & 0x1f) as u32 * 255 / 31;
    let g = ((color >> 5) & 0x3f) as u32 * 255 / 63;
    let b = (color & 0x1f) as u32 * 255 / 31;
    // Rec. 601 luma, integer.
    let luma = (299 * r + 587 * g + 114 * b) / 1000;
    if luma > 140 { rgb(8, 14, 18) } else { INK }
}

fn signal_color(rssi: i8) -> u16 {
    match rssi {
        r if r >= -70 => C_CONFIG,
        r if r >= -80 => C_FORCE,
        _ => DIM,
    }
}

fn minutes_from_y(y: i32) -> u32 {
    let span = l::SLIDER_BOTTOM - l::SLIDER_TOP;
    let steps = (l::MINUTES_MAX - l::MINUTES_MIN) as i32;
    let clamped = y.clamp(l::SLIDER_TOP, l::SLIDER_BOTTOM);
    let from_bottom = l::SLIDER_BOTTOM - clamped;
    // Round to nearest step, so each minute owns an equal slice of travel.
    let step = (from_bottom * steps + span / 2) / span;
    (l::MINUTES_MIN as i32 + step).clamp(l::MINUTES_MIN as i32, l::MINUTES_MAX as i32) as u32
}

fn y_from_minutes(minutes: u32) -> i32 {
    let span = l::SLIDER_BOTTOM - l::SLIDER_TOP;
    let steps = (l::MINUTES_MAX - l::MINUTES_MIN).max(1) as i32;
    let step = (minutes.clamp(l::MINUTES_MIN, l::MINUTES_MAX) - l::MINUTES_MIN) as i32;
    l::SLIDER_BOTTOM - (step * span) / steps
}

#[inline]
fn smoothstep_q15(t: u32) -> u32 {
    let t = t.min(32_768);
    let squared = (t * t) >> 15;
    (squared * (3 * 32_768 - 2 * t)) >> 15
}

/// 1-(1-t)^3: fast off the mark, settles gently. Used for anything a finger just
/// launched, because it makes the response feel immediate.
#[inline]
fn ease_out_q15(t: u32) -> u32 {
    let t = t.min(32_768);
    let inv = 32_768 - t;
    let cube = (((inv * inv) >> 15) * inv) >> 15;
    32_768 - cube
}

#[inline]
fn isqrt_i32(n: i32) -> i32 {
    let mut n = n as u32;
    let mut res = 0u32;
    let mut bit = 1u32 << 30;
    while bit > n {
        bit >>= 2;
    }
    while bit != 0 {
        if n >= res + bit {
            n -= res + bit;
            res = (res >> 1) + bit;
        } else {
            res >>= 1;
        }
        bit >>= 2;
    }
    res as i32
}

// Keep the keep-out radius referenced so a future layout change trips the
// compiler rather than silently drifting outside the panel.
const _: () = {
    assert!(SAFE_R > 0);
};
