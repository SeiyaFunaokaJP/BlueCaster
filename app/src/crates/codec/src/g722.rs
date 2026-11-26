//! G.722 encoder at 64 kbit/s (16 kHz mono in, one byte per two samples
//! out): the codec of ASHA hearing aids.
//!
//! A Rust port of the public-domain G.722 encoder of SpanDSP (Steve
//! Underwood, 2005, based on the CMU code of Chengxiang Lu and Alex
//! Hauptmann, 1993), as it is used for ASHA elsewhere. The names of the
//! steps ("Block 4, UPPOL2", ...) are those of ITU-T G.722.
//!
//! The decoder here ([`Decoder`]) exists for the tests and the emulator
//! checks: it turns the bytes back into 16 kHz PCM.

fn saturate(v: i32) -> i32 {
    v.clamp(-32768, 32767)
}

/// The adaptive predictor of one sub-band
#[derive(Debug, Clone, Default)]
struct Band {
    s: i32,
    sp: i32,
    sz: i32,
    r: [i32; 3],
    a: [i32; 3],
    ap: [i32; 3],
    p: [i32; 3],
    d: [i32; 7],
    b: [i32; 7],
    bp: [i32; 7],
    nb: i32,
    det: i32,
}

impl Band {
    /// Block 4: reconstruction, pole and zero predictor adaptation
    fn block4(&mut self, d: i32) {
        // RECONS, PARREC
        self.d[0] = d;
        self.r[0] = saturate(self.s + d);
        self.p[0] = saturate(self.sz + d);

        // UPPOL2
        let sg = [self.p[0] >> 15, self.p[1] >> 15, self.p[2] >> 15];
        let wd1 = saturate(self.a[1] << 2);
        let wd2 = if sg[0] == sg[1] { -wd1 } else { wd1 }.min(32767);
        let mut ap2 = (wd2 >> 7) + if sg[0] == sg[2] { 128 } else { -128 };
        ap2 += (self.a[2] * 32512) >> 15;
        self.ap[2] = ap2.clamp(-12288, 12288);

        // UPPOL1
        let wd1 = if sg[0] == sg[1] { 192 } else { -192 };
        let wd2 = (self.a[1] * 32640) >> 15;
        let wd3 = saturate(15360 - self.ap[2]);
        self.ap[1] = saturate(wd1 + wd2).clamp(-wd3, wd3);

        // UPZERO, FILTEZ
        let wd1 = if d == 0 { 0 } else { 128 };
        let sg0 = d >> 15;
        for i in 1..7 {
            let wd2 = if self.d[i] >> 15 == sg0 { wd1 } else { -wd1 };
            let wd3 = (self.b[i] * 32640) >> 15;
            self.bp[i] = saturate(wd2 + wd3);
        }

        // DELAYA
        let mut sz = 0;
        for i in (1..7).rev() {
            self.d[i] = self.d[i - 1];
            self.b[i] = self.bp[i];
            sz += (self.b[i] * saturate(self.d[i] + self.d[i])) >> 15;
        }
        self.sz = sz;
        for i in (1..3).rev() {
            self.r[i] = self.r[i - 1];
            self.p[i] = self.p[i - 1];
            self.a[i] = self.ap[i];
        }

        // FILTEP, PREDIC
        let wd1 = (self.a[1] * saturate(self.r[1] + self.r[1])) >> 15;
        let wd2 = (self.a[2] * saturate(self.r[2] + self.r[2])) >> 15;
        self.sp = saturate(wd1 + wd2);
        self.s = saturate(self.sp + self.sz);
    }

    /// Blocks 3L / 3H: the quantizer scale factor from the logarithmic one
    fn scale(&mut self, shift: i32) {
        let wd1 = ((self.nb >> 6) & 31) as usize;
        let wd2 = shift - (self.nb >> 11);
        let wd3 = if wd2 < 0 { ILB[wd1] << -wd2 } else { ILB[wd1] >> wd2 };
        self.det = wd3 << 2;
    }
}

