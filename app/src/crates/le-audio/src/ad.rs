//! Advertising data and UUIDs as text, for the probes (`--le-probe`): every
//! AD structure of an advertiser, the names of the services LE Audio and
//! its neighbours use, 128-bit UUIDs in their usual form.

use std::fmt::Write as _;

use crate::broadcast::{AD_BROADCAST_NAME, UUID_BROADCAST_AUDIO_ANNOUNCEMENT, UUID_PUBLIC_BROADCAST_ANNOUNCEMENT};
use crate::defs::*;

const AD_UUID128_INCOMPLETE: u8 = 0x06;
const AD_UUID128_COMPLETE: u8 = 0x07;
const AD_TX_POWER: u8 = 0x0A;
const AD_SERVICE_DATA_UUID128: u8 = 0x21;
const AD_MANUFACTURER: u8 = 0xFF;

/// The name of a 16-bit service UUID, "" for one not listed
pub fn service_name(uuid16: u16) -> &'static str {
    match uuid16 {
        0x1800 => "Generic Access",
        0x1801 => "Generic Attribute",
        0x180A => "Device Information",
        0x180F => "Battery",
        0x1812 => "HID",
        0x1843 => "Audio Input Control",
        0x1844 => "Volume Control",
        0x1845 => "Volume Offset Control",
        0x1846 => "Coordinated Set Identification",
        0x1848 => "Media Control",
        0x1849 => "Generic Media Control",
        0x184D => "Microphone Control",
        UUID_SERVICE_TBS => "Telephone Bearer",
        UUID_SERVICE_GTBS => "Generic Telephone Bearer",
        UUID_SERVICE_ASCS => "Audio Stream Control",
        UUID_SERVICE_BASS => "Broadcast Audio Scan (BASS)",
        UUID_SERVICE_PACS => "Published Audio Capabilities",
        UUID_BROADCAST_AUDIO_ANNOUNCEMENT => "Broadcast Audio Announcement",
        UUID_SERVICE_CAS => "Common Audio",
        UUID_SERVICE_HAS => "Hearing Access",
        UUID_SERVICE_TMAS => "Telephony and Media Audio",
        UUID_PUBLIC_BROADCAST_ANNOUNCEMENT => "Public Broadcast Announcement",
        UUID_SERVICE_GMAS => "Gaming Audio",
        0xFE2C => "Google Fast Pair",
        0xFE03 => "Amazon",
        _ => "",
    }
}

/// The name of a 16-bit characteristic UUID, "" for one not listed
pub fn char_name(uuid16: u16) -> &'static str {
    match uuid16 {
        UUID_CHR_DEVICE_NAME => "Device Name",
        UUID_CHR_APPEARANCE => "Appearance",
        UUID_CHR_SERVICE_CHANGED => "Service Changed",
        UUID_CHR_DATABASE_HASH => "Database Hash",
        0x2A19 => "Battery Level",
        0x2A24 => "Model Number",
        0x2A25 => "Serial Number",
        0x2A26 => "Firmware Revision",
        0x2A27 => "Hardware Revision",
        0x2A28 => "Software Revision",
        0x2A29 => "Manufacturer Name",
        0x2A50 => "PnP ID",
        UUID_CHR_BASS_CONTROL_POINT => "Broadcast Audio Scan Control Point",
        UUID_CHR_BROADCAST_RECEIVE_STATE => "Broadcast Receive State",
        UUID_CHR_SINK_ASE => "Sink ASE",
        UUID_CHR_SOURCE_ASE => "Source ASE",
        UUID_CHR_ASE_CONTROL_POINT => "ASE Control Point",
        UUID_CHR_SINK_PAC => "Sink PAC",
        UUID_CHR_SINK_AUDIO_LOCATIONS => "Sink Audio Locations",
        UUID_CHR_SOURCE_PAC => "Source PAC",
        UUID_CHR_SOURCE_AUDIO_LOCATIONS => "Source Audio Locations",
        UUID_CHR_AVAILABLE_AUDIO_CONTEXTS => "Available Audio Contexts",
        UUID_CHR_SUPPORTED_AUDIO_CONTEXTS => "Supported Audio Contexts",
        _ => "",
    }
}

/// A 128-bit UUID in its usual form; `bytes` most significant first
pub fn uuid128_str(bytes: &[u8; 16]) -> String {
    let mut s = String::with_capacity(36);
    for (i, b) in bytes.iter().enumerate() {
        if matches!(i, 4 | 6 | 8 | 10) {
            s.push('-');
        }
        let _ = write!(s, "{b:02X}");
    }
    s
}

