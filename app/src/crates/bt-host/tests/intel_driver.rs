//! Tests of the firmware lookup in Intel's Windows driver
//! (src/intel/driver.rs), on synthetic driver binaries

use bt_host::intel::driver::{self, Kind, Request};
use bt_host::intel::*;

const BASE: u64 = 0x1_4000_0000;
const SECTION_VA: u32 = 0x1000;
const SECTION_RAW: usize = 0x400;

/// A PE32+ file with one data section, built up piece by piece
struct Sys {
    data: Vec<u8>,
}

impl Sys {
    fn new() -> Sys {
        Sys { data: Vec::new() }
    }

    /// Puts bytes in the section (8-byte aligned): their pointer
    fn put(&mut self, bytes: &[u8]) -> u64 {
        while !self.data.len().is_multiple_of(8) {
            self.data.push(0xCC);
        }
        let at = self.data.len();
        self.data.extend_from_slice(bytes);
        BASE + SECTION_VA as u64 + at as u64
    }

    /// A descriptor: flag, key, blob, and the 20-byte record
    fn descriptor(&mut self, key: &[u8], blob: &[u8]) {
        let flag = self.put(&[1]);
        let k = self.put(key);
        let b = self.put(blob);
        let last = self.put(&[0x5A; 20]);
        let mut d = Vec::new();
        for (t, len, p) in [(0u32, 1usize, flag), (1, key.len(), k), (2, blob.len(), b), (3, 20, last)] {
            d.extend_from_slice(&t.to_le_bytes());
            d.extend_from_slice(&(len as u32).to_le_bytes());
            d.extend_from_slice(&p.to_le_bytes());
        }
        self.put(&d);
    }

    fn file(&self) -> Vec<u8> {
        let mut f = vec![0u8; SECTION_RAW];
        f[0..2].copy_from_slice(b"MZ");
        let pe = 0x80;
        f[0x3C..0x40].copy_from_slice(&(pe as u32).to_le_bytes());
        f[pe..pe + 4].copy_from_slice(b"PE\0\0");
        f[pe + 4..pe + 6].copy_from_slice(&0x8664u16.to_le_bytes());
        f[pe + 6..pe + 8].copy_from_slice(&1u16.to_le_bytes());
        let optional_size = 0xF0usize;
        f[pe + 20..pe + 22].copy_from_slice(&(optional_size as u16).to_le_bytes());
        let opt = pe + 24;
        f[opt..opt + 2].copy_from_slice(&0x20Bu16.to_le_bytes());
        f[opt + 24..opt + 32].copy_from_slice(&BASE.to_le_bytes());
        let s = opt + optional_size;
        f[s..s + 5].copy_from_slice(b".data");
        let size = self.data.len() as u32;
        f[s + 8..s + 12].copy_from_slice(&size.to_le_bytes());
        f[s + 12..s + 16].copy_from_slice(&SECTION_VA.to_le_bytes());
        f[s + 16..s + 20].copy_from_slice(&size.to_le_bytes());
        f[s + 20..s + 24].copy_from_slice(&(SECTION_RAW as u32).to_le_bytes());
        f.extend_from_slice(&self.data);
        f
    }
}

/// An .sfi-like blob: CSS header start, then a marker
fn sfi(marker: u8) -> Vec<u8> {
    let mut d = vec![0x06, 0, 0, 0, 0xA1, 0, 0, 0, 0, 0, 1, 0];
    d.extend([marker; 20]);
    d
}

/// A DDC blob: one Write DDC with two records, and its Command Complete
fn ddc_blob() -> Vec<u8> {
    vec![
        0x01, 0x8B, 0xFC, 0x09, 0x03, 0x28, 0x01, 0x18, 0x04, 0x29, 0x01, 0x03, 0x00, // command
        0x02, 0x0E, 0x06, 0x01, 0x8B, 0xFC, 0x00, 0x00, 0x00, // event
    ]
}

