//! The shell's own ways to open things: a web page or folder with its
//! default program, a file shown selected in Explorer, a program started
//! with a command line (the update's installer), and the right to take the
//! foreground handed to another process.

use std::ffi::OsStr;
use std::path::Path;

use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{ILCreateFromPathW, ILFree, SHOpenFolderAndSelectItems, ShellExecuteW};
use windows::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, SHOW_WINDOW_CMD, SW_SHOW, SW_SHOWNORMAL};
use windows::core::{HSTRING, PCWSTR, w};

/// Opens `target` (a URL, a folder) with its default program, as a double
/// click does (ShellExecute "open"). Does not wait for the program.
pub fn open(target: &OsStr) -> std::io::Result<()> {
    execute(target, None, SW_SHOWNORMAL)
}

/// Starts the program `file` with the command line `parameters`
/// (ShellExecute "open", shown): Windows' UAC prompt comes first when the
/// program's manifest asks for administrator rights. Does not wait.
pub fn run(file: &Path, parameters: &str) -> std::io::Result<()> {
    execute(file.as_os_str(), Some(parameters), SW_SHOW)
}

fn execute(file: &OsStr, parameters: Option<&str>, show: SHOW_WINDOW_CMD) -> std::io::Result<()> {
    let file = HSTRING::from(file);
    let parameters = parameters.map(HSTRING::from);
    let parameters = parameters.as_ref().map_or(PCWSTR::null(), |p| PCWSTR(p.as_ptr()));
    // SAFETY: NUL-terminated strings (HSTRING) that live for the call, or
    // null for the optional ones.
    let r = unsafe { ShellExecuteW(None, w!("open"), &file, parameters, PCWSTR::null(), show) };
    // ShellExecute reports success with a value above 32
    let code = r.0 as isize;
    if code > 32 { Ok(()) } else { Err(shell_execute_error(code)) }
}

/// A failed ShellExecute's value (32 or less) as an error. Some values are
/// Win32 error codes (file or path not found, access denied: also when the
/// user declines the UAC prompt), the others the shell's own.
fn shell_execute_error(code: isize) -> std::io::Error {
    use std::io::{Error, ErrorKind};
    match code {
        0 => Error::new(ErrorKind::OutOfMemory, "the system is out of memory or resources"),
        // SE_ERR_FNF, SE_ERR_PNF, SE_ERR_ACCESSDENIED, SE_ERR_OOM, ERROR_BAD_FORMAT
        2 | 3 | 5 | 8 | 11 => Error::from_raw_os_error(code as i32),
        // SE_ERR_SHARE
        26 => Error::other("a sharing violation occurred"),
        // SE_ERR_ASSOCINCOMPLETE, SE_ERR_NOASSOC
        27 | 31 => Error::other("no program is associated with it"),
        // SE_ERR_DDETIMEOUT, SE_ERR_DDEFAIL, SE_ERR_DDEBUSY
        28..=30 => Error::other("the DDE transaction failed"),
        // SE_ERR_DLLNOTFOUND
        32 => Error::other("a DLL was not found"),
        _ => Error::other(format!("ShellExecute failed with code {code}")),
    }
}

/// An item ID list made from a path, freed on drop
struct ItemIdList(*mut ITEMIDLIST);

impl ItemIdList {
    fn new(path: &Path) -> std::io::Result<ItemIdList> {
        // SAFETY: a NUL-terminated string that lives for the call; the list
        // returned is owned here and freed on drop.
        let list = unsafe { ILCreateFromPathW(&HSTRING::from(path)) };
        if list.is_null() {
            return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "no shell item for the path"));
        }
        Ok(ItemIdList(list))
    }
}

impl Drop for ItemIdList {
    fn drop(&mut self) {
        // SAFETY: a list ILCreateFromPathW allocated, freed only here.
        unsafe { ILFree(Some(self.0.cast_const())) };
    }
}

/// Opens Explorer at the folder of `path` with `path` selected
/// (SHOpenFolderAndSelectItems). Where the shell cannot find the item
/// although it exists, the folder opens without the selection. Joins this
/// thread to a single-threaded apartment for the call.
pub fn reveal(path: &Path) -> std::io::Result<()> {
    let Some(folder) = path.parent() else {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "the path has no folder"));
    };
    // SAFETY: plain call; balanced below when it succeeded (S_FALSE too:
    // the thread had joined the apartment already)
    let joined = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) }.is_ok();
    let result = reveal_in(folder, path);
    if joined {
        // SAFETY: this thread's own CoInitializeEx above
        unsafe { CoUninitialize() };
    }
    result
}

fn reveal_in(folder: &Path, path: &Path) -> std::io::Result<()> {
    let folder_list = ItemIdList::new(folder)?;
    let item = ItemIdList::new(path)?;
    let items = [item.0.cast_const()];
    // SAFETY: both lists live until the end of this function, past the call.
    match unsafe { SHOpenFolderAndSelectItems(folder_list.0.cast_const(), Some(&items), 0) } {
        Ok(()) => Ok(()),
        // Some systems report "file not found" for an item that is there
        // (Electron's and Tauri's openers fall back to opening the folder)
        Err(e) if e.code() == ERROR_FILE_NOT_FOUND.to_hresult() => open(folder.as_os_str()),
        Err(e) => Err(std::io::Error::other(e)),
    }
}

/// Lets process `pid` bring its window to the front (a second launch hands
/// the foreground right it got from the user's click to the running
/// instance). False when Windows did not allow it.
pub fn allow_set_foreground(pid: u32) -> bool {
    // SAFETY: plain call without pointers
    unsafe { AllowSetForegroundWindow(pid) }.is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_errors_keep_the_win32_codes() {
        assert_eq!(shell_execute_error(2).raw_os_error(), Some(2));
        assert_eq!(shell_execute_error(5).raw_os_error(), Some(5));
        assert_eq!(shell_execute_error(0).kind(), std::io::ErrorKind::OutOfMemory);
        assert_eq!(shell_execute_error(31).raw_os_error(), None);
    }

    #[test]
    fn a_path_without_a_folder_is_refused() {
        assert!(reveal(Path::new("")).is_err());
    }
}
