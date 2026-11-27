//! Classic L2CAP channels in basic mode to a device the stack has an ACL
//! link with, for a vendor's control protocol that runs on a PSM of its
//! own rather than over RFCOMM. The stack never connects or pairs for them:
//! without a link there is no channel.
//!
//! A channel is opened by its client ([`crate::BtHost::l2cap_open`]) or by
//! the device: while a client listens on a PSM for that device
//! ([`crate::BtHost::l2cap_listen`]), a channel the device opens there is
//! offered to it, and declined when the client does not take it or nobody
//! listens. One L2CAP packet is one message: data is sent and received
//! whole, never cut or joined.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;

use btstack_sys::consts::*;
use btstack_sys::{AclState, Stack};

use crate::events::{addr_at, event_type, le16, u8_at};

/// Channels open or opening at a time (all clients together)
const MAX_CHANNELS: usize = 8;
/// Listeners at a time
const MAX_LISTENERS: usize = 4;
/// Packets a channel queues for sending before a send is refused
const MAX_QUEUED: usize = 64;
/// The largest packet received (BTstack's largest for an ACL payload)
pub const MTU: u16 = 1691;

/// An open channel: its id (unique in this process, also across restarts
/// of the stack) and the largest packet the device takes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct L2capOpened {
    pub id: u32,
    pub mtu: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum L2capError {
    /// The stack has no A2DP connection to the device (or is not running)
    NotConnected,
    /// BTstack or the device refused (BTstack's or the device's status)
    Refused(u8),
    /// No open channel or listener with this id (closed, or the stack
    /// restarted)
    NoChannel,
    /// Too many channels or listeners, or the channel's send queue is full
    Busy,
    /// A packet larger than the device takes
    TooLong,
    /// No answer in time: the channel is closed if it opens later; a send
    /// or close may still be taken
    NoAnswer,
}

/// Why a channel closed (not for a close of its own client)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum L2capCloseReason {
    /// The device closed it, or the link to the device went down
    Remote,
    /// The A2DP connection to the device ended, together with the link
    Disconnected,
    /// The Bluetooth stack stopped (restart, adapter removed, core exit)
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum L2capEvent {
    /// A channel the device opened is open (the first event of an accepted
    /// channel; a channel of [`crate::BtHost::l2cap_open`] has none)
    Opened(L2capOpened),
    Data(Vec<u8>),
    /// The last event of the channel
    Closed(L2capCloseReason),
}

/// Receives a channel's events with its id, on the BTstack thread: it must
/// not block. False after [`L2capEvent::Data`] or
/// [`L2capEvent::Opened`]: close the channel (no event follows).
pub type L2capSink = Box<dyn FnMut(u32, L2capEvent) -> bool + Send>;

/// Asked on the BTstack thread when the device opens a channel to a PSM
/// listened on: a sink takes the channel, None declines it. Must not block.
pub type L2capAccept = Box<dyn FnMut() -> Option<L2capSink> + Send>;

static NEXT_ID: AtomicU32 = AtomicU32::new(1);

pub(crate) type OpenReply = mpsc::SyncSender<Result<L2capOpened, L2capError>>;

/// The outcome of [`crate::BtHost::l2cap_send`] / `l2cap_close`, which
/// return before the BTstack thread has taken them
pub struct L2capReply(pub(crate) mpsc::Receiver<Result<(), L2capError>>);

impl L2capReply {
    pub fn wait(self) -> Result<(), L2capError> {
        match self.0.recv_timeout(crate::JOB_TIMEOUT) {
            Ok(r) => r,
            Err(mpsc::RecvTimeoutError::Timeout) => Err(L2capError::NoAnswer),
            // The stack stopped: its channels are gone
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(L2capError::NoChannel),
        }
    }
}

struct Channel {
    id: u32,
    cid: u16,
    addr: [u8; 6],
    /// The device's MTU, once open
    mtu: u16,
    sink: L2capSink,
    /// An open of this side: answered once open or refused (None: answered
    /// already, or the device opened it)
    reply: Option<OpenReply>,
    /// Not open yet (CHANNEL_OPENED to come)
    opening: bool,
    queue: VecDeque<Vec<u8>>,
    can_send_asked: bool,
    /// Its client closed it: it sends what is queued, then disconnects, and
    /// its sink gets nothing more
    closing: bool,
}