const Q6: [i32; 32] = [
    0, 35, 72, 110, 150, 190, 233, 276, 323, 370, 422, 473, 530, 587, 650, 714, 786, 858, 940, 1023, 1121, 1219, 1339,
    1458, 1612, 1765, 1980, 2195, 2557, 2919, 0, 0,
];
const ILN: [i32; 32] = [
    0, 63, 62, 31, 30, 29, 28, 27, 26, 25, 24, 23, 22, 21, 20, 19, 18, 17, 16, 15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5,
    4, 0,
];
const ILP: [i32; 32] = [
    0, 61, 60, 59, 58, 57, 56, 55, 54, 53, 52, 51, 50, 49, 48, 47, 46, 45, 44, 43, 42, 41, 40, 39, 38, 37, 36, 35, 34,
    33, 32, 0,
];
const WL: [i32; 8] = [-60, -30, 58, 172, 334, 538, 1198, 3042];
const RL42: [usize; 16] = [0, 7, 6, 5, 4, 3, 2, 1, 7, 6, 5, 4, 3, 2, 1, 0];
const ILB: [i32; 32] = [
    2048, 2093, 2139, 2186, 2233, 2282, 2332, 2383, 2435, 2489, 2543, 2599, 2656, 2714, 2774, 2834, 2896, 2960, 3025,
    3091, 3158, 3228, 3298, 3371, 3444, 3520, 3597, 3676, 3756, 3838, 3922, 4008,
];
const QM4: [i32; 16] =
    [0, -20456, -12896, -8968, -6288, -4240, -2584, -1200, 20456, 12896, 8968, 6288, 4240, 2584, 1200, 0];
const QM2: [i32; 4] = [-7408, -1616, 7408, 1616];
const QMF: [i32; 12] = [3, -11, 12, 32, -210, 951, 3876, -805, 362, -156, 53, -11];
const IHN: [usize; 3] = [0, 1, 0];
const IHP: [usize; 3] = [0, 3, 2];
const WH: [i32; 3] = [0, -214, 798];
const RH2: [usize; 4] = [2, 1, 2, 1];

/// Lower band: log scale factor and predictor from the 4 top bits of the code
fn lower_step(band: &mut Band, ril: usize) {
    let dlow = (band.det * QM4[ril]) >> 15;
    // Block 3L, LOGSCL / SCALEL
    band.nb = (((band.nb * 127) >> 7) + WL[RL42[ril]]).clamp(0, 18432);
    band.scale(8);
    band.block4(dlow);
}

/// Higher band, from its 2 bits
fn higher_step(band: &mut Band, ihigh: usize) {
    let dhigh = (band.det * QM2[ihigh]) >> 15;
    // Block 3H, LOGSCH / SCALEH
    band.nb = (((band.nb * 127) >> 7) + WH[RH2[ihigh]]).clamp(0, 22528);
    band.scale(10);
    band.block4(dhigh);
}

/// G.722 encoder, 64 kbit/s
#[derive(Debug, Clone)]
pub struct Encoder {
    band: [Band; 2],
    /// The transmit QMF's signal history
    x: [i32; 24],
}

impl Default for Encoder {
    fn default() -> Self {
        Encoder::new()
    }
}

impl Encoder {
    pub fn new() -> Encoder {
        let mut band = [Band::default(), Band::default()];
        band[0].det = 32;
        band[1].det = 8;
        Encoder { band, x: [0; 24] }
    }

    /// Encodes pairs of 16 kHz samples into one byte each: `out` takes
    /// `pcm.len() / 2` bytes (an odd last sample is left out). Returns the
    /// number of bytes written.
    pub fn encode(&mut self, pcm: &[i16], out: &mut [u8]) -> usize {
        let mut n = 0;
        for (pair, byte) in pcm.as_chunks::<2>().0.iter().zip(out.iter_mut()) {
            // The transmit QMF: every other output is kept
            self.x.copy_within(2.., 0);
            self.x[22] = i32::from(pair[0]);
            self.x[23] = i32::from(pair[1]);
            let (mut sumeven, mut sumodd) = (0i32, 0i32);
            for i in 0..12 {
                sumodd += self.x[2 * i] * QMF[i];
                sumeven += self.x[2 * i + 1] * QMF[11 - i];
            }
            // 12 bits for the filters' gain, 1 for the sum of two filters,
            // 1 for G.722's 15-bit input
            let xlow = (sumeven + sumodd) >> 14;
            let xhigh = (sumeven - sumodd) >> 14;

            // Block 1L, SUBTRA / QUANTL
            let low = &mut self.band[0];
            let el = saturate(xlow - low.s);
            let wd = if el >= 0 { el } else { -(el + 1) };
            let mut i = 1;
            while i < 30 && wd >= (Q6[i] * low.det) >> 12 {
                i += 1;
            }
            let ilow = if el < 0 { ILN[i] } else { ILP[i] };
            lower_step(low, (ilow >> 2) as usize);

            // Block 1H, SUBTRA / QUANTH
            let high = &mut self.band[1];
            let eh = saturate(xhigh - high.s);
            let wd = if eh >= 0 { eh } else { -(eh + 1) };
            let mih = if wd >= (564 * high.det) >> 12 { 2 } else { 1 };
            let ihigh = if eh < 0 { IHN[mih] } else { IHP[mih] };
            higher_step(high, ihigh);

            *byte = ((ihigh as i32) << 6 | ilow) as u8;
            n += 1;
        }
        n
    }
}

