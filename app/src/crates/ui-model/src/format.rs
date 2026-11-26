//! Small display helpers (copying text stays with the window: it needs the
//! clipboard)

use std::sync::Mutex;

/// AVRCP absolute volume (0..127) as a percentage
pub fn volume_percent(v: u32) -> u32 {
    ((v as f64 * 100.0) / 127.0).round() as u32
}

pub fn percent_to_volume(p: u32) -> u32 {
    ((p as f64 * 127.0) / 100.0).round() as u32
}

pub fn kbps(bytes_per_second: f64) -> f64 {
    (bytes_per_second * 8.0) / 1000.0
}

/// A sample rate in kHz: "48", "44.1"
pub fn khz(hz: u32) -> String {
    let k = hz as f64 / 1000.0;
    if k.fract() == 0.0 { format!("{}", k as u32) } else { format!("{k:.1}") }
}

/// Core uptime in ms as h:mm:ss.mmm
pub fn clock(ms: u64) -> String {
    let h = ms / 3_600_000;
    let m = (ms / 60_000) % 60;
    let s = (ms / 1000) % 60;
    let f = ms % 1000;
    format!("{h}:{m:02}:{s:02}.{f:03}")
}

/// What JavaScript's `\s` matches (and `String.trim` removes)
pub(crate) fn js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}' | '\u{2028}' | '\u{2029}' | '\u{202F}' | '\u{205F}' | '\u{3000}' | '\u{FEFF}'
    )
}

/// JavaScript's `String.trim`
pub(crate) fn js_trim(s: &str) -> &str {
    s.trim_matches(js_space)
}

/// The app's own folders: a path is kept from one of these on
const APP_FOLDERS: &[&str] = &["bluecaster", "a2dpwb", "a2dp windows bridge"];
const NOBODY: &[&str] = &["public", "default", "default user", "all users"];

/// Names and addresses seen so far -> their number: the same one the same
/// `<device-N>` / `<addr-N>` while the window is open. The adapters' own
/// addresses (12 upper-case hex digits): `<adapter>`.
#[derive(Debug, Default)]
pub struct Masker {
    /// Names seen, numbered from 1 in this order
    names: Vec<String>,
    /// Addresses seen (12 upper-case hex digits), numbered from 1
    addrs: Vec<String>,
    /// The adapters' own addresses (12 upper-case hex digits)
    adapters: Vec<String>,
}

/// The one masker of the window ([`mask_identifiers`], [`know_adapters`])
static MASKER: Mutex<Masker> = Mutex::new(Masker::new());

/// The 12 hex digits of an address in any form, upper case
fn address_key(addr: &str) -> String {
    addr.chars().filter(|c| !matches!(c, ':' | '-')).map(|c| c.to_ascii_uppercase()).collect()
}

/// The adapters' own addresses, shown as `<adapter>` by [`mask_identifiers`]
/// (Snapshot.adapter.adapters[].address)
pub fn know_adapters<I, S>(addresses: I)
where
    I: IntoIterator<Item = Option<S>>,
    S: AsRef<str>,
{
    let mut m = MASKER.lock().unwrap_or_else(|e| e.into_inner());
    for a in addresses.into_iter().flatten() {
        m.know_adapter(a.as_ref());
    }
}

/// Masks what identifies the user, the PC and their devices, as the core's
/// mask_identifiers: device names in typographic quotes (“...”, how the core
/// logs them) -> "<device-N>", paths -> "<path>" and the part from the app's
/// own folder on ("<path>\BlueCaster\logs\x.log") or their last part alone
/// ("<path>\bluecaster-core.exe"), the Windows user name in a path without a
/// drive ("\Users\<name>\...") -> x's, Windows account SIDs (the IPC pipe's
/// name) -> "<sid>", Bluetooth addresses "AC:80:0A:12:34:56"
/// (or with '-', or the 12 hex digits alone: Windows' Bluetooth device IDs,
/// USB serials that are the adapter's address) -> "<addr-N>", the same
/// address the same N whatever its form, the adapters' own ([`know_adapters`])
/// -> "<adapter>"; nothing of an address stays, as its first half names the
/// maker. Other USB serial numbers -> x's. IDs Windows generates
/// ("5&2C7B4E3&0&2") stay.
pub fn mask_identifiers(text: &str) -> String {
    MASKER.lock().unwrap_or_else(|e| e.into_inner()).mask(text)
}

