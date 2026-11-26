//! Diagnostics' codec details card as rows of
//! text: what an A2DP stream's codec runs with, next to what the device
//! offered (the SBC bitpool range, channel modes, AAC object types ...; for
//! the other codecs the rates, bit depths and peak bit rate their endpoint
//! lists, issue #86), and how the codec frames fill a packet. The window
//! draws these; the numbers behind them are [`crate::codecdetails`].

use ipc::{AacDetails, CodecDetails, CodecDeviceCaps, SbcDetails, StreamInfo};

use crate::codecdetails::{
    AAC_CHANNELS, BitpoolBar, ChipState, SBC_ALLOCATIONS, SBC_BLOCKS, SBC_MODES, SBC_RATES, SBC_SUBBANDS, bitpool_bar,
    chips, frame_ms, khz, live_bitpool, packet_fill,
};
use crate::i18n::t;
use crate::t;

/// A setting's value as a chip: in use, offered by the device, not offered
#[derive(Debug, Clone, PartialEq)]
pub struct CdChip {
    pub text: String,
    pub state: ChipState,
}

impl CdChip {
    /// What a screen reader says: the text and its state
    pub fn accessible_text(&self) -> String {
        match self.state {
            ChipState::Used => format!("{} ({})", self.text, t("ui.diag.cd_used")),
            ChipState::Off => format!("{} ({})", self.text, t("ui.diag.cd_not_offered")),
            ChipState::Offered => self.text.clone(),
        }
    }
}

/// A range of the bitpool bar and its legend entry
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitpoolPart {
    Device,
    Config,
    Abr,
    Used,
}

/// The bitpool in use within the configured and the device's ranges
#[derive(Debug, Clone, PartialEq)]
pub struct BitpoolView {
    pub bar: BitpoolBar,
    /// The legend, in order (also the track's accessible name, joined by ", ")
    pub parts: Vec<(BitpoolPart, String)>,
    /// SBC ABR: the mark moves with the bitpool
    pub live: bool,
}

impl BitpoolView {
    pub fn label(&self) -> String {
        self.parts.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>().join(", ")
    }
}

/// What a row shows under its value
#[derive(Debug, Clone, PartialEq)]
pub enum CdExtra {
    Bitpool(BitpoolView),
    /// How full a packet is, in %
    Gauge(f64),
}

#[derive(Debug, Clone, PartialEq)]
pub enum CdRow {
    Value { label: String, value: String, extra: Option<CdExtra> },
    Chips { label: String, chips: Vec<CdChip> },
}

impl CdRow {
    fn value(label: String, value: String) -> CdRow {
        CdRow::Value { label, value, extra: None }
    }

    pub fn label(&self) -> &str {
        match self {
            CdRow::Value { label, .. } | CdRow::Chips { label, .. } => label,
        }
    }
}

