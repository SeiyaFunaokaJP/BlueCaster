//! Broadcast source (Auracast, developer mode) on the BTstack thread: per
//! broadcast of the configuration (`le_audio::broadcast::groups`: one, or
//! left and right as broadcasts of their own) an extended + periodic
//! advertising set announcing it (Broadcast Audio / Public Broadcast
//! Announcement, Broadcast Name, the BASE) and a BIG with its BIS, all fed
//! by the same jitter buffer and [`super::iso_send::IsoSender`] as a
//! unicast stream (one SDU set per interval across every BIG).
//!
//! start: advertising and LE Create BIG for each broadcast ->
//! GAP_SUBEVENT_BIG_CREATED for every BIG (BTstack has set up the ISO data
//! paths) -> sending on all BIS, `done(true)`. A BIG that fails ends the
//! others too. stop: stop sending, LE Terminate BIG for each ->
//! GAP_SUBEVENT_BIG_TERMINATED for every one -> advertising removed
//! ("Auracast: stopped"). A BIG the controller ends by itself counts as a
//! lost stream and ends the others.
//!
//! Slow sending (the controller takes only some of the rounds for a while,
//! seen on a Realtek RTL8761 at half the rate): the BIGs are terminated and
//! created again with the same parameters while the advertising stays, at
//! most [`MAX_RECREATES`] times per broadcast.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use btstack_sys::BigParams;
use btstack_sys::consts::{ERROR_CODE_COMMAND_DISALLOWED, ERROR_CODE_SUCCESS};
use btstack_sys::events;
use le_audio::broadcast::{
    ADV_INTERVAL, BroadcastConfig, BroadcastGroup, PA_INTERVAL, describe, group_adv_data, group_base_ad, group_plan,
    groups,
};
use le_audio::plan::StreamPlan;

use super::LeStreamStatus;
use super::core::{LeCtx, LeDone, LeTimerId, SharedSduQueue};
use super::iso_send::queue_depth;

/// A stop that gets no LE Terminate BIG Complete ends after this anyway
const TERMINATE_TIMEOUT_MS: u32 = 3000;

/// Times a broadcast's BIGs are set up again after slow sending at most
pub(crate) const MAX_RECREATES: u32 = 3;

/// A BIG created again is refused (Command Disallowed) while BTstack still
/// holds the one just terminated: asked again this often, this many times
const CREATE_RETRY_MS: u32 = 20;
const CREATE_TRIES: u32 = 50;
/// BIGs created again that are not all up after this long: the stream is
/// lost (nothing waits for them as a start does)
const RECREATE_TIMEOUT_MS: u32 = 5000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// LE Create BIG sent
    Creating,
    Streaming,
    /// Slow sending: LE Terminate BIG sent, created again once all ended
    Recreating,
    /// LE Terminate BIG sent (or waiting for a BIG still being created)
    Terminating,
}

/// One BIG of the broadcast
#[derive(Debug, Clone, PartialEq, Eq)]
enum Big {
    /// LE Create BIG sent, no answer yet
    Pending,
    /// Created: its BIS handles
    Up(Vec<u16>),
    /// LE Terminate BIG sent, no answer yet
    Ending,
    /// Not created, failed or terminated
    Gone,
}

/// The running broadcast (LeCore::broadcast)
pub(crate) struct Broadcast {
    state: State,
    plan: StreamPlan,
    groups: Vec<BroadcastGroup>,
    /// Per group
    bigs: Vec<Big>,
    /// Per group: its LE Create BIG
    params: Vec<BigParams>,
    /// Times the BIGs were set up again after slow sending
    recreates: u32,
    /// The start waiting for the BIGs
    started: Option<(LeDone, Arc<Mutex<String>>)>,
    /// Stops waiting for the BIGs to end
    stopped: Vec<LeDone>,
    terminate_timer: Option<LeTimerId>,
    /// BIGs set up again: LE Create BIG asked again after a refusal, then
    /// the wait for them to come up
    retry_timer: Option<LeTimerId>,
}

