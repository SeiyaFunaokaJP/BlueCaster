//! What a window of another toolkit needs from Windows itself: the title
//! bar's light or dark look, placing a window that has no frame (the tray
//! flyout) in physical pixels, and a minimum size that can change.

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWMWA_CAPTION_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
    DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GetWindowLongPtrW, GetWindowRect, IsIconic, MINMAXINFO, SPI_GETWORKAREA, SW_HIDE, SW_SHOW,
    SWP_NOACTIVATE, SWP_NOZORDER, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SetForegroundWindow, SetWindowLongPtrW,
    SetWindowPos, ShowWindow, SystemParametersInfoW, WM_GETMINMAXINFO, WM_NCDESTROY, WS_EX_NOACTIVATE,
};

/// A rectangle on the desktop (physical pixels, for a DPI-aware process)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Area {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Area {
    pub fn width(self) -> i32 {
        self.right - self.left
    }

    pub fn height(self) -> i32 {
        self.bottom - self.top
    }
}

impl From<RECT> for Area {
    fn from(r: RECT) -> Area {
        Area { left: r.left, top: r.top, right: r.right, bottom: r.bottom }
    }
}

fn handle(hwnd: isize) -> HWND {
    HWND(hwnd as *mut _)
}

/// Draws the window's title bar dark or light, whatever Windows' own app
/// theme is. False when Windows refused (before Windows 10 20H1).
pub fn set_dark_title_bar(hwnd: isize, dark: bool) -> bool {
    let value: i32 = dark.into();
    // SAFETY: the attribute takes a BOOL (4 bytes), read from `value` during
    // the call; a handle that is no window makes the call fail
    let result = unsafe {
        DwmSetWindowAttribute(
            handle(hwnd),
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            (&value as *const i32).cast(),
            size_of::<i32>() as u32,
        )
    };
    result.is_ok()
}

/// Colours the window's title bar (red, green, blue), so that it is one
/// surface with the window below it, as Windows 11's own apps are. False on
/// Windows 10, which keeps its own title bar colour.
pub fn set_caption_color(hwnd: isize, (r, g, b): (u8, u8, u8)) -> bool {
    // A COLORREF: 0x00BBGGRR
    let value: u32 = u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16);
    // SAFETY: the attribute takes a COLORREF (4 bytes), read from `value`
    // during the call; a handle that is no window makes the call fail
    let result = unsafe {
        DwmSetWindowAttribute(handle(hwnd), DWMWA_CAPTION_COLOR, (&value as *const u32).cast(), size_of::<u32>() as u32)
    };
    result.is_ok()
}

/// Rounds the corners of a window without a frame, as Windows 11 rounds
/// its own flyouts. False on Windows 10, which has square ones.
pub fn round_corners(hwnd: isize) -> bool {
    let value = DWMWCP_ROUND.0;
    // SAFETY: the attribute takes a DWM_WINDOW_CORNER_PREFERENCE (4 bytes),
    // read from `value` during the call; an invalid handle fails
    let result = unsafe {
        DwmSetWindowAttribute(
            handle(hwnd),
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&value as *const i32).cast(),
            size_of::<i32>() as u32,
        )
    };
    result.is_ok()
}

/// The window's DPI (96 = 100 %); None for a handle that is no window
pub fn dpi(hwnd: isize) -> Option<u32> {
    // SAFETY: GetDpiForWindow takes any handle (0 for one that is not a window)
    match unsafe { GetDpiForWindow(handle(hwnd)) } {
        0 => None,
        d => Some(d),
    }
}

