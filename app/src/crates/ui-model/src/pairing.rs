//! The device's Services dialog (issue #86): names of well-known service
//! UUIDs, the Bluetooth SIG's (classic and LE GATT services) and some vendors'.

use ipc::DeviceService;

const BASE: &str = "-0000-1000-8000-00805f9b34fb";

/// Bluetooth SIG names of the 16-bit service class UUIDs devices list
fn sig_name(n: u32) -> Option<&'static str> {
    Some(match n {
        0x1101 => "Serial Port",
        0x1105 => "OBEX Object Push",
        0x1106 => "OBEX File Transfer",
        0x1108 => "Headset",
        0x110a => "Audio Source",
        0x110b => "Audio Sink",
        0x110c => "A/V Remote Control Target",
        0x110d => "Advanced Audio Distribution",
        0x110e => "A/V Remote Control",
        0x110f => "A/V Remote Control Controller",
        0x1112 => "Headset Audio Gateway",
        0x1115 => "PANU",
        0x1116 => "NAP",
        0x111e => "Handsfree",
        0x111f => "Handsfree Audio Gateway",
        0x1124 => "Human Interface Device",
        0x112f => "Phonebook Access Server",
        0x1131 => "Headset HS",
        0x1132 => "Message Access Server",
        0x1200 => "PnP Information",
        0x1203 => "Generic Audio",
        0x1800 => "Generic Access",
        0x1801 => "Generic Attribute",
        0x180a => "Device Information",
        0x180f => "Battery",
        0x1812 => "Human Interface Device",
        // LE Audio and its neighbours
        0x1843 => "Audio Input Control",
        0x1844 => "Volume Control",
        0x1845 => "Volume Offset Control",
        0x1846 => "Coordinated Set Identification",
        0x1848 => "Media Control",
        0x1849 => "Generic Media Control",
        0x184b => "Telephone Bearer",
        0x184c => "Generic Telephone Bearer",
        0x184d => "Microphone Control",
        0x184e => "Audio Stream Control",
        0x184f => "Broadcast Audio Scan",
        0x1850 => "Published Audio Capabilities",
        0x1853 => "Common Audio",
        0x1854 => "Hearing Access",
        0x1855 => "Telephony and Media Audio",
        0x1858 => "Gaming Audio",
        // Members' 16-bit UUIDs (issue #86, as LE Audio earbuds list them)
        0xfd92 => "Qualcomm Service",
        0xfe2c => "Google Fast Pair",
        _ => return None,
    })
}

/// Vendors' 128-bit service UUIDs that devices list without a name (issue
/// #86): Qualcomm GAIA on many Qualcomm-based headsets, Google Fast Pair
const VENDOR_NAMES: &[(&str, &str)] = &[
    ("00001100-d102-11e1-9b23-00025b00a5a5", "Qualcomm GAIA Root Service"),
    ("00001101-d102-11e1-9b23-00025b00a5a5", "Qualcomm GAIA Command/Response"),
    ("00001102-d102-11e1-9b23-00025b00a5a5", "Qualcomm GAIA Data"),
    ("00001103-d102-11e1-9b23-00025b00a5a5", "Qualcomm GAIA Notification"),
    ("0000eb03-d102-11e1-9b23-00025b00a5a5", "Qualcomm GAIA Extension"),
    ("0000eb04-d102-11e1-9b23-00025b00a5a5", "Qualcomm GAIA Extension"),
    ("0000eb05-d102-11e1-9b23-00025b00a5a5", "Qualcomm GAIA Extension"),
    ("0000eb06-d102-11e1-9b23-00025b00a5a5", "Qualcomm GAIA Extension"),
    ("0000eb07-d102-11e1-9b23-00025b00a5a5", "Qualcomm GAIA Extension"),
    ("0000eb10-d102-11e1-9b23-00025b00a5a5", "Qualcomm GAIA OTA/Debug Log"),
    ("df21fe2c-2515-4fdb-8886-f12c4d67927c", "Google Fast Pair"),
    // LE Audio earbuds' (issue #86, names as a tester reported them)
    ("2587db3c-ce70-4fc9-935f-777ab4188fd7", "Advanced Audio Recording Service"),
    ("12994b7e-6d47-4215-8c9e-aae9a1095ba3", "Advanced Audio Gaming Service"),
];

/// The 16-bit form of a UUID of the Bluetooth base, else None
pub fn short_uuid(uuid: &str) -> Option<u32> {
    let u = uuid.to_lowercase();
    if !u.ends_with(BASE) || !u.starts_with("0000") {
        return None;
    }
    // parseInt: the hex digits at the start of the four characters
    let digits: String = u.chars().skip(4).take(4).take_while(char::is_ascii_hexdigit).collect();
    u32::from_str_radix(&digits, 16).ok()
}