/// TLV key: CNVi 0040, CNVr 0041, image type, timestamp, build type and
/// number
fn tlv_key(img_type: u8, ww: u8) -> Vec<u8> {
    vec![0x00, 0x40, 0x00, 0x41, img_type, ww, 0x13, 0x01, 0x26, 0x00, 0x00, 0x00]
}

fn with(key: &[u8], suffix: &[u8]) -> Vec<u8> {
    [key, suffix].concat()
}

fn request(kind: Kind, key: Vec<u8>) -> Request {
    Request { kind, key, prefix: 5 }
}

#[test]
fn the_whole_key_picks_the_usb_image() {
    let mut sys = Sys::new();
    sys.descriptor(&with(&tlv_key(1, 0x28), b"_pci"), &sfi(0xA0));
    sys.descriptor(&with(&tlv_key(1, 0x28), b"_usb"), &sfi(0xA1));
    sys.descriptor(&tlv_key(1, 0x0C), &sfi(0xA2));
    let file = sys.file();
    assert_eq!(driver::entries(&file).len(), 3);
    let found = driver::find(&file, &request(Kind::Firmware, tlv_key(1, 0x28))).unwrap();
    assert!(found.exact);
    assert_eq!(found.data, sfi(0xA1));
    let found = driver::find(&file, &request(Kind::Firmware, tlv_key(1, 0x0C))).unwrap();
    assert_eq!((found.exact, found.data), (true, sfi(0xA2)));
}

#[test]
fn the_chip_ids_alone_take_one_image_only() {
    let mut sys = Sys::new();
    // Two bootloader builds, one image
    sys.descriptor(&tlv_key(1, 0x28), &sfi(0xB0));
    sys.descriptor(&tlv_key(1, 0x0C), &sfi(0xB0));
    let file = sys.file();
    let found = driver::find(&file, &request(Kind::Firmware, tlv_key(1, 0x30))).unwrap();
    assert_eq!((found.exact, found.data), (false, sfi(0xB0)));
    // Another chip, another image type: nothing
    let mut other = tlv_key(1, 0x30);
    other[3] = 0x42;
    assert_eq!(driver::find(&file, &request(Kind::Firmware, other)), None);
    assert_eq!(driver::find(&file, &request(Kind::Firmware, tlv_key(2, 0x28))), None);

    // Two images for the chip and no whole match: none is guessed
    sys.descriptor(&tlv_key(1, 0x14), &sfi(0xB1));
    assert_eq!(driver::find(&sys.file(), &request(Kind::Firmware, tlv_key(1, 0x30))), None);
}

#[test]
fn ddc_blobs_become_ddc_files() {
    let mut sys = Sys::new();
    sys.descriptor(&tlv_key(1, 0x28), &sfi(0xC0));
    sys.descriptor(&tlv_key(3, 0x17), &ddc_blob());
    let file = sys.file();
    let found = driver::find(&file, &request(Kind::Ddc, tlv_key(3, 0x17))).unwrap();
    assert_eq!(found.data, [0x03, 0x28, 0x01, 0x18, 0x04, 0x29, 0x01, 0x03, 0x00]);
    assert_eq!(parse_ddc(&found.data).unwrap().len(), 2);
    // A DDC request never gets an image, nor an image request a DDC
    assert_eq!(driver::find(&file, &request(Kind::Ddc, tlv_key(1, 0x28))), None);
    assert_eq!(driver::find(&file, &request(Kind::Firmware, tlv_key(3, 0x17))), None);

    // A blob with other commands is no DDC
    let mut sys = Sys::new();
    let mut blob = ddc_blob();
    blob[1] = 0x8E;
    sys.descriptor(&tlv_key(3, 0x17), &blob);
    assert_eq!(driver::find(&sys.file(), &request(Kind::Ddc, tlv_key(3, 0x17))), None);
}

