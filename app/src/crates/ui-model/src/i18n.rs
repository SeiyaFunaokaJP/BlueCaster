//! Translations: the flat JSON files of app/lang (shared with the core), all
//! built in. A key missing from a language shows the English text, as in v1.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, RwLock};

use ipc::Msg;

type Dict = HashMap<String, String>;

/// The languages of app/lang: code (file name) -> name in that language
pub const LANGUAGES: &[(&str, &str)] = &[
    ("en", "English"),
    ("ja", "日本語"),
    ("de", "Deutsch"),
    ("fr", "Français"),
    ("es", "Español"),
    ("ko", "한국어"),
    ("zh-CN", "简体中文"),
    ("zh-TW", "繁體中文"),
    ("uk", "Українська"),
    ("ru", "Русский"),
];

fn source(code: &str) -> Option<&'static str> {
    Some(match code {
        "en" => include_str!("../../../../lang/en.json"),
        "ja" => include_str!("../../../../lang/ja.json"),
        "de" => include_str!("../../../../lang/de.json"),
        "fr" => include_str!("../../../../lang/fr.json"),
        "es" => include_str!("../../../../lang/es.json"),
        "ko" => include_str!("../../../../lang/ko.json"),
        "zh-CN" => include_str!("../../../../lang/zh-CN.json"),
        "zh-TW" => include_str!("../../../../lang/zh-TW.json"),
        "uk" => include_str!("../../../../lang/uk.json"),
        "ru" => include_str!("../../../../lang/ru.json"),
        _ => return None,
    })
}

fn parse(code: &str) -> Option<Arc<Dict>> {
    let dict: Dict = serde_json::from_str::<HashMap<String, serde_json::Value>>(source(code)?)
        .ok()?
        .into_iter()
        .filter_map(|(k, v)| v.as_str().map(|s| (k, s.to_owned())))
        .collect();
    Some(Arc::new(dict))
}

static ENGLISH: LazyLock<Arc<Dict>> = LazyLock::new(|| parse("en").expect("app/lang/en.json is valid"));

struct Current {
    code: &'static str,
    dict: Arc<Dict>,
}

static CURRENT: LazyLock<RwLock<Current>> =
    LazyLock::new(|| RwLock::new(Current { code: "en", dict: ENGLISH.clone() }));

/// The current language's code
pub fn lang() -> &'static str {
    CURRENT.read().unwrap_or_else(|e| e.into_inner()).code
}

/// A label with the colon its language puts after it: a full-width one in
/// Chinese, a space before it in French
pub fn with_colon(label: &str) -> String {
    match lang() {
        "zh-CN" | "zh-TW" => format!("{label}\u{ff1a}"),
        "fr" => format!("{label} :"),
        _ => format!("{label}:"),
    }
}

/// "<label>: <value>" as the language writes it ([`with_colon`]; no space
/// after a full-width colon)
pub fn labelled(label: &str, value: &str) -> String {
    match lang() {
        "zh-CN" | "zh-TW" => format!("{label}\u{ff1a}{value}"),
        _ => format!("{} {value}", with_colon(label)),
    }
}

/// The language of `code` (unknown codes: English)
fn resolve(code: &str) -> Current {
    let known = LANGUAGES.iter().find(|(c, _)| *c == code).map(|(c, _)| *c);
    match known.and_then(|c| parse(c).map(|d| (c, d))) {
        Some((c, dict)) if c != "en" => Current { code: c, dict },
        _ => Current { code: "en", dict: ENGLISH.clone() },
    }
}

/// Switches the language (unknown codes: English) -> whether it changed
pub fn set_language(code: &str) -> bool {
    if code == lang() {
        return false;
    }
    let next = resolve(code);
    let changed = next.code != lang();
    *CURRENT.write().unwrap_or_else(|e| e.into_inner()) = next;
    changed
}

/// A printf argument: text is inserted as it is, a number gets the
/// precision, sign and integer conversion the placeholder asks for
#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    Str(String),
    Num(f64),
}

