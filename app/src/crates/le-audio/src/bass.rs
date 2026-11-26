//! Broadcast Audio Scan Service (BASS 1.0), the receiver's side that a
//! Broadcast Assistant reads and drives: the Broadcast Receive State values
//! (which broadcast the receiver keeps, whether it is synchronized to it and
//! to which BIS), and the Broadcast Audio Scan Control Point operations that
//! tell it which broadcast to play (Add, Modify, Remove Source, Set
//! Broadcast_Code). Read by the probe (`--le-probe`), driven by `--assist`.

/// Control Point opcodes (BASS 3.1)
pub const OP_REMOTE_SCAN_STOPPED: u8 = 0x00;
pub const OP_REMOTE_SCAN_STARTED: u8 = 0x01;
pub const OP_ADD_SOURCE: u8 = 0x02;
pub const OP_MODIFY_SOURCE: u8 = 0x03;
pub const OP_SET_BROADCAST_CODE: u8 = 0x04;
pub const OP_REMOVE_SOURCE: u8 = 0x05;

/// PA_Sync of Add / Modify Source: do not synchronize (or stop)
pub const PA_SYNC_OFF: u8 = 0x00;
/// Synchronize, with PAST from the assistant
pub const PA_SYNC_PAST: u8 = 0x01;
/// Synchronize by scanning for it (no PAST)
pub const PA_SYNC_NO_PAST: u8 = 0x02;
/// PA_Interval unknown
pub const PA_INTERVAL_UNKNOWN: u16 = 0xFFFF;
/// BIS_Sync: no preference (the receiver picks)
pub const BIS_NO_PREFERENCE: u32 = 0xFFFF_FFFF;

/// The broadcast an Add Source names, and what the receiver is to
/// synchronize to: `bis_sync` per subgroup of its BASE (bit n: BIS index
/// n + 1; [`BIS_NO_PREFERENCE`])
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// 0 public, 1 random
    pub addr_type: u8,
    /// Most significant byte first, as shown
    pub addr: [u8; 6],
    pub adv_sid: u8,
    pub broadcast_id: u32,
    pub pa_sync: u8,
    pub pa_interval: u16,
    pub bis_sync: Vec<u32>,
}

fn put_subgroups(v: &mut Vec<u8>, bis_sync: &[u32]) {
    v.push(bis_sync.len() as u8);
    for b in bis_sync {
        v.extend_from_slice(&b.to_le_bytes());
        v.push(0); // no metadata
    }
}

/// Add Source
pub fn add_source(s: &Source) -> Vec<u8> {
    let mut v = vec![OP_ADD_SOURCE, s.addr_type];
    v.extend(s.addr.iter().rev());
    v.push(s.adv_sid);
    v.extend_from_slice(&s.broadcast_id.to_le_bytes()[..3]);
    v.push(s.pa_sync);
    v.extend_from_slice(&s.pa_interval.to_le_bytes());
    put_subgroups(&mut v, &s.bis_sync);
    v
}

/// Modify Source: `pa_sync` and the BIS of each subgroup (all 0 and
/// [`PA_SYNC_OFF`] before a Remove Source)
pub fn modify_source(source_id: u8, pa_sync: u8, pa_interval: u16, bis_sync: &[u32]) -> Vec<u8> {
    let mut v = vec![OP_MODIFY_SOURCE, source_id, pa_sync];
    v.extend_from_slice(&pa_interval.to_le_bytes());
    put_subgroups(&mut v, bis_sync);
    v
}

/// Set Broadcast_Code: `code` as typed (4-16 bytes), zero-padded to 16
pub fn set_broadcast_code(source_id: u8, code: &[u8]) -> Vec<u8> {
    let mut v = vec![OP_SET_BROADCAST_CODE, source_id];
    let mut c = [0u8; 16];
    let n = code.len().min(16);
    c[..n].copy_from_slice(&code[..n]);
    v.extend_from_slice(&c);
    v
}

