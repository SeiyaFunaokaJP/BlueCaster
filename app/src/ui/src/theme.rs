//! Dark / light / system, and the colours of the screens: Windows 11's look.
//!
//! - Greys: Windows 11's neutral ones (the Settings app's): the window
//!   #202020 / #F3F3F3, cards a step lighter with a hairline, controls a
//!   quiet fill with a 1 px outline.
//! - Controls take the Windows accent colour, as Windows' own do (issue
//!   #86): the Connect button, strong buttons, switches, sliders and what is
//!   selected. `accent_fill` is AccentDark1 on light and AccentLight2 on dark
//!   (the defaults are Windows' blue).
//! - The bitpool chart keeps two hues of its own (`chart_1`, `chart_2`).
//! - Status colours: good #34D399, warning #FBBF24, error #F87171 (in light
//!   mode text uses darker shades of the same hues for WCAG AA contrast).
//! - Type: 13 / 15 / 17 / 21 / 30 px in the system's UI font, tabular
//!   numerals for numbers that change.
//! - Shape: Windows 11's: controls 4 px, cards 8 px; 4 px spacing grid; 1 px
//!   borders, no shadows.
//!
//! [`Palette`] is what the screens draw with (`cx.palette()`); the component
//! library's own theme is set from it, so its controls match.

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{App, Global, Hsla, Pixels, Rgba, SharedString, WindowAppearance, px, rgb, rgba};

/// Text sizes: hints, the body (the component library's controls take it
/// too), card titles, section titles, the page's title
pub const FS_SMALL: Pixels = px(13.);
pub const FS_BODY: Pixels = px(15.);
pub const FS_LARGE: Pixels = px(17.);
pub const FS_TITLE: Pixels = px(21.);
pub const FS_PAGE: Pixels = px(30.);

/// Spacing (the 4 px grid: --s1 .. --s8)
pub const S1: Pixels = px(4.);
pub const S2: Pixels = px(8.);
pub const S3: Pixels = px(12.);
pub const S4: Pixels = px(16.);
pub const S5: Pixels = px(20.);
pub const S6: Pixels = px(24.);
pub const S8: Pixels = px(32.);

/// Corner radius of controls and of cards (Windows 11's)
pub const RADIUS: Pixels = px(4.);
pub const RADIUS_CARD: Pixels = px(8.);

/// The system's UI font, as Windows' own windows use (Segoe UI; Yu Gothic
/// UI on a Japanese Windows)
pub const FONT: &str = ".SystemUIFont";
pub const FONT_DISPLAY: &str = ".SystemUIFont";
pub const FONT_MONO: &str = "Cascadia Mono";

/// The colours of the screens
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub dark: bool,
    pub bg: Hsla,
    pub bg_nav: Hsla,
    pub surface: Hsla,
    pub surface_2: Hsla,
    pub surface_3: Hsla,
    pub border: Hsla,
    pub border_strong: Hsla,
    pub text: Hsla,
    pub text_2: Hsla,
    pub text_3: Hsla,
    pub strong_bg: Hsla,
    pub strong_fg: Hsla,
    /// Windows' accent colour for fills, and the text on it
    pub accent_fill: Hsla,
    pub on_accent: Hsla,
    pub disabled_text: Hsla,
    pub good: Hsla,
    pub warn: Hsla,
    pub bad: Hsla,
    /// The status colours as text (darker on light)
    pub good_text: Hsla,
    pub warn_text: Hsla,
    pub bad_text: Hsla,
    /// The status colours as borders
    pub good_line: Hsla,
    pub warn_line: Hsla,
    pub bad_line: Hsla,
    pub focus: Hsla,
    pub overlay: Hsla,
    pub chart_1: Hsla,
    pub chart_2: Hsla,
}

impl Global for Palette {}

fn hex(v: u32) -> Hsla {
    rgb(v).into()
}

fn alpha(c: Hsla, a: f32) -> Hsla {
    Hsla { a, ..c }
}

/// "#rrggbb" -> the colour
fn parse_hex(s: &str) -> Option<Hsla> {
    let v = u32::from_str_radix(s.strip_prefix('#')?, 16).ok()?;
    (s.len() == 7).then(|| hex(v))
}