/// A codec's own part of the card (SBC, AAC or the codec's name)
#[derive(Debug, Clone, PartialEq)]
pub struct CdSection {
    pub title: String,
    pub rows: Vec<CdRow>,
    pub hints: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CodecDetailsView {
    /// "Codec details", the codec's name beside it
    pub title: String,
    pub codec: String,
    pub rows: Vec<CdRow>,
    pub sections: Vec<CdSection>,
    /// Below everything: the chips' legend, a missing capabilities note, the help
    pub hints: Vec<String>,
}

fn chip_row<T: PartialEq + Clone>(
    label: String,
    all: &[T],
    offered: Option<&[T]>,
    used: T,
    text: impl Fn(&T) -> String,
) -> CdRow {
    let chips =
        chips(all, offered, used).into_iter().map(|c| CdChip { text: text(&c.value), state: c.state }).collect();
    CdRow::Chips { label, chips }
}

fn owned(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

fn bitpool_view(s: &SbcDetails) -> BitpoolView {
    let mut parts = Vec::new();
    if let Some(d) = &s.device {
        parts.push((BitpoolPart::Device, t!("ui.diag.cd_bitpool_device", d.min_bitpool, d.max_bitpool)));
    }
    if let Some((lo, hi)) = s.config_bitpool {
        parts.push((BitpoolPart::Config, t!("ui.diag.cd_bitpool_config", lo, hi)));
    }
    if let Some((lo, hi)) = s.abr_bitpool {
        parts.push((BitpoolPart::Abr, t!("ui.diag.cd_bitpool_abr_range", lo, hi)));
    }
    parts.push((BitpoolPart::Used, t!("ui.diag.cd_bitpool_used", s.bitpool)));
    BitpoolView { bar: bitpool_bar(s), parts, live: s.abr_bitpool.is_some() }
}

fn sbc_rows(sbc: &SbcDetails, info: &StreamInfo) -> Vec<CdRow> {
    let dev = sbc.device.as_ref();
    // Under ABR: "41 (ABR 19–53)", updating as it moves
    let bitpool = match live_bitpool(info) {
        Some(bp) => t!("ui.diag.cd_bitpool_abr", bp.now, bp.min, bp.max),
        None => sbc.bitpool.to_string(),
    };
    let modes = owned(&SBC_MODES);
    let allocations = owned(&SBC_ALLOCATIONS);
    vec![
        CdRow::Value {
            label: t("ui.diag.cd_bitpool"),
            value: bitpool,
            extra: Some(CdExtra::Bitpool(bitpool_view(sbc))),
        },
        chip_row(t("ui.diag.cd_rate"), &SBC_RATES, dev.map(|d| d.sample_rates.as_slice()), info.sample_rate, |r| {
            khz(*r)
        }),
        chip_row(
            t("ui.diag.cd_channel_mode"),
            &modes,
            dev.map(|d| d.channel_modes.as_slice()),
            sbc.channel_mode.clone(),
            |m| t(&format!("ui.diag.cd_mode.{m}")),
        ),
        chip_row(t("ui.diag.cd_blocks"), &SBC_BLOCKS, dev.map(|d| d.blocks.as_slice()), sbc.blocks, u32::to_string),
        chip_row(t("ui.diag.cd_subbands"), &SBC_SUBBANDS, dev.map(|d| d.subbands.as_slice()), sbc.subbands, |v| {
            v.to_string()
        }),
        chip_row(
            t("ui.diag.cd_allocation"),
            &allocations,
            dev.map(|d| d.allocations.as_slice()),
            sbc.allocation.clone(),
            |a| t(&format!("ui.diag.cd_alloc.{a}")),
        ),
    ]
}

fn peak_text(max_bitrate: u32) -> String {
    if max_bitrate > 0 {
        t!("ui.diag.cd_peak_value", (f64::from(max_bitrate) / 1000.0).round())
    } else {
        t("ui.diag.cd_peak_none")
    }
}

fn aac_rows(aac: &AacDetails, info: &StreamInfo) -> Vec<CdRow> {
    let dev = aac.device.as_ref();
    // The device lists only some of the many AAC values: show what it offers
    let object_types = dev.map(|d| d.object_types.as_slice());
    let rates = dev.map(|d| d.sample_rates.as_slice());
    let mut rows = vec![
        CdRow::value(t("ui.diag.cd_sent"), t!("ui.diag.cd_sent_cbr", (f64::from(aac.bit_rate) / 1000.0).round())),
        chip_row(
            t("ui.diag.cd_object_type"),
            object_types.unwrap_or_default(),
            object_types,
            aac.object_type.clone(),
            String::clone,
        ),
        chip_row(t("ui.diag.cd_rate"), rates.unwrap_or_default(), rates, info.sample_rate, |r| khz(*r)),
        chip_row(t("ui.diag.cd_channels"), &AAC_CHANNELS, dev.map(|d| d.channels.as_slice()), info.channels, |c| {
            c.to_string()
        }),
    ];
    if let Some(d) = dev {
        rows.push(CdRow::value(t("ui.diag.cd_peak"), peak_text(d.max_bitrate)));
        rows.push(CdRow::value(
            t("ui.diag.cd_vbr"),
            if d.vbr { t("ui.diag.cd_vbr_yes") } else { t("ui.diag.cd_vbr_no") },
        ));
    }
    rows
}

/// Any other codec: what its endpoint lists, as SBC and AAC show theirs
fn device_rows(device: &CodecDeviceCaps, info: &StreamInfo) -> Vec<CdRow> {
    // The device lists only some of the values: show what it offers
    let offered = |list: &[u32]| if list.is_empty() { None } else { Some(list.to_vec()) };
    let rates = offered(&device.sample_rates);
    let mut rows =
        vec![chip_row(t("ui.diag.cd_rate"), &device.sample_rates, rates.as_deref(), info.sample_rate, |r| khz(*r))];
    if info.bits > 0 {
        let bits = offered(&device.bit_depths);
        rows.push(chip_row(t("connection.bit_depth"), &device.bit_depths, bits.as_deref(), info.bits, |b| {
            format!("{b}-bit")
        }));
    }
    rows.push(CdRow::value(t("ui.diag.cd_peak"), peak_text(device.max_bitrate)));
    rows
}

fn headers_text(d: &CodecDetails) -> String {
    match (d.rtp, d.media_header > 0) {
        (true, true) => t!("ui.diag.cd_headers_rtp", d.media_header),
        (true, false) => t("ui.diag.cd_headers_rtp_only"),
        (false, true) => t!("ui.diag.cd_headers_plain", d.media_header),
        (false, false) => t("ui.diag.cd_headers_none"),
    }
}

fn frame_text(d: &CodecDetails, info: &StreamInfo) -> String {
    let ms = frame_ms(d.frame_samples, info.sample_rate);
    match d.frame_bytes {
        Some(bytes) => t!("ui.diag.cd_frame_bytes", bytes, d.frame_samples, ms),
        None => t!("ui.diag.cd_frame_samples", d.frame_samples, ms),
    }
}

pub fn codec_details_view(info: &StreamInfo, d: &CodecDetails) -> CodecDetailsView {
    let mut rows = vec![
        CdRow::value(t("ui.diag.cd_payload"), t!("ui.diag.cd_payload_value", d.mtu)),
        CdRow::value(t("ui.diag.cd_headers"), headers_text(d)),
        CdRow::value(t("ui.diag.cd_frame"), frame_text(d, info)),
    ];
    if let (Some(f), Some(frames)) = (packet_fill(d), d.frames_per_packet) {
        rows.push(CdRow::Value {
            label: t("ui.diag.cd_per_packet"),
            value: t!("ui.diag.cd_per_packet_value", frames, f.used, f.of),
            extra: Some(CdExtra::Gauge(f64::from(f.used) / f64::from(f.of) * 100.0)),
        });
    }
    let mut sections = Vec::new();
    if let Some(s) = &d.sbc {
        let mut hints = vec![t("ui.diag.cd_bitpool_help")];
        if s.abr_bitpool.is_some() {
            hints.push(t("ui.diag.cd_bitpool_abr_help"));
        }
        sections.push(CdSection { title: "SBC".to_owned(), rows: sbc_rows(s, info), hints });
    }
    if let Some(a) = &d.aac {
        sections.push(CdSection { title: "AAC".to_owned(), rows: aac_rows(a, info), hints: Vec::new() });
    }
    if let Some(dev) = &d.device {
        sections.push(CdSection { title: info.codec.clone(), rows: device_rows(dev, info), hints: Vec::new() });
    }
    // Without the device's capabilities the chips show only what is used
    let no_device =
        d.sbc.as_ref().is_some_and(|s| s.device.is_none()) || d.aac.as_ref().is_some_and(|a| a.device.is_none());
    let chips_shown = d.sbc.as_ref().is_some_and(|s| s.device.is_some())
        || d.aac.as_ref().is_some_and(|a| a.device.is_some())
        || d.device.is_some();
    let mut hints = Vec::new();
    if chips_shown {
        hints.push(t("ui.diag.cd_legend"));
    }
    if no_device {
        hints.push(t("ui.diag.cd_no_device"));
    }
    hints.push(t("ui.diag.cd_help"));
    CodecDetailsView { title: t("ui.diag.cd_title"), codec: info.codec.clone(), rows, sections, hints }
}

#[cfg(test)]
mod tests {
    use ipc::{AacDeviceCaps, SbcDeviceCaps};

    use super::*;

    fn sbc() -> CodecDetails {
        CodecDetails {
            mtu: 679,
            rtp: true,
            media_header: 1,
            frame_samples: 128,
            frame_bytes: Some(119),
            frames_per_packet: Some(5),
            sbc: Some(SbcDetails {
                channel_mode: "joint_stereo".into(),
                blocks: 16,
                subbands: 8,
                allocation: "loudness".into(),
                bitpool: 51,
                config_bitpool: Some((2, 51)),
                device: Some(SbcDeviceCaps {
                    sample_rates: vec![44100, 48000],
                    channel_modes: vec!["mono".into(), "joint_stereo".into()],
                    blocks: vec![4, 8, 12, 16],
                    subbands: vec![8],
                    allocations: vec!["loudness".into()],
                    min_bitpool: 2,
                    max_bitpool: 51,
                }),
                abr_bitpool: None,
            }),
            aac: None,
            device: None,
        }
    }

    fn info(details: &CodecDetails) -> StreamInfo {
        let mut i: StreamInfo = serde_json::from_value(serde_json::json!({
            "codec": "SBC", "bitrate_kbps": 317, "sample_rate": 48000, "channels": 2, "bits": 16,
            "source_sample_rate": 48000, "source_channels": 2, "source_bits": 32, "requested_codec": null,
            "requested_in_use": false, "abr": false, "bitrate_capped": false, "hi_res_set": false,
            "low_latency": false, "source": null, "le": null, "details": null
        }))
        .expect("a StreamInfo");
        i.details = Some(Box::new(details.clone()));
        i
    }

    fn value<'a>(v: &'a CodecDetailsView, label: &str) -> &'a CdRow {
        v.rows
            .iter()
            .chain(v.sections.iter().flat_map(|s| s.rows.iter()))
            .find(|r| r.label() == label)
            .unwrap_or_else(|| panic!("no row {label}"))
    }