#[test]
fn offload_images_and_other_files_are_left_out() {
    let mut sys = Sys::new();
    // Short keys: audio offload images
    sys.descriptor(&[0x00, 0x40], &sfi(0xD0));
    sys.descriptor(&[0x00, 0x40, 0x00, 0x41], &sfi(0xD1));
    let file = sys.file();
    assert_eq!(driver::entries(&file).len(), 2);
    assert_eq!(driver::find(&file, &request(Kind::Firmware, tlv_key(1, 0x28))), None);
    // Not a PE32+ file
    assert!(driver::entries(b"MZ not a driver").is_empty());
    let mut pe32 = file.clone();
    pe32[0x80 + 24..0x80 + 26].copy_from_slice(&0x10Bu16.to_le_bytes());
    assert!(driver::entries(&pe32).is_empty());
}

#[test]
fn requests_carry_the_version_the_driver_compares() {
    // CNVi top 0x400 step 0 ("0040"), CNVr 0x410 step 0 ("0041")
    let mut ret = vec![0x00];
    for (t, v) in [
        (0x10u8, 0x0000_0400u32.to_le_bytes().to_vec()),
        (0x11, 0x0000_0410u32.to_le_bytes().to_vec()),
        (0x12, ((0x18u32 << 16) | (0x37 << 8)).to_le_bytes().to_vec()),
        (0x1C, vec![IMG_BOOTLOADER]),
        (0x1D, vec![0x28, 0x13]),
        (0x1E, vec![0x01]),
        (0x1F, 0x26u32.to_le_bytes().to_vec()),
    ] {
        ret.extend([t, v.len() as u8]);
        ret.extend(v);
    }
    let Ok(Version::Tlv(v)) = parse_version(&ret) else { panic!() };
    let req = Request::tlv(Kind::Firmware, &v);
    assert_eq!(req.key, tlv_key(1, 0x28));
    assert_eq!(req.prefix, 5);

    let Some(l) = parse_legacy_version(&[0x00, 0x37, 0x13, 0x10, 0x06, 0x01, 0x00, 0x0B, 0x11, 0x00]) else { panic!() };
    assert_eq!(Request::legacy(Kind::Firmware, &l).key, [0x37, 0x13, 0x10, 0x06, 0x01, 0x00, 0x0B, 0x11, 0x00]);
    // The DDC is the operational firmware's
    assert_eq!(Request::legacy(Kind::Ddc, &l).key[..5], [0x37, 0x13, 0x10, 0x23, 0x01]);
}

#[test]
fn drivers_on_the_pc_are_searched_whole_keys_first() {
    let windows = std::env::temp_dir().join(format!("bc-intel-driver-{}", std::process::id()));
    let store = windows.join("System32").join("DriverStore").join("FileRepository");
    let (a, b) = (store.join("ibtusb.inf_amd64_a"), store.join("ibtusb.inf_amd64_b"));
    for d in [&a, &b] {
        std::fs::create_dir_all(d).unwrap();
    }
    // Package a: the chip's IDs only; package b: the whole key
    let mut prefix_only = Sys::new();
    prefix_only.descriptor(&tlv_key(1, 0x0C), &sfi(0xE0));
    let mut whole = Sys::new();
    whole.descriptor(&tlv_key(1, 0x28), &sfi(0xE1));
    std::fs::write(a.join("ibtusb.sys"), prefix_only.file()).unwrap();
    std::fs::write(b.join("ibtusb.sys"), whole.file()).unwrap();
    let exact = driver::from_driver_in(&windows, &request(Kind::Firmware, tlv_key(1, 0x28)));
    let near = driver::from_driver_in(&windows, &request(Kind::Firmware, tlv_key(1, 0x30)));
    let none = driver::from_driver_in(&windows, &request(Kind::Ddc, tlv_key(3, 0x17)));
    let _ = std::fs::remove_dir_all(&windows);
    assert_eq!(exact, Some(sfi(0xE1)));
    // Both have one image for the chip: one of them, not nothing
    assert!(near == Some(sfi(0xE0)) || near == Some(sfi(0xE1)), "{near:?}");
    assert_eq!(none, None);
}
