//! Web pages the UI opens (the same ones v1 opened)

use crate::format::js_space;

macro_rules! repo {
    () => {
        "https://github.com/SeiyaFunaokaJP/BlueCaster"
    };
}

const REPO: &str = repo!();

/// The pages
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Links {
    pub repo: &'static str,
    pub setup_guide: &'static str,
    /// The chip script format (docs/chip-support.md, published)
    pub chip_scripts: &'static str,
    /// Realtek firmware: linux-firmware rtl_bt (v1 wx_firmware_dialog.cpp)
    pub firmware_realtek: &'static str,
    pub firmware_intel: &'static str,
    /// MediaTek: read from its Windows driver; linux-firmware has the same files
    pub firmware_media_tek: &'static str,
    /// Qualcomm: read from its Windows driver; linux-firmware has the same files
    pub firmware_qualcomm: &'static str,
    /// linux-firmware has no Broadcom .hcd; this collects them from the Windows drivers
    pub firmware_broadcom: &'static str,
    pub releases: &'static str,
    pub issues: &'static str,
    pub sponsor: &'static str,
    pub vb_cable: &'static str,
    /// VB-CABLE's origin, which VB-Audio asks to be named where it is
    /// installed with another application
    pub vb_cable_home: &'static str,
    pub vb_audio_home: &'static str,
    /// VB-Audio's terms for its donationware
    pub vb_audio_licensing: &'static str,
    /// Its download on VB-CABLE's page (HiFi-Cable & ASIO-Bridge)
    pub hifi_cable: &'static str,
}

pub const LINKS: Links = Links {
    repo: REPO,
    setup_guide: concat!(repo!(), "/blob/main/docs/setup.md"),
    chip_scripts: "https://seiyafunaokajp.github.io/BlueCaster/chip-support#chip-scripts",
    firmware_realtek: "https://git.kernel.org/pub/scm/linux/kernel/git/firmware/linux-firmware.git/tree/rtl_bt",
    firmware_intel: "https://git.kernel.org/pub/scm/linux/kernel/git/firmware/linux-firmware.git/tree/intel",
    firmware_media_tek: "https://git.kernel.org/pub/scm/linux/kernel/git/firmware/linux-firmware.git/tree/mediatek",
    firmware_qualcomm: "https://git.kernel.org/pub/scm/linux/kernel/git/firmware/linux-firmware.git/tree/qca",
    firmware_broadcom: "https://github.com/winterheart/broadcom-bt-firmware",
    releases: concat!(repo!(), "/releases"),
    issues: concat!(repo!(), "/issues/new/choose"),
    sponsor: "https://github.com/sponsors/SeiyaFunaokaJP",
    vb_cable: "https://vb-audio.com/Cable/",
    vb_cable_home: "https://vb-cable.com/",
    vb_audio_home: "https://vb-audio.com/",
    vb_audio_licensing: "https://vb-audio.com/Services/licensing.htm",
    hifi_cable: "https://vb-audio.com/Cable/#DownloadASIOBridge",
};

/// The third-party licences of this version (a development build has no
/// licenses folder next to it): the release tag's copy, not the main
/// branch's, which may list other components
pub fn third_party_url(version: &str) -> String {
    format!("{REPO}/blob/v{version}/THIRD_PARTY_LICENSES.md")
}

/// A query string as URLSearchParams builds it (keys in the order first set)
#[derive(Debug, Clone, Default)]
struct Query(Vec<(String, String)>);

impl Query {
    fn new(pairs: &[(&str, &str)]) -> Query {
        Query(pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect())
    }

    /// URLSearchParams.set: replaces the first, drops the others, else appends
    fn set(&mut self, key: &str, value: &str) {
        match self.0.iter().position(|(k, _)| k == key) {
            Some(i) => {
                self.0[i].1 = value.to_owned();
                let mut n = 0;
                self.0.retain(|(k, _)| {
                    n += usize::from(k == key);
                    k != key || n == 1
                });
            }
            None => self.0.push((key.to_owned(), value.to_owned())),
        }
    }

    fn get(&self, key: &str) -> Option<&str> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    /// application/x-www-form-urlencoded, as URLSearchParams.toString
    fn to_query(&self) -> String {
        let enc = |s: &str| {
            let mut out = String::with_capacity(s.len());
            for &b in s.as_bytes() {
                match b {
                    b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'*' | b'-' | b'.' | b'_' => out.push(b as char),
                    b' ' => out.push('+'),
                    _ => out.push_str(&format!("%{b:02X}")),
                }
            }
            out
        };
        self.0.iter().map(|(k, v)| format!("{}={}", enc(k), enc(v))).collect::<Vec<_>>().join("&")
    }
}