impl Palette {
    pub fn new(dark: bool, accent: Option<Hsla>) -> Palette {
        let good = hex(0x34d399);
        let warn = hex(0xfbbf24);
        let bad = hex(0xf87171);
        if dark {
            Palette {
                dark,
                bg: hex(0x202020),
                bg_nav: hex(0x202020),
                surface: hex(0x2b2b2b),
                surface_2: hex(0x343434),
                surface_3: hex(0x3d3d3d),
                border: hex(0x383838),
                border_strong: hex(0x4a4a4a),
                text: hex(0xffffff),
                text_2: hex(0xc5c5c5),
                text_3: hex(0x9a9a9a),
                strong_bg: hex(0xffffff),
                strong_fg: hex(0x202020),
                accent_fill: accent.unwrap_or(hex(0x60cdff)),
                on_accent: hex(0x000000),
                disabled_text: rgba(0xffffff5c).into(),
                good,
                warn,
                bad,
                good_text: good,
                warn_text: warn,
                bad_text: bad,
                // A soft tint on dark, the text shade on light (#86)
                good_line: alpha(good, 0.55),
                warn_line: alpha(warn, 0.55),
                bad_line: alpha(bad, 0.55),
                focus: hex(0xffffff),
                overlay: rgba(0x0000008c).into(),
                chart_1: hex(0x4f8cff),
                chart_2: hex(0x8b5cf6),
            }
        } else {
            let (good_text, warn_text, bad_text) = (hex(0x047857), hex(0x92400e), hex(0xb91c1c));
            Palette {
                dark,
                bg: hex(0xf3f3f3),
                bg_nav: hex(0xf3f3f3),
                surface: hex(0xfbfbfb),
                surface_2: hex(0xececec),
                surface_3: hex(0xe2e2e2),
                border: hex(0xe5e5e5),
                border_strong: hex(0xcccccc),
                text: hex(0x1b1b1b),
                text_2: hex(0x5c5c5c),
                text_3: hex(0x6b6b6b),
                strong_bg: hex(0x1b1b1b),
                strong_fg: hex(0xffffff),
                accent_fill: accent.unwrap_or(hex(0x005fb8)),
                on_accent: hex(0xffffff),
                disabled_text: rgba(0x0000005c).into(),
                good,
                warn,
                bad,
                good_text,
                warn_text,
                bad_text,
                good_line: good_text,
                warn_line: warn_text,
                bad_line: bad_text,
                focus: hex(0x1b1b1b),
                overlay: Rgba { r: 20. / 255., g: 22. / 255., b: 28. / 255., a: 0.35 }.into(),
                chart_1: hex(0x4f8cff),
                chart_2: hex(0x8b5cf6),
            }
        }
    }
}

/// `cx.palette()`
pub trait ActivePalette {
    fn palette(&self) -> &Palette;
}

impl ActivePalette for App {
    fn palette(&self) -> &Palette {
        self.global::<Palette>()
    }
}

/// Mixes `over` onto `base` (0.0 = base, 1.0 = over), for hover and pressed fills
pub fn mix(base: Hsla, over: Hsla, amount: f32) -> Hsla {
    let (a, b): (Rgba, Rgba) = (base.into(), over.into());
    let m = |x: f32, y: f32| x + (y - x) * amount;
    Rgba { r: m(a.r, b.r), g: m(a.g, b.g), b: m(a.b, b.b), a: m(a.a, b.a) }.into()
}

/// The theme setting as saved: anything but "light" and "system" is dark,
/// as v1 read it (v1 files may say e.g. "white")
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeSetting {
    Dark,
    Light,
    System,
}

impl ThemeSetting {
    pub fn parse(s: &str) -> ThemeSetting {
        match s {
            "light" => ThemeSetting::Light,
            "system" => ThemeSetting::System,
            _ => ThemeSetting::Dark,
        }
    }
}

/// The setting in force, kept so that a change of Windows' own theme or
/// accent applies again
struct Current(ThemeSetting);
impl Global for Current {}

/// Applies the theme setting (and Windows' accent colour, read again)
pub fn apply(setting: ThemeSetting, cx: &mut App) {
    cx.set_global(Current(setting));
    refresh(cx);
}

/// Reads Windows' theme and accent again: when the window gets the focus,
/// so a change in Settings shows, and when Windows says its theme changed
pub fn refresh(cx: &mut App) {
    let setting = cx.try_global::<Current>().map_or(ThemeSetting::System, |c| c.0);
    let dark = match setting {
        ThemeSetting::Dark => true,
        ThemeSetting::Light => false,
        ThemeSetting::System => {
            matches!(cx.window_appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark)
        }
    };
    // Tests keep to the default accent: they read nothing of the PC's
    let accent = if cfg!(test) { None } else { win_sys::accent::accent_colors() }
        .and_then(|a| parse_hex(if dark { &a.dark } else { &a.light }));
    let palette = Palette::new(dark, accent);
    if cx.try_global::<Palette>() == Some(&palette) {
        return;
    }
    cx.set_global(palette);
    Theme::change(if dark { ThemeMode::Dark } else { ThemeMode::Light }, None, cx);
    Theme::update(cx, |t| style_components(t, &palette));
    crate::windows::apply_theme(cx);
}

