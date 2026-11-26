//! The geometry of the small charts: polylines in a drawing of a fixed size,
//! which the window stretches to the chart's box, and the status colour of
//! a value.

use crate::series::GAP_MS;

/// A value's status colour (`st-good`, `st-warn`, `st-bad`)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Good,
    Warn,
    Bad,
}

/// A point of a drawing (x right, y down)
pub type Pt = (f64, f64);

/// One polyline per run of known values (gaps where the value is unknown),
/// the values spread evenly over `width` (Sparkline)
pub fn points(values: &[Option<f64>], min: f64, max: f64, width: f64, height: f64) -> Vec<Vec<Pt>> {
    let mut runs = Vec::new();
    let mut cur: Vec<Pt> = Vec::new();
    let span = if max - min != 0.0 { max - min } else { 1.0 };
    let n = values.len().saturating_sub(1).max(1) as f64;
    for (i, v) in values.iter().enumerate() {
        match v {
            Some(v) if v.is_finite() => {
                let x = (i as f64 / n) * width;
                let y = height - ((v.max(min).min(max) - min) / span) * height;
                cur.push((x, y));
            }
            _ => {
                if !cur.is_empty() {
                    runs.push(std::mem::take(&mut cur));
                }
            }
        }
    }
    if !cur.is_empty() {
        runs.push(cur);
    }
    runs
}

/// The ends of a sparkline's scale: fixed where given, else the data's
pub fn bounds(values: &[Option<f64>], min: Option<f64>, max: Option<f64>) -> (f64, f64) {
    let known: Vec<f64> = values.iter().filter_map(|v| v.filter(|v| v.is_finite())).collect();
    let lo =
        min.unwrap_or_else(|| if known.is_empty() { 0.0 } else { known.iter().copied().fold(f64::INFINITY, f64::min) });
    let mut hi = max.unwrap_or_else(|| {
        if known.is_empty() { 1.0 } else { known.iter().copied().fold(f64::NEG_INFINITY, f64::max) }
    });
    if hi <= lo {
        hi = lo + 1.0;
    }
    (lo, hi)
}

/// [`crate::series::time_runs`] as points: the values over [start, start +
/// span], one run per stretch of known values, broken where a value is
/// unknown or samples are missing
#[allow(clippy::too_many_arguments)]
pub fn time_points(
    times: &[f64],
    values: &[Option<f64>],
    start: f64,
    span: f64,
    lo: f64,
    hi: f64,
    width: f64,
    height: f64,
) -> Vec<Vec<Pt>> {
    let mut runs = Vec::new();
    let mut cur: Vec<Pt> = Vec::new();
    let range = if hi - lo != 0.0 { hi - lo } else { 1.0 };
    let span = if span != 0.0 { span } else { 1.0 };
    let flush = |cur: &mut Vec<Pt>, runs: &mut Vec<Vec<Pt>>| {
        if !cur.is_empty() {
            runs.push(std::mem::take(cur));
        }
    };
    for (i, v) in values.iter().enumerate() {
        let Some(&time) = times.get(i) else { break };
        if i > 0 && time - times[i - 1] > GAP_MS as f64 {
            flush(&mut cur, &mut runs);
        }
        match v {
            Some(v) if v.is_finite() && time >= start => {
                let x = ((time - start) / span) * width;
                let y = height - ((v.max(lo).min(hi) - lo) / range) * height;
                cur.push((x, y));
            }
            _ => flush(&mut cur, &mut runs),
        }
    }
    flush(&mut cur, &mut runs);
    runs
}

/// The height of `v` on a scale of [lo, hi] drawn `height` tall
pub fn y_of(v: f64, lo: f64, hi: f64, height: f64) -> f64 {
    let range = if hi - lo != 0.0 { hi - lo } else { 1.0 };
    height - ((v.max(lo).min(hi) - lo) / range) * height
}

/// The time axis of a chart group: [end - span, end], `end` the latest
/// sample (the span itself before there is one)
pub fn group_range(end: Option<f64>, span: f64) -> (f64, f64) {
    let end = end.unwrap_or(span);
    (end - span, end)
}

/// The time axis of a chart outside a group: its own samples
pub fn own_range(times: &[f64]) -> (f64, f64) {
    match (times.first(), times.last()) {
        (Some(&a), Some(&b)) => (a, b.max(a + 1.0)),
        _ => (0.0, 1.0),
    }
}

/// The time axis' left label: "−60 s"
pub fn axis_start_text(span_ms: f64) -> String {
    format!("\u{2212}{} s", crate::series::js_round(span_ms / 1000.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::series::time_runs;

    fn svg(runs: &[Vec<Pt>]) -> Vec<String> {
        runs.iter()
            .map(|r| {
                r.iter()
                    .map(|(x, y)| format!("{},{}", crate::series::to_fixed(*x, 2), crate::series::to_fixed(*y, 2)))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect()
    }

    #[test]
    fn time_points_are_the_series_polylines() {
        let times = [0.0, 250.0, 500.0, 3000.0, 3250.0];
        let values = [Some(0.0), Some(1.0), None, Some(1.0), Some(0.0)];
        let p = time_points(&times, &values, 0.0, 4000.0, 0.0, 1.0, 400.0, 100.0);
        assert_eq!(svg(&p), time_runs(&times, &values, 0.0, 4000.0, 0.0, 1.0, 400.0, 100.0));
        assert_eq!(p.len(), 2);
        // Samples before the axis starts are left out
        let p = time_points(&[0.0, 250.0], &[Some(1.0), Some(1.0)], 100.0, 1000.0, 0.0, 1.0, 100.0, 10.0);
        assert_eq!(svg(&p), ["15.00,0.00"]);
    }

    #[test]
    fn sparkline_points_spread_evenly_and_break_at_unknown_values() {
        let p = points(&[Some(0.0), Some(1.0), None, Some(2.0)], 0.0, 2.0, 30.0, 10.0);
        assert_eq!(p, vec![vec![(0.0, 10.0), (10.0, 5.0)], vec![(30.0, 0.0)]]);
        assert_eq!(bounds(&[Some(3.0), Some(3.0)], None, None), (3.0, 4.0));
        assert_eq!(bounds(&[], None, None), (0.0, 1.0));
        assert_eq!(bounds(&[Some(5.0)], Some(0.0), Some(10.0)), (0.0, 10.0));
    }

    #[test]
    fn axes() {
        assert_eq!(group_range(Some(70_000.0), 60_000.0), (10_000.0, 70_000.0));
        assert_eq!(group_range(None, 60_000.0), (0.0, 60_000.0));
        assert_eq!(own_range(&[5.0, 5.0]), (5.0, 6.0));
        assert_eq!(own_range(&[]), (0.0, 1.0));
        assert_eq!(axis_start_text(60_000.0), "\u{2212}60 s");
        assert_eq!(y_of(150.0, 0.0, 100.0, 96.0), 0.0);
        assert_eq!(y_of(50.0, 0.0, 100.0, 96.0), 48.0);
    }
}
