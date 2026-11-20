//! Developer > Debug: one HCI command typed in as hex, sent once after HCI
//! init on a MediaTek controller (issue #86: trying a vendor command whose
//! effect is not documented). Nothing is sent unless one is entered; its
//! answer is logged. Whether an HCI Reset undoes it is not known: taking it
//! out and a reboot (or replugging) is the way back.
//!
//! Set to go before reset, it is sent instead once the firmware loader is
//! done, before HCI init (crate::mtk_debug_steps), as is the register write
//! typed in here too ([`parse_register`]).

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::bringup::{ControlData, ControlKind, ControlRecipient, ControlRequest};

static COMMAND: Mutex<Option<Vec<u8>>> = Mutex::new(None);
static BEFORE_RESET: AtomicBool = AtomicBool::new(false);
static REGISTER: Mutex<Option<ControlRequest>> = Mutex::new(None);

/// The command for the next sessions (None: none)
pub fn set(packet: Option<Vec<u8>>) {
    *COMMAND.lock().unwrap_or_else(|e| e.into_inner()) = packet;
}

/// Whether the command goes before HCI init instead of after it
pub fn set_before_reset(on: bool) {
    BEFORE_RESET.store(on, Ordering::Relaxed);
}

pub(crate) fn before_reset() -> bool {
    BEFORE_RESET.load(Ordering::Relaxed)
}

/// The register write for the next adapter starts (None: none)
pub fn set_register(request: Option<ControlRequest>) {
    *REGISTER.lock().unwrap_or_else(|e| e.into_inner()) = request;
}

pub(crate) fn register() -> Option<ControlRequest> {
    REGISTER.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// "40 02 12345678 9ABCDEF0": the request type and the request as two hex
/// digits each, then the register and the value as eight; a USB control
/// transfer to the device's default pipe with the register's upper half as
/// value, its lower half as index and the value's 4 bytes little-endian as
/// data (the layout the loader's register reads use). The request type must
/// be host-to-device and of the standard, class or vendor type; its
/// recipient bits go out as typed, also the ones USB reserves. Ok(None):
/// nothing entered.
pub fn parse_register(text: &str) -> Result<Option<ControlRequest>, String> {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    if tokens.is_empty() {
        return Ok(None);
    }
    let [request_type, request, register, value] = tokens[..] else {
        return Err("four values: request type, request, register, value".into());
    };
    let byte = |t: &str| match t.len() {
        2 => u8::from_str_radix(t, 16).map_err(|_| format!("\"{t}\" is not hex")),
        _ => Err(format!("\"{t}\": two hex digits")),
    };
    let word = |t: &str| match t.len() {
        8 => u32::from_str_radix(t, 16).map_err(|_| format!("\"{t}\" is not hex")),
        _ => Err(format!("\"{t}\": eight hex digits")),
    };
    let (request_type, request, register, value) = (byte(request_type)?, byte(request)?, word(register)?, word(value)?);
    if request_type & 0x80 != 0 {
        return Err("request type: host to device only".into());
    }
    let kind = match request_type & 0x60 {
        0x00 => ControlKind::Standard,
        0x20 => ControlKind::Class,
        0x40 => ControlKind::Vendor,
        _ => return Err("request type: reserved type".into()),
    };
    let recipient = match request_type & 0x1F {
        0 => ControlRecipient::Device,
        1 => ControlRecipient::Interface,
        2 => ControlRecipient::Endpoint,
        3 => ControlRecipient::Other,
        // Reserved by USB: sent as they are (hci-transport over WinUSB)
        bits => ControlRecipient::Reserved(bits),
    };
    Ok(Some(ControlRequest {
        kind,
        recipient,
        request,
        value: (register >> 16) as u16,
        index: register as u16,
        data: ControlData::Out(value.to_le_bytes().to_vec()),
    }))
}

/// Developer > Debug: SDUs queued per CIS on a MediaTek controller for the
/// next streams; 0: the MediaTek default (crate::le::iso_send)
pub fn set_iso_queue(depth: u8) {
    crate::le::iso_send::set_debug_depth(depth);
}

pub(crate) fn get() -> Option<Vec<u8>> {
    COMMAND.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// "FD98 08 01": the opcode as four hex digits, then each parameter byte
/// as two, separated by spaces; the command packet (opcode little-endian,
/// parameter length, parameters). Ok(None): nothing entered.
pub fn parse(text: &str) -> Result<Option<Vec<u8>>, String> {
    let mut tokens = text.split_whitespace();
    let Some(op) = tokens.next() else { return Ok(None) };
    if op.len() != 4 {
        return Err(format!("opcode \"{op}\": four hex digits"));
    }
    let opcode = u16::from_str_radix(op, 16).map_err(|_| format!("opcode \"{op}\" is not hex"))?;
    let params = tokens
        .map(|t| match t.len() {
            2 => u8::from_str_radix(t, 16).map_err(|_| format!("\"{t}\" is not hex")),
            _ => Err(format!("parameter \"{t}\": two hex digits")),
        })
        .collect::<Result<Vec<u8>, String>>()?;
    let len = u8::try_from(params.len()).map_err(|_| "more than 255 parameter bytes".to_string())?;
    let mut packet = opcode.to_le_bytes().to_vec();
    packet.push(len);
    packet.extend(params);
    Ok(Some(packet))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_opcode_and_parameters() {
        assert_eq!(parse("FD98 08 01"), Ok(Some(vec![0x98, 0xFD, 0x02, 0x08, 0x01])));
        assert_eq!(parse("  0c03 "), Ok(Some(vec![0x03, 0x0C, 0x00])));
        assert_eq!(parse(""), Ok(None));
    }

    #[test]
    fn refuses_malformed_input() {
        assert!(parse("FD9 08").is_err());
        assert!(parse("FD98 8").is_err());
        assert!(parse("FD98 GG").is_err());
        assert!(parse(&format!("FD98{}", " 00".repeat(256))).is_err());
    }

    #[test]
    fn parses_a_register_write() {
        assert_eq!(parse_register("  "), Ok(None));
        assert_eq!(
            parse_register("41 07 12345678 0A0B0C0D"),
            Ok(Some(ControlRequest {
                kind: ControlKind::Vendor,
                recipient: ControlRecipient::Interface,
                request: 0x07,
                value: 0x1234,
                index: 0x5678,
                data: ControlData::Out(vec![0x0D, 0x0C, 0x0B, 0x0A]),
            }))
        );
    }

    #[test]
    fn refuses_register_writes_it_cannot_send() {
        assert!(parse_register("40 07 12345678").is_err());
        assert!(parse_register("40 07 1234567 0A0B0C0D").is_err());
        assert!(parse_register("C0 07 12345678 0A0B0C0D").is_err(), "device to host");
        assert!(parse_register("60 07 12345678 0A0B0C0D").is_err(), "reserved type");
        // A reserved recipient is kept as typed
        let r = parse_register("5F 07 12345678 0A0B0C0D").unwrap().unwrap();
        assert_eq!((r.kind, r.recipient), (ControlKind::Vendor, ControlRecipient::Reserved(0x1F)));
        assert_eq!(r.setup_packet()[0], 0x5F);
    }
}