/// The length of encodeURIComponent(s)
fn uri_component_len(s: &str) -> usize {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || "-_.!~*'()".contains(c) { 1 } else { 3 * c.len_utf8() }).sum()
}

/// "Windows build 26100 (x86_64)" from the diagnostic info's
/// "OS : Windows build 26100 (x86_64)" line
fn windows_line(info: &str) -> Option<String> {
    let skip_space = |s: &str| -> usize { s.char_indices().find(|(_, c)| !js_space(*c)).map_or(s.len(), |(i, _)| i) };
    for (at, _) in info.match_indices("OS") {
        let mut rest = &info[at + 2..];
        rest = &rest[skip_space(rest)..];
        let Some(r) = rest.strip_prefix(':') else { continue };
        rest = &r[skip_space(r)..];
        let Some(r) = rest.strip_prefix("Windows build ") else { continue };
        let digits = r.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            continue;
        }
        let Some(r2) = r[digits..].strip_prefix(" (") else { continue };
        let Some(close) = r2.find(')').filter(|&c| c > 0) else { continue };
        return Some(format!("Windows build {} ({})", &r[..digits], &r2[..close]));
    }
    None
}

/// What a bug report is prefilled with (the issue form's field ids)
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BugReport {
    pub version: String,
    /// "copy diagnostic info" text (addresses masked): the Windows build
    /// comes from it, and as much of it as fits goes into the log field
    pub info: String,
    /// "RTL8761BU (2357:0604)"
    pub adapter: Option<String>,
    pub headphones: Option<String>,
    /// Profile.codec
    pub codec: Option<String>,
    /// Profile.capture_mode
    pub capture_mode: Option<String>,
    /// The error the app showed, as the start of "What happened?"
    pub what_happened: Option<String>,
    /// The bundle's file name; none when the report comes from an error
    pub bundle: Option<String>,
}

/// The issue form's dropdown options for a profile's codec
fn codec_option(codec: &str) -> Option<&'static str> {
    Some(match codec {
        "auto" => "Auto",
        "ldac" => "LDAC",
        "aptxhd" => "aptX HD",
        "aptxll" => "aptX Low Latency",
        "aptxad" => "aptX Adaptive",
        "aptx" => "aptX",
        "lhdcv5" => "LHDC V5",
        "opus" => "Opus",
        "opus05" => "Opus 05",
        "aac" => "AAC",
        "sbcxq" => "SBC XQ",
        "sbc" => "SBC",
        "faststream" => "FastStream",
        _ => return None,
    })
}

fn capture_option(mode: &str) -> Option<&'static str> {
    Some(match mode {
        "loopback" => "System Loopback",
        "virtual" => "Virtual Device",
        "app" => "Application",
        _ => return None,
    })
}

/// Characters of the (encoded) diagnostic info in the URL: GitHub refuses
/// very long URLs
const MAX_INFO_CHARS: usize = 6000;

/// Some(non-empty text)
fn given(s: &Option<String>) -> Option<&str> {
    s.as_deref().filter(|s| !s.is_empty())
}

/// A new bug report (.github/ISSUE_TEMPLATE/bug_report.yml) with what the
/// app knows filled in; the user still describes the problem and attaches
/// the bundle
pub fn bug_report_url(r: &BugReport) -> String {
    let mut q = Query::new(&[("template", "bug_report.yml"), ("version", &r.version), ("interface", "Window / tray")]);
    if let Some(os) = windows_line(&r.info) {
        q.set("windows", &os);
    }
    if let Some(a) = given(&r.adapter) {
        q.set("adapter", a);
    }
    if let Some(h) = given(&r.headphones) {
        q.set("headphones", h);
    }
    if let Some(codec) = given(&r.codec).and_then(codec_option) {
        q.set("codec", codec);
    }
    if let Some(capture) = given(&r.capture_mode).and_then(capture_option) {
        q.set("capture_mode", capture);
    }
    if let Some(w) = given(&r.what_happened) {
        q.set("what_happened", w);
    }
    if let Some(b) = given(&r.bundle) {
        q.set("bundle", &format!("{b}\n(drag the file here from the Explorer window BlueCaster opened)"));
    }
    // The field renders as a code block already: without the text's own fence
    let mut info = r.info.strip_prefix("```text\n").unwrap_or(&r.info).to_owned();
    let trimmed = info.trim_end_matches(js_space);
    if let Some(fenced) = trimmed.strip_suffix("```") {
        let body = fenced.strip_suffix('\n').unwrap_or(fenced);
        info = format!("{body}\n");
    }
    if uri_component_len(&info) > MAX_INFO_CHARS {
        let cut = "\n... (cut here: the full text is diagnostics.txt in the bundle)";
        let mut chars: Vec<char> = info.chars().collect();
        while !chars.is_empty() && uri_component_len(&chars.iter().collect::<String>()) > MAX_INFO_CHARS {
            chars.truncate(chars.len() * 9 / 10);
        }
        let keep = chars.iter().rposition(|&c| c == '\n').map_or(chars.len(), |i| i + 1);
        info = chars[..keep].iter().collect::<String>() + cut;
    }
    q.set("logs", &info);
    format!("{REPO}/issues/new?{}", q.to_query())
}