impl Broadcast {
    /// Bluetooth stopped (stack deinit): no BTstack calls; the waits end
    pub(crate) fn abandon(mut self) {
        if let Some((start, why)) = self.started.take() {
            set_why(&why, "Bluetooth stopped");
            start(false);
        }
        for d in self.stopped.drain(..) {
            d(true);
        }
    }

    fn all_gone(&self) -> bool {
        self.bigs.iter().all(|b| *b == Big::Gone)
    }

    fn group_of(&self, big_handle: u8) -> Option<usize> {
        self.groups.iter().position(|g| g.big_handle == big_handle)
    }

    /// Streaming, or its BIGs being set up again (the receivers keep it)
    fn on_air(&self) -> bool {
        self.state == State::Streaming || (self.recreates > 0 && self.state != State::Terminating)
    }
}

fn set_why(why: &Arc<Mutex<String>>, s: &str) {
    *why.lock().unwrap_or_else(|e| e.into_inner()) = s.into();
}

impl LeCtx<'_> {
    /// le_start_broadcast's work: advertising and LE Create BIG for each
    /// broadcast; `done(true)` once the BIS of all are sending
    pub(crate) fn start_broadcast(
        &mut self,
        config: BroadcastConfig,
        plan: StreamPlan,
        queue: SharedSduQueue,
        status: LeStreamStatus,
        why: Arc<Mutex<String>>,
        done: LeDone,
    ) {
        if self.core.broadcast.is_some() {
            set_why(&why, "a broadcast is still being stopped");
            done(false);
            return;
        }
        if self.core.status.is_some() {
            set_why(&why, "an LE Audio stream is running");
            done(false);
            return;
        }
        let groups = groups(&config);
        if groups.len() > usize::from(btstack_sys::BROADCAST_SLOTS) {
            set_why(&why, &format!("at most {} broadcasts at once", btstack_sys::BROADCAST_SLOTS));
            done(false);
            return;
        }
        for (i, g) in groups.iter().enumerate() {
            let adv = group_adv_data(&config, g);
            let pa = group_base_ad(&plan, g);
            if let Err(status) =
                self.stack.broadcast_advertise(g.big_handle, g.sid, ADV_INTERVAL, PA_INTERVAL, &adv, &pa)
            {
                tracing::warn!("Auracast: advertising \"{}\" failed (0x{status:02X})", g.name);
                for earlier in &groups[..i] {
                    self.stack.broadcast_advertise_stop(earlier.big_handle);
                }
                let what = if i == 0 { String::new() } else { format!(" {} broadcasts at once", i + 1) };
                set_why(&why, &format!("the adapter cannot advertise{what} (0x{status:02X})"));
                done(false);
                return;
            }
        }
        let mut bigs = Vec::with_capacity(groups.len());
        let mut all_params = Vec::with_capacity(groups.len());
        let mut failed = None;
        // The test knobs below only for more than one BIG: a single
        // broadcast keeps its preset
        let many = groups.len() > 1;
        for g in &groups {
            // Each BIG at its broadcast's own configuration (a matrix
            // layout's rear may run at another rate)
            let gp = group_plan(&plan, g);
            let p = &gp.preset;
            let params = BigParams {
                big_handle: g.big_handle,
                num_bis: g.bis.len() as u8,
                sdu_interval_us: p.sdu_interval_us(),
                max_sdu: p.octets,
                max_transport_latency_ms: many.then(latency_override).flatten().unwrap_or(p.max_transport_latency_ms),
                rtn: many.then(rtn_override).flatten().unwrap_or(p.rtn),
                phy: plan.phy,
                framing: p.framing,
                code: config.code,
            };
            if failed.is_some() {
                bigs.push(Big::Gone);
                all_params.push(params);
                continue;
            }
            let status_code = self.stack.big_create(&params);
            if status_code == ERROR_CODE_SUCCESS {
                bigs.push(Big::Pending);
            } else {
                tracing::warn!("Auracast: LE Create BIG {} refused (0x{status_code:02X})", g.big_handle);
                failed = Some(format!("LE Create BIG failed (0x{status_code:02X})"));
                bigs.push(Big::Gone);
            }
            all_params.push(params);
        }
        let names: Vec<&str> = groups.iter().map(|g| g.name.as_str()).collect();
        tracing::debug!("Auracast: advertising {names:?}, creating {} BIG(s): {}", groups.len(), describe(&plan));
        self.core.queue = Some(queue);
        self.core.status = Some(status);
        self.core.broadcast = Some(Broadcast {
            state: State::Creating,
            plan,
            groups,
            bigs,
            params: all_params,
            recreates: 0,
            started: Some((done, why)),
            stopped: Vec::new(),
            terminate_timer: None,
            retry_timer: None,
        });
        if let Some(reason) = failed {
            self.fail_start(&reason);
        }
    }

    /// The start failed: its wait ends with `reason`, the BIGs created so
    /// far are ended
    fn fail_start(&mut self, reason: &str) {
        let Some(b) = self.core.broadcast.as_mut() else { return };
        tracing::warn!("Auracast: {reason}");
        if let Some((start, w)) = b.started.take() {
            set_why(&w, reason);
            start(false);
        } else if b.state != State::Terminating && !self.core.shared.stream_lost.swap(true, Ordering::AcqRel) {
            // Started already (BIGs set up again): the stream ends
            tracing::warn!("LE Audio: stream lost: {reason}");
        }
        self.terminate_bigs();
    }

    /// le_stop_broadcast's work: stop sending (after the round going on, so
    /// every BIS ends on the same SDU), LE Terminate BIG for each, then the
    /// advertising goes; `done(true)` at the end
    pub(crate) fn stop_broadcast(&mut self, done: LeDone) {
        let Some(b) = self.core.broadcast.as_mut() else {
            self.core.iso.stop(self.stack);
            done(true);
            return;
        };
        if let Some((start, why)) = b.started.take() {
            set_why(&why, "stopped");
            start(false);
        }
        b.stopped.push(done);
        let state = b.state;
        if state == State::Streaming && self.core.iso.finish_round(self.stack) {
            // Goes on in on_last_round_sent
            return;
        }
        self.core.iso.stop(self.stack);
        if state != State::Terminating {
            self.terminate_bigs();
        }
    }

    /// The round going on when the stop came is out (or its time ran out)
    pub(crate) fn on_last_round_sent(&mut self) {
        if self.core.broadcast.as_ref().is_some_and(|b| b.state == State::Streaming) {
            self.terminate_bigs();
        } else {
            self.core.iso.stop(self.stack);
        }
    }

    /// Terminating: LE Terminate BIG for each BIG that is up (one still
    /// being created is ended once it is); finished when none is left
    fn terminate_bigs(&mut self) {
        self.core.iso.stop(self.stack);
        let Some(b) = self.core.broadcast.as_mut() else { return };
        b.state = State::Terminating;
        if let Some(t) = b.retry_timer.take() {
            self.cancel_timer(t);
        }
        let Some(b) = self.core.broadcast.as_mut() else { return };
        for (g, big) in b.groups.iter().zip(b.bigs.iter_mut()) {
            if matches!(big, Big::Up(_)) {
                let status = self.stack.big_terminate(g.big_handle);
                tracing::debug!("Auracast: terminating BIG {}: 0x{status:02X}", g.big_handle);
                if status != ERROR_CODE_SUCCESS {
                    *big = Big::Gone;
                }
            }
        }
        if b.all_gone() {
            self.finish_broadcast();
            return;
        }
        self.arm_terminate_timer();
    }

    /// The broadcast goes if LE Terminate BIG gets no answer in time
    fn arm_terminate_timer(&mut self) {
        if self.core.broadcast.as_ref().is_none_or(|b| b.terminate_timer.is_some()) {
            return;
        }
        let timer = self.run_after(
            TERMINATE_TIMEOUT_MS,
            Box::new(|ctx| {
                let Some(b) = ctx.core.broadcast.as_mut() else { return };
                b.terminate_timer = None;
                match b.state {
                    State::Terminating => {
                        tracing::warn!("Auracast: no answer to LE Terminate BIG, removing the broadcast");
                        ctx.finish_broadcast();
                    }
                    State::Recreating => {
                        if !ctx.core.shared.stream_lost.swap(true, Ordering::AcqRel) {
                            tracing::warn!("LE Audio: stream lost: no answer to LE Terminate BIG");
                        }
                        ctx.finish_broadcast();
                    }
                    State::Creating | State::Streaming => {}
                }
            }),
        );
        if let Some(b) = self.core.broadcast.as_mut() {
            b.terminate_timer = Some(timer);
        }
    }

    /// The ISO sender's slow sending: the BIGs are terminated and created
    /// again (the advertising stays), [`MAX_RECREATES`] times at most
    pub(crate) fn on_slow_broadcast(&mut self, why: &str) {
        let Some(b) = self.core.broadcast.as_mut() else { return };
        if b.state != State::Streaming {
            return;
        }
        if b.recreates >= MAX_RECREATES || recreate_off() {
            tracing::warn!("Auracast: {why}; the BIG(s) stay ({} set up again so far)", b.recreates);
            return;
        }
        b.recreates += 1;
        tracing::warn!("Auracast: {why}: setting the BIG(s) up again ({} of {MAX_RECREATES})", b.recreates);
        self.core.iso.stop(self.stack);
        let Some(b) = self.core.broadcast.as_mut() else { return };
        b.state = State::Recreating;
        for (g, big) in b.groups.iter().zip(b.bigs.iter_mut()) {
            if matches!(big, Big::Up(_)) {
                let status = self.stack.big_terminate(g.big_handle);
                tracing::debug!("Auracast: terminating BIG {} to create it again: 0x{status:02X}", g.big_handle);
                *big = if status == ERROR_CODE_SUCCESS { Big::Ending } else { Big::Gone };
            }
        }
        if b.all_gone() {
            self.create_bigs_again(0);
        } else {
            self.arm_terminate_timer();
        }
    }

    /// Every BIG ended after slow sending: LE Create BIG again for each not
    /// asked yet (on_big_created starts sending once all are up). BTstack
    /// refuses it while it still holds the BIG just terminated (it frees it
    /// after the event): asked again after [`CREATE_RETRY_MS`]. `tries`:
    /// the refusals so far.
    fn create_bigs_again(&mut self, tries: u32) {
        let Some(b) = self.core.broadcast.as_mut() else { return };
        let timer = b.terminate_timer.take();
        b.state = State::Creating;
        let mut failed = None;
        let mut refused = false;
        for (big, params) in b.bigs.iter_mut().zip(&b.params) {
            if *big != Big::Gone {
                continue;
            }
            let status = self.stack.big_create(params);
            if status == ERROR_CODE_SUCCESS {
                *big = Big::Pending;
            } else if status == ERROR_CODE_COMMAND_DISALLOWED && tries < CREATE_TRIES {
                refused = true;
            } else {
                tracing::warn!("Auracast: LE Create BIG {} refused again (0x{status:02X})", params.big_handle);
                failed = Some(format!("LE Create BIG failed (0x{status:02X})"));
                break;
            }
        }
        if let Some(t) = timer {
            self.cancel_timer(t);
        }
        if let Some(reason) = failed {
            self.fail_start(&reason);
            return;
        }
        if !refused {
            if tries > 0 {
                tracing::debug!("Auracast: LE Create BIG taken after {tries} refusal(s)");
            }
            // All asked: up within RECREATE_TIMEOUT_MS, or the stream is lost
            let Some(n) = self.core.broadcast.as_ref().map(|b| b.recreates) else { return };
            let timer = self.run_after(
                RECREATE_TIMEOUT_MS,
                Box::new(move |ctx| {
                    let Some(b) = ctx.core.broadcast.as_mut() else { return };
                    if b.recreates != n || b.state != State::Creating {
                        return;
                    }
                    b.retry_timer = None;
                    ctx.fail_start("no answer to LE Create BIG");
                }),
            );
            if let Some(b) = self.core.broadcast.as_mut() {
                b.retry_timer = Some(timer);
            }
            return;
        }
        let timer = self.run_after(
            CREATE_RETRY_MS,
            Box::new(move |ctx| {
                let Some(b) = ctx.core.broadcast.as_mut() else { return };
                b.retry_timer = None;
                if b.state == State::Creating {
                    ctx.create_bigs_again(tries + 1);
                }
            }),
        );
        if let Some(b) = self.core.broadcast.as_mut() {
            b.retry_timer = Some(timer);
        }
    }

    /// GAP_SUBEVENT_BIG_CREATED / _TERMINATED
    pub(crate) fn on_big_event(&mut self, p: &[u8]) {
        if let Some(c) = events::big_created(p) {
            self.on_big_created(c);
        } else if let Some(handle) = events::big_terminated(p) {
            tracing::debug!("Auracast: BIG {handle} terminated");
            let Some(b) = self.core.broadcast.as_mut() else { return };
            let Some(i) = b.group_of(handle) else { return };
            b.bigs[i] = Big::Gone;
            match b.state {
                State::Terminating => {
                    if b.all_gone() {
                        self.finish_broadcast();
                    }
                }
                State::Recreating => {
                    if b.all_gone() {
                        self.create_bigs_again(0);
                    }
                }
                State::Creating | State::Streaming => {
                    // The controller ended it (or BTstack after a failed
                    // ISO path): the others go too
                    if !self.core.shared.stream_lost.swap(true, Ordering::AcqRel) {
                        tracing::warn!("LE Audio: stream lost: BIG {handle} ended");
                    }
                    self.fail_start("the broadcast ended");
                }
            }
        }
    }

    fn on_big_created(&mut self, c: events::BigCreated) {
        let Some(b) = self.core.broadcast.as_mut() else { return };
        tracing::debug!(
            "Auracast: BIG {} created: status 0x{:02X}, BIS {:04X?}",
            c.big_handle,
            c.status,
            c.bis_handles
        );
        let Some(i) = b.group_of(c.big_handle) else { return };
        let ok = c.status == ERROR_CODE_SUCCESS && c.bis_handles.len() == b.groups[i].bis.len();
        match b.state {
            State::Terminating => {
                if c.status == ERROR_CODE_SUCCESS {
                    // Created after the stop asked for its end
                    b.bigs[i] = Big::Up(c.bis_handles);
                    let status = self.stack.big_terminate(c.big_handle);
                    if status != ERROR_CODE_SUCCESS {
                        b.bigs[i] = Big::Gone;
                    }
                } else {
                    b.bigs[i] = Big::Gone;
                }
                if b.all_gone() {
                    self.finish_broadcast();
                }
            }
            State::Streaming | State::Recreating => {}
            State::Creating => {
                if !ok {
                    b.bigs[i] = if c.status == ERROR_CODE_SUCCESS { Big::Up(c.bis_handles) } else { Big::Gone };
                    let why = if i == 0 {
                        format!("LE Create BIG failed (0x{:02X})", c.status)
                    } else {
                        format!(
                            "LE Create BIG failed for broadcast {} of {} (0x{:02X})",
                            i + 1,
                            b.groups.len(),
                            c.status
                        )
                    };
                    self.fail_start(&why);
                    return;
                }
                b.bigs[i] = Big::Up(c.bis_handles);
                if b.bigs.iter().any(|big| !matches!(big, Big::Up(_))) {
                    return;
                }
                // Every BIG is up: send
                let mut all = Vec::with_capacity(b.groups.len());
                for (g, big) in b.groups.iter().zip(&b.bigs) {
                    let Big::Up(handles) = big else { return };
                    for (bis, &h) in b.plan.cis[g.bis.clone()].iter_mut().zip(handles) {
                        bis.cis_handle = h;
                    }
                    all.push((g.big_handle, handles.clone()));
                }
                b.state = State::Streaming;
                if b.recreates > 0 {
                    tracing::info!("Auracast: BIG(s) set up again ({} of {MAX_RECREATES})", b.recreates);
                }
                let frame_us = b.plan.preset.sdu_interval_us();
                let plan = b.plan.clone();
                let start = b.started.take();
                let depth = queue_depth(self.core.shared.iso_buffers().1, plan.cis.len()).max(1);
                if let Some(s) = self.core.status.as_mut() {
                    s.active = true;
                    s.plan = plan;
                }
                let queue = self.core.queue.clone().unwrap_or_default();
                self.core.iso.start_bigs(self.stack, &all, queue, frame_us, depth);
                if let Some((start, _)) = start {
                    start(true);
                }
            }
        }
    }

    /// The broadcast is over: advertising off, state gone, stops completed
    fn finish_broadcast(&mut self) {
        self.core.iso.stop(self.stack);
        let Some(mut b) = self.core.broadcast.take() else { return };
        for g in &b.groups {
            self.stack.broadcast_advertise_stop(g.big_handle);
        }
        for t in [b.terminate_timer.take(), b.retry_timer.take()].into_iter().flatten() {
            self.cancel_timer(t);
        }
        if let Some((start, why)) = b.started.take() {
            set_why(&why, "the broadcast ended");
            start(false);
        }
        self.core.status = None;
        self.core.queue = None;
        tracing::info!("Auracast: stopped");
        for d in b.stopped.drain(..) {
            d(true);
        }
    }

    /// The running broadcast's plan (BIS handles filled in)
    pub(crate) fn broadcast_plan(&self) -> Option<StreamPlan> {
        self.core.broadcast.as_ref().map(|b| b.plan.clone())
    }

    /// The broadcasts of the running one, streaming (none before its BIGs
    /// are up or while it stops)
    pub(crate) fn broadcast_groups(&self) -> Vec<BroadcastGroup> {
        self.core.broadcast.as_ref().filter(|b| b.on_air()).map(|b| b.groups.clone()).unwrap_or_default()
    }
}