/// The primary screen without the taskbar
pub fn primary_work_area() -> Option<Area> {
    let mut r = RECT::default();
    // SAFETY: SPI_GETWORKAREA fills in the RECT `r`, which outlives the call
    unsafe {
        SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some((&mut r as *mut RECT).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .ok()?;
    }
    Some(r.into())
}

/// The work area of the screen the point is on (the nearest one otherwise)
pub fn work_area_at(x: i32, y: i32) -> Option<Area> {
    let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    // SAFETY: MonitorFromPoint takes any point; GetMonitorInfoW fills in
    // `info`, whose size is set
    unsafe {
        let monitor = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        GetMonitorInfoW(monitor, &mut info).ok().ok()?;
    }
    Some(info.rcWork.into())
}

/// Where the window is, frame included
pub fn outer_rect(hwnd: isize) -> Option<Area> {
    let mut r = RECT::default();
    // SAFETY: a RECT to fill in; an invalid handle fails
    unsafe { GetWindowRect(handle(hwnd), &mut r).ok()? };
    Some(r.into())
}

/// Moves and sizes the window without activating it or changing its place
/// among the other windows
pub fn set_bounds(hwnd: isize, x: i32, y: i32, width: i32, height: i32) -> bool {
    // SAFETY: an invalid handle fails
    unsafe { SetWindowPos(handle(hwnd), None, x, y, width, height, SWP_NOZORDER | SWP_NOACTIVATE).is_ok() }
}

/// Shows the window and brings it to the front, taking the focus
pub fn show_active(hwnd: isize) {
    // SAFETY: only changes whether the window shows and which one is in
    // front; an invalid handle fails
    unsafe {
        let _ = ShowWindow(handle(hwnd), SW_SHOW);
        let _ = SetForegroundWindow(handle(hwnd));
    }
}

/// Takes the window off the screen; it stays alive
pub fn hide(hwnd: isize) {
    // SAFETY: only changes whether the window shows; an invalid handle fails
    let _ = unsafe { ShowWindow(handle(hwnd), SW_HIDE) };
}

/// Whether clicks on the window activate it (WS_EX_NOACTIVATE off) or leave
/// the foreground where it is
pub fn set_no_activate(hwnd: isize, on: bool) {
    // SAFETY: reads and writes the window's extended style; an invalid
    // handle makes both calls fail (0 read, nothing written)
    unsafe {
        let style = GetWindowLongPtrW(handle(hwnd), GWL_EXSTYLE);
        let flag = WS_EX_NOACTIVATE.0 as isize;
        let next = if on { style | flag } else { style & !flag };
        if next != style {
            SetWindowLongPtrW(handle(hwnd), GWL_EXSTYLE, next);
        }
    }
}

/// Whether the window is minimized
pub fn is_minimized(hwnd: isize) -> bool {
    // SAFETY: a plain query; an invalid handle answers false
    unsafe { IsIconic(handle(hwnd)).as_bool() }
}

/// The subclass that [`raise_min_size`] installs (one per window)
const MIN_SIZE_SUBCLASS: usize = 0x4243_4d53;

/// Raises the window's minimum size by `width` × `height` logical pixels
/// over the one its toolkit fixed when it made the window (GPUI cannot
/// change it later); 0 × 0 takes the raise away. Call it on the thread that
/// owns the window. False when the window could not be subclassed.
pub fn raise_min_size(hwnd: isize, width: u16, height: u16) -> bool {
    let data = (usize::from(width) << 16) | usize::from(height);
    if data == 0 {
        // SAFETY: removes our own subclass; a window without it (or an
        // invalid handle) only makes the call fail
        let _ = unsafe { RemoveWindowSubclass(handle(hwnd), Some(min_size_proc), MIN_SIZE_SUBCLASS) };
        return true;
    }
    // SAFETY: min_size_proc stays valid for the program's lifetime and takes
    // its data by value; installing it again only replaces the data
    unsafe { SetWindowSubclass(handle(hwnd), Some(min_size_proc), MIN_SIZE_SUBCLASS, data) }.as_bool()
}

/// Lets the window answer WM_GETMINMAXINFO first, then adds the raise
unsafe extern "system" fn min_size_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    // SAFETY: passes the message on unchanged, as a subclass procedure must
    let result = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
    match msg {
        WM_GETMINMAXINFO if lparam.0 != 0 => {
            // SAFETY: a plain query on the window the message is for
            let dpi = match unsafe { GetDpiForWindow(hwnd) } {
                0 => 96,
                d => d as i32,
            };
            let scale = |v: usize| ((v & 0xFFFF) as i32 * dpi + 48) / 96;
            // SAFETY: WM_GETMINMAXINFO's lparam points to the MINMAXINFO the
            // system reads back after this call
            let info = unsafe { &mut *(lparam.0 as *mut MINMAXINFO) };
            info.ptMinTrackSize.x += scale(data >> 16);
            info.ptMinTrackSize.y += scale(data);
        }
        WM_NCDESTROY => {
            // SAFETY: removes this subclass from the window being destroyed
            let _ = unsafe { RemoveWindowSubclass(hwnd, Some(min_size_proc), id) };
        }
        _ => {}
    }
    result
}
