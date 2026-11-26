//! The 2.4 GHz band as the Radio tab draws it: Bluetooth channel n is at
//! 2402 + n MHz (79 channels), a Wi-Fi channel spreads +-11 MHz around its
//! center (the same width bt-host's afh module avoids).

pub const BT_CHANNELS: usize = 79;
pub const BT_FIRST_MHZ: u32 = 2402;
pub const WIFI_HALF_WIDTH_MHZ: u32 = 11;
/// Wi-Fi channels that do not overlap each other
pub const WIFI_CLEAR_CHANNELS: [u32; 3] = [1, 6, 11];

/// Center frequency of a 2.4 GHz Wi-Fi channel (1-14)
pub fn wifi_center_mhz(channel: u32) -> u32 {
    if channel == 14 { 2484 } else { 2407 + 5 * channel }
}

/// The link's channel map (10 bytes, bit n = channel n) as 79 flags: true
/// = the link hops over it. None without a reading.
pub fn channels_in_use(map: Option<&[u8]>) -> Option<Vec<bool>> {
    let map = map.filter(|m| m.len() >= 10)?;
    Some((0..BT_CHANNELS).map(|n| map[n >> 3] & (1 << (n & 7)) != 0).collect())
}

/// Bluetooth channels a Wi-Fi channel covers
pub fn wifi_covers(channel: u32) -> Vec<usize> {
    let c = wifi_center_mhz(channel);
    (0..BT_CHANNELS).filter(|&n| (BT_FIRST_MHZ + n as u32).abs_diff(c) <= WIFI_HALF_WIDTH_MHZ).collect()
}

/// "6, 11" -> [6, 11]
pub fn parse_wifi_channels(s: &str) -> Vec<u32> {
    s.split(|c: char| !c.is_ascii_digit())
        .filter(|run| !run.is_empty())
        .filter_map(|run| run.parse::<u32>().ok().or(Some(u32::MAX)))
        .filter(|n| (1..=14).contains(n))
        .collect()
}

/// The non-overlapping Wi-Fi channel (1, 6 or 11) whose Bluetooth channels
/// the link leaves out most, when it leaves out at least half of them:
/// where the interference most likely is.
pub fn busiest_wifi_channel(in_use: Option<&[bool]>) -> Option<u32> {
    let in_use = in_use?;
    let mut best = None;
    let mut best_share = 0.5;
    for ch in WIFI_CLEAR_CHANNELS {
        let covered = wifi_covers(ch);
        let left_out = covered.iter().filter(|&&n| !in_use.get(n).copied().unwrap_or(false)).count();
        let share = left_out as f64 / covered.len() as f64;
        if share >= best_share {
            best = Some(ch);
            best_share = share;
        }
    }
    best
}

/// Height (0-1) of an access point drawn at `rssi` dBm: -90 dBm and
/// quieter at the floor, -30 dBm at the top
pub fn ap_level(rssi: f64) -> f64 {
    ((rssi + 90.0) / 60.0).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [u8; 10] = [0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f];

    #[test]
    fn reads_the_channel_map_bit_by_bit() {
        assert_eq!(channels_in_use(None), None);
        assert_eq!(channels_in_use(Some(&[1, 2])), None);
        let all = channels_in_use(Some(&ALL)).unwrap();
        assert_eq!(all.len(), 79);
        assert!(all.iter().all(|&b| b));
        let m = channels_in_use(Some(&[0xfe, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x3f])).unwrap();
        assert!(!m[0]);
        assert!(m[1]);
        assert!(!m[78]);
    }

    #[test]
    fn places_wifi_channels_as_bt_hosts_afh_module_does() {
        assert_eq!(wifi_center_mhz(1), 2412);
        assert_eq!(wifi_center_mhz(14), 2484);
        // Channel 6: 2426-2448 MHz = Bluetooth channels 24-46
        let six = wifi_covers(6);
        assert_eq!(six[0], 24);
        assert_eq!(six[six.len() - 1], 46);
        assert_eq!(parse_wifi_channels("6, 11"), [6, 11]);
        assert_eq!(parse_wifi_channels("0;15;3"), [3]);
        assert_eq!(parse_wifi_channels("99999999999999999999, 1"), [1]);
    }

    #[test]
    fn names_the_wifi_channel_the_link_leaves_out_most() {
        assert_eq!(busiest_wifi_channel(None), None);
        assert_eq!(busiest_wifi_channel(channels_in_use(Some(&ALL)).as_deref()), None);
        let mut in_use = channels_in_use(Some(&ALL)).unwrap();
        for n in wifi_covers(11) {
            in_use[n] = false;
        }
        assert_eq!(busiest_wifi_channel(Some(&in_use)), Some(11));
        // Under half of a channel's width left out is not named
        let mut few = channels_in_use(Some(&ALL)).unwrap();
        for n in wifi_covers(1).into_iter().take(5) {
            few[n] = false;
        }
        assert_eq!(busiest_wifi_channel(Some(&few)), None);
    }

    #[test]
    fn scales_signal_to_the_drawing() {
        assert_eq!(ap_level(-95.0), 0.0);
        assert!((ap_level(-60.0) - 0.5).abs() < 0.005);
        assert_eq!(ap_level(-20.0), 1.0);
    }
}