/// Remove Source
pub fn remove_source(source_id: u8) -> Vec<u8> {
    vec![OP_REMOVE_SOURCE, source_id]
}

/// One subgroup of a source: the BIS it is synchronized to (bit n: BIS
/// index n + 1; 0xFFFFFFFF: failed to synchronize) and its metadata
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiveSubgroup {
    pub bis_sync: u32,
    pub metadata: Vec<u8>,
}

/// A Broadcast Receive State value with a source (BASS 3.2)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiveState {
    pub source_id: u8,
    pub addr_type: u8,
    /// As on the air (least significant byte first)
    pub addr: [u8; 6],
    pub adv_sid: u8,
    pub broadcast_id: u32,
    /// 0 not synchronized, 1 SyncInfo request, 2 synchronized, 3 failed,
    /// 4 no PAST
    pub pa_sync: u8,
    /// 0 not encrypted, 1 Broadcast_Code required, 2 decrypting, 3 bad code
    pub big_encryption: u8,
    pub bad_code: Option<[u8; 16]>,
    pub subgroups: Vec<ReceiveSubgroup>,
}

/// The value of a Broadcast Receive State: None when it is empty (no source
/// kept there); Err for a value too short for what it says
pub fn parse_receive_state(v: &[u8]) -> Result<Option<ReceiveState>, String> {
    if v.is_empty() {
        return Ok(None);
    }
    let short = || format!("{} bytes, cut short", v.len());
    let head = v.get(..14).ok_or_else(short)?;
    let mut addr = [0u8; 6];
    addr.copy_from_slice(&head[2..8]);
    let big_encryption = head[13];
    let mut i = 14;
    let bad_code = if big_encryption == 3 {
        let mut c = [0u8; 16];
        c.copy_from_slice(v.get(i..i + 16).ok_or_else(short)?);
        i += 16;
        Some(c)
    } else {
        None
    };
    let n = *v.get(i).ok_or_else(short)?;
    i += 1;
    let mut subgroups = Vec::with_capacity(n.into());
    for _ in 0..n {
        let s = v.get(i..i + 5).ok_or_else(short)?;
        let bis_sync = u32::from_le_bytes([s[0], s[1], s[2], s[3]]);
        let len = usize::from(s[4]);
        i += 5;
        let metadata = v.get(i..i + len).ok_or_else(short)?.to_vec();
        i += len;
        subgroups.push(ReceiveSubgroup { bis_sync, metadata });
    }
    Ok(Some(ReceiveState {
        source_id: head[0],
        addr_type: head[1],
        addr,
        adv_sid: head[8],
        broadcast_id: u32::from_le_bytes([head[9], head[10], head[11], 0]),
        pa_sync: head[12],
        big_encryption,
        bad_code,
        subgroups,
    }))
}

fn pa_sync_str(s: u8) -> String {
    match s {
        0 => "not synchronized".into(),
        1 => "asks for SyncInfo".into(),
        2 => "synchronized".into(),
        3 => "failed to synchronize".into(),
        4 => "no PAST".into(),
        _ => format!("state 0x{s:02X}"),
    }
}

fn encryption_str(s: u8) -> String {
    match s {
        0 => "not encrypted".into(),
        1 => "needs the Broadcast_Code".into(),
        2 => "decrypting".into(),
        3 => "bad Broadcast_Code".into(),
        _ => format!("encryption 0x{s:02X}"),
    }
}

fn bis_str(bis_sync: u32) -> String {
    match bis_sync {
        0 => "no BIS".into(),
        0xFFFF_FFFF => "failed to synchronize to the BIS".into(),
        b => format!(
            "BIS {}",
            (0..32).filter(|n| b & (1 << n) != 0).map(|n| (n + 1).to_string()).collect::<Vec<_>>().join(", ")
        ),
    }
}