/// "a.zip" of "C:\x\a.zip": the file a report asks to attach
pub fn base_name(path: &str) -> &str {
    match path.rfind(['\\', '/']) {
        Some(i) => &path[i + 1..],
        None => path,
    }
}

/// What a chip report is prefilled with (.github/ISSUE_TEMPLATE/chip_report.yml)
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChipReport {
    pub version: String,
    /// "MediaTek (0x0E8D:0x0608) [unsupported]": the user adds where it came from
    pub adapter: String,
    /// The probe's summary line
    pub summary: String,
    /// "copy diagnostic info" text: only the Windows build is taken from it
    pub info: String,
    /// The probe report itself, as JSON
    pub report: String,
}

/// Characters of the whole query of a chip report: GitHub refuses very long
/// URLs (a bug report's info is cut at MAX_INFO_CHARS, plus the other fields)
const MAX_CHIP_QUERY: usize = 7000;

/// A chip report's link, and whether the report has to be attached
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChipReportLink {
    pub url: String,
    pub attach: bool,
}

/// A new chip report with what the probe read. The report itself goes into
/// the issue when the URL can carry it (attach: false); otherwise its field
/// stays empty, so GitHub refuses the issue until the file is attached
/// (attach: true). A file name in the field passed as an answer, and many
/// reports came without the file (#518).
pub fn chip_report_url(r: &ChipReport) -> ChipReportLink {
    let title = format!("Chip support: {}", r.summary.split(',').next().unwrap_or_default());
    let mut q = Query::new(&[
        ("template", "chip_report.yml"),
        ("title", &title),
        ("version", &r.version),
        ("adapter", &r.adapter),
        ("chip", &r.summary),
    ]);
    if let Some(os) = windows_line(&r.info) {
        q.set("windows", &os);
    }
    let mut attach = true;
    // Not JSON: the file is attached instead
    if serde_json::from_str::<serde_json::Value>(&r.report).is_ok() {
        // Readable when it fits, one line when only that fits
        for text in [json_layout(&r.report, true), json_layout(&r.report, false)] {
            let mut p = q.clone();
            p.set("probe", &format!("```json\n{text}\n```"));
            if p.to_query().len() <= MAX_CHIP_QUERY {
                let probe = p.get("probe").unwrap_or_default().to_owned();
                q.set("probe", &probe);
                attach = false;
                break;
            }
        }
    }
    ChipReportLink { url: format!("{REPO}/issues/new?{}", q.to_query()), attach }
}