struct Listener {
    id: u32,
    addr: [u8; 6],
    psm: u16,
    accept: L2capAccept,
}

/// The channels and listeners (BTstack thread only)
#[derive(Default)]
pub(crate) struct L2cap {
    channels: Vec<Channel>,
    listeners: Vec<Listener>,
}

impl L2cap {
    pub fn open(&mut self, stack: &Stack, addr: [u8; 6], psm: u16, sink: L2capSink, reply: OpenReply) {
        if !matches!(stack.acl_state(&addr), AclState::Open(_)) {
            tracing::info!("L2CAP: no link to the device, not opening PSM 0x{psm:04X}");
            let _ = reply.send(Err(L2capError::NotConnected));
            return;
        }
        if self.channels.len() >= MAX_CHANNELS {
            tracing::info!("L2CAP: {MAX_CHANNELS} channels open or opening, not opening another");
            let _ = reply.send(Err(L2capError::Busy));
            return;
        }
        match stack.l2cap_create_channel(&addr, psm, MTU) {
            Ok(cid) => {
                let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
                tracing::info!("L2CAP: opening #{id} to PSM 0x{psm:04X}");
                self.channels.push(Channel {
                    id,
                    cid,
                    addr,
                    mtu: 0,
                    sink,
                    reply: Some(reply),
                    opening: true,
                    queue: VecDeque::new(),
                    can_send_asked: false,
                    closing: false,
                });
            }
            Err(status) => {
                tracing::info!("L2CAP: PSM 0x{psm:04X} not opened (0x{status:02X})");
                let _ = reply.send(Err(L2capError::Refused(status)));
            }
        }
    }

    /// Offers channels the device opens to `psm` to `accept`
    pub fn listen(&mut self, stack: &Stack, addr: [u8; 6], psm: u16, accept: L2capAccept) -> Result<u32, L2capError> {
        if self.listeners.len() >= MAX_LISTENERS {
            return Err(L2capError::Busy);
        }
        if !self.listeners.iter().any(|l| l.psm == psm) {
            let status = stack.l2cap_register_service(psm, MTU);
            // Registered already (a run of an earlier listener): fine
            if status != ERROR_CODE_SUCCESS && status != L2CAP_SERVICE_ALREADY_REGISTERED {
                tracing::info!("L2CAP: cannot listen on PSM 0x{psm:04X} (0x{status:02X})");
                return Err(L2capError::Refused(status));
            }
        }
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        tracing::info!("L2CAP: listening on PSM 0x{psm:04X} (#{id})");
        self.listeners.push(Listener { id, addr, psm, accept });
        Ok(id)
    }

    pub fn unlisten(&mut self, stack: &Stack, id: u32) -> Result<(), L2capError> {
        let i = self.listeners.iter().position(|l| l.id == id).ok_or(L2capError::NoChannel)?;
        let l = self.listeners.remove(i);
        tracing::info!("L2CAP: no longer listening on PSM 0x{:04X} (#{id})", l.psm);
        self.unregister_unused(stack, l.psm);
        Ok(())
    }

    fn unregister_unused(&self, stack: &Stack, psm: u16) {
        if !self.listeners.iter().any(|l| l.psm == psm) {
            stack.l2cap_unregister_service(psm);
        }
    }

