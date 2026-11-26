//! The SBC frame setup a profile may ask for (bitpool; in developer mode also
//! blocks, subbands, allocation, channel mode) and the bit rate it gives, as
//! the core works it out
//! (crates/codec/src/sbc/mod.rs, caps.rs RemoteCaps::sbc_params): a value the
//! headphones do not list falls back to the default when connecting, Auto
//! takes the default, the bitpool stays within the headphones' range.

use ipc::Device;

use crate::codecs::CodecFields;
use crate::format::parse_address;

pub const SBC_BLOCKS: [u32; 4] = [4, 8, 12, 16];
pub const SBC_SUBBANDS: [u32; 2] = [4, 8];
pub const SBC_ALLOCATIONS: [&str; 2] = ["loudness", "snr"];
/// SBC only: dual channel is SBC XQ's, which is always that (issue #86)
pub const SBC_CHANNEL_MODES: [&str; 3] = ["joint_stereo", "stereo", "mono"];

/// What a device's SBC endpoint offers (Device.sbc, CodecDetails' SbcDeviceCaps)
pub type SbcCaps = ipc::SbcDeviceCaps;

/// What the device listed when it last connected (None: not known)
pub fn known_sbc_caps<'a>(devices: &'a [Device], address: &str) -> Option<&'a SbcCaps> {
    let a = parse_address(address)?;
    devices.iter().find(|d| !d.le_audio && d.address.to_uppercase() == a)?.sbc.as_ref()
}

/// The lists of [`SbcCaps`] [`sbc_offered`] looks in
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SbcField {
    Blocks,
    Subbands,
    Allocations,
    ChannelModes,
}

/// A value of an [`SbcField`]: a number (blocks, subbands) or a name
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SbcValue<'a> {
    Num(u32),
    Str(&'a str),
}

impl From<u32> for SbcValue<'_> {
    fn from(n: u32) -> Self {
        SbcValue::Num(n)
    }
}
impl From<u8> for SbcValue<'_> {
    fn from(n: u8) -> Self {
        SbcValue::Num(n.into())
    }
}
impl<'a> From<&'a str> for SbcValue<'a> {
    fn from(s: &'a str) -> Self {
        SbcValue::Str(s)
    }
}
impl<'a> From<&'a String> for SbcValue<'a> {
    fn from(s: &'a String) -> Self {
        SbcValue::Str(s)
    }
}

/// Whether the device lists a value (anything goes while not known)
pub fn sbc_offered<'a>(caps: Option<&SbcCaps>, field: SbcField, value: impl Into<SbcValue<'a>>) -> bool {
    let Some(caps) = caps else { return true };
    match (field, value.into()) {
        (SbcField::Blocks, SbcValue::Num(n)) => caps.blocks.contains(&n),
        (SbcField::Subbands, SbcValue::Num(n)) => caps.subbands.contains(&n),
        (SbcField::Allocations, SbcValue::Str(s)) => caps.allocations.iter().any(|a| a == s),
        (SbcField::ChannelModes, SbcValue::Str(s)) => caps.channel_modes.iter().any(|m| m == s),
        _ => false,
    }
}

/// The joint stereo bitpool of a quality (mono and stereo the same)
fn quality_bitpool(quality: &str) -> u32 {
    match quality {
        "sq" => 35,
        "mq" => 19,
        _ => 53,
    }
}

/// SBC XQ's bitpool per channel of a quality at a rate
fn xq_bitpool(quality: &str, rate: u32) -> u32 {
    let (at44, at48) = match quality {
        "sq" => (43, 39),
        "mq" => (32, 29),
        _ => (47, 43),
    };
    if rate == 44100 { at44 } else { at48 }
}

/// SBC XQ's largest bitpool per channel at a rate: 47 at 44.1 kHz, 43 at
/// 48 kHz (the core's xq_max_bitpool; issue #86: a slider up to 47 at
/// 48 kHz changed nothing past 43)
pub fn xq_max_bitpool(rate: u32) -> u32 {
    xq_bitpool("hq", rate)
}

/// Bytes of one SBC frame: the A2DP specification's formula (12.9), with the
/// bitpool limited to what the channel mode allows
pub fn sbc_frame_length(mode: &str, blocks: u32, subbands: u32, bitpool: u32) -> u32 {
    let shared = mode == "stereo" || mode == "joint_stereo";
    let bp = bitpool.min(if shared { if subbands == 8 { 255 } else { 128 } } else { 16 * subbands });
    let ch = if mode == "mono" { 1 } else { 2 };
    let data = match mode {
        "joint_stereo" => subbands + blocks * bp,
        "stereo" => blocks * bp,
        _ => blocks * ch * bp,
    };
    4 + (4 * subbands * ch) / 8 + data.div_ceil(8)
}

