//! Sample format conversion and downmix, as v1's encode thread did them:
//! float is clamped to [-1, 1] and scaled (truncating like a C cast), 16-bit
//! feeding a 24-bit encoder is shifted to the top of 32 bits, and channels
//! beyond the ones sent are dropped (stereo from multichannel = front L/R).

/// Sample type of captured audio
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleType {
    F32,
    I16,
    /// 32-bit signed (24-bit audio MSB-aligned)
    I32,
}

impl SampleType {
    pub fn bytes(self) -> usize {
        match self {
            SampleType::I16 => 2,
            SampleType::F32 | SampleType::I32 => 4,
        }
    }
}

#[inline]
fn f32_to_i16(s: f32) -> i16 {
    (s.clamp(-1.0, 1.0) * 32767.0) as i16
}

#[inline]
fn f32_to_i32(s: f32) -> i32 {
    (s.clamp(-1.0, 1.0) as f64 * 2147483647.0) as i32
}

#[inline]
fn sample(raw: &[u8], i: usize, t: SampleType) -> [u8; 4] {
    let b = t.bytes();
    let mut s = [0u8; 4];
    s[..b].copy_from_slice(&raw[i * b..i * b + b]);
    s
}

/// Appends `raw` (interleaved, `in_ch` channels of `t`) to `out` as 16-bit
/// with `out_ch` channels. `out` must have the capacity (no allocation).
pub fn append_i16(raw: &[u8], t: SampleType, in_ch: usize, out_ch: usize, out: &mut Vec<i16>) {
    let frames = raw.len() / (in_ch * t.bytes());
    let use_ch = out_ch.min(in_ch);
    for f in 0..frames {
        for c in 0..out_ch {
            // Mono input sent as stereo: the one channel on both
            let i = f * in_ch + c.min(use_ch - 1);
            let s = sample(raw, i, t);
            out.push(match t {
                SampleType::F32 => f32_to_i16(f32::from_le_bytes(s)),
                SampleType::I16 => i16::from_le_bytes([s[0], s[1]]),
                SampleType::I32 => (i32::from_le_bytes(s) >> 16) as i16,
            });
        }
    }
}

/// As [`append_i16`], to 32-bit MSB-aligned
pub fn append_i32(raw: &[u8], t: SampleType, in_ch: usize, out_ch: usize, out: &mut Vec<i32>) {
    let frames = raw.len() / (in_ch * t.bytes());
    let use_ch = out_ch.min(in_ch);
    for f in 0..frames {
        for c in 0..out_ch {
            let i = f * in_ch + c.min(use_ch - 1);
            let s = sample(raw, i, t);
            out.push(match t {
                SampleType::F32 => f32_to_i32(f32::from_le_bytes(s)),
                SampleType::I16 => i16::from_le_bytes([s[0], s[1]]) as i32 * 65536,
                SampleType::I32 => i32::from_le_bytes(s),
            });
        }
    }
}

/// `raw` (interleaved, `in_ch` channels of `t`) as float with `out_ch`
/// channels, the channels picked as [`append_i16`] does, into `out` (one
/// value per sample sent; frames beyond its length are left out). For the
/// playback EQ, which works on float.
pub fn to_f32(raw: &[u8], t: SampleType, in_ch: usize, out_ch: usize, out: &mut [f32]) {
    let frames = (raw.len() / (in_ch * t.bytes())).min(out.len() / out_ch);
    let use_ch = out_ch.min(in_ch);
    for f in 0..frames {
        for c in 0..out_ch {
            let i = f * in_ch + c.min(use_ch - 1);
            let s = sample(raw, i, t);
            out[f * out_ch + c] = match t {
                SampleType::F32 => f32::from_le_bytes(s),
                SampleType::I16 => f32::from(i16::from_le_bytes([s[0], s[1]])) / 32768.0,
                SampleType::I32 => (f64::from(i32::from_le_bytes(s)) / 2_147_483_648.0) as f32,
            };
        }
    }
}

/// Float samples appended as 16-bit, as float input is (clamped, truncated)
pub fn append_f32_as_i16(src: &[f32], out: &mut Vec<i16>) {
    out.extend(src.iter().map(|&s| f32_to_i16(s)));
}

/// Float samples appended as 32-bit MSB-aligned, as float input is
pub fn append_f32_as_i32(src: &[f32], out: &mut Vec<i32>) {
    out.extend(src.iter().map(|&s| f32_to_i32(s)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_path_matches_the_direct_one_for_float_input() {
        let v = [0.5f32, -0.25, 1.5, -0.001, 0.75, 0.1];
        let raw = f32_bytes(&v);
        let mut direct = Vec::with_capacity(6);
        append_i16(&raw, SampleType::F32, 2, 2, &mut direct);
        let mut f = [0.0f32; 6];
        to_f32(&raw, SampleType::F32, 2, 2, &mut f);
        let mut via = Vec::with_capacity(6);
        append_f32_as_i16(&f, &mut via);
        assert_eq!(direct, via);
        // Mono doubled, and integers scaled to [-1, 1)
        let mono: Vec<u8> = [16384i16, -32768].iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut f = [0.0f32; 4];
        to_f32(&mono, SampleType::I16, 1, 2, &mut f);
        assert_eq!(f, [0.5, 0.5, -1.0, -1.0]);
    }

    fn f32_bytes(v: &[f32]) -> Vec<u8> {
        v.iter().flat_map(|s| s.to_le_bytes()).collect()
    }

    #[test]
    fn float_to_16_clamps_and_truncates() {
        let mut out = Vec::with_capacity(8);
        append_i16(&f32_bytes(&[0.5, -0.5, 2.0, -2.0]), SampleType::F32, 2, 2, &mut out);
        assert_eq!(out, vec![16383, -16383, 32767, -32767]);
    }

    #[test]
    fn multichannel_keeps_front_pair_and_mono_doubles() {
        let mut out = Vec::with_capacity(8);
        append_i16(&f32_bytes(&[0.5, -0.5, 0.25, 0.1]), SampleType::F32, 4, 2, &mut out);
        assert_eq!(out, vec![16383, -16383]);
        out.clear();
        let mono: Vec<u8> = [1000i16, 2000].iter().flat_map(|s| s.to_le_bytes()).collect();
        append_i16(&mono, SampleType::I16, 1, 2, &mut out);
        assert_eq!(out, vec![1000, 1000, 2000, 2000]);
    }

    #[test]
    fn sixteen_to_32_is_msb_aligned() {
        let mut out = Vec::with_capacity(2);
        let raw: Vec<u8> = [-1i16, 1].iter().flat_map(|s| s.to_le_bytes()).collect();
        append_i32(&raw, SampleType::I16, 2, 2, &mut out);
        assert_eq!(out, vec![-65536, 65536]);
    }
}
