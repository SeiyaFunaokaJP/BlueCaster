//! Time series for the Diagnostics charts: the link samples on a time axis
//! (gaps where samples are missing), the nearest sample to a hovered time,
//! and per-second rates from the cumulative counters.

use ipc::{LinkSample, LinkTotals};

/// Two samples further apart than this are not joined by a line (4 a second normally)
pub const GAP_MS: u64 = 1000;

const GAP: f64 = GAP_MS as f64;

/// A number as JavaScript's `toFixed(digits)` writes it: an exact tie rounds
/// up (Rust's formatting rounds it to even), and -0 has no sign
pub(crate) fn to_fixed(x: f64, digits: usize) -> String {
    if x.is_nan() {
        return "NaN".to_owned();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Infinity" } else { "-Infinity" }.to_owned();
    }
    let a = x.abs();
    // The exact decimal value: a tie is a 5 and nothing after it
    const EXTRA: usize = 80;
    let exact = format!("{a:.prec$}", prec = digits + EXTRA);
    let (head, tail) = exact.split_at(exact.len() - EXTRA);
    let s = if tail.starts_with('5') && tail[1..].bytes().all(|b| b == b'0') {
        increment_decimal(head.trim_end_matches('.'))
    } else {
        format!("{a:.digits$}")
    };
    if x < 0.0 { format!("-{s}") } else { s }
}

/// "1.29" -> "1.30", "9.9" -> "10.0"
fn increment_decimal(s: &str) -> String {
    let mut bytes = s.as_bytes().to_vec();
    let mut i = bytes.len();
    while i > 0 {
        i -= 1;
        match bytes[i] {
            b'.' => continue,
            b'9' => bytes[i] = b'0',
            b => {
                bytes[i] = b + 1;
                return String::from_utf8(bytes).unwrap_or_default();
            }
        }
    }
    let mut out = String::from("1");
    out.push_str(&String::from_utf8(bytes).unwrap_or_default());
    out
}

/// JavaScript's `Math.round`: a tie rounds up (towards +infinity), where
/// Rust's `round` rounds it away from zero
pub(crate) fn js_round(x: f64) -> f64 {
    let r = x.round();
    if x - r == 0.5 { r + 1.0 } else { r }
}

/// The smallest round number (1, 1.2, 1.5, 2, 2.5, 3, 4, 5, 6, 8 × 10^n) at or
/// above v: an automatic scale's top, close enough to keep the line tall
pub fn nice_ceil(v: f64) -> f64 {
    if !v.is_finite() || v <= 0.0 {
        return 1.0;
    }
    let p = 10f64.powf(v.log10().floor());
    for m in [1.0, 1.2, 1.5, 2.0, 2.5, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0] {
        if m * p >= v - 1e-9 * p {
            return m * p;
        }
    }
    10.0 * p
}

/// The scale of a chart: fixed ends where given, else the data's (the top rounded up)
pub fn scale_of(series: &[&[Option<f64>]], min: Option<f64>, max: Option<f64>) -> (f64, f64) {
    let known: Vec<f64> = series.iter().flat_map(|s| s.iter()).filter_map(|v| v.filter(|v| v.is_finite())).collect();
    let lo =
        min.unwrap_or_else(|| if known.is_empty() { 0.0 } else { known.iter().copied().fold(f64::INFINITY, f64::min) });
    let mut hi = max.unwrap_or_else(|| {
        if known.is_empty() { 1.0 } else { known.iter().copied().fold(f64::NEG_INFINITY, f64::max) }
    });
    if max.is_none() && lo >= 0.0 {
        hi = nice_ceil(hi);
    }
    if hi <= lo {
        hi = lo + 1.0;
    }
    (lo, hi)
}

/// Polylines (SVG points) of the values over [start, start + span]: one per
/// run of known values, broken where a value is unknown or samples are
/// missing
#[allow(clippy::too_many_arguments)]
pub fn time_runs(
    times: &[f64],
    values: &[Option<f64>],
    start: f64,
    span: f64,
    lo: f64,
    hi: f64,
    width: f64,
    height: f64,
) -> Vec<String> {
    let mut runs = Vec::new();
    let mut cur: Vec<String> = Vec::new();
    let range = if hi - lo != 0.0 { hi - lo } else { 1.0 };
    let span = if span != 0.0 { span } else { 1.0 };
    let mut flush = |cur: &mut Vec<String>| {
        if !cur.is_empty() {
            runs.push(cur.join(" "));
        }
        cur.clear();
    };
    for (i, v) in values.iter().enumerate() {
        let Some(&time) = times.get(i) else { break };
        if i > 0 && time - times[i - 1] > GAP {
            flush(&mut cur);
        }
        let v = match v {
            Some(v) if v.is_finite() && time >= start => *v,
            _ => {
                flush(&mut cur);
                continue;
            }
        };
        let x = ((time - start) / span) * width;
        let y = height - ((v.max(lo).min(hi) - lo) / range) * height;
        cur.push(format!("{},{}", to_fixed(x, 2), to_fixed(y, 2)));
    }
    flush(&mut cur);
    runs
}