impl Masker {
    pub const fn new() -> Masker {
        Masker { names: Vec::new(), addrs: Vec::new(), adapters: Vec::new() }
    }

    /// An adapter's own address, shown as `<adapter>` (any form)
    pub fn know_adapter(&mut self, addr: &str) {
        let key = address_key(addr);
        if key.len() == 12 && key.bytes().all(|c| c.is_ascii_hexdigit()) && !self.adapters.contains(&key) {
            self.adapters.push(key);
        }
    }

    fn pseudonym(&mut self, name: &str) -> String {
        let n = match self.names.iter().position(|n| n == name) {
            Some(i) => i + 1,
            None => {
                self.names.push(name.to_owned());
                self.names.len()
            }
        };
        format!("<device-{n}>")
    }

    fn address_pseudonym(&mut self, addr: &str) -> String {
        let key = address_key(addr);
        if self.adapters.contains(&key) {
            return "<adapter>".into();
        }
        let n = match self.addrs.iter().position(|a| *a == key) {
            Some(i) => i + 1,
            None => {
                self.addrs.push(key);
                self.addrs.len()
            }
        };
        format!("<addr-{n}>")
    }

    /// See [`mask_identifiers`]
    pub fn mask(&mut self, text: &str) -> String {
        let out = self.mask_quoted_names(text);
        let out = mask_user_paths(&out);
        // After the user name: a path that ends in the user's folder keeps that part
        let out = mask_drive_paths(&out);
        let out = mask_sids(&out);
        // Before the addresses: a serial that is an address is left to them
        let out = mask_usb_serials(&out);
        let out = self.mask_separated(&out, b':');
        let out = self.mask_separated(&out, b'-');
        self.mask_bare(&out)
    }

    /// A device name as the core logs it: “Taro's AirPods” (one line, up to
    /// 256 characters) -> `<device-N>`
    fn mask_quoted_names(&mut self, text: &str) -> String {
        const OPEN: char = '\u{201C}';
        const CLOSE: char = '\u{201D}';
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(start) = rest.find(OPEN) {
            let after = &rest[start + OPEN.len_utf8()..];
            let end = after.find([CLOSE, OPEN, '\n']).filter(|&e| after[e..].starts_with(CLOSE));
            match end {
                Some(e) if e > 0 && after[..e].encode_utf16().count() <= 256 => {
                    out.push_str(&rest[..start]);
                    out.push_str(&self.pseudonym(&after[..e]));
                    rest = &after[e + CLOSE.len_utf8()..];
                }
                _ => {
                    out.push_str(&rest[..start + OPEN.len_utf8()]);
                    rest = after;
                }
            }
        }
        out.push_str(rest);
        out
    }

    /// "AC:80:0A:12:34:56" or with '-'; not in a longer run (IPv6,
    /// "...:55:66"), a letter before or a colon after ("AC:80:0A:12:34:56:
    /// pairing") is fine
    fn mask_separated(&mut self, text: &str, sep: u8) -> String {
        const LEN: usize = 17;
        let b = text.as_bytes();
        let hex = |at: usize| b.get(at).is_some_and(u8::is_ascii_hexdigit);
        let mut out = String::with_capacity(text.len());
        let mut i = 0;
        while i < b.len() {
            let found = i + LEN <= b.len()
                && (0..LEN).all(|k| if k % 3 == 2 { b[i + k] == sep } else { b[i + k].is_ascii_hexdigit() })
                && (i == 0 || b[i - 1] != sep)
                && !hex(i + LEN)
                && !(b.get(i + LEN) == Some(&sep) && hex(i + LEN + 1) && hex(i + LEN + 2));
            if found {
                out.push_str(&self.address_pseudonym(&text[i..i + LEN]));
                i += LEN;
                continue;
            }
            let c = text[i..].chars().next().expect("at a char boundary");
            out.push(c);
            i += c.len_utf8();
        }
        out
    }