    /// Source::L2capClassic: channel events, and data (packet type
    /// L2CAP_DATA_PACKET, `channel` its local cid)
    pub fn on_packet(&mut self, stack: &Stack, packet_type: u8, channel: u16, p: &[u8]) {
        if packet_type == L2CAP_DATA_PACKET {
            let Some(i) = self.channels.iter().position(|c| c.cid == channel) else {
                tracing::debug!("L2CAP: {} bytes for unknown cid 0x{channel:04X}", p.len());
                return;
            };
            let c = &mut self.channels[i];
            if c.closing || c.opening {
                return;
            }
            tracing::trace!("L2CAP: #{} received {} bytes", c.id, p.len());
            if !(c.sink)(c.id, L2capEvent::Data(p.to_vec())) {
                let c = self.channels.remove(i);
                tracing::info!("L2CAP: #{} closed, its client does not take its data", c.id);
                stack.l2cap_disconnect(c.cid);
            }
            return;
        }
        if packet_type != HCI_EVENT_PACKET {
            return;
        }
        match event_type(p) {
            Some(L2CAP_EVENT_INCOMING_CONNECTION) => self.on_incoming(stack, p),
            Some(L2CAP_EVENT_CHANNEL_OPENED) => self.on_opened(stack, p),
            Some(L2CAP_EVENT_CHANNEL_CLOSED) => {
                let Some(cid) = le16(p, 2) else { return };
                if let Some(i) = self.channels.iter().position(|c| c.cid == cid) {
                    let mut c = self.channels.remove(i);
                    if c.closing {
                        tracing::info!("L2CAP: #{} closed before its queue was sent", c.id);
                    } else if c.opening {
                        // BTstack reports a failed open as CHANNEL_OPENED;
                        // this is for a channel declined meanwhile
                        tracing::info!("L2CAP: #{} closed while opening", c.id);
                        if let Some(reply) = c.reply.take() {
                            let _ = reply.send(Err(L2capError::Refused(ERROR_CODE_UNSPECIFIED_ERROR)));
                        }
                    } else {
                        tracing::info!("L2CAP: #{} closed by the device or the link", c.id);
                        (c.sink)(c.id, L2capEvent::Closed(L2capCloseReason::Remote));
                    }
                }
            }
            Some(L2CAP_EVENT_CAN_SEND_NOW) => {
                let Some(cid) = le16(p, 2) else { return };
                if let Some(i) = self.channels.iter().position(|c| c.cid == cid) {
                    let c = &mut self.channels[i];
                    c.can_send_asked = false;
                    c.send_next(stack);
                    if c.closing && c.queue.is_empty() {
                        let c = self.channels.remove(i);
                        tracing::info!("L2CAP: #{} sent its queue, closing", c.id);
                        stack.l2cap_disconnect(c.cid);
                    }
                }
            }
            _ => {}
        }
    }

    /// L2CAP_EVENT_INCOMING_CONNECTION: address, handle, PSM, local cid
    fn on_incoming(&mut self, stack: &Stack, p: &[u8]) {
        let (Some(addr), Some(psm), Some(cid)) = (addr_at(p, 2), le16(p, 10), le16(p, 12)) else { return };
        let full = self.channels.len() >= MAX_CHANNELS;
        let sink = if full {
            None
        } else {
            self.listeners.iter_mut().find(|l| l.psm == psm && l.addr == addr).and_then(|l| (l.accept)())
        };
        let Some(sink) = sink else {
            tracing::info!("L2CAP: the device opens PSM 0x{psm:04X}, declined (nobody takes it)");
            stack.l2cap_decline(cid);
            return;
        };
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        tracing::info!("L2CAP: the device opens PSM 0x{psm:04X}, accepted as #{id}");
        self.channels.push(Channel {
            id,
            cid,
            addr,
            mtu: 0,
            sink,
            reply: None,
            opening: true,
            queue: VecDeque::new(),
            can_send_asked: false,
            closing: false,
        });
        stack.l2cap_accept(cid);
    }

    /// L2CAP_EVENT_CHANNEL_OPENED: status 2, local cid 13, remote MTU 19
    fn on_opened(&mut self, stack: &Stack, p: &[u8]) {
        let (Some(status), Some(cid), Some(mtu)) = (u8_at(p, 2), le16(p, 13), le16(p, 19)) else { return };
        let Some(i) = self.channels.iter().position(|c| c.cid == cid && c.opening) else {
            tracing::debug!("L2CAP: channel opened event for cid 0x{cid:04X} of no open (0x{status:02X})");
            return;
        };
        if status != ERROR_CODE_SUCCESS {
            let mut c = self.channels.remove(i);
            tracing::info!("L2CAP: #{} refused (0x{status:02X})", c.id);
            if let Some(reply) = c.reply.take() {
                let _ = reply.send(Err(L2capError::Refused(status)));
            } else {
                (c.sink)(c.id, L2capEvent::Closed(L2capCloseReason::Remote));
            }
            return;
        }
        let c = &mut self.channels[i];
        c.opening = false;
        c.mtu = mtu.max(1);
        let opened = L2capOpened { id: c.id, mtu: c.mtu };
        tracing::info!("L2CAP: #{} open (packets up to {} bytes)", c.id, c.mtu);
        let taken = match c.reply.take() {
            Some(reply) => reply.send(Ok(opened)).is_ok(),
            None => (c.sink)(c.id, L2capEvent::Opened(opened)),
        };
        if !taken {
            let c = self.channels.remove(i);
            tracing::info!("L2CAP: #{} opened after its client stopped waiting, closing it", c.id);
            stack.l2cap_disconnect(c.cid);
        }
    }

