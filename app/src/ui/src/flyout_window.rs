//! The tray flyout's window: 360 wide, as tall as its page, without a
//! frame, near the notification area. It closes when it loses the focus, as
//! the system flyouts do. With its setting on, it also opens and closes
//! with Windows 11's Quick Settings, above them, without taking the focus
//! (that would close them).
//!
//! The page in it: screens/flyout.rs.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::time::{Duration, Instant};

use gpui_kit::{
    AnyView, AnyWindowHandle, App, AppContext as _, AsyncApp, Bounds, Context, FocusHandle, Focusable, Global,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Pixels, Render, Styled as _, Window,
    WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions, canvas, div, point, px, size,
};
use tokio::sync::mpsc;
use win_sys::quick_settings::Panels;
use win_sys::window::{self as native, Area};

use crate::screens;
use crate::theme::{self, ActivePalette as _};
use crate::windows::hwnd;

/// Gap between the flyout and the screen edge / taskbar (logical pixels)
const FLYOUT_MARGIN: f32 = 12.;
/// The flyout's width, and its height until its page says how tall it is
/// (issue #86: no empty space under the controls)
const FLYOUT_WIDTH: f32 = 360.;
const FLYOUT_HEIGHT: f32 = 340.;
/// The heights the flyout takes (logical pixels). The shortest page with a
/// device (idle: ring, chooser, Connect) is 154 px; the minimum stays under
/// it so that page gets no empty space
const FLYOUT_MIN_HEIGHT: f32 = 150.;
const FLYOUT_MAX_HEIGHT: f32 = 560.;
/// How long the flyout counts as opening after it was shown (#140).
/// Meanwhile another click on the tray icon (a double click) does not close
/// it, and losing the focus does not hide it: the taskbar can take the
/// focus back just after a click on the icon.
const FLYOUT_OPENING: Duration = Duration::from_millis(800);
/// How often the flyout checks on Quick Settings while it goes with them
const FOLLOW_EVERY: Duration = Duration::from_millis(150);
/// How often it looks for the pointer meanwhile
const POINTER_EVERY: Duration = Duration::from_millis(30);

#[derive(Default)]
struct Flyout {
    window: Option<AnyWindowHandle>,
    hwnd: isize,
    /// When the flyout was last shown
    opened: Option<Instant>,
    /// The flyout goes with Quick Settings, which are here (the cards other
    /// apps keep on them included)
    quick: Option<Panels>,
    /// Shared with the thread that follows Quick Settings: the window to
    /// watch the pointer over
    watched: Option<Arc<AtomicIsize>>,
    /// The page's height (logical pixels)
    height: Option<f32>,
}

impl Global for Flyout {}

fn flyout(cx: &mut App) -> &mut Flyout {
    cx.default_global::<Flyout>()
}

fn opening(cx: &mut App) -> bool {
    flyout(cx).opened.is_some_and(|t| t.elapsed() < FLYOUT_OPENING)
}

/// Whether the flyout is on screen
fn open(cx: &mut App) -> bool {
    let Some(window) = flyout(cx).window else { return false };
    if window.update(cx, |_, _, _| ()).is_ok() {
        return true;
    }
    flyout(cx).window = None;
    false
}

/// A click on the tray icon: opens the flyout, or closes it when it is open
pub fn toggle(cx: &mut App) {
    if !open(cx) {
        show(cx);
    } else if opening(cx) {
        tracing::info!("[flyout] still opening; stays");
        native::show_active(flyout(cx).hwnd);
    } else {
        hide(cx);
    }
}

/// Shows the flyout above the notification area (bottom right of the work
/// area, where the taskbar's tray is in the default layout)
pub fn show(cx: &mut App) {
    if !open(cx) && !create(cx) {
        return;
    }
    let hwnd = flyout(cx).hwnd;
    native::set_no_activate(hwnd, false);
    place(cx);
    flyout(cx).opened = Some(Instant::now());
    native::show_active(hwnd);
    tracing::info!("[flyout] shown");
}

/// Closes the flyout (Escape, a lost focus, the main window opening)
pub fn hide(cx: &mut App) {
    let f = flyout(cx);
    let hwnd = std::mem::take(&mut f.hwnd);
    if let Some(watched) = &f.watched {
        watched.store(0, Ordering::Relaxed);
    }
    let Some(window) = f.window.take() else { return };
    // Off the screen at once; the window itself goes after this update:
    // Escape and the lost focus call this from inside the window's own
    // update, where the window cannot be reached to remove it
    native::hide(hwnd);
    cx.defer(move |cx| {
        let _ = window.update(cx, |_, window, _| window.remove_window());
    });
}