    /// A word of 12 hex digits, not a GUID's last part
    fn mask_bare(&mut self, text: &str) -> String {
        let b = text.as_bytes();
        let mut out = String::with_capacity(text.len());
        let mut i = 0;
        while i < b.len() {
            if b[i].is_ascii_alphanumeric() && (i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'-')) {
                let mut j = i;
                while j < b.len() && b[j].is_ascii_alphanumeric() {
                    j += 1;
                }
                if j - i == 12 && b[i..j].iter().all(u8::is_ascii_hexdigit) {
                    out.push_str(&self.address_pseudonym(&text[i..j]));
                    i = j;
                    continue;
                }
                // Any other word as it is
                out.push_str(&text[i..j]);
                i = j;
                continue;
            }
            let c = text[i..].chars().next().expect("at a char boundary");
            out.push(c);
            i += c.len_utf8();
        }
        out
    }
}

/// What a file or user name cannot contain, and where a path stops
fn name_ends(c: u8) -> bool {
    c < 0x20 || b"\\/:*?\"<>|".contains(&c)
}

fn is_sep(c: u8) -> bool {
    c == b'\\' || c == b'/'
}

/// `\Users\<name>\` (or '/', JSON's doubled '\'): the name, up to what a
/// name cannot contain, as x's; Public, Default and All Users are nobody's
fn mask_user_paths(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < b.len() {
        if is_sep(b[i])
            && b.get(i + 1..i + 6).is_some_and(|w| w.eq_ignore_ascii_case(b"Users"))
            && b.get(i + 6).is_some_and(|&c| is_sep(c))
        {
            let mut j = i + 6;
            while j < b.len() && is_sep(b[j]) {
                j += 1;
            }
            let start = j;
            while j < b.len() && !name_ends(b[j]) {
                j += 1;
            }
            let name = &text[start..j];
            if !name.is_empty() {
                out.push_str(&text[i..start]);
                if NOBODY.contains(&name.to_lowercase().as_str()) {
                    out.push_str(name);
                } else {
                    // As many x's as UTF-16 units, as the UI had it
                    out.extend(std::iter::repeat_n('x', name.encode_utf16().count()));
                }
                i = j;
                continue;
            }
        }
        let c = text[i..].chars().next().expect("at a char boundary");
        out.push(c);
        i += c.len_utf8();
    }
    out
}

/// `X:\` (or '/') at `i`, not in a word
fn path_start_at(b: &[u8], i: usize) -> bool {
    b.get(i).is_some_and(u8::is_ascii_alphabetic)
        && b.get(i + 1) == Some(&b':')
        && b.get(i + 2).is_some_and(|&c| is_sep(c))
        && (i == 0 || !b[i - 1].is_ascii_alphanumeric())
}

