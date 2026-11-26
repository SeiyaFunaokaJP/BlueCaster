//! The playback EQ editor's rules: the graph's
//! geometry, what dragging, the keys, the wheel and the menu do to a point,
//! where Add point puts one, the fields' values and which controls are
//! enabled. The maths of the curve is [`crate::eq`].

use ipc::{EqBandType, EqPoint, MediaEq};

use crate::eq::{
    DEFAULT_POINTS, FREQ_MAX, FREQ_MIN, GAIN_LIMIT, MAX_POINTS, format_freq, format_gain, has_gain, has_q, held_point,
    is_default_points, round_freq, round_gain, round_q,
};
use crate::i18n::t;
use crate::series::to_fixed;
use crate::t;

/// The exported file's suggested name
pub const EXPORT_NAME: &str = "BlueCaster EQ.txt";
/// Shortest time between two previews sent while dragging (ms)
pub const PREVIEW_MS: u64 = 40;

pub fn type_key(b: EqBandType) -> &'static str {
    match b {
        EqBandType::Peaking => "eq.type.peaking",
        EqBandType::LowShelf => "eq.type.low_shelf",
        EqBandType::HighShelf => "eq.type.high_shelf",
        EqBandType::HighPass => "eq.type.high_pass",
        EqBandType::LowPass => "eq.type.low_pass",
    }
}

pub fn type_label(b: EqBandType) -> String {
    t(type_key(b))
}

// ---- the graph's geometry -----------------------------------------------------
//
// The graph is drawn in a box of W x H units, stretched to the width it gets
// (the aspect ratio kept).

pub const W: f64 = 640.0;
pub const H: f64 = 240.0;
pub const PLOT_LEFT: f64 = 38.0;
pub const PLOT_RIGHT: f64 = W - 10.0;
pub const PLOT_TOP: f64 = 10.0;
pub const PLOT_BOTTOM: f64 = H - 22.0;
const DB_RANGE: f64 = GAIN_LIMIT;
pub const GRID_F: [f64; 10] = [20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10000.0, 20000.0];
pub const GRID_DB: [f64; 9] = [-12.0, -9.0, -6.0, -3.0, 0.0, 3.0, 6.0, 9.0, 12.0];

fn log_span() -> f64 {
    (FREQ_MAX / FREQ_MIN).log10()
}

pub fn x_of(f: f64) -> f64 {
    PLOT_LEFT + ((FREQ_MAX.min(FREQ_MIN.max(f)) / FREQ_MIN).log10() / log_span()) * (PLOT_RIGHT - PLOT_LEFT)
}

pub fn f_of(x: f64) -> f64 {
    FREQ_MIN * 10f64.powf(1f64.min(0f64.max((x - PLOT_LEFT) / (PLOT_RIGHT - PLOT_LEFT))) * log_span())
}

pub fn y_of(db: f64) -> f64 {
    PLOT_TOP + ((DB_RANGE - db) / (2.0 * DB_RANGE)) * (PLOT_BOTTOM - PLOT_TOP)
}

pub fn db_of(y: f64) -> f64 {
    DB_RANGE - ((y - PLOT_TOP) / (PLOT_BOTTOM - PLOT_TOP)) * 2.0 * DB_RANGE
}

/// Where a point sits (a pass filter on the 0 dB line)
pub fn point_y(b: &EqPoint) -> f64 {
    y_of(if has_gain(b.band_type) { f64::from(b.gain) } else { 0.0 })
}

/// An axis label of the frequency grid: "50", "1k"
pub fn freq_axis_label(f: f64) -> String {
    if f >= 1000.0 { format!("{}k", crate::i18n::js_number(f / 1000.0)) } else { crate::i18n::js_number(f) }
}

/// An axis label of the gain grid: "+6", "−6", "0"
pub fn db_axis_label(db: f64) -> String {
    if db > 0.0 {
        format!("+{}", crate::i18n::js_number(db))
    } else if db < 0.0 {
        format!("−{}", crate::i18n::js_number(-db))
    } else {
        "0".to_owned()
    }
}

// ---- the points ---------------------------------------------------------------