    fn text(r: &CdRow) -> &str {
        match r {
            CdRow::Value { value, .. } => value,
            CdRow::Chips { .. } => panic!("a chip row"),
        }
    }

    fn all_chips(v: &CodecDetailsView) -> Vec<CdChip> {
        v.sections
            .iter()
            .flat_map(|s| s.rows.iter())
            .flat_map(|r| match r {
                CdRow::Chips { chips, .. } => chips.clone(),
                CdRow::Value { .. } => Vec::new(),
            })
            .collect()
    }

    fn bitpool(v: &CodecDetailsView) -> BitpoolView {
        match value(v, "Bitpool") {
            CdRow::Value { extra: Some(CdExtra::Bitpool(b)), .. } => b.clone(),
            _ => panic!("no bitpool bar"),
        }
    }

    #[test]
    fn shows_the_sbc_setup_against_what_the_device_offered() {
        let d = sbc();
        let v = codec_details_view(&info(&d), &d);
        assert_eq!((v.title.as_str(), v.codec.as_str()), ("Codec details", "SBC"));
        assert_eq!(text(value(&v, "Media payload")), "Up to 679 bytes");
        assert_eq!(text(value(&v, "Headers")), "RTP 12 B + media header 1 B");
        assert_eq!(text(value(&v, "Codec frame")), "119 bytes, 128 samples (2.67 ms)");
        assert_eq!(text(value(&v, "Frames per packet")), "5 (596 of 679 bytes used)");
        assert_eq!(bitpool(&v).label(), "Device 2–51, Configured 2–51, In use 51");
        let chips = all_chips(&v);
        let used: Vec<&str> = chips.iter().filter(|c| c.state == ChipState::Used).map(|c| c.text.as_str()).collect();
        assert_eq!(used, ["48 kHz", "Joint stereo", "16", "8", "Loudness"]);
        let off: Vec<String> =
            chips.iter().filter(|c| c.state == ChipState::Off).map(CdChip::accessible_text).collect();
        assert!(off.contains(&"16 kHz (not offered)".to_owned()));
        assert!(off.contains(&"Dual channel (not offered)".to_owned()));
        assert!(off.contains(&"SNR (not offered)".to_owned()));
        assert!(v.hints.iter().any(|h| h.starts_with("Filled: in use.")));
    }