/// The component library's controls in the screens' colours and Windows 11's shapes
fn style_components(t: &mut Theme, p: &Palette) {
    t.font_family = SharedString::new_static(FONT);
    t.font_size = FS_BODY;
    t.mono_font_family = SharedString::new_static(FONT_MONO);
    t.mono_font_size = px(13.);
    t.radius = RADIUS;
    t.radius_lg = RADIUS_CARD;
    t.shadow = false;

    let c = &mut t.colors;
    c.background = p.bg;
    c.foreground = p.text;
    c.border = p.border;
    c.muted = p.surface_2;
    c.muted_foreground = p.text_2;
    c.ring = p.focus;
    c.caret = p.text;
    c.selection = alpha(p.accent_fill, 0.35);
    c.overlay = p.overlay;

    // The accent: primary buttons, switches, sliders, what is selected
    c.primary = p.accent_fill;
    c.primary_hover = mix(p.accent_fill, p.bg, 0.10);
    c.primary_active = mix(p.accent_fill, p.bg, 0.20);
    c.primary_foreground = p.on_accent;
    c.button_primary = p.accent_fill;
    c.button_primary_hover = c.primary_hover;
    c.button_primary_active = c.primary_active;
    c.button_primary_foreground = p.on_accent;
    // Off: a grey track the thumb stands out on, as Windows' own
    c.switch = p.text_3;
    c.slider_bar = p.accent_fill;
    c.slider_thumb = p.accent_fill;
    c.progress_bar = p.accent_fill;
    c.link = p.accent_fill;
    c.link_hover = p.accent_fill;
    c.link_active = p.accent_fill;

    // Windows' standard button: a quiet fill with a 1 px outline
    c.secondary = p.surface_2;
    c.secondary_hover = p.surface_3;
    c.secondary_active = mix(p.surface_3, p.bg, 0.4);
    c.secondary_foreground = p.text;
    c.button = p.surface_2;
    c.button_hover = p.surface_3;
    c.button_active = mix(p.surface_3, p.bg, 0.4);
    c.button_foreground = p.text;
    c.button_secondary = p.surface_2;
    c.button_secondary_hover = p.surface_3;
    c.button_secondary_active = mix(p.surface_3, p.bg, 0.4);
    c.button_secondary_foreground = p.text;
    c.accent = p.surface_3;
    c.accent_foreground = p.text;

    c.danger = p.bad;
    c.danger_hover = mix(p.bad, p.bg, 0.10);
    c.danger_active = mix(p.bad, p.bg, 0.20);
    c.danger_foreground = hex(0x000000);
    c.button_danger = p.bad;
    c.button_danger_hover = c.danger_hover;
    c.button_danger_active = c.danger_active;
    c.button_danger_foreground = hex(0x000000);
    c.success = p.good;
    c.warning = p.warn;

    c.input = p.border_strong;
    c.popover = p.surface;
    c.popover_foreground = p.text;
    c.list = p.surface;
    c.list_hover = p.surface_2;
    c.list_active = p.surface_3;
    c.group_box = p.surface;
    c.group_box_foreground = p.text;
    c.sidebar = p.bg_nav;
    c.sidebar_foreground = p.text;
    c.sidebar_border = p.border;
    c.sidebar_accent = p.surface_2;
    c.sidebar_accent_foreground = p.text;
    c.tab = gpui_kit::transparent_black();
    c.tab_bar = gpui_kit::transparent_black();
    // The segmented choice: a well, the chosen one in the window's colour
    // (the library draws it so)
    c.tab_bar_segmented = if p.dark { hex(0x161616) } else { p.surface_3 };
    c.tab_active = p.surface_2;
    c.tab_active_foreground = p.text;
    c.tab_foreground = p.text_2;
    c.title_bar = p.bg_nav;
    c.title_bar_border = p.border;
    c.scrollbar = gpui_kit::transparent_black();
    c.scrollbar_thumb = p.border_strong;
    c.scrollbar_thumb_hover = p.text_3;
}