/// Makes the window, not shown yet -> false when it could not be made
fn create(cx: &mut App) -> bool {
    let height = flyout(cx).height.unwrap_or(FLYOUT_HEIGHT);
    let options = WindowOptions {
        // Placed before it shows (place)
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            point(px(0.), px(0.)),
            size(px(FLYOUT_WIDTH), px(height)),
        ))),
        titlebar: None,
        kind: WindowKind::PopUp,
        focus: false,
        show: false,
        is_resizable: false,
        is_minimizable: false,
        is_movable: false,
        window_background: WindowBackgroundAppearance::Opaque,
        ..Default::default()
    };
    let mut native_handle = 0;
    let opened = gpui_kit::open_window(options, cx, |window, cx| {
        native_handle = hwnd(window);
        native::round_corners(native_handle);
        cx.new(|cx| FlyoutRoot::new(window, cx))
    });
    match opened {
        Ok((handle, _)) => {
            let f = flyout(cx);
            f.window = Some(handle);
            f.hwnd = native_handle;
            true
        }
        Err(e) => {
            tracing::error!("[flyout] cannot open the window: {e}");
            false
        }
    }
}

/// Bottom right of the work area, above the notification area; by Quick
/// Settings while the flyout goes with them
fn place(cx: &mut App) {
    let (hwnd, quick, height) = {
        let f = flyout(cx);
        (f.hwnd, f.quick, f.height.unwrap_or(FLYOUT_HEIGHT))
    };
    let Some(dpi) = native::dpi(hwnd) else { return };
    let scale = dpi as f32 / 96.;
    let physical = |logical: f32| (logical * scale).round() as i32;
    let (width, height, margin) = (physical(FLYOUT_WIDTH), physical(height), physical(FLYOUT_MARGIN));
    let (x, y) = match quick {
        Some(panels) => {
            let top = native::work_area_at(panels.left, panels.bottom).map_or(0, |a| a.top);
            by_panels(panels, width, height, margin, top)
        }
        None => {
            let Some(area) = native::primary_work_area() else { return };
            by_tray(area, width, height, margin)
        }
    };
    // The window keeps invisible resize borders (left, right, bottom)
    // around the page: the page gets the width and height above, the
    // borders come on top
    let outer = native::outer_rect(hwnd);
    let viewport = flyout(cx).window.and_then(|w| w.update(cx, |_, window, _| window.viewport_size()).ok());
    let (frame_w, frame_h) = match (outer, viewport) {
        (Some(o), Some(v)) => (
            (o.right - o.left - physical(f32::from(v.width))).max(0),
            (o.bottom - o.top - physical(f32::from(v.height))).max(0),
        ),
        _ => (0, 0),
    };
    let (x, width, height) = (x - frame_w / 2, width + frame_w, height + frame_h);
    if outer != Some(Area { left: x, top: y, right: x + width, bottom: y + height }) {
        native::set_bounds(hwnd, x, y, width, height);
        // The window's resize event came while the app was busy (this
        // runs in an update) and was dropped: without this the page keeps
        // its old size and a taller window shows nothing under it
        if let Some(window) = flyout(cx).window {
            let _ = window.update(cx, |_, window, cx| window.bounds_changed(cx));
        }
    }
}

fn by_tray(area: Area, width: i32, height: i32, margin: i32) -> (i32, i32) {
    ((area.right - width - margin).max(area.left), (area.bottom - height - margin).max(area.top))
}

/// Above the panels, as far from them as they are from each other; to their
/// left when the screen is too short for that
fn by_panels(panels: Panels, width: i32, height: i32, margin: i32, screen_top: i32) -> (i32, i32) {
    let above = panels.top - margin - height;
    if above >= screen_top + margin {
        (panels.right - width, above)
    } else {
        (panels.left - margin - width, panels.bottom - height)
    }
}

/// The flyout's page is `height` tall: the window takes that height, its
/// bottom staying above the notification area
fn fit(height: Pixels, cx: &mut App) {
    let height = f32::from(height);
    if !height.is_finite() {
        return;
    }
    let height = height.clamp(FLYOUT_MIN_HEIGHT, FLYOUT_MAX_HEIGHT).ceil();
    if flyout(cx).height.is_some_and(|now| (now - height).abs() < 1.) {
        return;
    }
    flyout(cx).height = Some(height);
    place(cx);
}

