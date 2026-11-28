//! The firmware in Intel's Windows Bluetooth driver (ibtusb.sys)
//!
//! Intel's driver loads the operational firmware itself, so ibtusb.sys
//! carries the images (the same CSS-headed .sfi format as linux-firmware's)
//! and the device configuration of every controller it serves. A PC that
//! had Intel's driver keeps its packages in the driver store after the
//! adapter moved to WinUSB (crate::driver_files), so the loader can read the
//! firmware from there when the firmware folder lacks it (issue #602).
//!
//! What the driver binary holds, from our own reading of it (no other
//! driver's code or tables): a list of 64-byte descriptors in its data, each
//! four records of (type u32, length u32, pointer u64):
//! - type 0, 1 byte: a flag of the driver's own (not used here)
//! - type 1: the key, the controller's Read Version as the driver compares
//!   it ([`Request`])
//! - type 2: the blob: an .sfi image (CSS header, then the command buffer)
//!   or the DDC, as .bseq records of Write DDC commands and their events
//! - type 3, 20 bytes: not used here
//!
//! Several keys point to the same blob (bootloader builds of one chip).
//! TLV keys may end in "_usb" or "_pci", for the interface of the chip.
//! Short keys (2 or 4 bytes) belong to audio offload images, not loaded here.

use std::path::{Path, PathBuf};

use super::{FW_VARIANT_OPERATIONAL, LegacyVersion, OP_WRITE_DDC, TlvVersion, parse_bseq, parse_ddc, top_id};
use crate::driver_files;

/// What the loader looks for
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// An .sfi image
    Firmware,
    /// The device configuration, returned in .ddc form
    Ddc,
}

/// A firmware lookup in the driver: the key the driver would use for this
/// controller, and how much of it must match when no key matches whole
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub kind: Kind,
    /// TLV controllers (12 bytes): CNVi and CNVr ID as in the file name
    /// (big-endian: "0040" is 00 40), image type, timestamp, build type,
    /// build number. Legacy controllers (9 bytes): the Read Version answer
    /// after its status.
    pub key: Vec<u8>,
    /// The part that names the chip (TLV: the IDs and the image type;
    /// legacy: platform, variant, revision, firmware variant and revision).
    /// Without a whole match, a blob is taken when every key with this
    /// part points to the same blob.
    pub prefix: usize,
}

impl Request {
    /// The image a TLV controller in the state `v` takes, or its DDC once
    /// the operational firmware runs (`v` then is its Read Version)
    pub fn tlv(kind: Kind, v: &TlvVersion) -> Request {
        let mut key = Vec::with_capacity(12);
        key.extend_from_slice(&top_id(v.cnvi_top).to_be_bytes());
        key.extend_from_slice(&top_id(v.cnvr_top).to_be_bytes());
        key.push(v.img_type);
        key.extend_from_slice(&v.timestamp.to_le_bytes());
        key.push(v.build_type);
        key.extend_from_slice(&v.build_num.to_le_bytes());
        Request { kind, key, prefix: 5 }
    }

    /// The image of a legacy bootloader `v`, or the DDC of the operational
    /// firmware booted from it (its build is not known before: the prefix
    /// with the operational variant decides)
    pub fn legacy(kind: Kind, v: &LegacyVersion) -> Request {
        let fw_variant = if kind == Kind::Ddc { FW_VARIANT_OPERATIONAL } else { v.fw_variant };
        let key = vec![
            v.hw_platform,
            v.hw_variant,
            v.hw_revision,
            fw_variant,
            v.fw_revision,
            v.fw_build_num,
            v.fw_build_ww,
            v.fw_build_yy,
            v.fw_patch_num,
        ];
        Request { kind, key, prefix: 5 }
    }
}

/// A descriptor of the driver: its key and its blob
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry<'a> {
    pub key: &'a [u8],
    pub blob: &'a [u8],
}

/// The sections of a PE32+ file: what a pointer in it refers to
struct Pe<'a> {
    data: &'a [u8],
    base: u64,
    /// Virtual address, size in the file, file offset
    sections: Vec<(u32, u32, u32)>,
}

fn u16_at(d: &[u8], at: usize) -> Option<u16> {
    d.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]))
}