    #[test]
    fn shows_only_what_is_used_when_the_device_did_not_report_its_capabilities() {
        let mut d = sbc();
        let s = d.sbc.as_mut().unwrap();
        s.device = None;
        s.config_bitpool = None;
        let v = codec_details_view(&info(&d), &d);
        assert!(all_chips(&v).iter().all(|c| c.state == ChipState::Used));
        assert_eq!(bitpool(&v).label(), "In use 51");
        assert!(v.hints.iter().any(|h| h.contains("The device did not report these capabilities")));
        assert!(!v.hints.iter().any(|h| h.starts_with("Filled: in use.")));
    }

    #[test]
    fn shows_what_any_other_codecs_endpoint_lists() {
        let mut d = sbc();
        d.frame_bytes = None;
        d.frames_per_packet = None;
        d.sbc = None;
        d.device = Some(CodecDeviceCaps {
            codec: "ldac".into(),
            sample_rates: vec![44100, 48000, 96000],
            bit_depths: vec![16, 24],
            max_bitrate: 0,
            lhdcv5_low_latency: false,
        });
        let mut i = info(&d);
        i.codec = "LDAC".into();
        i.sample_rate = 96000;
        i.bits = 24;
        let v = codec_details_view(&i, &d);
        assert_eq!(v.sections[0].title, "LDAC");
        let chips: Vec<String> = all_chips(&v).iter().map(CdChip::accessible_text).collect();
        assert_eq!(chips, ["44.1 kHz", "48 kHz", "96 kHz (in use)", "16-bit", "24-bit (in use)"]);
        assert_eq!(text(value(&v, "Device's peak bit rate")), "Not given");
        assert!(v.hints.iter().any(|h| h.starts_with("Filled: in use.")));
    }