/// A drive path, X:\ (or '/'), not in a word: its parts, each after
/// separators; a part stops at what a file name cannot contain, or at a space
/// that starts another path, also one in quotes (BTstack's "firmware '<path>'
/// and config '<path>'", issue #86). -> `<path>` and the part from the app's
/// own folder on, else from the last part that is not empty.
fn mask_drive_paths(text: &str) -> String {
    let b = text.as_bytes();
    // A space at `j` that starts another path
    let next_path = |j: usize| {
        if !text.is_char_boundary(j) {
            return false;
        }
        let Some(c) = text[j..].chars().next().filter(|&c| js_space(c)) else { return false };
        let k = j + c.len_utf8();
        path_start_at(b, k) || (matches!(b.get(k), Some(b'\'' | b'"')) && path_start_at(b, k + 1))
    };
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < b.len() {
        if path_start_at(b, i) {
            // (the separators before it, the part): the part may be empty
            // after a trailing separator
            let mut parts: Vec<(usize, usize, usize)> = Vec::new();
            let mut j = i + 2;
            loop {
                let seps = j;
                while j < b.len() && is_sep(b[j]) {
                    j += 1;
                }
                let start = j;
                while j < b.len() && !name_ends(b[j]) && !next_path(j) {
                    j += 1;
                }
                parts.push((seps, start, j));
                if !(j < b.len() && is_sep(b[j])) {
                    break;
                }
            }
            let keep = parts
                .iter()
                .position(|&(_, s, e)| APP_FOLDERS.contains(&text[s..e].to_lowercase().as_str()))
                .or_else(|| parts.iter().rposition(|&(_, s, e)| e > s))
                .unwrap_or(parts.len() - 1);
            out.push_str("<path>");
            out.push_str(&text[parts[keep].0..j]);
            i = j;
            continue;
        }
        let c = text[i..].chars().next().expect("at a char boundary");
        out.push(c);
        i += c.len_utf8();
    }
    out
}

/// A SID with a machine part, S-1-5-21-a-b-c-rid (five or more numbers);
/// not in a longer word (a '-' on either side is fine: the pipe's
/// bluecaster-<sid>-<instance>) -> `<sid>`
fn mask_sids(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < b.len() {
        if b[i..].starts_with(b"S-1") && (i == 0 || !b[i - 1].is_ascii_alphanumeric()) {
            let mut j = i + 3;
            let mut groups = 0;
            while j < b.len() && b[j] == b'-' && b.get(j + 1).is_some_and(u8::is_ascii_digit) {
                j += 1;
                while j < b.len() && b[j].is_ascii_digit() {
                    j += 1;
                }
                groups += 1;
            }
            if groups >= 5 && !b.get(j).is_some_and(u8::is_ascii_alphanumeric) {
                out.push_str("<sid>");
                i = j;
                continue;
            }
        }
        let c = text[i..].chars().next().expect("at a char boundary");
        out.push(c);
        i += c.len_utf8();
    }
    out
}

/// `VID_xxxx<joint>PID_xxxx` at `i` (any case) -> the joint
fn vid_pid_at(b: &[u8], i: usize) -> Option<u8> {
    let hex4 = |at: usize| b.get(at..at + 4).is_some_and(|h| h.iter().all(u8::is_ascii_hexdigit));
    let joint = *b.get(i + 8)?;
    (b.get(i..i + 4)?.eq_ignore_ascii_case(b"VID_")
        && hex4(i + 4)
        && (joint == b'&' || joint == b'_')
        && b.get(i + 9..i + 13)?.eq_ignore_ascii_case(b"PID_")
        && hex4(i + 13))
    .then_some(joint)
}

/// USB\VID_0BDA&PID_8771\<serial>, a device path's #<serial>#, and a driver
/// record's name, USB_VID_0BDA_PID_8771_<serial>: the serial as x's, unless
/// Windows generated it (an instance ID's "5&2C7B4E3&0&2") or it is an
/// address (12 hex digits, left to the addresses)
fn mask_usb_serials(text: &str) -> String {
    // \\#\s"',;:.()[]{} and anything not ASCII
    let ends = |c: u8| {
        !c.is_ascii() || matches!(c, b'\t' | b'\n' | 0x0B | 0x0C | b'\r' | b' ') || b"\\#\"',;:.()[]{}".contains(&c)
    };
    let mut b = text.as_bytes().to_vec();
    let mut i = 0;
    while i < b.len() {
        let Some(joint) = vid_pid_at(&b, i) else {
            i += 1;
            continue;
        };
        i += 17;
        if joint == b'&' {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'&') {
                i += 1;
            }
            let sep = i;
            while i < b.len() && (b[i] == b'\\' || b[i] == b'#') {
                i += 1;
            }
            if i == sep {
                continue;
            }
        } else if b.get(i) == Some(&b'_') {
            i += 1;
        } else {
            continue;
        }
        let start = i;
        while i < b.len() && !ends(b[i]) {
            i += 1;
        }
        let serial = &b[start..i];
        let generated = joint == b'&' && serial.contains(&b'&');
        let address = serial.len() == 12 && serial.iter().all(u8::is_ascii_hexdigit);
        if !generated && !address {
            b[start..i].fill(b'x');
        }
    }
    String::from_utf8(b).expect("only ASCII bytes were replaced")
}