/// Valid JSON laid out as JSON.stringify(value, null, 1) (`pretty`) or
/// JSON.stringify(value) does, keys in their order; numbers as JavaScript
/// writes them
fn json_layout(json: &str, pretty: bool) -> String {
    let chars: Vec<char> = json.chars().collect();
    let mut out = String::with_capacity(json.len());
    let mut depth = 0usize;
    let newline = |out: &mut String, depth: usize| {
        if pretty {
            out.push('\n');
            out.extend(std::iter::repeat_n(' ', depth));
        }
    };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '"' => {
                let start = i;
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    if chars[i] == '\\' {
                        i += 1;
                    }
                    i += 1;
                }
                let raw: String = chars[start..=i.min(chars.len() - 1)].iter().collect();
                // Re-escaped as JSON.stringify does
                match serde_json::from_str::<String>(&raw) {
                    Ok(s) => out.push_str(&serde_json::to_string(&s).unwrap_or(raw)),
                    Err(_) => out.push_str(&raw),
                }
                i += 1;
                continue;
            }
            '{' | '[' => {
                let close = if c == '{' { '}' } else { ']' };
                let next = chars[i + 1..].iter().position(|c| !c.is_whitespace()).map(|p| chars[i + 1 + p]);
                out.push(c);
                if next == Some(close) {
                    out.push(close);
                    i += 1 + chars[i + 1..].iter().position(|&x| x == close).unwrap_or(0) + 1;
                    continue;
                }
                depth += 1;
                newline(&mut out, depth);
            }
            '}' | ']' => {
                depth = depth.saturating_sub(1);
                newline(&mut out, depth);
                out.push(c);
            }
            ',' => {
                out.push(',');
                newline(&mut out, depth);
            }
            ':' => out.push_str(if pretty { ": " } else { ":" }),
            c if c.is_whitespace() => {}
            '-' | '0'..='9' => {
                let start = i;
                while i < chars.len() && matches!(chars[i], '-' | '+' | '.' | 'e' | 'E' | '0'..='9') {
                    i += 1;
                }
                let raw: String = chars[start..i].iter().collect();
                match raw.parse::<f64>() {
                    Ok(n) => out.push_str(&crate::i18n::js_number(n)),
                    Err(_) => out.push_str(&raw),
                }
                continue;
            }
            // true, false, null
            _ => out.push(c),
        }
        i += 1;
    }
    out
}

/// The firmware download page for an adapter's vendor
pub fn firmware_url(vendor: &str) -> &'static str {
    vendor_firmware_url(vendor).unwrap_or(LINKS.firmware_realtek)
}