    pub fn send(&mut self, stack: &Stack, id: u32, data: Vec<u8>) -> Result<(), L2capError> {
        let c =
            self.channels.iter_mut().find(|c| c.id == id && !c.closing && !c.opening).ok_or(L2capError::NoChannel)?;
        if data.len() > usize::from(c.mtu) {
            tracing::info!("L2CAP: #{id} {} bytes refused, the device takes {}", data.len(), c.mtu);
            return Err(L2capError::TooLong);
        }
        if c.queue.len() >= MAX_QUEUED {
            tracing::info!("L2CAP: #{id} send queue full ({} packets)", c.queue.len());
            return Err(L2capError::Busy);
        }
        c.queue.push_back(data);
        c.send_next(stack);
        Ok(())
    }

    /// A close of the channel's own client, once what it queued is sent:
    /// no event follows
    pub fn close(&mut self, stack: &Stack, id: u32) -> Result<(), L2capError> {
        let i = self.channels.iter().position(|c| c.id == id && !c.closing).ok_or(L2capError::NoChannel)?;
        if self.channels[i].queue.is_empty() || self.channels[i].opening {
            let c = self.channels.remove(i);
            tracing::info!("L2CAP: #{id} closed by its client");
            stack.l2cap_disconnect(c.cid);
        } else {
            let c = &mut self.channels[i];
            tracing::info!("L2CAP: #{id} closed by its client, after its {} queued packets", c.queue.len());
            c.closing = true;
        }
        Ok(())
    }

    /// The A2DP connection to `addr` ended: its channels and listeners go
    /// with it, so that the link goes down as it does without them
    pub fn close_device(&mut self, stack: &Stack, addr: [u8; 6]) {
        let (gone, kept) = std::mem::take(&mut self.channels).into_iter().partition(|c| c.addr == addr);
        self.channels = kept;
        for mut c in gone {
            tracing::info!("L2CAP: #{} closed with the A2DP connection", c.id);
            stack.l2cap_disconnect(c.cid);
            if let Some(reply) = c.reply.take() {
                let _ = reply.send(Err(L2capError::NotConnected));
            } else if !c.closing {
                (c.sink)(c.id, L2capEvent::Closed(L2capCloseReason::Disconnected));
            }
        }
        let (gone, kept): (Vec<_>, Vec<_>) =
            std::mem::take(&mut self.listeners).into_iter().partition(|l| l.addr == addr);
        self.listeners = kept;
        for l in gone {
            tracing::info!("L2CAP: listener #{} ends with the A2DP connection", l.id);
            self.unregister_unused(stack, l.psm);
        }
    }

    /// The run loop exits: every channel and listener ends (BTstack's
    /// services go with l2cap_deinit)
    pub fn stop(&mut self) {
        for mut c in self.channels.drain(..) {
            tracing::info!("L2CAP: #{} closed, the stack stops", c.id);
            if let Some(reply) = c.reply.take() {
                let _ = reply.send(Err(L2capError::NotConnected));
            } else if !c.closing {
                (c.sink)(c.id, L2capEvent::Closed(L2capCloseReason::Stopped));
            }
        }
        self.listeners.clear();
    }
}

impl Channel {
    /// Sends the queued packets while L2CAP takes them, then asks for the
    /// moment it can take more
    fn send_next(&mut self, stack: &Stack) {
        if self.can_send_asked {
            return;
        }
        while let Some(packet) = self.queue.front() {
            let status = stack.l2cap_send(self.cid, packet);
            if status != ERROR_CODE_SUCCESS {
                // No buffers now: once it can
                tracing::trace!("L2CAP: #{} send deferred (0x{status:02X})", self.id);
                break;
            }
            self.queue.pop_front();
        }
        if !self.queue.is_empty() {
            self.can_send_asked = stack.l2cap_request_can_send_now(self.cid) == ERROR_CODE_SUCCESS;
        }
    }
}