impl ReceiveState {
    /// One line: the source, PA and BIG states, the BIS of each subgroup
    pub fn describe(&self) -> String {
        let addr = self.addr.iter().rev().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":");
        let subgroups = self
            .subgroups
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let meta = if s.metadata.is_empty() {
                    String::new()
                } else {
                    format!(" (metadata {})", s.metadata.iter().map(|b| format!("{b:02X}")).collect::<String>())
                };
                format!("subgroup {}: {}{meta}", i + 1, bis_str(s.bis_sync))
            })
            .collect::<Vec<_>>();
        format!(
            "source {}: Broadcast_ID 0x{:06X} at {addr} ({}, SID {}), PA {}, {}; {}",
            self.source_id,
            self.broadcast_id,
            if self.addr_type == 0 { "public" } else { "random" },
            self.adv_sid,
            pa_sync_str(self.pa_sync),
            encryption_str(self.big_encryption),
            if subgroups.is_empty() { "no subgroups".into() } else { subgroups.join("; ") }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(encryption: u8) -> Vec<u8> {
        let mut v = vec![1, 1, 0x66, 0x55, 0x44, 0x33, 0x22, 0xC1, 2, 0x56, 0x34, 0x12, 2, encryption];
        if encryption == 3 {
            v.extend([0xEE; 16]);
        }
        v.push(2);
        v.extend([0b01, 0, 0, 0, 0]);
        v.extend([0b10, 0, 0, 0, 3, 2, 1, 0]);
        v
    }

    #[test]
    fn an_empty_state_has_no_source() {
        assert_eq!(parse_receive_state(&[]), Ok(None));
    }

    #[test]
    fn a_state_with_two_subgroups_is_read() {
        let s = parse_receive_state(&state(0)).unwrap().unwrap();
        assert_eq!(s.broadcast_id, 0x123456);
        assert_eq!(s.addr, [0x66, 0x55, 0x44, 0x33, 0x22, 0xC1]);
        assert_eq!(s.bad_code, None);
        assert_eq!(s.subgroups.len(), 2);
        assert_eq!(s.subgroups[1].metadata, vec![2, 1, 0]);
        assert_eq!(
            s.describe(),
            "source 1: Broadcast_ID 0x123456 at C1:22:33:44:55:66 (random, SID 2), PA synchronized, not encrypted; \
             subgroup 1: BIS 1; subgroup 2: BIS 2 (metadata 020100)"
        );
    }

    #[test]
    fn a_bad_code_is_read_before_the_subgroups() {
        let s = parse_receive_state(&state(3)).unwrap().unwrap();
        assert_eq!(s.bad_code, Some([0xEE; 16]));
        assert_eq!(s.subgroups[0].bis_sync, 1);
    }

    #[test]
    fn control_point_operations_are_built_as_bass_says() {
        let s = Source {
            addr_type: 1,
            addr: [0xC1, 0x22, 0x33, 0x44, 0x55, 0x66],
            adv_sid: 2,
            broadcast_id: 0x123456,
            pa_sync: PA_SYNC_NO_PAST,
            pa_interval: PA_INTERVAL_UNKNOWN,
            bis_sync: vec![0b01, 0b10],
        };
        assert_eq!(
            add_source(&s),
            vec![
                0x02, 1, 0x66, 0x55, 0x44, 0x33, 0x22, 0xC1, 2, 0x56, 0x34, 0x12, 2, 0xFF, 0xFF, 2, 1, 0, 0, 0, 0, 2,
                0, 0, 0, 0
            ]
        );
        assert_eq!(modify_source(3, PA_SYNC_OFF, 0xFFFF, &[0]), vec![0x03, 3, 0, 0xFF, 0xFF, 1, 0, 0, 0, 0, 0]);
        assert_eq!(remove_source(3), vec![0x05, 3]);
        let code = set_broadcast_code(1, b"1234");
        assert_eq!((code.len(), &code[..6], code[17]), (18, &[0x04, 1, b'1', b'2', b'3', b'4'][..], 0));
    }

    #[test]
    fn a_cut_value_is_an_error() {
        let v = state(0);
        assert!(parse_receive_state(&v[..v.len() - 1]).is_err());
        assert!(parse_receive_state(&v[..10]).is_err());
    }
}
