//! Opening web pages, folders and files, saving a text file the user
//! names, and what the window reads from Windows (accent colours, the
//! licence texts' folder). They check their arguments: opening URLs or
//! paths is limited to what the screens need.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tokio::sync::oneshot;

/// Opens a web page in the user's browser (https only)
pub fn open_url(url: &str) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err("only https URLs are opened".into());
    }
    win_sys::shell::open(url.as_ref()).map_err(|e| e.to_string())
}

/// Opens a folder in Explorer (the settings folder), creating it first if it
/// is not there yet, as v1's Open Config Folder did (the firmware files go
/// there before anything else wrote to it). Folders only: a file path would
/// run the file's program.
pub fn open_folder(path: &Path) -> Result<(), String> {
    let shown = path.display();
    if !path.is_absolute() {
        return Err(format!("{shown} is not a folder"));
    }
    if !path.exists() {
        std::fs::create_dir_all(path).map_err(|e| format!("{shown}: {e}"))?;
    }
    if !path.is_dir() {
        return Err(format!("{shown} is not a folder"));
    }
    win_sys::shell::open(path.as_os_str()).map_err(|e| e.to_string())
}

/// Shows a file selected in Explorer (an HCI capture)
pub fn reveal_path(path: &Path) -> Result<(), String> {
    if !path.is_absolute() || !path.exists() {
        return Err(format!("{} does not exist", path.display()));
    }
    win_sys::shell::reveal(path).map_err(|e| e.to_string())
}

/// Longest text [`save_text_file`] writes
const SAVE_TEXT_MAX: usize = 1 << 20;

/// A file name to suggest (no folder in it) and a text short enough
fn check_save(name: &str, text: &str) -> Result<(), String> {
    if text.len() > SAVE_TEXT_MAX || name.is_empty() || name.contains(['\\', '/', ':']) {
        return Err("not a file name, or too long".into());
    }
    Ok(())
}

/// Writes `text` to a .txt file the user picks in Windows' Save As dialog
/// (`name`: the file name suggested; an exported EQ) -> false when the
/// dialog was cancelled. Only where the user chose: the caller names no
/// path. `owner`: the window's HWND (0: none), so that the dialog is
/// modal to it. The dialog runs on a thread of its own; the window's
/// event loop runs on, and the answer comes on the receiver.
pub fn save_text_file(owner: isize, name: String, text: String) -> oneshot::Receiver<Result<bool, String>> {
    let (tx, rx) = oneshot::channel();
    if let Err(e) = check_save(&name, &text) {
        let _ = tx.send(Err(e));
        return rx;
    }
    let spawned = std::thread::Builder::new().name("save-dialog".into()).spawn(move || {
        let result = (|| {
            let picked = win_sys::file_dialog::save_file(owner, &name, "txt").map_err(|e| e.to_string())?;
            let Some(path) = picked else { return Ok(false) };
            std::fs::write(&path, &text).map_err(|e| e.to_string())?;
            tracing::info!("text file saved ({} bytes)", text.len());
            Ok(true)
        })();
        let _ = tx.send(result);
    });
    if let Err(e) = spawned {
        tracing::warn!("cannot start the Save As dialog's thread: {e}");
    }
    rx
}

/// The licence texts next to the programs (installer and portable zip,
/// installer/build-release.ps1); None in a development build
pub fn licenses_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.join("licenses");
    dir.is_dir().then_some(dir)
}

/// Windows' accent colour for each theme ("#rrggbb")
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Accent {
    pub light: String,
    pub dark: String,
}

/// Windows' accent colours (read again when the window gets the focus, so a
/// change in Settings shows); None keeps the UI's default
pub fn accent_colors() -> Option<Accent> {
    win_sys::accent::accent_colors().map(|a| Accent { light: a.light, dark: a.dark })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Refused before anything opens
    #[test]
    fn only_https_pages_absolute_folders_and_existing_files() {
        assert!(open_url("http://example.invalid/").is_err());
        assert!(open_url("file:///C:/Windows/notepad.exe").is_err());
        assert!(open_url("C:\\Windows\\notepad.exe").is_err());
        assert!(open_folder(Path::new("relative\\folder")).is_err());
        // A file, not a folder
        let exe = std::env::current_exe().unwrap();
        assert!(open_folder(&exe).is_err());
        assert!(reveal_path(Path::new("relative.txt")).is_err());
        assert!(reveal_path(&exe.with_file_name("no-such-file-ui-host.txt")).is_err());
    }

    #[test]
    fn save_names_carry_no_folder() {
        assert!(check_save("eq.txt", "text").is_ok());
        assert!(check_save("", "text").is_err());
        assert!(check_save("..\\eq.txt", "text").is_err());
        assert!(check_save("C:eq.txt", "text").is_err());
        assert!(check_save("eq.txt", &"x".repeat(SAVE_TEXT_MAX + 1)).is_err());
        // Refused on the receiver, without a dialog
        let refused = crate::connection::tests_support::block_on(save_text_file(0, "a/b".into(), String::new()));
        assert!(refused.unwrap().is_err());
    }
}