// ---- Quick Settings ---------------------------------------------------------

/// What the thread that follows Quick Settings saw
enum Follow {
    /// They are open here: the flyout shows above them
    Here(Panels),
    /// The pointer came over the flyout: before a click can come, the flyout
    /// becomes the active window of its thread while Quick Settings stay
    /// the foreground. Only now, as it costs other apps' cards.
    InUse,
}

static FOLLOWING: AtomicBool = AtomicBool::new(false);

/// Shows the flyout above Quick Settings, which just opened, until they
/// close. It takes no focus: that would close them.
pub fn show_with_quick_settings(cx: &mut App) {
    if FOLLOWING.swap(true, Ordering::Relaxed) {
        return;
    }
    let (tx, mut rx) = mpsc::unbounded_channel::<Follow>();
    let watched = Arc::new(AtomicIsize::new(0));
    flyout(cx).watched = Some(watched.clone());
    let margin = FLYOUT_MARGIN;
    // Its own thread: COM, and reading the panels takes a while
    let spawned = std::thread::Builder::new().name("quick-settings".into()).spawn(move || {
        if let Err(e) = follow_quick_settings(&tx, &watched, margin) {
            tracing::warn!("[flyout] above Quick Settings: {e}");
        }
    });
    if spawned.is_err() {
        FOLLOWING.store(false, Ordering::Relaxed);
        return;
    }
    cx.spawn(async move |cx: &mut AsyncApp| {
        while let Some(seen) = rx.recv().await {
            cx.update(|cx| match seen {
                Follow::Here(panels) => follow_here(panels, cx),
                Follow::InUse => native_set_active(cx),
            });
        }
        // They closed (the thread ended)
        cx.update(|cx| {
            let was_following = flyout(cx).quick.take().is_some();
            flyout(cx).watched = None;
            if was_following {
                hide(cx);
            }
        });
        FOLLOWING.store(false, Ordering::Relaxed);
    })
    .detach();
}

fn native_set_active(cx: &mut App) {
    let hwnd = flyout(cx).hwnd;
    if hwnd != 0 {
        win_sys::quick_settings::set_active(hwnd);
    }
}

fn follow_here(panels: Panels, cx: &mut App) {
    let first = flyout(cx).quick.is_none();
    flyout(cx).quick = Some(panels);
    if !first {
        // The panels change height as a list unfolds in them
        place(cx);
        return;
    }
    if !open(cx) && !create(cx) {
        return;
    }
    let hwnd = flyout(cx).hwnd;
    native::set_no_activate(hwnd, true);
    place(cx);
    // Not a plain show, which activates it
    win_sys::quick_settings::show_inactive(hwnd);
    if let Some(watched) = &flyout(cx).watched {
        watched.store(hwnd, Ordering::Relaxed);
    }
    tracing::info!("[flyout] shown above Quick Settings");
}

fn follow_quick_settings(
    tx: &mpsc::UnboundedSender<Follow>,
    watched: &AtomicIsize,
    margin: f32,
) -> Result<(), Box<dyn std::error::Error>> {
    let _com = win_sys::audio_policy::ComApartment::init_mta()?;
    let reader = win_sys::quick_settings::Reader::new()?;
    // The top of the panels and the cards on them, kept while the flyout is
    // in use: a card leaving then must not move the flyout under the pointer
    let mut cards_top: Option<i32> = None;
    let mut in_use = false;
    let mut with_cards = |panels: Panels, in_use: bool, hwnd: isize| {
        let gap = native::dpi(hwnd).map_or(margin, |dpi| margin * dpi as f32 / 96.).round() as i32;
        let held = cards_top.filter(|_| in_use).map(|t| t.min(panels.top));
        let top = held.unwrap_or_else(|| win_sys::quick_settings::with_other_cards(panels, 2 * gap).top);
        cards_top = Some(top);
        Panels { top, ..panels }
    };
    // Closed again: a first start can take longer than they stay open
    let Some(panels) = reader.read() else { return Ok(()) };
    if tx.send(Follow::Here(with_cards(panels, false, 0))).is_err() {
        return Ok(());
    }
    let mut read = Instant::now();
    loop {
        std::thread::sleep(POINTER_EVERY);
        let hwnd = watched.load(Ordering::Relaxed);
        // Before a click can come
        if hwnd != 0 && !in_use && win_sys::quick_settings::pointer_over(hwnd) {
            in_use = true;
            if tx.send(Follow::InUse).is_err() {
                break;
            }
        }
        if read.elapsed() < FOLLOW_EVERY {
            continue;
        }
        read = Instant::now();
        let Some(panels) = reader.read() else { break };
        if tx.send(Follow::Here(with_cards(panels, in_use, hwnd))).is_err() {
            break;
        }
    }
    Ok(())
}

