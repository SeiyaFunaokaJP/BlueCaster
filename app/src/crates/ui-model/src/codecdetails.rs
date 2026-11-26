//! What Diagnostics' codec details card shows: each codec setting with what
//! the device offered, the SBC bitpool bar's scale and how full a packet is.

use ipc::{CodecDetails, StreamInfo};

pub use ipc::{AacDetails, SbcDetails};

use crate::i18n::js_number;

/// "used": what the stream runs with; "offered": the device takes it too;
/// "off": the device does not take it
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChipState {
    Used,
    Offered,
    Off,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chip<T> {
    pub value: T,
    pub state: ChipState,
}

/// Every value of a setting with its state, in the order of `all`. Without
/// what the device offered (it did not say), only the value in use.
pub fn chips<T: PartialEq + Clone>(all: &[T], offered: Option<&[T]>, used: T) -> Vec<Chip<T>> {
    let Some(offered) = offered else { return vec![Chip { value: used, state: ChipState::Used }] };
    let mut list = all.to_vec();
    if !all.contains(&used) {
        list.push(used.clone());
    }
    list.into_iter()
        .map(|value| {
            let state = if value == used {
                ChipState::Used
            } else if offered.contains(&value) {
                ChipState::Offered
            } else {
                ChipState::Off
            };
            Chip { value, state }
        })
        .collect()
}

// The values of the SBC codec information (A2DP 4.3.2)
pub const SBC_RATES: [u32; 4] = [16000, 32000, 44100, 48000];
pub const SBC_MODES: [&str; 4] = ["mono", "dual_channel", "stereo", "joint_stereo"];
pub const SBC_BLOCKS: [u32; 4] = [4, 8, 12, 16];
pub const SBC_SUBBANDS: [u32; 2] = [4, 8];
pub const SBC_ALLOCATIONS: [&str; 2] = ["loudness", "snr"];
pub const AAC_CHANNELS: [u32; 2] = [1, 2];

/// A range on the bitpool bar, in % of the bar
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BarSpan {
    pub from: f64,
    pub width: f64,
}

/// The bitpool bar: its end (64, or the largest value shown rounded up to
/// 16) and where each range and the bitpool in use sit, in % of the bar
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BitpoolBar {
    pub max: u32,
    pub device: Option<BarSpan>,
    pub config: Option<BarSpan>,
    /// SBC ABR: the range the bitpool moves in
    pub abr: Option<BarSpan>,
    pub used: f64,
}

pub fn bitpool_bar(s: &SbcDetails) -> BitpoolBar {
    let top = 64
        .max(s.device.as_ref().map_or(0, |d| d.max_bitpool))
        .max(s.config_bitpool.map_or(0, |c| c.1))
        .max(s.abr_bitpool.map_or(0, |c| c.1))
        .max(s.bitpool);
    let max = top.div_ceil(16) * 16;
    let pct = |v: u32| f64::from(v.min(max)) / f64::from(max) * 100.0;
    let span = |lo: u32, hi: u32| BarSpan { from: pct(lo), width: (pct(hi) - pct(lo)).max(0.0) };
    BitpoolBar {
        max,
        device: s.device.as_ref().map(|d| span(d.min_bitpool, d.max_bitpool)),
        config: s.config_bitpool.map(|c| span(c.0, c.1)),
        abr: s.abr_bitpool.map(|c| span(c.0, c.1)),
        used: pct(s.bitpool),
    }
}

/// The bitpool now and the range it moves in
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveBitpool {
    pub now: u32,
    pub min: u32,
    pub max: u32,
}

/// SBC / SBC XQ under ABR: the bitpool in use now and the range it moves in
/// (floor .. the bitpool the stream started with). The core sends a new
/// StreamInfo while it changes. None without SBC ABR (a fixed bitpool, or
/// another codec).
pub fn live_bitpool(info: &StreamInfo) -> Option<LiveBitpool> {
    let s = info.details.as_ref()?.sbc.as_ref()?;
    let (min, max) = s.abr_bitpool.filter(|_| info.abr)?;
    Some(LiveBitpool { now: s.bitpool, min, max })
}

/// A codec frame's duration in ms (0 while the rate is not known)
pub fn frame_ms(samples: u32, sample_rate: u32) -> f64 {
    if sample_rate > 0 { f64::from(samples) * 1000.0 / f64::from(sample_rate) } else { 0.0 }
}

/// Bytes a full packet carries and what it may carry
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PacketFill {
    pub used: u32,
    pub of: u32,
}

/// Bytes a full packet carries (media payload header and frames) and what
/// it may carry; None when the frames do not have a fixed length
pub fn packet_fill(d: &CodecDetails) -> Option<PacketFill> {
    let (frames, bytes) = (d.frames_per_packet?, d.frame_bytes?);
    if d.mtu == 0 {
        return None;
    }
    Some(PacketFill { used: d.media_header + frames * bytes, of: d.mtu })
}

/// 44100 -> "44.1 kHz"
pub fn khz(rate: u32) -> String {
    format!("{} kHz", js_number(f64::from(rate) / 1000.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chips_show_the_value_in_use_and_what_the_device_offered() {
        let c = chips(&SBC_BLOCKS, Some(&[8, 16]), 16);
        let states: Vec<_> = c.iter().map(|c| (c.value, c.state)).collect();
        assert_eq!(states, [(4, ChipState::Off), (8, ChipState::Offered), (12, ChipState::Off), (16, ChipState::Used)]);
        assert_eq!(chips(&SBC_BLOCKS, None, 16), [Chip { value: 16, state: ChipState::Used }]);
        // A value outside the list goes last
        assert_eq!(chips(&[1, 2], Some(&[1]), 3).last(), Some(&Chip { value: 3, state: ChipState::Used }));
        assert_eq!(khz(44100), "44.1 kHz");
        assert_eq!(khz(48000), "48 kHz");
    }
}