/// The values a point shows: its type, frequency, gain and Q (those it has)
pub fn point_values(b: &EqPoint) -> Vec<String> {
    let mut v = vec![type_label(b.band_type), format_freq(f64::from(b.frequency))];
    if has_gain(b.band_type) {
        v.push(format_gain(f64::from(b.gain)));
    }
    if has_q(b.band_type) {
        v.push(format!("Q {}", to_fixed(f64::from(b.q), 2)));
    }
    v
}

/// "Point 1: Peak, 1.00 kHz, +6.0 dB, Q 1.00" (its name for screen readers)
pub fn point_label(b: &EqPoint, i: usize) -> String {
    t!("eq.point_label", i + 1, point_values(b).join(", "))
}

/// The hovered or dragged point's values beside it
pub fn tip_text(b: &EqPoint) -> String {
    point_values(b).join(" · ")
}

/// `bands` with point `i` replaced, held to the ranges the core plays
pub fn replace(bands: &[EqPoint], i: usize, b: &EqPoint) -> Vec<EqPoint> {
    let mut out = bands.to_vec();
    if let Some(slot) = out.get_mut(i) {
        *slot = held_point(b);
    }
    out
}

/// `bands` without point `i`
pub fn without(bands: &[EqPoint], i: usize) -> Vec<EqPoint> {
    bands.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, b)| *b).collect()
}

/// A new Peak point at a place of the graph (double-click, the menu's "Add
/// point here") -> the points and the new one's index; None when full
pub fn add_at(bands: &[EqPoint], x: f64, y: f64) -> Option<(Vec<EqPoint>, usize)> {
    if bands.len() >= MAX_POINTS || !x.is_finite() || !y.is_finite() {
        return None;
    }
    let p = EqPoint {
        frequency: round_freq(f_of(x)) as f32,
        gain: round_gain(db_of(y)) as f32,
        q: 1.0,
        band_type: EqBandType::Peaking,
    };
    let mut out = bands.to_vec();
    out.push(p);
    Some((out, bands.len()))
}

/// Add point (the button): where no point is yet, from 1 kHz outwards, then
/// between the default bands -> the points and the new one's index; None
/// when full
pub fn add_point(bands: &[EqPoint]) -> Option<(Vec<EqPoint>, usize)> {
    if bands.len() >= MAX_POINTS {
        return None;
    }
    let taken = |f: f64| bands.iter().any(|b| (f64::from(b.frequency) / f).log2().abs() < 0.5);
    let free = [
        1000.0, 2000.0, 500.0, 4000.0, 250.0, 8000.0, 125.0, 16000.0, 63.0, 31.0, 1400.0, 700.0, 2800.0, 350.0, 5600.0,
        180.0, 11000.0, 90.0, 45.0,
    ]
    .into_iter()
    .find(|f| !taken(*f))
    .unwrap_or(1000.0);
    let mut out = bands.to_vec();
    out.push(EqPoint { frequency: free as f32, gain: 0.0, q: 1.0, band_type: EqBandType::Peaking });
    Some((out, bands.len()))
}

/// A point dragged to a place of the graph (left/right the frequency, up/down
/// the gain); None when that changes nothing
pub fn dragged(b: &EqPoint, x: f64, y: f64) -> Option<EqPoint> {
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    let moved = EqPoint {
        frequency: round_freq(f_of(x)) as f32,
        gain: if has_gain(b.band_type) { round_gain(db_of(y)) as f32 } else { 0.0 },
        ..*b
    };
    (moved.frequency != b.frequency || moved.gain != b.gain).then_some(moved)
}

/// The wheel over a point changes its Q (`delta_y`: as a browser's deltaY,
/// positive when scrolled down); None when it has no Q or nothing changes
pub fn wheel_q(b: &EqPoint, delta_y: f64) -> Option<EqPoint> {
    if !has_q(b.band_type) {
        return None;
    }
    let q = round_q(f64::from(b.q) * 1.0015f64.powf(-delta_y)) as f32;
    (q != b.q).then_some(EqPoint { q, ..*b })
}