// ---- The window's root --------------------------------------------------------

/// What the flyout's window holds: its page, measured so that the window
/// fits it. Escape closes it, and so does losing the focus.
struct FlyoutRoot {
    page: AnyView,
    focus: FocusHandle,
}

impl FlyoutRoot {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> FlyoutRoot {
        cx.observe_window_activation(window, |_, window, cx| {
            // While it goes with Quick Settings it never has the focus
            // The demo's screenshots: it stays while the focus is elsewhere
            if window.is_window_active() || flyout(cx).quick.is_some() || crate::demo::route().is_some() {
                return;
            }
            if opening(cx) {
                tracing::info!("[flyout] lost the focus while opening; taking it back");
                native::show_active(flyout(cx).hwnd);
            } else {
                hide(cx);
            }
        })
        .detach();
        cx.observe_window_appearance(window, |_, _, cx| theme::refresh(cx)).detach();
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        FlyoutRoot { page: screens::flyout::open(window, cx), focus }
    }
}

impl Focusable for FlyoutRoot {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for FlyoutRoot {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = cx.palette();
        div()
            .size_full()
            .overflow_hidden()
            .bg(p.bg)
            .text_color(p.text)
            .text_size(theme::FS_BODY)
            .font_family(theme::FONT)
            .track_focus(&self.focus)
            .on_key_down(|event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    hide(cx);
                }
            })
            .child(
                // As tall as the page: the window then takes this height
                div().relative().w_full().child(self.page.clone()).child(
                    canvas(
                        |bounds, _, cx| {
                            let height = bounds.size.height;
                            cx.defer(move |cx| fit(height, cx));
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .size_full(),
                ),
            )
            .child(crate::ui::frame_drawn())
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::TestAppContext;
    use gpui_kit::test::TestWindowExt as _;

    use super::*;
    use crate::testing::{Harness, snapshot};

    /// The flyout's page in a test window, known as the flyout
    fn open_flyout(cx: &mut TestAppContext) -> (Harness, AnyWindowHandle) {
        let mut h = Harness::start(cx, snapshot());
        let (w, _) =
            h.open_sized(cx, (FLYOUT_WIDTH, FLYOUT_HEIGHT), |window, cx| cx.new(|cx| FlyoutRoot::new(window, cx)));
        cx.update(|cx| flyout(cx).window = Some(w));
        h.settle(cx);
        (h, w)
    }

    fn is_open(cx: &mut TestAppContext) -> bool {
        cx.update(open)
    }

    #[gpui_kit::test]
    fn escape_closes_the_flyout(cx: &mut TestAppContext) {
        let (mut h, w) = open_flyout(cx);
        // Another key leaves it
        h.on(cx, w, |window, cx| window.press("a", cx));
        assert!(is_open(cx));
        h.on(cx, w, |window, cx| window.press("escape", cx));
        assert!(!is_open(cx), "Escape closed it");
        assert!(cx.update(|cx| cx.windows().iter().all(|x| x.window_id() != w.window_id())), "its window went");
    }

    #[test]
    fn flyout_stands_above_the_panels_or_beside_them() {
        let panels = Panels { left: 3282, top: 1281, right: 3822, bottom: 2070 };
        // 150%: as wide as the panels, their gap apart, their right edges in line
        assert_eq!(by_panels(panels, 540, 510, 18, 0), (3282, 753));
        // No room above
        assert_eq!(by_panels(panels, 540, 1300, 18, 0), (2724, 770));
        // The whole window, when the panels could not be read
        let window = Panels { left: 3264, top: 0, right: 3840, bottom: 2088 };
        assert_eq!(by_panels(window, 540, 510, 18, 0), (2706, 1578));
    }

    #[test]
    fn flyout_stands_above_the_tray() {
        let area = Area { left: 0, top: 0, right: 1920, bottom: 1032 };
        assert_eq!(by_tray(area, 360, 340, 12), (1548, 680));
        // Taller than the screen: its top stays on it
        assert_eq!(by_tray(area, 360, 2000, 12), (1548, 0));
    }
}