impl From<&str> for Arg {
    fn from(s: &str) -> Arg {
        Arg::Str(s.to_owned())
    }
}
impl From<String> for Arg {
    fn from(s: String) -> Arg {
        Arg::Str(s)
    }
}
impl From<&String> for Arg {
    fn from(s: &String) -> Arg {
        Arg::Str(s.clone())
    }
}
macro_rules! num_arg {
    ($($t:ty),*) => {$(
        impl From<$t> for Arg {
            fn from(n: $t) -> Arg {
                Arg::Num(n as f64)
            }
        }
    )*};
}
num_arg!(i8, i16, i32, i64, isize, u8, u16, u32, u64, usize, f32, f64);

fn template(key: &str) -> String {
    let cur = CURRENT.read().unwrap_or_else(|e| e.into_inner());
    cur.dict.get(key).or_else(|| ENGLISH.get(key)).cloned().unwrap_or_else(|| key.to_owned())
}

/// The text of `key` in the current language. Shown as it is, as v1 did:
/// some strings have a literal "%" ("about 80% little room") that is no
/// placeholder
pub fn t(key: &str) -> String {
    template(key)
}

/// The text of `key` in the current language with printf arguments
pub fn tf(key: &str, args: &[Arg]) -> String {
    let template = template(key);
    if args.is_empty() { template } else { printf(&template, args) }
}

/// `t!("key")`, or `t!("key", a, b)` with printf arguments (text or numbers)
#[macro_export]
macro_rules! t {
    ($key:expr) => {
        $crate::i18n::t($key)
    };
    ($key:expr, $($arg:expr),+ $(,)?) => {
        $crate::i18n::tf($key, &[$($crate::i18n::Arg::from($arg)),+])
    };
}

fn msg_args(msg: &Msg) -> Vec<Arg> {
    msg.args.iter().map(Arg::from).collect()
}

/// The text of a core message
pub fn tm(msg: Option<&Msg>) -> String {
    match msg {
        Some(m) if !m.key.is_empty() => tf(&m.key, &msg_args(m)),
        _ => String::new(),
    }
}

/// The English text of a core message, as the core's Msg::english makes it
/// (the text fields next to a Msg, such as FlowStepView.detail)
pub fn english_text(msg: &Msg) -> String {
    let template = ENGLISH.get(&msg.key).cloned().unwrap_or_else(|| msg.key.clone());
    if msg.args.is_empty() { template } else { printf(&template, &msg_args(msg)) }
}

/// Whether a key exists in English
pub fn has_key(key: &str) -> bool {
    ENGLISH.contains_key(key)
}

/// printf-style formatting of the translation strings (%s %d %u %zu %llu
/// %.1f %+d %%), as the core's Msg::english does
/// (crates/core/src/messages.rs)
pub fn printf(template: &str, args: &[Arg]) -> String {
    let chars: Vec<char> = template.chars().collect();
    let mut out = String::with_capacity(template.len() + 16);
    let mut next = 0;
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '%' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        // %% or %[flags][width][.prec][length]conv; anything else stays
        let mut j = i + 1;
        if chars.get(j) == Some(&'%') {
            out.push('%');
            i = j + 1;
            continue;
        }
        let mut flags = String::new();
        while let Some(c) = chars.get(j).filter(|c| "-+#0".contains(**c)) {
            flags.push(*c);
            j += 1;
        }
        let mut width = String::new();
        while let Some(c) = chars.get(j).filter(|c| c.is_ascii_digit()) {
            width.push(*c);
            j += 1;
        }
        let mut prec: Option<usize> = None;
        if chars.get(j) == Some(&'.') && chars.get(j + 1).is_some_and(|c| c.is_ascii_digit()) {
            let mut p = String::new();
            j += 1;
            while let Some(c) = chars.get(j).filter(|c| c.is_ascii_digit()) {
                p.push(*c);
                j += 1;
            }
            prec = p.parse().ok();
        }
        for len in ["hh", "ll", "h", "l", "z", "j", "t"] {
            let l: Vec<char> = len.chars().collect();
            if chars[j.min(chars.len())..].starts_with(&l) {
                j += l.len();
                break;
            }
        }
        let Some(conv) = chars.get(j).copied().filter(|c| "diuxXfFeEgGcsp".contains(*c)) else {
            out.push('%');
            i += 1;
            continue;
        };
        i = j + 1;
        let arg = args.get(next);
        next += 1;
        let Some(arg) = arg else { continue };
        let mut s = match arg {
            Arg::Str(s) => s.clone(),
            Arg::Num(n) => {
                let mut s = match conv {
                    'f' | 'F' => format!("{:.*}", prec.unwrap_or(6), n),
                    'd' | 'i' | 'u' => format!("{}", n.trunc() as i64),
                    'x' => format!("{:x}", n.trunc() as i64),
                    'X' => format!("{:X}", n.trunc() as i64),
                    _ => js_number(*n),
                };
                if flags.contains('+') && *n >= 0.0 && "dif".contains(conv) {
                    s.insert(0, '+');
                }
                s
            }
        };
        if let Ok(w) = width.parse::<usize>() {
            let len = s.chars().count();
            if len < w {
                let pad = w - len;
                if flags.contains('-') {
                    s.extend(std::iter::repeat_n(' ', pad));
                } else {
                    let fill = if flags.contains('0') { '0' } else { ' ' };
                    s = std::iter::repeat_n(fill, pad).chain(s.chars()).collect();
                }
            }
        }
        out.push_str(&s);
    }
    out
}