/// G.722 decoder, 64 kbit/s. The lower band is rebuilt from the 4 top bits
/// of its code (the quantizer the encoder's predictor uses too), not all 6:
/// a little noisier than a full decoder, and enough to check what was sent.
#[derive(Debug, Clone)]
pub struct Decoder {
    band: [Band; 2],
    /// The receive QMF's signal history
    x: [i32; 24],
}

impl Default for Decoder {
    fn default() -> Self {
        Decoder::new()
    }
}

impl Decoder {
    pub fn new() -> Decoder {
        let mut band = [Band::default(), Band::default()];
        band[0].det = 32;
        band[1].det = 8;
        Decoder { band, x: [0; 24] }
    }

    /// Decodes each byte into two 16 kHz samples, appended to `pcm`
    pub fn decode(&mut self, data: &[u8], pcm: &mut Vec<i16>) {
        for &code in data {
            let ril = usize::from((code >> 2) & 0x0F);
            let ihigh = usize::from(code >> 6);
            // Blocks 5L / 6L and 5H / 6H: the reconstructed sub-band signals
            let low = &mut self.band[0];
            let rlow = (low.s + ((low.det * QM4[ril]) >> 15)).clamp(-16384, 16383);
            lower_step(low, ril);
            let high = &mut self.band[1];
            let rhigh = (high.s + ((high.det * QM2[ihigh]) >> 15)).clamp(-16384, 16383);
            higher_step(high, ihigh);

            // The receive QMF
            self.x.copy_within(2.., 0);
            self.x[22] = rlow + rhigh;
            self.x[23] = rlow - rhigh;
            let (mut xout1, mut xout2) = (0i32, 0i32);
            for i in 0..12 {
                xout2 += self.x[2 * i] * QMF[i];
                xout1 += self.x[2 * i + 1] * QMF[11 - i];
            }
            pcm.push(saturate(xout1 >> 11) as i16);
            pcm.push(saturate(xout2 >> 11) as i16);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(hz: f64, amplitude: f64, samples: usize) -> Vec<i16> {
        (0..samples)
            .map(|n| (amplitude * (2.0 * std::f64::consts::PI * hz * n as f64 / 16000.0).sin()) as i16)
            .collect()
    }

    /// Signal to noise ratio in dB of `out` against `reference` delayed by
    /// the best of the first `max_delay` samples
    fn snr(reference: &[i16], out: &[i16], max_delay: usize) -> f64 {
        (0..max_delay)
            .map(|delay| {
                let (mut signal, mut noise) = (0f64, 0f64);
                // After the predictors settled
                for n in 2000..reference.len() - max_delay {
                    let (a, b) = (f64::from(reference[n]), f64::from(out[n + delay]));
                    signal += a * a;
                    noise += (a - b) * (a - b);
                }
                10.0 * (signal / noise.max(1.0)).log10()
            })
            .fold(f64::MIN, f64::max)
    }

    #[test]
    fn one_byte_per_two_samples() {
        let mut out = [0u8; 160];
        assert_eq!(Encoder::new().encode(&[0i16; 320], &mut out), 160);
        assert_eq!(Encoder::new().encode(&[0i16; 321], &mut out), 160);
        assert_eq!(Encoder::new().encode(&[0i16; 320], &mut out[..10]), 10);
    }

    #[test]
    fn silence_is_the_idle_pattern() {
        // No signal: the smallest step of each band, alternating in sign in
        // the lower one once the predictors are at rest
        let mut out = [0u8; 160];
        Encoder::new().encode(&[0i16; 320], &mut out);
        assert!(out[80..].iter().all(|&b| b >> 6 == 3 || b >> 6 == 1), "{:02x?}", &out[80..90]);
    }

    #[test]
    fn a_tone_survives_the_round_trip() {
        for hz in [300.0, 1000.0, 3000.0, 6000.0] {
            let pcm = sine(hz, 8000.0, 16000);
            let mut coded = vec![0u8; pcm.len() / 2];
            assert_eq!(Encoder::new().encode(&pcm, &mut coded), coded.len());
            let mut out = Vec::new();
            Decoder::new().decode(&coded, &mut out);
            assert_eq!(out.len(), pcm.len());
            let ratio = snr(&pcm, &out, 40);
            assert!(ratio > 15.0, "{hz} Hz: {ratio:.1} dB");
        }
    }

    #[test]
    fn encoding_in_pieces_gives_the_same_bytes() {
        let pcm = sine(440.0, 12000.0, 3200);
        let mut whole = vec![0u8; 1600];
        Encoder::new().encode(&pcm, &mut whole);
        let mut pieces = vec![0u8; 1600];
        let mut encoder = Encoder::new();
        for (p, o) in pcm.chunks(320).zip(pieces.chunks_mut(160)) {
            encoder.encode(p, o);
        }
        assert_eq!(whole, pieces);
    }
}