/// Full-width characters an IME types ("ＡＣ：８０") as their ASCII forms,
/// as v1 (ascii_from_fullwidth), plus the dashes a Japanese IME gives for
/// "-" ("ー", "−")
pub fn ascii_from_fullwidth(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
            '\u{3000}' => ' ',
            '\u{30FC}' | '\u{2010}'..='\u{2015}' | '\u{2212}' => '-',
            _ => c,
        })
        .collect()
}

/// A Bluetooth address as typed: 12 hex digits, optionally in pairs
/// separated by colons, hyphens, dots or spaces, also full-width (v1
/// accepted the same). Returns "AA:BB:CC:DD:EE:FF" or None.
pub fn parse_address(input: &str) -> Option<String> {
    let converted = ascii_from_fullwidth(input);
    let s = js_trim(&converted).as_bytes();
    let mut pairs: Vec<String> = Vec::with_capacity(6);
    let mut i = 0;
    for n in 0..6 {
        if n > 0 && matches!(s.get(i), Some(b':' | b'-' | b'.' | b' ')) {
            i += 1;
        }
        let pair = s.get(i..i + 2).filter(|p| p.iter().all(u8::is_ascii_hexdigit))?;
        pairs.push(String::from_utf8_lossy(pair).to_ascii_uppercase());
        i += 2;
    }
    (i == s.len()).then(|| pairs.join(":"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The number an address got (the masker lives as long as the process)
    fn addr_of(addr: &str) -> String {
        let m = mask_identifiers(addr);
        let start = m.find("<addr-").expect("an address") + 6;
        let end = start + m[start..].find('>').unwrap();
        m[start..end].to_owned()
    }

    fn m(s: &str) -> String {
        mask_identifiers(s)
    }

    #[test]
    fn accepts_the_address_forms_v1_accepted() {
        assert_eq!(parse_address("aa:bb:cc:dd:ee:ff").as_deref(), Some("AA:BB:CC:DD:EE:FF"));
        assert_eq!(parse_address("AA-BB-CC-DD-EE-FF").as_deref(), Some("AA:BB:CC:DD:EE:FF"));
        assert_eq!(parse_address("AABBCCDDEEFF").as_deref(), Some("AA:BB:CC:DD:EE:FF"));
        assert_eq!(parse_address("AA BB CC DD EE FF").as_deref(), Some("AA:BB:CC:DD:EE:FF"));
        assert_eq!(parse_address("AA:BB:CC:DD:EE"), None);
        assert_eq!(parse_address("GG:BB:CC:DD:EE:FF"), None);
    }

    #[test]
    fn accepts_full_width_input_from_an_ime_as_v1() {
        assert_eq!(ascii_from_fullwidth("ＡＣ：８０"), "AC:80");
        assert_eq!(parse_address("ＡＣ：８０：０Ａ：１２：３４：５６").as_deref(), Some("AC:80:0A:12:34:56"));
        assert_eq!(parse_address("ａｃ－８０－０ａ－１２－３４－５６").as_deref(), Some("AC:80:0A:12:34:56"));
        // Ideographic space around, the long vowel mark a Japanese IME types for "-"
        assert_eq!(parse_address("　acー80ー0a-12-34-56　").as_deref(), Some("AC:80:0A:12:34:56"));
        assert_eq!(parse_address("ＡＣ８００Ａ１２３４５６").as_deref(), Some("AC:80:0A:12:34:56"));
    }

    #[test]
    fn accepts_the_separators_v1_did_and_still_rejects_the_rest() {
        assert_eq!(parse_address("ac.80.0a.12.34.56").as_deref(), Some("AC:80:0A:12:34:56"));
        assert_eq!(parse_address("ＡＣ：８０：０Ａ：１２：３４"), None);
        assert_eq!(parse_address("ＧＧ：８０：０Ａ：１２：３４：５６"), None);
        assert_eq!(parse_address("AC:80:0A:12:34:56:78"), None);
    }

    #[test]
    fn masks_addresses_as_addr_n_the_same_address_the_same_n_in_every_form() {
        let n = addr_of("AC:80:0A:12:34:56");
        assert_eq!(m("connecting to AC:80:0A:12:34:56 now"), format!("connecting to <addr-{n}> now"));
        assert_eq!(m("ac-80-0a-12-34-56 and AC800A123456"), format!("<addr-{n}> and <addr-{n}>"));
        assert_ne!(addr_of("00:1A:7D:DA:71:13"), n);
    }

    // The same cases as the core's mask_identifiers tests
    #[test]
    fn masks_addresses_in_other_forms() {
        let a = addr_of("00-1A-7D-DA-71-13");
        let b = addr_of("46:62:99:4A:5B:6C");
        let c = addr_of("58:18:62:3a:1b:2c");
        assert_eq!(m("00-1A-7D-DA-71-13"), format!("<addr-{a}>"));
        // A colon after it ends the address (the LE Audio pairing lines of #81)
        assert_eq!(
            m("LE Audio: 46:62:99:4A:5B:6C: pairing (Just Works)"),
            format!("LE Audio: <addr-{b}>: pairing (Just Works)")
        );
        assert_eq!(m("to 46:62:99:4A:5B:6C:"), format!("to <addr-{b}>:"));
        assert_eq!(m("46-62-99-4A-5B-6C-x"), format!("<addr-{b}>-x"));
        assert_eq!(
            m("BluetoothLE#BluetoothLE00:1a:7d:da:71:13-58:18:62:3a:1b:2c"),
            format!("BluetoothLE#BluetoothLE<addr-{a}>-<addr-{c}>")
        );
        assert_eq!(m("fe80::00:11:22:33:44:55"), "fe80::00:11:22:33:44:55");
        assert_eq!(m("00:11:22:33:44:55:66"), "00:11:22:33:44:55:66");
        assert_eq!(m("00:11:22:33:44:556"), "00:11:22:33:44:556");
        assert_eq!(m("ヘッドホン5818623A1B2C接続"), format!("ヘッドホン<addr-{c}>接続"));
        assert_eq!(
            m(r"BTHENUM\{0000110b-0000-1000-8000-00805f9b34fb}_LOCALMFG&005d\7&1a2b3c4d&0&5818623A1B2C_C00000000"),
            format!(
                r"BTHENUM\{{0000110b-0000-1000-8000-00805f9b34fb}}_LOCALMFG&005d\7&1a2b3c4d&0&<addr-{c}>_C00000000"
            )
        );
        assert_eq!(
            m("5818623A1B2C5818623A1B2C, 0x5818623A1B2C, 5818623A1B2G"),
            "5818623A1B2C5818623A1B2C, 0x5818623A1B2C, 5818623A1B2G"
        );
    }

    #[test]
    fn shows_the_adapters_own_addresses_as_adapter() {
        know_adapters([Some("00:e0:4c:00:00:01"), None, Some("nope")]);
        assert_eq!(
            m(r"local 00:E0:4C:00:00:01, USB\VID_0BDA&PID_4853\00E04C000001"),
            r"local <adapter>, USB\VID_0BDA&PID_4853\<adapter>"
        );
    }

    #[test]
    fn masks_usb_serial_numbers() {
        let n = addr_of("00E04C000002");
        assert_eq!(
            m(r"opened 0BDA:4853 USB\VID_0BDA&PID_4853\00E04C000002 (event 0x81)"),
            format!(r"opened 0BDA:4853 USB\VID_0BDA&PID_4853\<addr-{n}> (event 0x81)")
        );
        assert_eq!(m(r"USB\VID_18D1&PID_4EE7\HT7A1B234567"), r"USB\VID_18D1&PID_4EE7\xxxxxxxxxxxx");
        assert_eq!(
            m(r"\\?\usb#vid_0a12&pid_0001#serial42#{a5dcbf10-6530-11d2-901f-00c04fb951ed}"),
            r"\\?\usb#vid_0a12&pid_0001#xxxxxxxx#{a5dcbf10-6530-11d2-901f-00c04fb951ed}"
        );
        assert_eq!(m(r"USB\VID_8087&PID_0026&MI_00\SN123"), r"USB\VID_8087&PID_0026&MI_00\xxxxx");
        assert_eq!(m(r"USB\VID_8087&PID_0026\5&2C7B4E3&0&14."), r"USB\VID_8087&PID_0026\5&2C7B4E3&0&14.");
        assert_eq!(m("USB_VID_0BDA_PID_8771_SN123.json"), "USB_VID_0BDA_PID_8771_xxxxx.json");
        assert_eq!(
            m("RTL8852BU (0BDA:4853 -> 0xB85B), VID_0BDA&PID_4853."),
            "RTL8852BU (0BDA:4853 -> 0xB85B), VID_0BDA&PID_4853."
        );
    }

    #[test]
    fn masks_a_path_above_the_apps_own_folder_or_all_but_its_last_part() {
        assert_eq!(
            m(r"settings folder: C:\Users\Taro Yamada\AppData\Roaming\BlueCaster"),
            r"settings folder: <path>\BlueCaster"
        );
        assert_eq!(
            m(r"dump: C:\Users\admin\AppData\Roaming\BlueCaster\logs\crash\bluecaster-core-20261022-235220-831.dmp"),
            r"dump: <path>\BlueCaster\logs\crash\bluecaster-core-20261022-235220-831.dmp"
        );
        assert_eq!(m(r"C:\Program Files\BlueCaster\bluecaster-ui.exe"), r"<path>\BlueCaster\bluecaster-ui.exe");
        assert_eq!(
            m(r"starting: pid 1, I:\A2DP-Windows-Bridge\target\release\bluecaster-core.exe"),
            r"starting: pid 1, <path>\bluecaster-core.exe"
        );
        assert_eq!(m(r"C:\Users\山田太郎\Music\foobar2000\foobar2000.exe"), r"<path>\foobar2000.exe");
        // The last part takes the text after it, but not another path; a trailing separator keeps the folder
        assert_eq!(
            m(r"HCI capture running: C:\x\BlueCaster\hci.pklg (3.0 MB)"),
            r"HCI capture running: <path>\BlueCaster\hci.pklg (3.0 MB)"
        );
        assert_eq!(m(r"moved C:\a\b\x.bin to C:\a\c\x.bin"), r"moved <path>\x.bin to <path>\x.bin");
        // Issue #86: BTstack's Realtek line, the second path in quotes
        assert_eq!(
            m(
                r"Using firmware 'C:\Users\taro\AppData\Roaming\BlueCaster\firmware/rtl8761bu_fw' and config 'C:\Users\taro\AppData\Roaming\BlueCaster\firmware/rtl8761bu_config'"
            ),
            r"Using firmware '<path>\BlueCaster\firmware/rtl8761bu_fw' and config '<path>\BlueCaster\firmware/rtl8761bu_config'"
        );
        assert_eq!(m("folder C:\\a\\b\\\n"), "folder <path>\\b\\\n");
        assert_eq!(m(r"C:\ only"), r"<path>\ only");
        // JSON's doubled '\', '/', a device path, a user's folder at the end (its name as x's)
        assert_eq!(m(r#"{"path":"C:\\Users\\taro\\x.txt"}"#), r#"{"path":"<path>\\x.txt"}"#);
        assert_eq!(m("file:///C:/users/taro/x.txt"), "file:///<path>/x.txt");
        assert_eq!(m(r"\\?\C:\USERS\TARO\x"), r"\\?\<path>\x");
        assert_eq!(m("dump: C:\\Users\\taro\n"), "dump: <path>\\xxxx\n");
        // Not paths
        assert_eq!(
            m("at 23:52:25 LE Audio: 46 kHz, x:y, C: drive, https://a.b/c"),
            "at 23:52:25 LE Audio: 46 kHz, x:y, C: drive, https://a.b/c"
        );
        assert_eq!(m(r"\\.\pipe\bluecaster-x"), r"\\.\pipe\bluecaster-x");
    }

    #[test]
    fn masks_the_windows_user_name_in_a_path_without_a_drive() {
        assert_eq!(
            m(r"\Users\Taro Yamada\AppData\Roaming\BlueCaster"),
            r"\Users\xxxxxxxxxxx\AppData\Roaming\BlueCaster"
        );
        assert_eq!(m(r"\Users\山田太郎\AppData"), r"\Users\xxxx\AppData");
        assert_eq!(m("/users/taro/x.txt"), "/users/xxxx/x.txt");
        assert_eq!(m(r"\Users\Public\Desktop and \Users\All Users\y"), r"\Users\Public\Desktop and \Users\All Users\y");
        assert_eq!(m("3 Users, \\Users\\\n"), "3 Users, \\Users\\\n");
        assert_eq!(m("in Users\\taro"), "in Users\\taro");
    }

    #[test]
    fn masks_account_sids() {
        assert_eq!(
            m(r"IPC: listening on \\.\pipe\bluecaster-S-1-5-21-2123068572-4202720387-3275098450-1001"),
            r"IPC: listening on \\.\pipe\bluecaster-<sid>"
        );
        assert_eq!(m("user S-1-5-21-1-2-3-500, then"), "user <sid>, then");
        // Well-known SIDs, the start of one, a longer word
        assert_eq!(m("S-1-5-18 S-1-5-32-544 S-1-5-21- S-1-"), "S-1-5-18 S-1-5-32-544 S-1-5-21- S-1-");
        assert_eq!(
            m("xS-1-5-21-1-2-3-4 S-1-5-21-1-2-3-4x bluecaster-S-1-5-21-1-2-3-4-dev"),
            "xS-1-5-21-1-2-3-4 S-1-5-21-1-2-3-4x bluecaster-<sid>-dev"
        );
    }

    #[test]
    fn masks_quoted_device_names_the_same_name_the_same_number() {
        let first = m("stream: device AA:BB:CC:DD:EE:FF source virtual device “Taro's AirPods”");
        let start = first.rfind("<device-").expect("a device") + 8;
        assert!(first.ends_with('>'));
        let n = first[start..first.len() - 1].to_owned();
        assert!(n.parse::<u32>().is_ok());
        let a = addr_of("AA:BB:CC:DD:EE:FF");
        assert_eq!(first, format!("stream: device <addr-{a}> source virtual device <device-{n}>"));
        assert_eq!(
            m("remote name of AA:BB:CC:DD:EE:FF: name=“Taro's AirPods”"),
            format!("remote name of <addr-{a}>: name=<device-{n}>")
        );
        let other = m("name=“Hanako's iPhone”");
        assert_ne!(other, format!("name=<device-{n}>"));
        let num = other.strip_prefix("name=<device-").and_then(|r| r.strip_suffix('>')).unwrap();
        assert!(num.parse::<u32>().is_ok());
        // Empty, unclosed and multi-line quotes stay
        assert_eq!(m("name=“” and “open"), "name=“” and “open");
        assert_eq!(m("“a\nb”"), "“a\nb”");
    }
}