/// A double-click on a point puts its gain back to 0; None when it has none
pub fn gain_reset(b: &EqPoint) -> Option<EqPoint> {
    (has_gain(b.band_type) && b.gain != 0.0).then_some(EqPoint { gain: 0.0, ..*b })
}

/// The keys a focused point takes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EqKey {
    Left,
    Right,
    Up,
    Down,
    PageUp,
    PageDown,
    Delete,
    ContextMenu,
}

impl EqKey {
    /// A key by GPUI's name ("left", "pageup", "menu" ...)
    pub fn from_name(key: &str) -> Option<EqKey> {
        Some(match key {
            "left" => EqKey::Left,
            "right" => EqKey::Right,
            "up" => EqKey::Up,
            "down" => EqKey::Down,
            "pageup" => EqKey::PageUp,
            "pagedown" => EqKey::PageDown,
            "delete" | "backspace" => EqKey::Delete,
            "menu" => EqKey::ContextMenu,
            _ => return None,
        })
    }
}

/// What a key does to the focused point
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum KeyAction {
    /// The point changed (saved at once)
    Change(EqPoint),
    Delete,
    OpenMenu,
    /// Not for this point (no gain, no Q): the key is not taken
    Ignore,
}

/// A focused point moves with the arrow keys (Shift: bigger steps), Page
/// Up / Page Down change its Q, Delete removes it
pub fn key_action(b: &EqPoint, key: EqKey, shift: bool) -> KeyAction {
    match key {
        EqKey::Left | EqKey::Right => {
            // 1/12 octave, an octave with Shift
            let step = (if shift { 1.0 } else { 1.0 / 12.0 }) * if key == EqKey::Left { -1.0 } else { 1.0 };
            let f = f64::from(b.frequency);
            let mut next = round_freq(f * 2f64.powf(step));
            if next == f {
                next = f + if step > 0.0 { 1.0 } else { -1.0 };
            }
            KeyAction::Change(EqPoint { frequency: next as f32, ..*b })
        }
        EqKey::Up | EqKey::Down => {
            if !has_gain(b.band_type) {
                return KeyAction::Ignore;
            }
            let step = (if shift { 3.0 } else { 0.5 }) * if key == EqKey::Up { 1.0 } else { -1.0 };
            KeyAction::Change(EqPoint { gain: round_gain(f64::from(b.gain) + step) as f32, ..*b })
        }
        EqKey::PageUp | EqKey::PageDown => {
            if !has_q(b.band_type) {
                return KeyAction::Ignore;
            }
            let k = if key == EqKey::PageUp { 1.25 } else { 0.8 };
            KeyAction::Change(EqPoint { q: round_q(f64::from(b.q) * k) as f32, ..*b })
        }
        EqKey::Delete => KeyAction::Delete,
        EqKey::ContextMenu => KeyAction::OpenMenu,
    }
}

// ---- the editor -----------------------------------------------------------------

/// The EQ turned on or off: a new EQ starts with the default points
pub fn toggled(eq: &MediaEq, on: bool) -> MediaEq {
    MediaEq { enabled: on, bands: if on && eq.bands.is_empty() { DEFAULT_POINTS.to_vec() } else { eq.bands.clone() } }
}

/// The points the graph shows: off without points, the ones turning it on
/// gives (greyed out)
pub fn shown_bands(eq: &MediaEq) -> Vec<EqPoint> {
    if !eq.enabled && eq.bands.is_empty() { DEFAULT_POINTS.to_vec() } else { eq.bands.clone() }
}

/// A selection that still points at a point
pub fn valid_selection(eq: &MediaEq, selected: Option<usize>) -> Option<usize> {
    selected.filter(|i| *i < eq.bands.len())
}

/// Which of the editor's controls take input. Nothing hides (issue #86):
/// off, everything stays but disabled; the fields are empty while no point
/// is selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EqControls {
    /// The switch: the device has a profile to save it in
    pub toggle: bool,
    /// Editing: on, with a profile to save it in (the graph takes input)
    pub active: bool,
    pub point_pick: bool,
    pub delete: bool,
    pub type_pick: bool,
    pub frequency: bool,
    pub gain: bool,
    pub q: bool,
    pub add: bool,
    pub reset: bool,
    pub export: bool,
}