/// The sample closest to `time`, when one is within GAP_MS of it
pub fn nearest_index(times: &[f64], time: f64) -> Option<usize> {
    let mut best = None;
    let mut dist = f64::INFINITY;
    for (i, t) in times.iter().enumerate() {
        let d = (t - time).abs();
        if d < dist {
            dist = d;
            best = Some(i);
        }
    }
    best.filter(|_| dist <= GAP)
}

/// The index of the last known value
pub fn last_known_index(values: &[Option<f64>]) -> Option<usize> {
    values.iter().rposition(|v| v.is_some_and(f64::is_finite))
}

/// The last known value
pub fn last_known(values: &[Option<f64>]) -> Option<f64> {
    last_known_index(values).and_then(|i| values[i])
}

/// The sample's capture rate, else `fallback` (`x.source_rate || fallback`)
fn rate_or(x: &LinkSample, fallback: u32) -> u32 {
    x.source_rate.filter(|&r| r != 0).unwrap_or(fallback)
}

/// Captured audio Windows lost in its own gaps, per sample in ms: None
/// where a gap came without a known loss (per-app capture never measures
/// it) or the rate is unknown
pub fn capture_lost_per_sample(samples: &[LinkSample], fallback_rate: u32) -> Vec<Option<f64>> {
    samples
        .iter()
        .enumerate()
        .map(|(i, x)| {
            let Some(p) = i.checked_sub(1).map(|j| &samples[j]) else { return Some(0.0) };
            let lost = x.capture_lost.unwrap_or(0).saturating_sub(p.capture_lost.unwrap_or(0));
            let gaps = x.capture_discontinuities.unwrap_or(0).saturating_sub(p.capture_discontinuities.unwrap_or(0));
            let rate = rate_or(x, fallback_rate);
            if lost == 0 {
                return if gaps == 0 { Some(0.0) } else { None };
            }
            if rate > 0 { Some((lost as f64 * 1000.0) / rate as f64) } else { None }
        })
        .collect()
}

/// A cumulative counter as a rate per second between samples (None for the
/// first sample, without counters, or where the counter went back);
/// `field` picks the counter: `|t| t.capture_frames`
pub fn rate_per_second(samples: &[LinkSample], field: impl Fn(&LinkTotals) -> u64) -> Vec<Option<f64>> {
    samples
        .iter()
        .enumerate()
        .map(|(i, x)| {
            let p = &samples[i.checked_sub(1)?];
            let (xc, pc) = (x.counters.as_ref()?, p.counters.as_ref()?);
            let dt = x.time_ms as i64 - p.time_ms as i64;
            let d = field(&xc.total) as i64 - field(&pc.total) as i64;
            if dt <= 0 || dt > GAP_MS as i64 || d < 0 {
                return None;
            }
            Some((d as f64 * 1000.0) / dt as f64)
        })
        .collect()
}

