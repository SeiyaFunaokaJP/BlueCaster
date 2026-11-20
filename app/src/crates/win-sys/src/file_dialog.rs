//! Windows' Save As dialog (IFileSaveDialog).

use std::path::PathBuf;

use windows::Win32::Foundation::{E_FAIL, ERROR_CANCELLED, HWND};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoCreateInstance, CoInitializeEx,
    CoTaskMemFree, CoUninitialize,
};
use windows::Win32::UI::Shell::{
    FOS_FORCEFILESYSTEM, FOS_OVERWRITEPROMPT, FOS_PATHMUSTEXIST, FileSaveDialog, IFileSaveDialog, SIGDN_FILESYSPATH,
};
use windows::core::HSTRING;

/// Asks where to save a file: the Save As dialog, owned by the window
/// `owner` (an HWND; 0: none), suggesting the file name `name`. `extension`
/// (without the dot) is added to a name typed without one. None: the dialog
/// was cancelled. Blocks until it closes, and joins this thread to a
/// single-threaded apartment for that time: call it on a thread of its own.
pub fn save_file(owner: isize, name: &str, extension: &str) -> windows::core::Result<Option<PathBuf>> {
    // SAFETY: plain call; balanced below when it succeeded
    let joined = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) }.is_ok();
    let result = show(owner, name, extension);
    if joined {
        // SAFETY: this thread's own CoInitializeEx above
        unsafe { CoUninitialize() };
    }
    result
}

fn show(owner: isize, name: &str, extension: &str) -> windows::core::Result<Option<PathBuf>> {
    // SAFETY: COM calls on an interface created here, with strings that
    // outlive each call
    unsafe {
        let dialog: IFileSaveDialog = CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)?;
        let options = dialog.GetOptions()?;
        dialog.SetOptions(options | FOS_OVERWRITEPROMPT | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST)?;
        dialog.SetFileName(&HSTRING::from(name))?;
        dialog.SetDefaultExtension(&HSTRING::from(extension))?;
        let parent = (owner != 0).then_some(HWND(owner as *mut _));
        if let Err(e) = dialog.Show(parent) {
            return if e.code() == ERROR_CANCELLED.to_hresult() { Ok(None) } else { Err(e) };
        }
        let item = dialog.GetResult()?;
        let path = item.GetDisplayName(SIGDN_FILESYSPATH)?;
        let text = path.to_string();
        CoTaskMemFree(Some(path.0 as *const _));
        Ok(Some(PathBuf::from(text.map_err(|_| windows::core::Error::from(E_FAIL))?)))
    }
}
