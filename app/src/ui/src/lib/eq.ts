// The playback EQ's response curve, computed as the core's filters are
// (dsp::biquad: RBJ "Audio EQ Cookbook" biquads), and the editing rules of
// its points (the ranges the core holds them to, rounding, defaults).

import type { EqBandType, EqPoint } from "../ipc/types";

/** Most points (dsp::MAX_EQ_POINTS) */
export const MAX_POINTS = 16;
export const FREQ_MIN = 20;
export const FREQ_MAX = 20000;
export const GAIN_LIMIT = 12;
export const Q_MIN = 0.1;
export const Q_MAX = 12;
/** The rate the graph is drawn for (the curve differs only near the top) */
export const GRAPH_RATE = 48000;

export const BAND_TYPES: readonly EqBandType[] = ["Peaking", "LowShelf", "HighShelf", "HighPass", "LowPass"];

/** Whether a point of this type has a gain (the pass filters do not) */
export function hasGain(t: EqBandType): boolean {
  return t !== "HighPass" && t !== "LowPass";
}

/** Whether a point of this type has a Q (the shelves do not) */
export function hasQ(t: EqBandType): boolean {
  return t !== "LowShelf" && t !== "HighShelf";
}

interface Coeffs {
  b0: number;
  b1: number;
  b2: number;
  a1: number;
  a2: number;
}

function normalized(b0: number, b1: number, b2: number, a0: number, a1: number, a2: number): Coeffs {
  return { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0 };
}

function peaking(rate: number, f: number, q: number, gain: number): Coeffs {
  const w0 = (2 * Math.PI * f) / rate;
  const [sin, cos] = [Math.sin(w0), Math.cos(w0)];
  const alpha = sin / (2 * q);
  const a = Math.pow(10, gain / 40);
  return normalized(1 + alpha * a, -2 * cos, 1 - alpha * a, 1 + alpha / a, -2 * cos, 1 - alpha / a);
}

function shelf(rate: number, f: number, gain: number, high: boolean): Coeffs {
  const w0 = (2 * Math.PI * f) / rate;
  const [sin, cos] = [Math.sin(w0), Math.cos(w0)];
  const a = Math.pow(10, gain / 40);
  // Shelf slope S = 1, as the core's shelves always use
  const slope = 1;
  const alpha = (sin / 2) * Math.sqrt((a + 1 / a) * (1 / slope - 1) + 2);
  const sa = 2 * Math.sqrt(a) * alpha;
  if (!high) {
    return normalized(
      a * (a + 1 - (a - 1) * cos + sa),
      2 * a * (a - 1 - (a + 1) * cos),
      a * (a + 1 - (a - 1) * cos - sa),
      a + 1 + (a - 1) * cos + sa,
      -2 * (a - 1 + (a + 1) * cos),
      a + 1 + (a - 1) * cos - sa,
    );
  }
  return normalized(
    a * (a + 1 + (a - 1) * cos + sa),
    -2 * a * (a - 1 + (a + 1) * cos),
    a * (a + 1 + (a - 1) * cos - sa),
    a + 1 - (a - 1) * cos + sa,
    2 * (a - 1 - (a + 1) * cos),
    a + 1 - (a - 1) * cos - sa,
  );
}

function pass(rate: number, f: number, q: number, high: boolean): Coeffs {
  const w0 = (2 * Math.PI * f) / rate;
  const [sin, cos] = [Math.sin(w0), Math.cos(w0)];
  const alpha = sin / (2 * q);
  return high
    ? normalized((1 + cos) / 2, -(1 + cos), (1 + cos) / 2, 1 + alpha, -2 * cos, 1 - alpha)
    : normalized((1 - cos) / 2, 1 - cos, (1 - cos) / 2, 1 + alpha, -2 * cos, 1 - alpha);
}

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

/** A point's filter, or null when it changes nothing (as Coeffs::for_band) */
function forBand(b: EqPoint, rate: number): Coeffs | null {
  const f = clamp(b.frequency, 10, (rate / 2) * 0.98);
  const q = clamp(b.q, 0.05, 30);
  switch (b.band_type) {
    case "HighPass":
      return pass(rate, f, q, true);
    case "LowPass":
      return pass(rate, f, q, false);
  }
  if (Math.abs(b.gain) < 0.05) return null;
  switch (b.band_type) {
    case "LowShelf":
      return shelf(rate, f, b.gain, false);
    case "HighShelf":
      return shelf(rate, f, b.gain, true);
    default:
      return peaking(rate, f, q, b.gain);
  }
}

function responseDb(c: Coeffs, f: number, rate: number): number {
  const w = (2 * Math.PI * f) / rate;
  const [s1, c1, s2, c2] = [Math.sin(w), Math.cos(w), Math.sin(2 * w), Math.cos(2 * w)];
  const nr = c.b0 + c.b1 * c1 + c.b2 * c2;
  const ni = -(c.b1 * s1 + c.b2 * s2);
  const dr = 1 + c.a1 * c1 + c.a2 * c2;
  const di = -(c.a1 * s1 + c.a2 * s2);
  const num = nr * nr + ni * ni;
  const den = Math.max(dr * dr + di * di, 1e-30);
  return 10 * Math.log10(Math.max(num / den, 1e-30));
}

/** The response of all points at `f` (dB), as dsp::biquad::eq_response_db */
export function eqResponseDb(bands: readonly EqPoint[], f: number, rate = GRAPH_RATE): number {
  let sum = 0;
  for (const b of bands) {
    const c = forBand(b, rate);
    if (c) sum += responseDb(c, f, rate);
  }
  return sum;
}