/// Max transport latency for every BIG from `BLUECASTER_AURACAST_LATENCY_MS`
/// (a test knob): with the presets' 60-65 ms the Zephyr LL (nRF5340) picks a
/// 50 ms ISO interval with five SDUs per event, and stalls on the next BIG
fn latency_override() -> Option<u16> {
    let ms = std::env::var("BLUECASTER_AURACAST_LATENCY_MS").ok()?.trim().parse::<u16>().ok()?;
    (5..=4000).contains(&ms).then(|| {
        tracing::info!("Auracast: max transport latency {ms} ms (BLUECASTER_AURACAST_LATENCY_MS)");
        ms
    })
}

/// `BLUECASTER_AURACAST_NO_RECREATE=1` (a test knob): BIGs that send slowly
/// are left as they are
fn recreate_off() -> bool {
    std::env::var("BLUECASTER_AURACAST_NO_RECREATE").is_ok_and(|v| v.trim() == "1")
}

/// Retransmissions for every BIG from `BLUECASTER_AURACAST_RTN` (a test
/// knob): fewer repeats, less air time for four BIS at 48 kHz
fn rtn_override() -> Option<u8> {
    let rtn = std::env::var("BLUECASTER_AURACAST_RTN").ok()?.trim().parse::<u8>().ok()?;
    (rtn <= 15).then(|| {
        tracing::info!("Auracast: RTN {rtn} (BLUECASTER_AURACAST_RTN)");
        rtn
    })
}
