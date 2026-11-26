//! The playback EQ's response curve, computed as the core's filters are
//! (dsp::biquad: RBJ "Audio EQ Cookbook" biquads), and the editing rules of
//! its points (the ranges the core holds them to, rounding, defaults).
//!
//! The points are the core's `EqPoint` (f32); everything here computes in
//! f64 from the f32 values.

use std::f64::consts::{FRAC_1_SQRT_2, PI};

use ipc::{EqBandType, EqPoint};

use crate::i18n::js_number;
use crate::series::{js_round, to_fixed};

/// Most points (dsp::MAX_EQ_POINTS)
pub const MAX_POINTS: usize = 16;
pub const FREQ_MIN: f64 = 20.0;
pub const FREQ_MAX: f64 = 20000.0;
pub const GAIN_LIMIT: f64 = 12.0;
pub const Q_MIN: f64 = 0.1;
pub const Q_MAX: f64 = 12.0;
/// The rate the graph is drawn for (the curve differs only near the top)
pub const GRAPH_RATE: f64 = 48000.0;

pub const BAND_TYPES: [EqBandType; 5] =
    [EqBandType::Peaking, EqBandType::LowShelf, EqBandType::HighShelf, EqBandType::HighPass, EqBandType::LowPass];

/// Whether a point of this type has a gain (the pass filters do not)
pub fn has_gain(t: EqBandType) -> bool {
    t != EqBandType::HighPass && t != EqBandType::LowPass
}

/// Whether a point of this type has a Q (the shelves do not)
pub fn has_q(t: EqBandType) -> bool {
    t != EqBandType::LowShelf && t != EqBandType::HighShelf
}

