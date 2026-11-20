//! One control OUT transfer with a SETUP packet sent exactly as given,
//! through the WinUSB handle of an interface claimed with nusb (the
//! vendored copy's `Interface::winusb_handle`). nusb builds bmRequestType
//! from its own recipient values (device, interface, endpoint, other) and
//! cannot send the reserved ones; Developer > Debug's register write
//! (issue #86) may need them. Nothing else uses this.
//!
//! The handle is nusb's (WinUSB lets only one open handle use a device), so
//! the transfer follows the rules of usb_isoch.rs: its OVERLAPPED's event
//! carries the low bit, so no completion packet reaches nusb's completion
//! port. A transfer that does not complete in time leaks its buffer,
//! OVERLAPPED and event rather than freeing memory the driver may still
//! write.

use std::time::Duration;

use nusb::Interface;
use windows::Win32::Devices::Usb::{
    WINUSB_INTERFACE_HANDLE, WINUSB_SETUP_PACKET, WinUsb_ControlTransfer, WinUsb_GetOverlappedResult,
};
use windows::Win32::Foundation::{ERROR_IO_PENDING, HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::IO::OVERLAPPED;
use windows::Win32::System::Threading::WaitForSingleObject;

use crate::usb_isoch::{Event, Win32Error, code, millis};

const ERROR_INVALID_PARAMETER: Win32Error = 87;
const ERROR_SEM_TIMEOUT: Win32Error = 121;

/// The WinUSB form of an 8-byte SETUP packet (bmRequestType, bRequest,
/// wValue, wIndex, wLength; little endian)
pub fn setup_packet(setup: [u8; 8]) -> WINUSB_SETUP_PACKET {
    WINUSB_SETUP_PACKET {
        RequestType: setup[0],
        Request: setup[1],
        Value: u16::from_le_bytes([setup[2], setup[3]]),
        Index: u16::from_le_bytes([setup[4], setup[5]]),
        Length: u16::from_le_bytes([setup[6], setup[7]]),
    }
}

/// Sends `data` with the SETUP packet `setup` on the default pipe; Ok: the
/// bytes sent. Host-to-device only, and wLength must be `data`'s length
/// (else ERROR_INVALID_PARAMETER, nothing sent).
pub fn control_out_raw(
    interface: &Interface,
    setup: [u8; 8],
    data: &[u8],
    timeout: Duration,
) -> Result<u32, Win32Error> {
    let packet = setup_packet(setup);
    if packet.RequestType & 0x80 != 0 || usize::from(packet.Length) != data.len() {
        return Err(ERROR_INVALID_PARAMETER);
    }
    let handle = WINUSB_INTERFACE_HANDLE(interface.winusb_handle());
    let event = Event::new(true)?;
    let tagged = HANDLE((event.raw().0 as usize | 1) as *mut std::ffi::c_void);
    let ov: *mut OVERLAPPED = Box::into_raw(Box::new(OVERLAPPED { hEvent: tagged, ..Default::default() }));
    let buffer: *mut [u8] = Box::into_raw(data.to_vec().into_boxed_slice());
    let free = |ov: *mut OVERLAPPED, buffer: *mut [u8]| {
        // SAFETY: both were made by Box::into_raw above and the transfer is
        // over (completed or never started), so the driver holds neither.
        unsafe {
            drop(Box::from_raw(ov));
            drop(Box::from_raw(buffer));
        }
    };
    // SAFETY: the handle is the claimed interface's, alive for the call;
    // buffer and ov stay allocated until the transfer is over (freed below
    // or leaked on timeout).
    let started = unsafe { WinUsb_ControlTransfer(handle, packet, Some(&mut *buffer), None, Some(ov.cast_const())) };
    if let Err(e) = started
        && code(&e) != ERROR_IO_PENDING.0
    {
        free(ov, buffer);
        return Err(code(&e));
    }
    // SAFETY: the event belongs to this call.
    if unsafe { WaitForSingleObject(event.raw(), millis(timeout)) } != WAIT_OBJECT_0 {
        // Still with the driver: never freed
        std::mem::forget(event);
        return Err(ERROR_SEM_TIMEOUT);
    }
    let mut sent = 0u32;
    // SAFETY: ov is the OVERLAPPED of the transfer on `handle`, which has
    // completed (its event is signalled).
    let result = unsafe { WinUsb_GetOverlappedResult(handle, ov.cast_const(), &mut sent, false) };
    free(ov, buffer);
    result.map(|()| sent).map_err(|e| code(&e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_packet_fields_are_taken_as_they_are() {
        let p = setup_packet([0x5F, 0x07, 0x34, 0x12, 0x78, 0x56, 0x04, 0x00]);
        assert_eq!((p.RequestType, p.Request, p.Value, p.Index, p.Length), (0x5F, 0x07, 0x1234, 0x5678, 4));
    }
}