/// The SIG name of a service UUID, None for one not in the list (a vendor's)
pub fn service_name(uuid: &str) -> Option<&'static str> {
    short_uuid(uuid).and_then(sig_name)
}

/// A vendor's name of a 128-bit service UUID, None for one not in the list
pub fn vendor_service_name(uuid: &str) -> Option<&'static str> {
    let u = uuid.to_lowercase();
    VENDOR_NAMES.iter().find(|(id, _)| *id == u).map(|(_, name)| *name)
}

/// A service's name: the SIG name, else the device's own name, else a
/// vendor's known name, else None (an unknown service). One name, for the
/// Name column (issue #86)
pub fn service_label(s: &DeviceService) -> Option<String> {
    s.uuids
        .iter()
        .find_map(|u| service_name(u))
        .map(str::to_owned)
        .or_else(|| s.name.clone().filter(|n| !n.is_empty()))
        .or_else(|| s.uuids.iter().find_map(|u| vendor_service_name(u)).map(str::to_owned))
}

// The pairing dialog's helpers
#[cfg(test)]
mod tests {
    use super::*;

    fn svc(uuids: &[&str], name: Option<&str>) -> DeviceService {
        DeviceService { uuids: uuids.iter().map(|u| (*u).to_owned()).collect(), name: name.map(str::to_owned) }
    }

    fn label(uuids: &[&str], name: Option<&str>) -> Option<String> {
        service_label(&svc(uuids, name))
    }

    #[test]
    fn names_the_sig_services_and_leaves_vendors_alone() {
        assert_eq!(service_name("0000110B-0000-1000-8000-00805F9B34FB"), Some("Audio Sink"));
        assert_eq!(service_name("0000111e-0000-1000-8000-00805f9b34fb"), Some("Handsfree"));
        assert_eq!(service_name("0000eb04-d102-11e1-9b23-00025b00a5a5"), None);
        assert_eq!(service_name("0000fff0-0000-1000-8000-00805f9b34fb"), None);
        assert_eq!(short_uuid("df21fe2c-2515-4fdb-8886-f12c4d67927c"), None);
        assert_eq!(short_uuid("0000180f-0000-1000-8000-00805f9b34fb"), Some(0x180f));
    }

    #[test]
    fn gives_a_service_one_name() {
        let sink = "0000110b-0000-1000-8000-00805f9b34fb";
        let spp = "00001101-0000-1000-8000-00805f9b34fb";
        assert_eq!(label(&[sink], Some("Audio Sink")).as_deref(), Some("Audio Sink"));
        assert_eq!(label(&[spp], Some("GAIA")).as_deref(), Some("Serial Port"));
        assert_eq!(label(&["df21fe2c-2515-4fdb-8886-f12c4d67927c"], Some("Vendor")).as_deref(), Some("Vendor"));
        assert_eq!(label(&["0000fff0-1234-5678-9abc-def012345678"], None), None);
    }

    #[test]
    fn names_qualcomm_gaia_and_fast_pair_when_the_device_gives_no_name() {
        let gaia = "0000EB04-D102-11E1-9B23-00025B00A5A5";
        let fast_pair = "df21fe2c-2515-4fdb-8886-f12c4d67927c";
        assert_eq!(label(&[gaia], None).as_deref(), Some("Qualcomm GAIA Extension"));
        assert_eq!(
            label(&["00001101-d102-11e1-9b23-00025b00a5a5"], Some("")).as_deref(),
            Some("Qualcomm GAIA Command/Response")
        );
        assert_eq!(label(&[fast_pair], None).as_deref(), Some("Google Fast Pair"));
        assert_eq!(label(&[fast_pair], Some("Vendor")).as_deref(), Some("Vendor"));
    }

    #[test]
    fn names_the_services_le_audio_earbuds_list() {
        assert_eq!(label(&["0000FE2C-0000-1000-8000-00805F9B34FB"], None).as_deref(), Some("Google Fast Pair"));
        assert_eq!(label(&["0000fd92-0000-1000-8000-00805f9b34fb"], None).as_deref(), Some("Qualcomm Service"));
        assert_eq!(
            label(&["2587db3c-ce70-4fc9-935f-777ab4188fd7"], None).as_deref(),
            Some("Advanced Audio Recording Service")
        );
        assert_eq!(
            label(&["12994B7E-6D47-4215-8C9E-AAE9A1095BA3"], None).as_deref(),
            Some("Advanced Audio Gaming Service")
        );
    }
}
