//! The services each bonded LE device has, kept between connections
//! (`<bond_dir>\le_gatt_cache.txt`, beside the bonds; issue #86).
//!
//! GATT discovery took about 3.5 s of every LE Audio connection. A device
//! that has a Database Hash characteristic (0x2B2A in Generic Attribute,
//! Core 5.1 "GATT Caching") tells by its value whether its database
//! changed: reading it on a later connection (one request, at the handle
//! it had) and finding the kept value means the kept services are its
//! services, and discovery is skipped. Any other answer, or a device
//! without that characteristic, is discovered as before. The kept list is
//! also what the Services tag shows for an LE device.
//!
//! One block per device, its identity address first:
//!
//! ```text
//! D <address> <address type> <hash: 32 hex digits, or ->
//! S <uuid16> <uuid128> <start> <end> <p|s> <included by>
//! C <uuid16> <uuid128> <start> <value> <end> <properties>
//! ```
//!
//! Numbers in hex. A file that does not parse is taken as empty.

use std::path::Path;
use std::sync::Mutex;

use super::gatt_client::{GattChar, GattService};

pub const FILE_NAME: &str = "le_gatt_cache.txt";

/// Generic Attribute service and its Database Hash characteristic
pub const UUID_GATT_SERVICE: u16 = 0x1801;
pub const UUID_DATABASE_HASH: u16 = 0x2B2A;

/// One device's services as kept
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedDb {
    /// Identity address, upper case ("AA:BB:CC:DD:EE:FF")
    pub address: String,
    pub address_type: u8,
    /// The Database Hash read with these services; None: the device has
    /// none (kept for the Services tag only)
    pub hash: Option<[u8; 16]>,
    pub services: Vec<GattService>,
}