/// Audio dropped at capture per sample, in ms (frames where the rate is unknown)
pub fn capture_drops_per_sample(samples: &[LinkSample], fallback_rate: u32) -> Vec<f64> {
    samples
        .iter()
        .enumerate()
        .map(|(i, x)| {
            let Some(p) = i.checked_sub(1).map(|j| &samples[j]) else { return 0.0 };
            let d = match (&x.counters, &p.counters) {
                (Some(xc), Some(pc)) => xc.total.capture_dropped as f64 - pc.total.capture_dropped as f64,
                _ => x.capture_dropped as f64 - p.capture_dropped as f64,
            };
            let rate = rate_or(x, fallback_rate);
            (if rate > 0 { (d * 1000.0) / rate as f64 } else { d }).max(0.0)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ipc::LinkCounters;

    fn totals(f: impl FnOnce(&mut LinkTotals)) -> LinkTotals {
        let mut t = LinkTotals::default();
        f(&mut t);
        t
    }

    fn sample(time_ms: u64, total: LinkTotals) -> LinkSample {
        LinkSample {
            time_ms,
            source_rate: Some(48000),
            counters: Some(LinkCounters { total, stream_start: LinkTotals::default(), stream_starts: 1 }),
            ..Default::default()
        }
    }

    #[test]
    fn to_fixed_rounds_as_javascript() {
        assert_eq!(to_fixed(0.25, 1), "0.3");
        assert_eq!(to_fixed(0.125, 2), "0.13");
        assert_eq!(to_fixed(2.5, 0), "3");
        assert_eq!(to_fixed(9.95, 1), "9.9"); // 9.95 is below the tie in binary
        assert_eq!(to_fixed(99.5, 0), "100");
        assert_eq!(to_fixed(-0.0, 1), "0.0");
        assert_eq!(to_fixed(-0.04, 1), "-0.0");
        assert_eq!(to_fixed(-2.5, 0), "-3");
        assert_eq!(js_round(-2.5), -2.0);
        assert_eq!(js_round(2.5), 3.0);
        assert_eq!(js_round(-2.6), -3.0);
    }

    #[test]
    fn rounds_an_automatic_scales_top_up_to_a_round_number() {
        assert_eq!(nice_ceil(0.0), 1.0);
        assert_eq!(nice_ceil(7.0), 8.0);
        assert_eq!(nice_ceil(1010.0), 1200.0);
        assert_eq!(nice_ceil(18.0), 20.0);
        assert_eq!(nice_ceil(21.0), 25.0);
        assert_eq!(nice_ceil(330.0), 400.0);
        assert_eq!(nice_ceil(1000.0), 1000.0);
        // Fixed ends stay; references widen an automatic scale
        assert_eq!(scale_of(&[&[Some(1.0), Some(2.0), None]], Some(0.0), None), (0.0, 2.0));
        assert_eq!(scale_of(&[&[Some(1.0), Some(2.0)], &[Some(140.0)]], Some(0.0), None), (0.0, 150.0));
        assert_eq!(scale_of(&[&[Some(-30.0), Some(-5.0)]], Some(-40.0), Some(10.0)), (-40.0, 10.0));
        assert_eq!(scale_of(&[&[]], Some(0.0), None), (0.0, 1.0));
    }

    #[test]
    fn draws_samples_on_the_time_axis_and_breaks_the_line_at_gaps() {
        let times = [0.0, 250.0, 500.0, 3000.0, 3250.0];
        let runs =
            time_runs(&times, &[Some(0.0), Some(1.0), None, Some(1.0), Some(0.0)], 0.0, 4000.0, 0.0, 1.0, 400.0, 100.0);
        // Unknown value at 500, nothing between 500 and 3000
        assert_eq!(runs, ["0.00,100.00 25.00,0.00", "300.00,0.00 325.00,100.00"]);
        // Samples before the axis starts are left out
        assert_eq!(
            time_runs(&[0.0, 250.0], &[Some(1.0), Some(1.0)], 100.0, 1000.0, 0.0, 1.0, 100.0, 10.0),
            ["15.00,0.00"]
        );
    }

    #[test]
    fn finds_the_sample_under_the_cursor() {
        assert_eq!(nearest_index(&[0.0, 250.0, 500.0], 300.0), Some(1));
        assert_eq!(nearest_index(&[0.0, 250.0, 500.0], 5000.0), None);
        assert_eq!(nearest_index(&[], 0.0), None);
        assert_eq!(last_known(&[Some(1.0), None, Some(3.0), None]), Some(3.0));
        assert_eq!(last_known(&[None]), None);
        assert_eq!(last_known_index(&[Some(1.0), None, Some(3.0), None]), Some(2));
        assert_eq!(last_known_index(&[]), None);
    }

    #[test]
    fn turns_counters_into_rates_and_capture_drops_into_audio_time() {
        let s = vec![
            sample(0, totals(|t| t.capture_frames = 0)),
            sample(250, totals(|t| t.capture_frames = 12000)),
            sample(
                500,
                totals(|t| {
                    t.capture_frames = 24000;
                    t.capture_dropped = 480;
                }),
            ),
            // A new core: the counters went back
            sample(750, totals(|t| t.capture_frames = 100)),
        ];
        assert_eq!(rate_per_second(&s, |t| t.capture_frames), [None, Some(48000.0), Some(48000.0), None]);
        assert_eq!(capture_drops_per_sample(&s, 0), [0.0, 0.0, 10.0, 0.0]);
        // Without the stream's rate: the fallback, else frames
        let no_rate: Vec<LinkSample> = s.iter().map(|x| LinkSample { source_rate: Some(0), ..x.clone() }).collect();
        assert_eq!(capture_drops_per_sample(&no_rate, 96000)[2], 5.0);
        assert_eq!(capture_drops_per_sample(&no_rate, 0)[2], 480.0);
    }

    #[test]
    fn puts_windows_capture_gaps_in_audio_time_unknown_where_a_gap_lost_nothing_measured() {
        let gap = |time: u64, gaps: u64, lost: u64| LinkSample {
            capture_discontinuities: Some(gaps),
            capture_lost: Some(lost),
            ..sample(time, LinkTotals::default())
        };
        let s = vec![
            sample(0, LinkTotals::default()),
            gap(250, 1, 960),
            gap(500, 1, 960),
            // Per-app capture: a gap without the frames it lost
            gap(750, 2, 960),
        ];
        assert_eq!(capture_lost_per_sample(&s, 0), [Some(0.0), Some(20.0), Some(0.0), None]);
        let no_rate: Vec<LinkSample> = s.iter().map(|x| LinkSample { source_rate: Some(0), ..x.clone() }).collect();
        assert_eq!(capture_lost_per_sample(&no_rate, 0)[1], None);
    }
}