/// kbps of frames of `len` bytes, each blocks x subbands samples (rounded down, as the core)
pub fn sbc_kbps(len: u32, rate: u32, blocks: u32, subbands: u32) -> u32 {
    let samples = u64::from(blocks) * u64::from(subbands);
    if samples == 0 {
        return 0;
    }
    (8 * u64::from(len) * u64::from(rate) / (samples * 1000)) as u32
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SbcSetup {
    pub blocks: u32,
    pub subbands: u32,
    pub allocation: String,
    pub mode: String,
    pub bitpool: u32,
    pub frame_bytes: u32,
    pub kbps: u32,
}

/// The setup a profile streams SBC / SBC XQ with at a rate (stereo input):
/// what the device does not list taken as the default
pub fn sbc_setup(p: &impl CodecFields, rate: u32, caps: Option<&SbcCaps>) -> SbcSetup {
    let p = p.codec_choice();
    let xq = p.codec == "sbcxq";
    let blocks = if p.sbc_blocks != 0 && sbc_offered(caps, SbcField::Blocks, p.sbc_blocks) {
        u32::from(p.sbc_blocks)
    } else {
        16
    };
    let subbands = if p.sbc_subbands != 0 && sbc_offered(caps, SbcField::Subbands, p.sbc_subbands) {
        u32::from(p.sbc_subbands)
    } else {
        8
    };
    let allocation = if !p.sbc_allocation.is_empty() && sbc_offered(caps, SbcField::Allocations, &p.sbc_allocation) {
        p.sbc_allocation.clone()
    } else {
        "loudness".to_owned()
    };
    let mode = if xq {
        "dual_channel".to_owned()
    } else if SBC_CHANNEL_MODES.contains(&p.sbc_channel_mode.as_str())
        && sbc_offered(caps, SbcField::ChannelModes, &p.sbc_channel_mode)
    {
        p.sbc_channel_mode.clone()
    } else {
        "joint_stereo".to_owned()
    };
    // SBC XQ not held to the device's list: up to SBC's 53 (issue #86)
    let max = if xq && !p.unlisted_options { xq_max_bitpool(rate) } else { 53 };
    let asked = u32::from(if xq { p.sbcxq_bitpool } else { p.sbc_bitpool });
    let auto = if xq { xq_bitpool(&p.quality, rate) } else { quality_bitpool(&p.quality) };
    // Never below the profile's minimum (issue #86)
    let min = u32::from(if xq { p.sbcxq_min_bitpool } else { p.sbc_min_bitpool });
    let mut bitpool = (if asked != 0 { asked } else { auto }).max(min).min(max);
    if let Some(c) = caps.filter(|c| c.max_bitpool >= 2) {
        bitpool = bitpool.min(c.max_bitpool).max(c.min_bitpool.min(c.max_bitpool)).max(2);
    }
    let frame_bytes = sbc_frame_length(&mode, blocks, subbands, bitpool);
    SbcSetup {
        blocks,
        subbands,
        allocation,
        mode,
        bitpool,
        frame_bytes,
        kbps: sbc_kbps(frame_bytes, rate, blocks, subbands),
    }
}

/// The rate the bit rate is shown at: the profile's, or on Auto 48 kHz,
/// 44.1 kHz on a device without it, as the core streams it (one figure is
/// enough, issue #86). `sample_rate`: the profile's.
pub fn sbc_rate(sample_rate: u32, caps: Option<&SbcCaps>) -> u32 {
    if sample_rate == 44100 || sample_rate == 48000 {
        return sample_rate;
    }
    match caps {
        Some(c) if !c.sample_rates.contains(&48000) && c.sample_rates.contains(&44100) => 44100,
        _ => 48000,
    }
}

/// "≈ 552 kbps": the bit rate SBC / SBC XQ streams with at that rate
pub fn sbc_kbps_text(p: &impl CodecFields, caps: Option<&SbcCaps>) -> String {
    let rate = sbc_rate(p.codec_choice().sample_rate, caps);
    format!("≈ {} kbps", sbc_setup(p, rate, caps).kbps)
}

/// Test data shared by the modules' tests
#[cfg(test)]
pub(crate) mod mock {
    use ipc::{Device, MediaEq, MicProcessing, Profile};

    use super::SbcCaps;

    /// What the WH-1000XM5's SBC endpoint lists (no 4 subbands, no SNR, as many headphones)
    pub fn xm5_sbc() -> SbcCaps {
        SbcCaps {
            sample_rates: vec![44100, 48000],
            channel_modes: ["mono", "dual_channel", "stereo", "joint_stereo"].map(String::from).to_vec(),
            blocks: vec![4, 8, 12, 16],
            subbands: vec![8],
            allocations: vec!["loudness".into()],
            min_bitpool: 2,
            max_bitpool: 53,
        }
    }

    pub fn sample_profile() -> Profile {
        Profile {
            name: "WH-1000XM5 (LDAC)".into(),
            device_address: "AC:80:0A:12:34:56".into(),
            device_name: "WH-1000XM5".into(),
            codec: "ldac".into(),
            quality: "hq".into(),
            abr: false,
            lhdcv5_low_latency: false,
            codec_fallback: true,
            codec_forced: false,
            unlisted_options: false,
            sample_rate: 0,
            bit_depth: 0,
            capture_mode: "loopback".into(),
            audio_device_id: String::new(),
            audio_device_name: String::new(),
            auto_switch_device: false,
            app_exe: String::new(),
            app_name: String::new(),
            volume_sync: "auto".into(),
            volume_device_id: String::new(),
            volume_device_name: String::new(),
            max_media_payload: 1023,
            le_audio: false,
            le_low_latency: false,
            le_swap_channels: false,
            le_alone_channel: String::new(),
            headset_mic: "off".into(),
            mic_device_id: String::new(),
            mic_device_name: String::new(),
            mic_muted: false,
            mic_default_input: true,
            voice_codec: String::new(),
            mic_rate: 0,
            mic_processing: MicProcessing::default(),
            sbc_bitpool: 0,
            sbcxq_bitpool: 0,
            sbc_min_bitpool: 0,
            sbcxq_min_bitpool: 0,
            sbc_blocks: 0,
            sbc_subbands: 0,
            sbc_allocation: String::new(),
            sbc_channel_mode: String::new(),
            codec_settings: Default::default(),
            codec_order: Vec::new(),
            media_eq: MediaEq::default(),
        }
    }

    /// sampleProfile with the codec set
    pub fn profile_with(codec: &str) -> Profile {
        Profile { codec: codec.into(), ..sample_profile() }
    }

    /// A device entry: classic, not saved
    pub fn sample_device(address: &str) -> Device {
        Device {
            address: address.into(),
            name: String::new(),
            saved: false,
            audio: true,
            rssi: None,
            codecs: Vec::new(),
            sbc: None,
            codec_caps: Vec::new(),
            codecs_on_request: Vec::new(),
            le_audio: false,
            le_audio_set: false,
            hid_le: None,
            set_addresses: Vec::new(),
        }
    }
}

// The SBC bit rate as the core works it out (crates/codec/src/sbc/mod.rs
// tests have the same numbers)
#[cfg(test)]
mod tests {
    use super::mock::*;
    use super::*;
    use ipc::Profile;

    fn sbc() -> Profile {
        profile_with("sbc")
    }

    #[test]
    fn follows_the_a2dp_specifications_formula() {
        // (rate, blocks, subbands, mode, bitpool, frame length, kbps)
        let cases: [(u32, u32, u32, &str, u32, u32, u32); 10] = [
            (44100, 16, 8, "joint_stereo", 53, 119, 327),
            (44100, 16, 8, "stereo", 53, 118, 325),
            (44100, 16, 8, "dual_channel", 32, 140, 385),
            (44100, 16, 8, "mono", 31, 70, 192),
            (44100, 8, 4, "joint_stereo", 35, 44, 485),
            (48000, 8, 4, "stereo", 35, 43, 516),
            (44100, 4, 8, "joint_stereo", 53, 40, 441),
            (48000, 12, 8, "joint_stereo", 53, 93, 372),
            (48000, 12, 4, "dual_channel", 20, 68, 544),
            (44100, 4, 4, "mono", 30, 21, 463),
        ];
        for (rate, blocks, subbands, mode, bitpool, len, kbps) in cases {
            assert_eq!(sbc_frame_length(mode, blocks, subbands, bitpool), len);
            assert_eq!(sbc_kbps(len, rate, blocks, subbands), kbps);
        }
    }

    #[test]
    fn works_out_a_profiles_setup_as_the_core() {
        // Auto: joint stereo, 16 blocks, 8 subbands, the quality's bitpool
        let s = sbc_setup(&sbc(), 44100, None);
        assert_eq!((s.blocks, s.subbands, s.mode.as_str(), s.bitpool, s.kbps), (16, 8, "joint_stereo", 53, 327));
        let s = sbc_setup(&Profile { quality: "sq".into(), ..sbc() }, 48000, None);
        assert_eq!((s.bitpool, s.kbps), (35, 249));
        // Plain SBC has no dual channel (SBC XQ is that, issue #86): joint stereo; a bitpool asked for wins
        let s = sbc_setup(&Profile { sbc_channel_mode: "dual_channel".into(), ..sbc() }, 44100, None);
        assert_eq!((s.mode.as_str(), s.bitpool), ("joint_stereo", 53));
        let s = sbc_setup(&Profile { sbc_bitpool: 40, sbc_blocks: 8, sbc_subbands: 4, ..sbc() }, 44100, None);
        assert_eq!((s.frame_bytes, s.kbps), (49, 540));
        // SBC XQ: dual channel whatever was asked, its own bitpools
        let xq = Profile { sbc_channel_mode: "stereo".into(), ..profile_with("sbcxq") };
        let s = sbc_setup(&xq, 44100, None);
        assert_eq!((s.mode.as_str(), s.bitpool, s.frame_bytes, s.kbps), ("dual_channel", 47, 200, 551));
        assert_eq!(sbc_setup(&Profile { sbcxq_bitpool: 53, ..xq.clone() }, 48000, None).bitpool, 43);
        let s = sbc_setup(&Profile { sbcxq_bitpool: 53, unlisted_options: true, ..xq.clone() }, 48000, None);
        assert_eq!(s.bitpool, 53);
        // The minimum (issue #86): the bitpool never below it, nor above the cap
        let s = sbc_setup(&Profile { quality: "mq".into(), sbc_min_bitpool: 30, ..sbc() }, 44100, None);
        assert_eq!(s.bitpool, 30);
        assert_eq!(sbc_setup(&Profile { sbc_bitpool: 25, sbc_min_bitpool: 30, ..sbc() }, 44100, None).bitpool, 30);
        assert_eq!(sbc_setup(&Profile { sbcxq_min_bitpool: 53, ..xq }, 48000, None).bitpool, 43);
        // ... and the device's range still wins
        let caps = SbcCaps { max_bitpool: 35, ..xm5_sbc() };
        assert_eq!(sbc_setup(&Profile { sbc_min_bitpool: 45, ..sbc() }, 44100, Some(&caps)).bitpool, 35);
    }

    #[test]
    fn takes_what_the_device_does_not_list_as_the_default_and_its_bitpool_range() {
        let caps = SbcCaps { max_bitpool: 35, ..xm5_sbc() };
        let p = Profile { sbc_subbands: 4, sbc_allocation: "snr".into(), sbc_blocks: 8, ..sbc() };
        let s = sbc_setup(&p, 44100, Some(&caps));
        assert_eq!((s.blocks, s.subbands, s.allocation.as_str(), s.bitpool), (8, 8, "loudness", 35));
        assert!(!sbc_offered(Some(&caps), SbcField::Subbands, 4u32));
        assert!(sbc_offered(Some(&caps), SbcField::Blocks, 4u32));
        assert!(sbc_offered(None, SbcField::Subbands, 4u32));
        assert_eq!(sbc_rate(0, Some(&SbcCaps { sample_rates: vec![48000], ..caps.clone() })), 48000);
        assert_eq!(sbc_rate(0, Some(&SbcCaps { sample_rates: vec![44100], ..caps.clone() })), 44100);
        assert_eq!(sbc_rate(44100, None), 44100);
        // Auto: 48 kHz, 44.1 kHz only on a device without it
        assert_eq!(sbc_rate(0, None), 48000);
        assert_eq!(sbc_kbps_text(&sbc(), None), format!("≈ {} kbps", sbc_setup(&sbc(), 48000, None).kbps));
    }

    #[test]
    fn finds_the_caps_of_the_profiles_device() {
        let list =
            [Device { sbc: Some(xm5_sbc()), ..sample_device("AC:80:0A:12:34:56") }, sample_device("00:1A:7D:DA:71:13")];
        assert_eq!(known_sbc_caps(&list, "ac-80-0a-12-34-56"), Some(&xm5_sbc()));
        assert_eq!(known_sbc_caps(&list, "00:1A:7D:DA:71:13"), None);
        assert_eq!(known_sbc_caps(&list, "nope"), None);
    }
}