/// The Database Hash characteristic's value handle in `services`
pub fn hash_handle(services: &[GattService]) -> Option<u16> {
    services
        .iter()
        .filter(|s| s.uuid16 == UUID_GATT_SERVICE)
        .flat_map(|s| s.chars.iter())
        .find(|c| c.uuid16 == UUID_DATABASE_HASH)
        .map(|c| c.value_handle)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex<const N: usize>(s: &str) -> Option<[u8; N]> {
    if s.len() != 2 * N || !s.is_ascii() {
        return None;
    }
    let mut out = [0u8; N];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

fn num(s: &str) -> Option<u16> {
    u16::from_str_radix(s, 16).ok()
}

pub fn format(dbs: &[CachedDb]) -> String {
    let mut out = String::new();
    for db in dbs {
        let hash = db.hash.map(|h| hex(&h)).unwrap_or_else(|| "-".into());
        out.push_str(&format!("D {} {:x} {hash}\n", db.address, db.address_type));
        for s in &db.services {
            out.push_str(&format!(
                "S {:04x} {} {:04x} {:04x} {} {:04x}\n",
                s.uuid16,
                hex(&s.uuid128),
                s.start_handle,
                s.end_handle,
                if s.primary { "p" } else { "s" },
                s.included_by
            ));
            for c in &s.chars {
                out.push_str(&format!(
                    "C {:04x} {} {:04x} {:04x} {:04x} {:04x}\n",
                    c.uuid16,
                    hex(&c.uuid128),
                    c.start_handle,
                    c.value_handle,
                    c.end_handle,
                    c.properties
                ));
            }
        }
    }
    out
}

/// None: the text does not parse (taken as no cache at all)
pub fn parse(text: &str) -> Option<Vec<CachedDb>> {
    let mut dbs: Vec<CachedDb> = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let f: Vec<&str> = line.split_whitespace().collect();
        match f.as_slice() {
            ["D", addr, kind, hash] => dbs.push(CachedDb {
                address: crate::events::parse_addr(addr).map(|a| crate::events::addr_str(&a).to_uppercase())?,
                address_type: u8::from_str_radix(kind, 16).ok()?,
                hash: if *hash == "-" { None } else { Some(unhex::<16>(hash)?) },
                services: Vec::new(),
            }),
            ["S", u16_, u128_, start, end, kind, by] => dbs.last_mut()?.services.push(GattService {
                uuid16: num(u16_)?,
                uuid128: unhex::<16>(u128_)?,
                start_handle: num(start)?,
                end_handle: num(end)?,
                primary: match *kind {
                    "p" => true,
                    "s" => false,
                    _ => return None,
                },
                included_by: num(by)?,
                chars: Vec::new(),
            }),
            ["C", u16_, u128_, start, value, end, props] => dbs.last_mut()?.services.last_mut()?.chars.push(GattChar {
                uuid16: num(u16_)?,
                uuid128: unhex::<16>(u128_)?,
                start_handle: num(start)?,
                value_handle: num(value)?,
                end_handle: num(end)?,
                properties: num(props)?,
            }),
            _ => return None,
        }
    }
    Some(dbs)
}

fn read_all(file: &Path) -> Vec<CachedDb> {
    let Ok(text) = std::fs::read_to_string(file) else { return Vec::new() };
    parse(&text).unwrap_or_else(|| {
        tracing::warn!("LE Audio: {} does not parse, services are discovered again", file.display());
        Vec::new()
    })
}

/// Temp file + rename: a quit or crash while writing never leaves a cut
/// file (one that does not parse would lose every device's services)
fn write_all(file: &Path, dbs: &[CachedDb]) {
    let tmp = file.with_extension("txt.tmp");
    let written = std::fs::write(&tmp, format(dbs)).and_then(|()| {
        std::fs::rename(&tmp, file).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
    });
    if let Err(e) = written {
        tracing::warn!("LE Audio: cannot keep the services in {}: {e}", file.display());
    }
}

/// One reader or writer of the file at a time: the BTstack thread keeps a
/// device's services while a request of the core reads them or forgets a
/// device
static FILE_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// The kept services of the device with this identity address
pub fn load(file: &Path, address: &str) -> Option<CachedDb> {
    let _one = lock();
    read_all(file).into_iter().find(|d| d.address.eq_ignore_ascii_case(address))
}

fn has_primary(db: &CachedDb) -> bool {
    db.services.iter().any(|s| s.primary)
}

/// `db` into the kept list; false when the list stays as it is: the same
/// again, or a discovery that found no service at all (it never replaces
/// a kept list, nor is it kept: the Services tag would show nothing)
fn merge(dbs: &mut Vec<CachedDb>, db: CachedDb) -> bool {
    if !has_primary(&db) {
        return false;
    }
    if let Some(i) = dbs.iter().position(|d| d.address.eq_ignore_ascii_case(&db.address)) {
        if dbs[i] == db {
            return false;
        }
        dbs.remove(i);
    }
    dbs.push(db);
    true
}

/// Keeps (or replaces) a device's services (see [`merge`]); false when
/// nothing changed
pub fn store(file: &Path, db: CachedDb) -> bool {
    let _one = lock();
    let mut dbs = read_all(file);
    if !merge(&mut dbs, db) {
        return false;
    }
    write_all(file, &dbs);
    true
}

/// A forgotten device's services go with it; true if it had any
pub fn remove(file: &Path, address: &str) -> bool {
    let _one = lock();
    let mut dbs = read_all(file);
    let before = dbs.len();
    dbs.retain(|d| !d.address.eq_ignore_ascii_case(address));
    if dbs.len() == before {
        return false;
    }
    write_all(file, &dbs);
    true
}

/// The services of a device for the Services tag: each service's UUID in
/// its 128-bit form, lower case, primary ones only (secondary services are
/// parts of them); None without a kept list
pub fn service_uuids(file: &Path, address: &str) -> Option<Vec<String>> {
    let db = load(file, address)?;
    Some(db.services.iter().filter(|s| s.primary).map(|s| uuid_string(s.uuid16, &s.uuid128)).collect())
}

/// "0000184e-0000-1000-8000-00805f9b34fb" for a 16-bit UUID; a 128-bit one
/// from BTstack's order (most significant octet first)
fn uuid_string(uuid16: u16, uuid128: &[u8; 16]) -> String {
    if uuid16 != 0 {
        return format!("0000{uuid16:04x}-0000-1000-8000-00805f9b34fb");
    }
    let h = hex(uuid128);
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> CachedDb {
        let mut vendor = [0u8; 16];
        vendor[0] = 0xDF;
        vendor[15] = 0x7C;
        CachedDb {
            address: "00:A2:D0:00:00:12".into(),
            address_type: 0,
            hash: Some([0xAB; 16]),
            services: vec![
                GattService {
                    uuid16: UUID_GATT_SERVICE,
                    uuid128: [0; 16],
                    start_handle: 1,
                    end_handle: 9,
                    primary: true,
                    included_by: 0,
                    chars: vec![GattChar {
                        uuid16: UUID_DATABASE_HASH,
                        uuid128: [0; 16],
                        start_handle: 6,
                        value_handle: 7,
                        end_handle: 7,
                        properties: 0x02,
                    }],
                },
                GattService {
                    uuid16: 0,
                    uuid128: vendor,
                    start_handle: 0x10,
                    end_handle: 0x20,
                    primary: true,
                    included_by: 0,
                    chars: vec![],
                },
                GattService {
                    uuid16: 0x1843,
                    uuid128: [0; 16],
                    start_handle: 0x30,
                    end_handle: 0x38,
                    primary: false,
                    included_by: 0x21,
                    chars: vec![],
                },
            ],
        }
    }

    #[test]
    fn round_trips_through_the_file_format() {
        let dbs = vec![sample(), CachedDb { address: "00:A2:D0:00:00:13".into(), hash: None, ..sample() }];
        assert_eq!(parse(&format(&dbs)), Some(dbs));
    }

    #[test]
    fn a_broken_file_is_no_cache() {
        assert_eq!(parse("D 00:A2:D0:00:00:12 0 -\nS 1801 zz"), None);
        assert_eq!(parse("S 1801 00000000000000000000000000000000 0001 0009 p 0000"), None);
        assert_eq!(parse(""), Some(vec![]));
    }

    #[test]
    fn finds_the_hash_handle() {
        assert_eq!(hash_handle(&sample().services), Some(7));
        assert_eq!(hash_handle(&sample().services[1..]), None);
    }

    #[test]
    fn a_discovery_without_services_never_replaces_kept_ones() {
        let mut dbs = vec![sample()];
        let empty = CachedDb { hash: None, services: vec![], ..sample() };
        // Only a secondary service: no primary one either
        let secondary = CachedDb { services: sample().services[2..].to_vec(), ..sample() };
        assert!(!merge(&mut dbs, empty.clone()));
        assert!(!merge(&mut dbs, secondary));
        assert!(!merge(&mut dbs, sample()));
        assert_eq!(dbs, vec![sample()]);
        // Nor is it kept for a device with nothing kept
        let other = CachedDb { address: "00:A2:D0:00:00:13".into(), ..empty };
        assert!(!merge(&mut dbs, other));
        assert_eq!(dbs.len(), 1);
        // Other services do replace them
        let changed = CachedDb { hash: Some([0xCD; 16]), services: sample().services[..1].to_vec(), ..sample() };
        assert!(merge(&mut dbs, changed.clone()));
        assert_eq!(dbs, vec![changed]);
    }

    #[test]
    fn stores_replaces_lists_and_removes() {
        let dir = std::env::temp_dir().join(format!("bc-gatt-cache-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(FILE_NAME);
        let _ = std::fs::remove_file(&file);
        store(&file, sample());
        store(&file, CachedDb { hash: None, ..sample() });
        let kept = load(&file, "00:a2:d0:00:00:12").unwrap();
        assert_eq!(kept.hash, None);
        assert_eq!(
            service_uuids(&file, "00:A2:D0:00:00:12").unwrap(),
            vec!["00001801-0000-1000-8000-00805f9b34fb", "df000000-0000-0000-0000-00000000007c"]
        );
        assert!(remove(&file, "00:A2:D0:00:00:12"));
        assert!(!remove(&file, "00:A2:D0:00:00:12"));
        assert_eq!(load(&file, "00:A2:D0:00:00:12"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
