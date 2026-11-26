//! The WebView2 window of 3.0.x kept its browser data (cache and storage,
//! often tens of MB) in `%LOCALAPPDATA%\<identifier>\EBWebView`. This window
//! has no use for it, so a start removes it, and the identifier's folder too
//! when nothing else is left in it. Files that are still held (a WebView2
//! process of the old window that has not ended yet) stay until a later start.

use std::path::{Path, PathBuf};

/// The folder WebView2 kept the data in, below the identifier's folder
const DATA_DIR: &str = "EBWebView";

/// Removes the old window's WebView2 data on a thread of its own
pub fn remove_in_background(identifier: &str) {
    let Some(base) = std::env::var_os("LOCALAPPDATA") else { return };
    let dir = PathBuf::from(base).join(identifier);
    let _ = std::thread::Builder::new().name("webview-data".into()).spawn(move || remove(&dir));
}

/// Removes `dir`\EBWebView, then `dir` if it is empty. A link in place of
/// either folder is left alone.
fn remove(dir: &Path) {
    let data = dir.join(DATA_DIR);
    if !std::fs::symlink_metadata(&data).is_ok_and(|m| m.is_dir()) {
        return;
    }
    match std::fs::remove_dir_all(&data) {
        Ok(()) => {
            tracing::info!("removed the WebView2 data of the 3.0.x window");
            if std::fs::symlink_metadata(dir).is_ok_and(|m| m.is_dir()) {
                let _ = std::fs::remove_dir(dir);
            }
        }
        Err(e) => tracing::debug!("the WebView2 data of the 3.0.x window stays for now: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_the_data_and_the_empty_folder() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("id");
        std::fs::create_dir_all(dir.join(DATA_DIR).join("Default")).unwrap();
        std::fs::write(dir.join(DATA_DIR).join("Default").join("x"), "x").unwrap();
        remove(&dir);
        assert!(!dir.exists());
    }

    #[test]
    fn keeps_a_folder_with_other_files() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("id");
        std::fs::create_dir_all(dir.join(DATA_DIR)).unwrap();
        std::fs::write(dir.join("other"), "x").unwrap();
        remove(&dir);
        assert!(!dir.join(DATA_DIR).exists());
        assert!(dir.join("other").exists());
    }

    #[test]
    fn nothing_to_remove() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("id");
        std::fs::create_dir_all(&dir).unwrap();
        remove(&dir);
        assert!(dir.exists(), "a folder without WebView2 data stays");
    }
}