/** The curve at `n` + 1 log-spaced frequencies from FREQ_MIN to FREQ_MAX */
export function responseCurve(bands: readonly EqPoint[], n: number, rate = GRAPH_RATE): { f: number; db: number }[] {
  const coeffs = bands.map((b) => forBand(b, rate)).filter((c): c is Coeffs => c !== null);
  const out: { f: number; db: number }[] = [];
  for (let i = 0; i <= n; i++) {
    const f = FREQ_MIN * Math.pow(FREQ_MAX / FREQ_MIN, i / n);
    out.push({ f, db: coeffs.reduce((s, c) => s + responseDb(c, f, rate), 0) });
  }
  return out;
}

/** A frequency rounded as dragging sets it (1 Hz, 5 Hz, 50 Hz steps) */
export function roundFreq(f: number): number {
  const v = clamp(f, FREQ_MIN, FREQ_MAX);
  if (v < 100) return Math.round(v);
  if (v < 1000) return Math.round(v / 5) * 5;
  return Math.round(v / 50) * 50;
}

export const roundGain = (g: number) => clamp(Math.round(g * 10) / 10, -GAIN_LIMIT, GAIN_LIMIT);
export const roundQ = (q: number) => clamp(Math.round(q * 100) / 100, Q_MIN, Q_MAX);

/** A point held to the ranges the core plays it with */
export function heldPoint(b: EqPoint): EqPoint {
  return {
    frequency: clamp(Number.isFinite(b.frequency) ? b.frequency : 1000, FREQ_MIN, FREQ_MAX),
    gain: hasGain(b.band_type) ? roundGain(Number.isFinite(b.gain) ? b.gain : 0) : 0,
    q: roundQ(Number.isFinite(b.q) ? b.q : 1),
    band_type: b.band_type,
  };
}

/** A point with another type: a pass filter has no gain and a sensible Q */
export function withType(b: EqPoint, t: EqBandType): EqPoint {
  const out = { ...b, band_type: t };
  if (!hasGain(t)) {
    out.gain = 0;
    if (out.q < 0.3) out.q = 0.71;
  }
  return out;
}

/** "1.25 kHz", "80 Hz" */
export function formatFreq(f: number): string {
  if (f >= 1000) {
    const k = f / 1000;
    return `${k >= 10 ? k.toFixed(1) : k.toFixed(2)} kHz`;
  }
  return `${Math.round(f)} Hz`;
}

/** "+3.0 dB" */
export function formatGain(g: number): string {
  return `${g > 0 ? "+" : g < 0 ? "−" : ""}${Math.abs(g).toFixed(1)} dB`;
}

/** The highest boost of the response (dB, 0 when it only cuts), as the
 *  core's preamp takes it (pipeline::eq::max_boost_db): scanned over the
 *  audible range and at each point's own frequency */
export function maxBoostDb(bands: readonly EqPoint[], rate = GRAPH_RATE): number {
  const coeffs = bands.map((b) => forBand(b, rate)).filter((c): c is Coeffs => c !== null);
  const response = (f: number) => coeffs.reduce((s, c) => s + responseDb(c, f, rate), 0);
  const top = Math.min(FREQ_MAX, rate * 0.49);
  const scan = 192;
  let max = 0;
  for (let i = 0; i < scan; i++) max = Math.max(max, response(FREQ_MIN * Math.pow(top / FREQ_MIN, i / (scan - 1))));
  for (const b of bands) if (b.frequency < rate * 0.49) max = Math.max(max, response(b.frequency));
  return max;
}

/** Equalizer APO's names of the filters (as AutoEq writes them): the
 *  shelves are the cookbook's with the core's slope 1, which is Q 1/√2 at
 *  the same frequency (LSC/HSC), the pass filters take their Q (HPQ/LPQ) */
const APO_TYPE: Record<EqBandType, string> = {
  Peaking: "PK",
  LowShelf: "LSC",
  HighShelf: "HSC",
  HighPass: "HPQ",
  LowPass: "LPQ",
};
const SHELF_Q = Math.SQRT1_2;

/** The points as an Equalizer APO / AutoEq parametric EQ: the preamp the
 *  core plays them with, then one line per point */
export function eqApoText(bands: readonly EqPoint[]): string {
  const num = (v: number, digits: number) => {
    const s = v.toFixed(digits);
    return Number(s) === 0 ? (0).toFixed(digits) : s;
  };
  const fc = (f: number) => (Number.isInteger(f) ? String(f) : num(f, 1));
  // Rounded up: the preamp never lets the loudest part go over
  const preamp = -Math.ceil(maxBoostDb(bands) * 10 - 1e-6) / 10;
  const lines = [`Preamp: ${num(preamp, 1)} dB`];
  bands.forEach((b, i) => {
    const parts = [`Filter ${i + 1}: ON ${APO_TYPE[b.band_type]} Fc ${fc(b.frequency)} Hz`];
    if (hasGain(b.band_type)) parts.push(`Gain ${num(b.gain, 1)} dB`);
    parts.push(`Q ${num(hasQ(b.band_type) ? b.q : SHELF_Q, 2)}`);
    lines.push(parts.join(" "));
  });
  return lines.join("\r\n") + "\r\n";
}

/** The points a new EQ starts with, and Reset puts back: the classic ten
 *  bands an octave apart, flat (the core skips a 0 dB peak, so they change
 *  nothing and cost nothing until one is moved) */
export const DEFAULT_POINTS: readonly EqPoint[] = [31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000].map(
  (frequency) => ({ frequency, gain: 0, q: 1.41, band_type: "Peaking" }),
);

/** Whether `bands` are the default points as they come */
export function isDefaultPoints(bands: readonly EqPoint[]): boolean {
  return JSON.stringify(bands) === JSON.stringify(DEFAULT_POINTS);
}