/// A 128-bit UUID as advertising data carries it (least significant first)
fn uuid128_le_str(le: &[u8]) -> String {
    let mut be = [0u8; 16];
    for (d, s) in be.iter_mut().zip(le.iter().rev()) {
        *d = *s;
    }
    uuid128_str(&be)
}

fn hex(v: &[u8]) -> String {
    v.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ")
}

fn uuid16_named(u: u16) -> String {
    match service_name(u) {
        "" => format!("0x{u:04X}"),
        n => format!("0x{u:04X} {n}"),
    }
}

/// One line per AD structure of `data` (advertising or scan response data);
/// a structure that runs past the end is shown as such
pub fn describe_ad(data: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < data.len() {
        let len = data[i] as usize;
        if len == 0 {
            i += 1;
            continue;
        }
        let Some(item) = data.get(i + 1..i + 1 + len) else {
            out.push(format!("cut short: {}", hex(&data[i..])));
            break;
        };
        i += 1 + len;
        let (ty, v) = (item[0], &item[1..]);
        let rd16 = |p: &[u8]| u16::from_le_bytes([p[0], p[1]]);
        out.push(match ty {
            AD_FLAGS if !v.is_empty() => format!("flags 0x{:02X}", v[0]),
            AD_UUID16_INCOMPLETE | AD_UUID16_COMPLETE => format!(
                "services{}: {}",
                if ty == AD_UUID16_INCOMPLETE { " (incomplete)" } else { "" },
                v.as_chunks::<2>()
                    .0
                    .iter()
                    .map(|u| uuid16_named(u16::from_le_bytes(*u)))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            AD_UUID128_INCOMPLETE | AD_UUID128_COMPLETE => format!(
                "128-bit services{}: {}",
                if ty == AD_UUID128_INCOMPLETE { " (incomplete)" } else { "" },
                v.as_chunks::<16>().0.iter().map(|u| uuid128_le_str(u)).collect::<Vec<_>>().join(", ")
            ),
            AD_NAME_SHORT | AD_NAME_COMPLETE => format!(
                "{} name \"{}\"",
                if ty == AD_NAME_SHORT { "short" } else { "complete" },
                String::from_utf8_lossy(v)
            ),
            AD_BROADCAST_NAME => format!("broadcast name \"{}\"", String::from_utf8_lossy(v)),
            AD_TX_POWER if !v.is_empty() => format!("TX power {} dBm", v[0] as i8),
            AD_APPEARANCE if v.len() >= 2 => format!("appearance 0x{:04X}", rd16(v)),
            AD_SERVICE_DATA_UUID16 if v.len() >= 2 => {
                format!("service data {}: {}", uuid16_named(rd16(v)), hex(&v[2..]))
            }
            AD_SERVICE_DATA_UUID128 if v.len() >= 16 => {
                format!("service data {}: {}", uuid128_le_str(&v[..16]), hex(&v[16..]))
            }
            AD_MANUFACTURER if v.len() >= 2 => format!("manufacturer 0x{:04X}: {}", rd16(v), hex(&v[2..])),
            AD_RSI => format!("set member (RSI) {}", hex(v)),
            _ => format!("type 0x{ty:02X}: {}", hex(v)),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_ad_structure_is_described() {
        let mut ad = vec![2, AD_FLAGS, 0x06];
        ad.extend([5, AD_UUID16_COMPLETE, 0x4F, 0x18, 0x0F, 0x18]);
        ad.extend([5, AD_SERVICE_DATA_UUID16, 0x4F, 0x18, 0xAA, 0xBB]);
        ad.extend([4, AD_NAME_COMPLETE, b'G', b'O', b'4']);
        ad.extend([4, AD_MANUFACTURER, 0x57, 0x00, 0x01]);
        let mut u = vec![17, AD_UUID128_COMPLETE];
        u.extend((0u8..16).rev());
        ad.extend(u);
        ad.extend([2, 0x42, 0x99]);
        assert_eq!(
            describe_ad(&ad),
            vec![
                "flags 0x06",
                "services: 0x184F Broadcast Audio Scan (BASS), 0x180F Battery",
                "service data 0x184F Broadcast Audio Scan (BASS): AA BB",
                "complete name \"GO4\"",
                "manufacturer 0x0057: 01",
                "128-bit services: 00010203-0405-0607-0809-0A0B0C0D0E0F",
                "type 0x42: 99",
            ]
        );
    }

    #[test]
    fn a_structure_past_the_end_is_cut_short() {
        assert_eq!(describe_ad(&[0, 5, AD_NAME_COMPLETE, b'a']), vec!["cut short: 05 09 61"]);
    }
}