/// The firmware page for a non-Realtek adapter's own files (v1: Intel's
/// ibt-* in linux-firmware, Broadcom's .hcd); None for vendors with none
pub fn vendor_firmware_url(vendor: &str) -> Option<&'static str> {
    let v = vendor.to_lowercase();
    if v.contains("intel") {
        Some(LINKS.firmware_intel)
    } else if v.contains("mediatek") {
        Some(LINKS.firmware_media_tek)
    } else if v.contains("qualcomm") {
        Some(LINKS.firmware_qualcomm)
    } else if v.contains("broadcom") {
        Some(LINKS.firmware_broadcom)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The query of a URL as URLSearchParams reads it
    fn params(url: &str) -> Vec<(String, String)> {
        let dec = |s: &str| {
            let b = s.as_bytes();
            let mut out = Vec::new();
            let mut i = 0;
            while i < b.len() {
                match b[i] {
                    b'+' => out.push(b' '),
                    b'%' => {
                        out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap());
                        i += 2;
                    }
                    c => out.push(c),
                }
                i += 1;
            }
            String::from_utf8(out).unwrap()
        };
        let query = url.split_once('?').map_or("", |(_, q)| q);
        query
            .split('&')
            .filter(|p| !p.is_empty())
            .map(|p| {
                let (k, v) = p.split_once('=').unwrap_or((p, ""));
                (dec(k), dec(v))
            })
            .collect()
    }

    fn get(url: &str, key: &str) -> Option<String> {
        params(url).into_iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    #[test]
    fn links_the_third_party_licences_of_the_release_tag() {
        assert_eq!(
            third_party_url("2.0.0-alpha.1"),
            "https://github.com/SeiyaFunaokaJP/BlueCaster/blob/v2.0.0-alpha.1/THIRD_PARTY_LICENSES.md"
        );
    }

    const INFO: &str = "```text\nBlueCaster diagnostics - 2026-10-22\nVersion      : 2.0.3\nOS           : Windows build 26100 (x86_64)\n```\n";

    fn base() -> BugReport {
        BugReport {
            version: "2.0.3".into(),
            info: INFO.into(),
            bundle: Some("bluecaster-diag-20261022_120000.zip".into()),
            ..Default::default()
        }
    }

    #[test]
    fn fills_the_issue_forms_fields_the_app_knows() {
        let url = bug_report_url(&BugReport {
            adapter: Some("RTL8761BU (2357:0604)".into()),
            headphones: Some("WH-1000XM5".into()),
            codec: Some("ldac".into()),
            capture_mode: Some("app".into()),
            ..base()
        });
        assert_eq!(url.split('?').next(), Some("https://github.com/SeiyaFunaokaJP/BlueCaster/issues/new"));
        assert_eq!(get(&url, "template").as_deref(), Some("bug_report.yml"));
        assert_eq!(get(&url, "version").as_deref(), Some("2.0.3"));
        assert_eq!(get(&url, "windows").as_deref(), Some("Windows build 26100 (x86_64)"));
        assert_eq!(get(&url, "adapter").as_deref(), Some("RTL8761BU (2357:0604)"));
        assert_eq!(get(&url, "headphones").as_deref(), Some("WH-1000XM5"));
        // The dropdowns take their option's text
        assert_eq!(get(&url, "codec").as_deref(), Some("LDAC"));
        assert_eq!(get(&url, "capture_mode").as_deref(), Some("Application"));
        assert_eq!(get(&url, "interface").as_deref(), Some("Window / tray"));
        assert!(get(&url, "bundle").unwrap().starts_with("bluecaster-diag-20261022_120000.zip\n"));
        // The log field is a code block already: no fence of its own
        assert_eq!(
            get(&url, "logs").as_deref(),
            Some(
                "BlueCaster diagnostics - 2026-10-22\nVersion      : 2.0.3\nOS           : Windows build 26100 (x86_64)\n"
            )
        );
    }

    #[test]
    fn leaves_out_what_it_does_not_know_and_cuts_a_long_log() {
        let long = INFO.to_owned() + &"  T+1.000 INFO bt_host: connecting to AA:BB:CC:xx:xx:xx\n".repeat(400);
        let url = bug_report_url(&BugReport { info: long, codec: Some("not-a-codec".into()), ..base() });
        assert_eq!(get(&url, "codec"), None);
        assert_eq!(get(&url, "adapter"), None);
        assert!(url.len() < 8000);
        let logs = get(&url, "logs").unwrap();
        assert!(logs.ends_with("\n... (cut here: the full text is diagnostics.txt in the bundle)"));
        assert!(logs.starts_with("BlueCaster diagnostics"));
    }

    #[test]
    fn lays_out_json_as_javascript() {
        let json = r#"{"b": [1, 2.0, {}], "a": {"x": "é\u00e9", "y": []}, "c": null}"#;
        assert_eq!(json_layout(json, false), r#"{"b":[1,2,{}],"a":{"x":"éé","y":[]},"c":null}"#);
        assert_eq!(
            json_layout(json, true),
            "{\n \"b\": [\n  1,\n  2,\n  {}\n ],\n \"a\": {\n  \"x\": \"éé\",\n  \"y\": []\n },\n \"c\": null\n}"
        );
        let link = chip_report_url(&ChipReport {
            version: "3.0.4".into(),
            adapter: "MediaTek (0x0E8D:0x0608) [unsupported]".into(),
            summary: "MT7921, ROM 0x8a".into(),
            info: INFO.into(),
            report: json.into(),
        });
        assert!(!link.attach);
        assert_eq!(get(&link.url, "title").as_deref(), Some("Chip support: MT7921"));
        assert_eq!(get(&link.url, "windows").as_deref(), Some("Windows build 26100 (x86_64)"));
        assert!(get(&link.url, "probe").unwrap().starts_with("```json\n{\n \"b\""));
        assert!(chip_report_url(&ChipReport { report: "not json".into(), ..Default::default() }).attach);
    }

    // The chip report's URL

    fn chip_base(report: String) -> ChipReport {
        ChipReport {
            version: "3.0.2".into(),
            adapter: "Barrot (0x33FA:0x0012)".into(),
            summary: "Barrot, USB 33FA:0012".into(),
            info: String::new(),
            report,
        }
    }

    #[test]
    fn puts_the_chip_report_in_the_issue_when_it_fits() {
        let link = chip_report_url(&chip_base(r#"{"steps":[{"answer":"0e0c"}]}"#.into()));
        assert!(!link.attach);
        assert_eq!(
            get(&link.url, "probe").as_deref(),
            Some("```json\n{\n \"steps\": [\n  {\n   \"answer\": \"0e0c\"\n  }\n ]\n}\n```")
        );
    }

    #[test]
    fn falls_back_to_one_line_then_to_an_empty_field_the_user_must_fill() {
        let steps = |n: usize| format!(r#"{{"steps":[{}]}}"#, vec![r#"{"a":"0e"}"#; n].join(","));
        // Too long indented, short enough on one line
        let mid = chip_report_url(&chip_base(steps(250)));
        assert!(!mid.attach);
        assert_eq!(get(&mid.url, "probe"), Some(format!("```json\n{}\n```", steps(250))));
        // Too long either way: GitHub refuses the issue until the file is attached
        let big = chip_report_url(&chip_base(format!(r#"{{"x":"{}"}}"#, "0".repeat(8000))));
        assert!(big.attach);
        assert_eq!(get(&big.url, "probe"), None);
        assert!(chip_report_url(&chip_base(String::new())).attach);
    }
}