fn u32_at(d: &[u8], at: usize) -> Option<u32> {
    d.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn u64_at(d: &[u8], at: usize) -> Option<u64> {
    d.get(at..at + 8).map(|b| u64::from_le_bytes(b.try_into().unwrap_or_default()))
}

impl<'a> Pe<'a> {
    fn parse(data: &'a [u8]) -> Option<Pe<'a>> {
        if data.get(0..2) != Some(b"MZ") {
            return None;
        }
        let pe = u32_at(data, 0x3C)? as usize;
        if data.get(pe..pe + 4) != Some(b"PE\0\0") {
            return None;
        }
        let count = u16_at(data, pe + 6)? as usize;
        let optional_size = u16_at(data, pe + 20)? as usize;
        let optional = pe + 24;
        // PE32+ only: the 64-bit drivers (x64, ARM64) have 8-byte pointers
        if u16_at(data, optional)? != 0x20B {
            return None;
        }
        let base = u64_at(data, optional + 24)?;
        let sections = (0..count)
            .map(|i| {
                let s = optional + optional_size + 40 * i;
                Some((u32_at(data, s + 12)?, u32_at(data, s + 16)?, u32_at(data, s + 20)?))
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Pe { data, base, sections })
    }

    /// The `len` bytes a pointer refers to, if all are in the file
    fn slice(&self, pointer: u64, len: usize) -> Option<&'a [u8]> {
        let rva = u32::try_from(pointer.checked_sub(self.base)?).ok()?;
        let &(va, raw_size, raw) = self.sections.iter().find(|&&(va, size, _)| rva >= va && rva - va < size)?;
        let inside = (rva - va) as usize;
        if inside + len > raw_size as usize {
            return None;
        }
        let at = raw as usize + inside;
        self.data.get(at..at + len)
    }
}

/// The firmware descriptors of a driver binary (none: not a PE32+ file, or
/// one without them)
pub fn entries(sys: &[u8]) -> Vec<Entry<'_>> {
    let Some(pe) = Pe::parse(sys) else { return Vec::new() };
    let mut found = Vec::new();
    for &(_, raw_size, raw) in &pe.sections {
        let (start, end) = (raw as usize, (raw as usize + raw_size as usize).min(sys.len()));
        let mut at = start;
        while at + 64 <= end {
            if let Some(entry) = descriptor(&pe, at) {
                found.push(entry);
                at += 64;
            } else {
                at += 8;
            }
        }
    }
    found
}

/// A descriptor at this file offset
fn descriptor<'a>(pe: &Pe<'a>, at: usize) -> Option<Entry<'a>> {
    let d = pe.data;
    let record = |i: usize| Some((u32_at(d, at + 16 * i)?, u32_at(d, at + 16 * i + 4)?, u64_at(d, at + 16 * i + 8)?));
    let ((t0, l0, _), (t1, key_len, key), (t2, blob_len, blob), (t3, _, _)) =
        (record(0)?, record(1)?, record(2)?, record(3)?);
    if (t0, l0, t1, t2, t3) != (0, 1, 1, 2, 3) || !(1..=64).contains(&key_len) || blob_len == 0 {
        return None;
    }
    Some(Entry { key: pe.slice(key, key_len as usize)?, blob: pe.slice(blob, blob_len as usize)? })
}

/// An .sfi blob: a CSS header (module type 6)
fn is_sfi(blob: &[u8]) -> bool {
    blob.len() > 12 && blob[0..4] == [0x06, 0, 0, 0]
}

/// A DDC blob (.bseq records of Write DDC commands) as a .ddc: the
/// commands' parameters, which are the records
fn ddc_of(blob: &[u8]) -> Option<Vec<u8>> {
    let steps = parse_bseq(blob).ok()?;
    let mut out = Vec::new();
    for s in &steps {
        if s.opcode() != OP_WRITE_DDC {
            return None;
        }
        out.extend_from_slice(s.command.get(3..)?);
    }
    parse_ddc(&out).ok().filter(|c| !c.is_empty())?;
    Some(out)
}

/// What a driver has for a request
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// The .sfi or the .ddc
    pub data: Vec<u8>,
    /// The whole key matched (else only its prefix, with one blob)
    pub exact: bool,
}

/// Looks the request up in a driver binary
pub fn find(sys: &[u8], req: &Request) -> Option<Found> {
    let usable = |e: &Entry| -> Option<Vec<u8>> {
        // A key of the request's length, or one for USB chips
        let n = req.key.len();
        if !(e.key.len() == n || (e.key.len() == n + 4 && &e.key[n..] == b"_usb")) {
            return None;
        }
        match req.kind {
            Kind::Firmware => is_sfi(e.blob).then(|| e.blob.to_vec()),
            Kind::Ddc => ddc_of(e.blob),
        }
    };
    let all = entries(sys);
    let n = req.key.len();
    if let Some(data) = all.iter().filter(|e| e.key.get(..n) == Some(&req.key[..])).find_map(usable) {
        return Some(Found { data, exact: true });
    }
    let p = req.prefix.min(n);
    let mut candidates: Vec<Vec<u8>> =
        all.iter().filter(|e| e.key.get(..p) == Some(&req.key[..p])).filter_map(usable).collect();
    candidates.dedup();
    match candidates.len() {
        1 => Some(Found { data: candidates.remove(0), exact: false }),
        0 => None,
        several => {
            tracing::debug!("intel: Intel's driver has {several} different images for this chip; none taken");
            None
        }
    }
}

/// The request's data from Intel's Windows driver, if it is installed or
/// was: its packages in the driver store and the copy in System32\drivers,
/// newest first. A whole key match in any of them comes before a prefix
/// match.
pub fn from_windows_driver(req: &Request) -> Option<Vec<u8>> {
    let windows = std::env::var_os("SystemRoot").map(PathBuf::from)?;
    from_driver_in(&windows, req)
}

/// [`from_windows_driver`] with this Windows folder
pub fn from_driver_in(windows: &Path, req: &Request) -> Option<Vec<u8>> {
    let names = ["ibtusb.sys".to_string()];
    let mut by_prefix = None;
    for path in driver_files::find(windows, driver_files::Names::Exact(&names), &driver_files::any_package, &[]) {
        let Some(found) = std::fs::read(&path).ok().and_then(|sys| find(&sys, req)) else { continue };
        // The package folder, for the log (no user path in it)
        let package = path.parent().map(driver_files::file_name).unwrap_or_default();
        if found.exact {
            tracing::info!("intel: {:?} read from Intel's driver ({package}), {} bytes", req.kind, found.data.len());
            return Some(found.data);
        }
        if by_prefix.is_none() {
            by_prefix = Some((package, found.data));
        }
    }
    let (package, data) = by_prefix?;
    tracing::info!(
        "intel: {:?} read from Intel's driver ({package}), {} bytes; only the chip's IDs matched",
        req.kind,
        data.len()
    );
    Some(data)
}