    #[test]
    fn shows_aacs_object_type_peak_bit_rate_and_vbr() {
        let d = CodecDetails {
            mtu: 894,
            rtp: true,
            media_header: 0,
            frame_samples: 1024,
            frame_bytes: None,
            frames_per_packet: None,
            sbc: None,
            aac: Some(AacDetails {
                object_type: "MPEG-2 AAC-LC".into(),
                bit_rate: 256000,
                device: Some(AacDeviceCaps {
                    object_types: vec!["MPEG-2 AAC-LC".into(), "MPEG-4 HE-AAC".into()],
                    sample_rates: vec![44100, 48000, 96000],
                    channels: vec![1, 2],
                    max_bitrate: 0,
                    vbr: true,
                }),
            }),
            device: None,
        };
        let mut i = info(&d);
        i.codec = "AAC".into();
        i.sample_rate = 44100;
        let v = codec_details_view(&i, &d);
        assert_eq!(text(value(&v, "Headers")), "RTP 12 B");
        assert_eq!(text(value(&v, "Codec frame")), "1024 samples (23.22 ms)");
        assert_eq!(text(value(&v, "Sent as")), "CBR 256 kbps");
        assert_eq!(text(value(&v, "Device's peak bit rate")), "Not given");
        assert_eq!(text(value(&v, "Device accepts VBR")), "Yes");
        // No gauge without a fixed frame length
        assert!(!v.rows.iter().any(|r| matches!(r, CdRow::Value { extra: Some(CdExtra::Gauge(_)), .. })));
        let chips = all_chips(&v);
        let used: Vec<&str> = chips.iter().filter(|c| c.state == ChipState::Used).map(|c| c.text.as_str()).collect();
        assert_eq!(used, ["MPEG-2 AAC-LC", "44.1 kHz", "2"]);
        assert!(!chips.iter().any(|c| c.state == ChipState::Off));
    }

    #[test]
    fn shows_sbc_abrs_range_and_the_bitpool_as_it_moves() {
        let abr = |bitpool: u32| {
            let mut d = sbc();
            let s = d.sbc.as_mut().unwrap();
            s.bitpool = bitpool;
            s.config_bitpool = Some((2, 53));
            s.device.as_mut().unwrap().max_bitpool = 53;
            s.abr_bitpool = Some((19, 53));
            d
        };
        let live = |bitpool: u32| {
            let d = abr(bitpool);
            let mut i = info(&d);
            i.abr = true;
            (i, d)
        };
        let (i, d) = live(53);
        let v = codec_details_view(&i, &d);
        assert_eq!(text(value(&v, "Bitpool")), "53 (ABR 19–53)");
        let b = bitpool(&v);
        assert_eq!(b.label(), "Device 2–53, Configured 2–53, ABR 19–53, In use 53");
        assert!(b.live && b.bar.abr.is_some());
        assert!(v.sections[0].hints.iter().any(|h| h.starts_with("ABR lowers it when the send queue backs up")));
        // A new stream info moves it
        let (i, d) = live(41);
        let v = codec_details_view(&i, &d);
        assert_eq!(text(value(&v, "Bitpool")), "41 (ABR 19–53)");
        assert!((bitpool(&v).bar.used - (41.0 / 64.0) * 100.0).abs() < 1e-9);
    }

    #[test]
    fn shows_a_fixed_bitpool_without_abr() {
        let d = sbc();
        let v = codec_details_view(&info(&d), &d);
        assert_eq!(text(value(&v, "Bitpool")), "51");
        let b = bitpool(&v);
        assert!(!b.live && b.bar.abr.is_none());
        assert!(!v.sections[0].hints.iter().any(|h| h.contains("ABR")));
    }
}