pub fn eq_controls(eq: &MediaEq, has_entry: bool, selected: Option<usize>) -> EqControls {
    let active = eq.enabled && has_entry;
    let sel = valid_selection(eq, selected).and_then(|i| eq.bands.get(i));
    let on_sel = active && sel.is_some();
    EqControls {
        toggle: has_entry,
        active,
        point_pick: active && !eq.bands.is_empty(),
        delete: on_sel,
        type_pick: on_sel,
        frequency: on_sel,
        gain: on_sel && sel.is_some_and(|b| has_gain(b.band_type)),
        q: on_sel && sel.is_some_and(|b| has_q(b.band_type)),
        add: active && eq.bands.len() < MAX_POINTS,
        reset: active && !is_default_points(&eq.bands),
        export: active && !eq.bands.is_empty(),
    }
}

/// A number field's text: the value, empty while there is none (or the field is disabled)
pub fn field_text(value: Option<f32>, enabled: bool) -> String {
    match value {
        // The f32's shortest form: 0.1, not 0.10000000149011612
        Some(v) if enabled => crate::i18n::js_number(v.to_string().parse().unwrap_or(f64::from(v))),
        _ => String::new(),
    }
}

/// A number typed in -> the value held to its range; None when it is not a
/// number (the field then shows the value again)
pub fn typed_number(text: &str, min: f64, max: f64) -> Option<f64> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let v: f64 = text.parse().ok().filter(|v: &f64| v.is_finite())?;
    Some(max.min(min.max(v)))
}