/// A number as JavaScript's String() writes it: no ".0" on whole numbers
pub fn js_number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 { format!("{}", n as i64) } else { format!("{n}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<Arg> {
        v.iter().map(|x| Arg::from(*x)).collect()
    }

    #[test]
    fn printf_fills_the_placeholders_the_language_files_use() {
        assert_eq!(
            printf("try %d/%d, %zu found, %.1f kHz, 100%%", &s(&["2", "3", "4", "48.0"])),
            "try 2/3, 4 found, 48.0 kHz, 100%"
        );
        assert_eq!(
            printf("%s (%s), Bluetooth %s, L2CAP MTU %u", &s(&["sink", "host", "5.0", "672"])),
            "sink (host), Bluetooth 5.0, L2CAP MTU 672"
        );
        assert_eq!(printf("%llu dropped (%.2f%%)", &s(&["12", "0.50"])), "12 dropped (0.50%)");
    }

    #[test]
    fn printf_formats_numbers_by_the_conversion() {
        assert_eq!(printf("%.1f kHz", &[44.1.into()]), "44.1 kHz");
        assert_eq!(printf("%.0f kbps", &[989.6.into()]), "990 kbps");
        assert_eq!(printf("%d", &[3.7.into()]), "3");
        assert_eq!(printf("RSSI %+d dB", &[4.into()]), "RSSI +4 dB");
        assert_eq!(printf("RSSI %+d dB", &[(-4).into()]), "RSSI -4 dB");
    }

    #[test]
    fn printf_leaves_missing_arguments_empty() {
        assert_eq!(printf("a %s b %s", &s(&["x"])), "a x b ");
    }

    // The language is global and tests run in parallel, so no test
    // switches it: the other modules' tests read English texts
    #[test]
    fn translations() {
        assert_eq!(t!("status.connecting_retry", 2, 3), "Connection failed, trying again (2/3)...");
        let msg = Msg { key: "status.scan_complete".into(), args: vec!["4".into()] };
        assert_eq!(tm(Some(&msg)), "Scan complete: 4 device(s) found");
        // An unknown key shows itself, so it is noticed
        assert_eq!(t("no.such.key"), "no.such.key");
        let text = |code: &str, key: &str| resolve(code).dict.get(key).cloned().unwrap_or_default();
        assert_eq!(resolve("ja").code, "ja");
        assert_eq!(text("ja", "ui.nav.home"), "ホーム");
        assert_eq!(text("de", "ui.nav.home"), "Start");
        assert_ne!(text("de", "connection.connect"), "");
        assert_eq!(resolve("xx").code, "en");
        assert_eq!(text("xx", "ui.nav.home"), "Home");
    }

    #[test]
    fn lists_every_language_file() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../lang");
        let mut files: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok()?.file_name().to_str()?.strip_suffix(".json").map(str::to_owned))
            .collect();
        files.sort();
        let mut codes: Vec<String> = LANGUAGES.iter().map(|(c, _)| (*c).to_owned()).collect();
        codes.sort();
        assert_eq!(codes, files);
        for (code, name) in LANGUAGES {
            assert_eq!(parse(code).unwrap().get("_language_name").map(String::as_str), Some(*name));
        }
    }
}
