//! Where vendors' Windows Bluetooth drivers keep their firmware on this PC,
//! for the loaders that read it from there (Realtek, MediaTek, Qualcomm).
//!
//! A driver package sits in the driver store
//! (System32\DriverStore\FileRepository\<inf>_<arch>_<hash>), and its install
//! may copy files into the Windows folder, System32 or System32\drivers,
//! sometimes under another name. The copies stay after the adapter moved to
//! WinUSB, and those outside the driver store may stay after the package
//! itself was removed (issue #86). Callers check the content themselves.

use std::path::{Path, PathBuf};

/// The file names to look for
pub(crate) enum Names<'a> {
    /// These names (Windows ignores their case)
    Exact(&'a [String]),
    /// Every file whose lower-case name passes this test
    Matching(&'a dyn Fn(&str) -> bool),
}

/// The driver's files on this PC, newest first: in the driver store's
/// packages whose lower-case folder name passes `package`, and in the
/// Windows folder, System32, System32\drivers and `extra`
pub(crate) fn find(windows: &Path, names: Names, package: &dyn Fn(&str) -> bool, extra: &[PathBuf]) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    let store = windows.join("System32").join("DriverStore").join("FileRepository");
    if let Ok(entries) = std::fs::read_dir(store) {
        for e in entries.flatten() {
            if package(&e.file_name().to_string_lossy().to_ascii_lowercase()) {
                found.extend(in_folder(&e.path(), &names));
            }
        }
    }
    let folders =
        [windows.to_path_buf(), windows.join("System32"), windows.join("System32").join("drivers")].into_iter();
    for folder in folders.chain(extra.iter().cloned()) {
        found.extend(in_folder(&folder, &names));
    }
    found.sort();
    found.dedup();
    let modified = |p: &PathBuf| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    found.sort_by_key(|p| std::cmp::Reverse(modified(p)));
    found
}

fn in_folder(folder: &Path, names: &Names) -> Vec<PathBuf> {
    match names {
        Names::Exact(list) => list.iter().map(|n| folder.join(n)).filter(|p| p.is_file()).collect(),
        Names::Matching(test) => std::fs::read_dir(folder)
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|e| test(&e.file_name().to_string_lossy().to_ascii_lowercase()))
                    .map(|e| e.path())
                    .filter(|p| p.is_file())
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// Any package of the driver store (OEM packages name their INF differently)
pub(crate) fn any_package(_: &str) -> bool {
    true
}

/// The file name of a path, for the log (a folder may hold the user's name)
pub(crate) fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_store_the_windows_folders_and_extra_folders_are_searched() {
        let windows = std::env::temp_dir().join(format!("bc-driver-files-{}", std::process::id()));
        let package = windows.join("System32").join("DriverStore").join("FileRepository").join("vendor.inf_amd64_1");
        let other = windows.join("System32").join("DriverStore").join("FileRepository").join("other.inf_amd64_2");
        let drivers = windows.join("System32").join("drivers");
        let extra = windows.join("Extra");
        for d in [&package, &other, &drivers, &extra] {
            std::fs::create_dir_all(d).unwrap();
        }
        for p in [
            package.join("fw.dat"),
            other.join("fw.dat"),
            windows.join("fw"),
            drivers.join("fw.dat"),
            extra.join("FW.DAT"),
        ] {
            std::fs::write(p, b"x").unwrap();
        }
        let names = ["fw.dat".to_string(), "fw".to_string()];
        let only_vendor = |n: &str| n.starts_with("vendor.inf");
        let exact = find(&windows, Names::Exact(&names), &only_vendor, std::slice::from_ref(&extra));
        let matching =
            find(&windows, Names::Matching(&|n: &str| n.starts_with("fw")), &any_package, std::slice::from_ref(&extra));
        let _ = std::fs::remove_dir_all(&windows);
        // The other package is left out; the extra folder's file matches
        // whatever its case
        assert_eq!(exact.len(), 4, "{exact:?}");
        assert!(exact.iter().all(|p| !p.starts_with(&other)));
        assert_eq!(matching.len(), 5, "{matching:?}");
    }
}