#[derive(Debug, Clone, Copy)]
struct Coeffs {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

fn normalized(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> Coeffs {
    Coeffs { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0 }
}

fn peaking(rate: f64, f: f64, q: f64, gain: f64) -> Coeffs {
    let w0 = (2.0 * PI * f) / rate;
    let (sin, cos) = (w0.sin(), w0.cos());
    let alpha = sin / (2.0 * q);
    let a = 10f64.powf(gain / 40.0);
    normalized(1.0 + alpha * a, -2.0 * cos, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * cos, 1.0 - alpha / a)
}

fn shelf(rate: f64, f: f64, gain: f64, high: bool) -> Coeffs {
    let w0 = (2.0 * PI * f) / rate;
    let (sin, cos) = (w0.sin(), w0.cos());
    let a = 10f64.powf(gain / 40.0);
    // Shelf slope S = 1, as the core's shelves always use
    let slope = 1.0;
    let alpha = (sin / 2.0) * ((a + 1.0 / a) * (1.0 / slope - 1.0) + 2.0).sqrt();
    let sa = 2.0 * a.sqrt() * alpha;
    if !high {
        return normalized(
            a * (a + 1.0 - (a - 1.0) * cos + sa),
            2.0 * a * (a - 1.0 - (a + 1.0) * cos),
            a * (a + 1.0 - (a - 1.0) * cos - sa),
            a + 1.0 + (a - 1.0) * cos + sa,
            -2.0 * (a - 1.0 + (a + 1.0) * cos),
            a + 1.0 + (a - 1.0) * cos - sa,
        );
    }
    normalized(
        a * (a + 1.0 + (a - 1.0) * cos + sa),
        -2.0 * a * (a - 1.0 + (a + 1.0) * cos),
        a * (a + 1.0 + (a - 1.0) * cos - sa),
        a + 1.0 - (a - 1.0) * cos + sa,
        2.0 * (a - 1.0 - (a + 1.0) * cos),
        a + 1.0 - (a - 1.0) * cos - sa,
    )
}

fn pass(rate: f64, f: f64, q: f64, high: bool) -> Coeffs {
    let w0 = (2.0 * PI * f) / rate;
    let (sin, cos) = (w0.sin(), w0.cos());
    let alpha = sin / (2.0 * q);
    if high {
        normalized((1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha)
    } else {
        normalized((1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha)
    }
}

/// `Math.min(hi, Math.max(lo, v))`
fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    hi.min(lo.max(v))
}

/// A point's filter, or None when it changes nothing (as Coeffs::for_band)
fn for_band(b: &EqPoint, rate: f64) -> Option<Coeffs> {
    let f = clamp(f64::from(b.frequency), 10.0, (rate / 2.0) * 0.98);
    let q = clamp(f64::from(b.q), 0.05, 30.0);
    let gain = f64::from(b.gain);
    match b.band_type {
        EqBandType::HighPass => return Some(pass(rate, f, q, true)),
        EqBandType::LowPass => return Some(pass(rate, f, q, false)),
        _ => {}
    }
    if gain.abs() < 0.05 {
        return None;
    }
    Some(match b.band_type {
        EqBandType::LowShelf => shelf(rate, f, gain, false),
        EqBandType::HighShelf => shelf(rate, f, gain, true),
        _ => peaking(rate, f, q, gain),
    })
}

fn response_db(c: &Coeffs, f: f64, rate: f64) -> f64 {
    let w = (2.0 * PI * f) / rate;
    let (s1, c1, s2, c2) = (w.sin(), w.cos(), (2.0 * w).sin(), (2.0 * w).cos());
    let nr = c.b0 + c.b1 * c1 + c.b2 * c2;
    let ni = -(c.b1 * s1 + c.b2 * s2);
    let dr = 1.0 + c.a1 * c1 + c.a2 * c2;
    let di = -(c.a1 * s1 + c.a2 * s2);
    let num = nr * nr + ni * ni;
    let den = (dr * dr + di * di).max(1e-30);
    10.0 * (num / den).max(1e-30).log10()
}

fn all_coeffs(bands: &[EqPoint], rate: f64) -> Vec<Coeffs> {
    bands.iter().filter_map(|b| for_band(b, rate)).collect()
}

/// The response of all points at `f` (dB), as dsp::biquad::eq_response_db;
/// `rate` is usually [`GRAPH_RATE`]
pub fn eq_response_db(bands: &[EqPoint], f: f64, rate: f64) -> f64 {
    all_coeffs(bands, rate).iter().map(|c| response_db(c, f, rate)).sum()
}

/// A point of [`response_curve`]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurvePoint {
    pub f: f64,
    pub db: f64,
}

/// The curve at `n` + 1 log-spaced frequencies from FREQ_MIN to FREQ_MAX;
/// `rate` is usually [`GRAPH_RATE`]
pub fn response_curve(bands: &[EqPoint], n: usize, rate: f64) -> Vec<CurvePoint> {
    let coeffs = all_coeffs(bands, rate);
    (0..=n)
        .map(|i| {
            let f = FREQ_MIN * (FREQ_MAX / FREQ_MIN).powf(i as f64 / n as f64);
            CurvePoint { f, db: coeffs.iter().map(|c| response_db(c, f, rate)).sum() }
        })
        .collect()
}

/// A frequency rounded as dragging sets it (1 Hz, 5 Hz, 50 Hz steps)
pub fn round_freq(f: f64) -> f64 {
    let v = clamp(f, FREQ_MIN, FREQ_MAX);
    if v < 100.0 {
        js_round(v)
    } else if v < 1000.0 {
        js_round(v / 5.0) * 5.0
    } else {
        js_round(v / 50.0) * 50.0
    }
}

pub fn round_gain(g: f64) -> f64 {
    clamp(js_round(g * 10.0) / 10.0, -GAIN_LIMIT, GAIN_LIMIT)
}

pub fn round_q(q: f64) -> f64 {
    clamp(js_round(q * 100.0) / 100.0, Q_MIN, Q_MAX)
}

/// A value, or `fallback` where it is not a finite number
fn finite_or(v: f32, fallback: f64) -> f64 {
    if v.is_finite() { f64::from(v) } else { fallback }
}

/// A point held to the ranges the core plays it with
pub fn held_point(b: &EqPoint) -> EqPoint {
    EqPoint {
        frequency: clamp(finite_or(b.frequency, 1000.0), FREQ_MIN, FREQ_MAX) as f32,
        gain: if has_gain(b.band_type) { round_gain(finite_or(b.gain, 0.0)) as f32 } else { 0.0 },
        q: round_q(finite_or(b.q, 1.0)) as f32,
        band_type: b.band_type,
    }
}

/// A point with another type: a pass filter has no gain and a sensible Q
pub fn with_type(b: &EqPoint, t: EqBandType) -> EqPoint {
    let mut out = EqPoint { band_type: t, ..*b };
    if !has_gain(t) {
        out.gain = 0.0;
        if f64::from(out.q) < 0.3 {
            out.q = 0.71;
        }
    }
    out
}

/// "1.25 kHz", "80 Hz"
pub fn format_freq(f: f64) -> String {
    if f >= 1000.0 {
        let k = f / 1000.0;
        return format!("{} kHz", to_fixed(k, if k >= 10.0 { 1 } else { 2 }));
    }
    format!("{} Hz", js_number(js_round(f)))
}

/// "+3.0 dB"
pub fn format_gain(g: f64) -> String {
    let sign = if g > 0.0 {
        "+"
    } else if g < 0.0 {
        "−"
    } else {
        ""
    };
    format!("{sign}{} dB", to_fixed(g.abs(), 1))
}

/// The highest boost of the response (dB, 0 when it only cuts), as the
/// core's preamp takes it (pipeline::eq::max_boost_db): scanned over the
/// audible range and at each point's own frequency; `rate` is usually
/// [`GRAPH_RATE`]
pub fn max_boost_db(bands: &[EqPoint], rate: f64) -> f64 {
    let coeffs = all_coeffs(bands, rate);
    let response = |f: f64| -> f64 { coeffs.iter().map(|c| response_db(c, f, rate)).sum() };
    let top = FREQ_MAX.min(rate * 0.49);
    const SCAN: usize = 192;
    let mut max = 0.0f64;
    for i in 0..SCAN {
        max = max.max(response(FREQ_MIN * (top / FREQ_MIN).powf(i as f64 / (SCAN - 1) as f64)));
    }
    for b in bands {
        let f = f64::from(b.frequency);
        if f < rate * 0.49 {
            max = max.max(response(f));
        }
    }
    max
}

/// Equalizer APO's names of the filters (as AutoEq writes them): the
/// shelves are the cookbook's with the core's slope 1, which is Q 1/√2 at
/// the same frequency (LSC/HSC), the pass filters take their Q (HPQ/LPQ)
fn apo_type(t: EqBandType) -> &'static str {
    match t {
        EqBandType::Peaking => "PK",
        EqBandType::LowShelf => "LSC",
        EqBandType::HighShelf => "HSC",
        EqBandType::HighPass => "HPQ",
        EqBandType::LowPass => "LPQ",
    }
}
const SHELF_Q: f64 = FRAC_1_SQRT_2;

/// The points as an Equalizer APO / AutoEq parametric EQ: the preamp the
/// core plays them with, then one line per point
pub fn eq_apo_text(bands: &[EqPoint]) -> String {
    let num = |v: f64, digits: usize| -> String {
        let s = to_fixed(v, digits);
        if s.parse::<f64>().is_ok_and(|n| n == 0.0) { to_fixed(0.0, digits) } else { s }
    };
    let fc = |f: f64| -> String { if f.fract() == 0.0 { js_number(f) } else { num(f, 1) } };
    // Rounded up: the preamp never lets the loudest part go over
    let preamp = -(max_boost_db(bands, GRAPH_RATE) * 10.0 - 1e-6).ceil() / 10.0;
    let mut lines = vec![format!("Preamp: {} dB", num(preamp, 1))];
    for (i, b) in bands.iter().enumerate() {
        let mut parts =
            vec![format!("Filter {}: ON {} Fc {} Hz", i + 1, apo_type(b.band_type), fc(f64::from(b.frequency)))];
        if has_gain(b.band_type) {
            parts.push(format!("Gain {} dB", num(f64::from(b.gain), 1)));
        }
        parts.push(format!("Q {}", num(if has_q(b.band_type) { f64::from(b.q) } else { SHELF_Q }, 2)));
        lines.push(parts.join(" "));
    }
    lines.join("\r\n") + "\r\n"
}

const fn flat_peak(frequency: f32) -> EqPoint {
    EqPoint { frequency, gain: 0.0, q: 1.41, band_type: EqBandType::Peaking }
}

/// The points a new EQ starts with, and Reset puts back: the classic ten
/// bands an octave apart, flat (the core skips a 0 dB peak, so they change
/// nothing and cost nothing until one is moved)
pub const DEFAULT_POINTS: [EqPoint; 10] = [
    flat_peak(31.0),
    flat_peak(62.0),
    flat_peak(125.0),
    flat_peak(250.0),
    flat_peak(500.0),
    flat_peak(1000.0),
    flat_peak(2000.0),
    flat_peak(4000.0),
    flat_peak(8000.0),
    flat_peak(16000.0),
];

/// Whether `bands` are the default points as they come
pub fn is_default_points(bands: &[EqPoint]) -> bool {
    bands == DEFAULT_POINTS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(frequency: f32, gain: f32, q: f32, band_type: EqBandType) -> EqPoint {
        EqPoint { frequency, gain, q, band_type }
    }

    /// `toBeCloseTo(want, digits)`
    fn close(got: f64, want: f64, digits: i32) {
        assert!((got - want).abs() < 10f64.powi(-digits) / 2.0, "{got} is not close to {want}");
    }

    #[test]
    fn matches_the_cores_filters() {
        // The same points and values as the core's test
        // (pipeline/src/eq.rs, response_table_shared_with_the_ui); the
        // points are f32 as the core keeps them
        use EqBandType::*;
        let bands = [
            p(60.0, 4.5, 0.7, LowShelf),
            p(250.0, -3.0, 1.4, Peaking),
            p(2500.0, 6.0, 0.8, Peaking),
            p(9000.0, -5.0, 1.0, HighShelf),
            p(30.0, 0.0, 0.71, HighPass),
            p(18000.0, 0.0, 0.9, LowPass),
        ];
        let table: [(f64, f64, f64); 9] = [
            (44100.0, 20.0, -3.366949397),
            (44100.0, 2500.0, 5.976168683),
            (44100.0, 19000.0, -9.579220886),
            (48000.0, 45.0, 2.593260591),
            (48000.0, 250.0, -2.88454724),
            (48000.0, 1000.0, 1.484084636),
            (48000.0, 12000.0, -3.400823786),
            (96000.0, 100.0, 0.204033936),
            (96000.0, 6000.0, 1.171083516),
        ];
        for (rate, f, want) in table {
            close(eq_response_db(&bands, f, rate), want, 5);
        }
    }

    #[test]
    fn is_flat_without_points_and_a_peak_reaches_its_gain_at_its_centre() {
        assert_eq!(eq_response_db(&[], 1000.0, GRAPH_RATE), 0.0);
        let peak = p(1000.0, 6.0, 1.0, EqBandType::Peaking);
        close(eq_response_db(&[peak], 1000.0, GRAPH_RATE), 6.0, 3);
        // Under 0.05 dB: left out, as the core does
        assert_eq!(eq_response_db(&[EqPoint { gain: 0.01, ..peak }], 1000.0, GRAPH_RATE), 0.0);
        let curve = response_curve(&[peak], 100, GRAPH_RATE);
        assert_eq!(curve.len(), 101);
        close(curve[0].f, 20.0, 2);
        close(curve[100].f, 20000.0, 2);
    }

    #[test]
    fn points_round_and_hold_values_as_the_graph_sets_them() {
        assert_eq!(round_freq(5.0), 20.0);
        assert_eq!(round_freq(87.4), 87.0);
        assert_eq!(round_freq(432.0), 430.0);
        assert_eq!(round_freq(1234.0), 1250.0);
        assert_eq!(round_freq(99999.0), 20000.0);
        assert_eq!(held_point(&p(1.0, 40.0, 0.0, EqBandType::Peaking)), p(20.0, 12.0, 0.1, EqBandType::Peaking));
        assert_eq!(held_point(&p(80.0, 5.0, 0.7, EqBandType::HighPass)).gain, 0.0);
    }

    #[test]
    fn points_drop_the_gain_for_a_pass_filter() {
        let out = with_type(&p(80.0, 5.0, 0.2, EqBandType::Peaking), EqBandType::HighPass);
        assert_eq!(out, p(80.0, 0.0, 0.71, EqBandType::HighPass));
    }

    #[test]
    fn points_format() {
        assert_eq!(format_freq(80.0), "80 Hz");
        assert_eq!(format_freq(1250.0), "1.25 kHz");
        assert_eq!(format_freq(12000.0), "12.0 kHz");
        assert_eq!(format_gain(3.0), "+3.0 dB");
        assert_eq!(format_gain(-2.5), "−2.5 dB");
        assert_eq!(format_gain(0.0), "0.0 dB");
    }

    const CRLF: &str = "\r\n";

    #[test]
    fn writes_equalizer_apos_parametric_format_with_the_cores_preamp() {
        use EqBandType::*;
        let bands = [
            p(105.0, 4.5, 3.0, LowShelf),
            p(250.0, -3.0, 1.4, Peaking),
            p(1250.5, 2.0, 1.0, Peaking),
            p(9000.0, -5.0, 1.0, HighShelf),
            p(30.0, 0.0, 0.71, HighPass),
            p(18000.0, 0.0, 0.9, LowPass),
        ];
        let text = eq_apo_text(&bands);
        let lines: Vec<&str> = text.split(CRLF).collect();
        let preamp = (max_boost_db(&bands, GRAPH_RATE) * 10.0).ceil() / 10.0;
        assert!(preamp > 0.0);
        assert_eq!(
            lines,
            [
                format!("Preamp: -{} dB", to_fixed(preamp, 1)).as_str(),
                "Filter 1: ON LSC Fc 105 Hz Gain 4.5 dB Q 0.71",
                "Filter 2: ON PK Fc 250 Hz Gain -3.0 dB Q 1.40",
                "Filter 3: ON PK Fc 1250.5 Hz Gain 2.0 dB Q 1.00",
                "Filter 4: ON HSC Fc 9000 Hz Gain -5.0 dB Q 0.71",
                "Filter 5: ON HPQ Fc 30 Hz Q 0.71",
                "Filter 6: ON LPQ Fc 18000 Hz Q 0.90",
                "",
            ]
        );
    }

    #[test]
    fn lowers_the_level_by_the_highest_boost_only() {
        assert_eq!(
            eq_apo_text(&[p(1000.0, 6.0, 1.0, EqBandType::Peaking)]),
            format!("Preamp: -6.0 dB{CRLF}Filter 1: ON PK Fc 1000 Hz Gain 6.0 dB Q 1.00{CRLF}")
        );
        assert_eq!(
            eq_apo_text(&[p(1000.0, -6.0, 1.0, EqBandType::Peaking)]).split(CRLF).next(),
            Some("Preamp: 0.0 dB")
        );
        assert_eq!(eq_apo_text(&[]), format!("Preamp: 0.0 dB{CRLF}"));
    }

    #[test]
    fn default_points_are_flat_peaks() {
        assert!(is_default_points(&DEFAULT_POINTS));
        assert!(!is_default_points(&DEFAULT_POINTS[1..]));
        assert_eq!(eq_response_db(&DEFAULT_POINTS, 1000.0, GRAPH_RATE), 0.0);
    }
}