/// The ranges and steps of the fields (frequency, gain, Q)
pub const FREQ_FIELD: (f64, f64, f64) = (FREQ_MIN, FREQ_MAX, 1.0);
pub const GAIN_FIELD: (f64, f64, f64) = (-GAIN_LIMIT, GAIN_LIMIT, 0.1);
pub const Q_FIELD: (f64, f64, f64) = (crate::eq::Q_MIN, crate::eq::Q_MAX, 0.01);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eq::{eq_apo_text, with_type};

    fn peak(frequency: f32, gain: f32, q: f32) -> EqPoint {
        EqPoint { frequency, gain, q, band_type: EqBandType::Peaking }
    }

    fn on(bands: Vec<EqPoint>) -> MediaEq {
        MediaEq { enabled: true, bands }
    }

    fn change(a: KeyAction) -> EqPoint {
        match a {
            KeyAction::Change(p) => p,
            other => panic!("not a change: {other:?}"),
        }
    }

    #[test]
    fn is_off_until_turned_on_and_adds_a_point_where_the_graph_is_double_clicked() {
        let off = MediaEq::default();
        // Off: shown, but disabled, with the points turning it on gives
        assert_eq!(shown_bands(&off).len(), 10);
        let c = eq_controls(&off, true, None);
        assert!(c.toggle && !c.active && !c.add && !c.frequency);
        assert_eq!(toggled(&off, true), on(DEFAULT_POINTS.to_vec()));
        // 1 kHz is 2/3 of the way along the log axis; +6 dB a quarter from the top
        let (bands, i) = add_at(&[], 38.0 + (592.0 * 50f64.log10()) / 3.0, 10.0 + 208.0 * 0.25).unwrap();
        assert_eq!(i, 0);
        let p = bands[0];
        assert_eq!(p.band_type, EqBandType::Peaking);
        assert_eq!(p.frequency, 1000.0);
        assert!((p.gain - 6.0).abs() < 0.05);
        assert_eq!(point_label(&p, 0), "Point 1: Peak, 1.00 kHz, +6.0 dB, Q 1.00");
        // The selected point's fields
        let c = eq_controls(&on(bands.clone()), true, Some(0));
        assert!(c.frequency);
        assert_eq!(field_text(Some(p.frequency), c.frequency), "1000");
        // A stored f32 shows as typed
        assert_eq!(field_text(Some(0.1), true), "0.1");
        assert_eq!(field_text(Some(-2.3), true), "-2.3");
        assert_eq!(field_text(Some(0.99), true), "0.99");
    }

    #[test]
    fn moves_a_focused_point_with_the_keyboard_and_deletes_it() {
        let (bands, _) = add_point(&[]).unwrap();
        let p = bands[0];
        assert_eq!(p, peak(1000.0, 0.0, 1.0));
        let p = change(key_action(&p, EqKey::Up, false));
        assert_eq!(p.gain, 0.5);
        let p = change(key_action(&p, EqKey::Right, true));
        assert_eq!(p.frequency, 2000.0);
        let p = change(key_action(&p, EqKey::PageUp, false));
        assert_eq!(p.q, 1.25);
        assert_eq!(key_action(&p, EqKey::Delete, false), KeyAction::Delete);
        assert_eq!(without(&[p], 0), []);
        // A small step that rounds to the same frequency still moves it
        let low = change(key_action(&peak(20.0, 0.0, 1.0), EqKey::Right, false));
        assert_eq!(low.frequency, 21.0);
        // No gain to move on a pass filter
        let cut = with_type(&p, EqBandType::HighPass);
        assert_eq!(key_action(&cut, EqKey::Up, false), KeyAction::Ignore);
        assert_eq!(key_action(&with_type(&p, EqBandType::LowShelf), EqKey::PageUp, false), KeyAction::Ignore);
        assert_eq!(EqKey::from_name("menu"), Some(EqKey::ContextMenu));
        assert_eq!(EqKey::from_name("backspace"), Some(EqKey::Delete));
    }

    #[test]
    fn changes_a_points_type_from_its_menu_and_resets() {
        let (bands, _) = add_point(&[]).unwrap();
        let p = change(key_action(&bands[0], EqKey::Up, true));
        assert_eq!(p.gain, 3.0);
        let bands = replace(&[p], 0, &with_type(&p, EqBandType::HighPass));
        assert_eq!(bands[0].band_type, EqBandType::HighPass);
        assert_eq!(bands[0].gain, 0.0);
        // A pass filter has no gain to type
        let c = eq_controls(&on(bands.clone()), true, Some(0));
        assert!(!c.gain && c.q && c.type_pick);
        assert!(c.reset);
        assert!(!eq_controls(&on(DEFAULT_POINTS.to_vec()), true, None).reset);
    }

    #[test]
    fn starts_with_ten_flat_points_which_reset_puts_back_and_export_writes() {
        let eq = on(DEFAULT_POINTS.to_vec());
        assert!(!eq_controls(&eq, true, None).reset);
        let p6 = change(key_action(&eq.bands[5], EqKey::Up, false));
        let bands = replace(&eq.bands, 5, &p6);
        assert_eq!((bands[5].frequency, bands[5].gain), (1000.0, 0.5));
        let text = eq_apo_text(&bands);
        let lines: Vec<&str> = text.split("\r\n").collect();
        assert_eq!(lines.len(), 12);
        assert_eq!(lines[1], "Filter 1: ON PK Fc 31 Hz Gain 0.0 dB Q 1.41");
        assert_eq!(lines[6], "Filter 6: ON PK Fc 1000 Hz Gain 0.5 dB Q 1.41");
        assert_eq!(lines[10], "Filter 10: ON PK Fc 16000 Hz Gain 0.0 dB Q 1.41");
        assert!(eq_controls(&on(bands), true, None).reset);
    }

    #[test]
    fn takes_typed_values_held_to_their_range() {
        let (lo, hi, _) = GAIN_FIELD;
        assert_eq!(typed_number("40", lo, hi), Some(12.0));
        let (lo, hi, _) = Q_FIELD;
        assert_eq!(typed_number("2.5", lo, hi), Some(2.5));
        // Not a number: the field shows the value again
        assert_eq!(typed_number("", lo, hi), None);
        assert_eq!(typed_number("abc", lo, hi), None);
        let p = held_point(&EqPoint { gain: 12.0, ..peak(1000.0, 0.0, 1.0) });
        assert_eq!(p.gain, 12.0);
    }

    #[test]
    fn saves_a_dragged_point_once_it_is_let_go() {
        let p = add_point(&[]).unwrap().0[0];
        // The far left, the top: 20 Hz, +12 dB (beyond the plot: held to it)
        let moved = dragged(&p, 10.0, 0.0).unwrap();
        assert_eq!((moved.frequency, moved.gain), (20.0, 12.0));
        assert_eq!(dragged(&moved, 38.0, 10.0), None);
        assert!(point_label(&moved, 0).starts_with("Point 1: Peak, 20 Hz, +12.0 dB"));
        // A pass filter moves along the 0 dB line
        let cut = with_type(&p, EqBandType::HighPass);
        assert_eq!(dragged(&cut, 38.0, 10.0).unwrap().gain, 0.0);
    }

    #[test]
    fn turns_off_and_keeps_its_points() {
        let eq = on(vec![peak(1000.0, 0.0, 1.0)]);
        let off = toggled(&eq, false);
        assert!(!off.enabled);
        assert_eq!(off.bands.len(), 1);
        assert_eq!(shown_bands(&off).len(), 1);
        let c = eq_controls(&off, true, Some(0));
        assert!(!c.active && !c.add && !c.reset && !c.export && !c.point_pick && !c.delete);
        // No profile to save it in: the switch too
        assert!(!eq_controls(&eq, false, None).toggle);
    }

    #[test]
    fn keeps_its_fields_empty_until_a_point_is_picked() {
        let empty = on(vec![]);
        let c = eq_controls(&empty, true, None);
        assert!(!c.point_pick && !c.type_pick && !c.frequency && !c.delete && !c.export);
        assert_eq!(field_text(None, c.frequency), "");
        let (bands, _) = add_point(&[]).unwrap();
        let (bands, i) = add_point(&bands).unwrap();
        assert_eq!(i, 1);
        assert_eq!(bands[1].frequency, 2000.0);
        let eq = on(bands);
        // None selected: everything below the graph empty, the Point box still picks one
        let c = eq_controls(&eq, true, None);
        assert!(c.point_pick && !c.frequency);
        assert!(eq_controls(&eq, true, Some(0)).frequency);
        // A selection beyond the points is none
        assert_eq!(valid_selection(&eq, Some(5)), None);
    }

    #[test]
    fn exports_its_points_for_equalizer_apo() {
        assert!(!eq_controls(&on(vec![]), true, None).export);
        let (bands, _) = add_point(&[]).unwrap();
        assert!(eq_controls(&on(bands.clone()), true, None).export);
        assert_eq!(EXPORT_NAME, "BlueCaster EQ.txt");
        assert_eq!(
            eq_apo_text(&bands).split("\r\n").collect::<Vec<_>>(),
            ["Preamp: 0.0 dB", "Filter 1: ON PK Fc 1000 Hz Gain 0.0 dB Q 1.00", ""]
        );
    }

    #[test]
    fn the_wheel_changes_q_and_a_double_click_flattens() {
        let p = peak(1000.0, 3.0, 1.0);
        assert!(wheel_q(&p, -100.0).unwrap().q > 1.0);
        assert!(wheel_q(&p, 100.0).unwrap().q < 1.0);
        assert_eq!(wheel_q(&with_type(&p, EqBandType::HighShelf), 100.0), None);
        assert_eq!(gain_reset(&p).unwrap().gain, 0.0);
        assert_eq!(gain_reset(&peak(1000.0, 0.0, 1.0)), None);
        // Add point is full at 16
        let full = vec![peak(1000.0, 0.0, 1.0); MAX_POINTS];
        assert_eq!(add_point(&full), None);
        assert_eq!(add_at(&full, 100.0, 100.0), None);
    }

    #[test]
    fn graph_axes() {
        assert_eq!(freq_axis_label(1000.0), "1k");
        assert_eq!(freq_axis_label(50.0), "50");
        assert_eq!(db_axis_label(6.0), "+6");
        assert_eq!(db_axis_label(-6.0), "−6");
        assert!((x_of(20.0) - PLOT_LEFT).abs() < 1e-9);
        assert!((x_of(20000.0) - PLOT_RIGHT).abs() < 1e-9);
        assert!((y_of(0.0) - (PLOT_TOP + PLOT_BOTTOM) / 2.0).abs() < 1e-9);
    }
}
