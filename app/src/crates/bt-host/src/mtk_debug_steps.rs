//! Developer > Debug (issue #86), MediaTek only: two steps added around the
//! firmware loader (crate::mediatek), both typed in by the tester and both
//! off when their fields are empty.
//!
//! - A register write (a USB control transfer, crate::debug_command::
//!   parse_register): sent right before the loader switches Bluetooth on,
//!   or, when the chip ran its firmware already and nothing is switched on,
//!   once the loader is done.
//! - The Debug HCI command, when it is set to go before reset: sent once the
//!   loader is done, so before HCI init and its HCI Reset (otherwise it goes
//!   after HCI init, crate::app).
//!
//! What either does to the chip is not documented; that is what the test
//! is for. Only results are logged (success, status), never return bytes.
//! A failed or unanswered step is logged and the adapter starts anyway.

use crate::bringup::{Action, ChipLoader, ControlRequest, HciEvent, Outcome, parse_event};

/// How long the command's answer is waited for
const COMMAND_TIMEOUT_MS: u32 = 2000;

enum Phase {
    /// The loader runs; its actions pass through
    Loading,
    /// The register write was sent; its result comes next
    Writing,
    /// The command was sent; waiting for its answer
    Command {
        opcode: u16,
    },
    Finished,
}

pub struct DebugSteps {
    inner: Box<dyn ChipLoader>,
    register: Option<ControlRequest>,
    command: Option<Vec<u8>>,
    phase: Phase,
    /// The loader's own result, given on once the steps are through
    outcome: Option<Outcome>,
}

impl DebugSteps {
    /// The loader as it is when there is nothing to add
    pub fn wrap(
        inner: Box<dyn ChipLoader>,
        register: Option<ControlRequest>,
        command: Option<Vec<u8>>,
    ) -> Box<dyn ChipLoader> {
        if register.is_none() && command.is_none() {
            return inner;
        }
        Box::new(DebugSteps { inner, register, command, phase: Phase::Loading, outcome: None })
    }

    /// The loader's actions: the write goes in before "Bluetooth on", and
    /// its successful end starts the steps
    fn pass(&mut self, actions: Vec<Action>) -> Vec<Action> {
        let mut out = Vec::with_capacity(actions.len() + 1);
        for action in actions {
            match action {
                Action::Command(c) if crate::mediatek::is_func_on(&c) && self.register.is_some() => {
                    if let Some(r) = self.register.take() {
                        self.phase = Phase::Writing;
                        out.push(Action::UsbControl(r));
                    }
                    out.push(Action::Command(c));
                }
                Action::Done(Ok(outcome)) => {
                    out.extend(self.after_loader(outcome));
                    break;
                }
                a => out.push(a),
            }
        }
        out
    }

    fn after_loader(&mut self, outcome: Outcome) -> Vec<Action> {
        self.outcome = Some(outcome);
        if let Some(r) = self.register.take() {
            self.phase = Phase::Writing;
            return vec![Action::UsbControl(r)];
        }
        self.send_command()
    }

    fn send_command(&mut self) -> Vec<Action> {
        match self.command.take() {
            Some(c) if c.len() >= 2 => {
                self.phase = Phase::Command { opcode: u16::from_le_bytes([c[0], c[1]]) };
                vec![Action::Command(c), Action::Timer(COMMAND_TIMEOUT_MS)]
            }
            _ => self.finish(),
        }
    }

    fn finish(&mut self) -> Vec<Action> {
        self.phase = Phase::Finished;
        match self.outcome.take() {
            Some(o) => vec![Action::Done(Ok(o))],
            None => Vec::new(),
        }
    }
}

impl ChipLoader for DebugSteps {
    fn vendor(&self) -> &'static str {
        self.inner.vendor()
    }

    fn start(&mut self) -> Vec<Action> {
        let actions = self.inner.start();
        self.pass(actions)
    }

    fn on_event(&mut self, event: &[u8]) -> Vec<Action> {
        match self.phase {
            Phase::Loading => {
                let actions = self.inner.on_event(event);
                self.pass(actions)
            }
            Phase::Command { opcode } => match parse_event(event) {
                HciEvent::Complete { opcode: o, status, .. } | HciEvent::Status { opcode: o, status }
                    if o == opcode =>
                {
                    tracing::info!(
                        "Developer > Debug: command 0x{opcode:04X} sent before reset, status 0x{status:02X}"
                    );
                    self.finish()
                }
                _ => Vec::new(),
            },
            Phase::Writing | Phase::Finished => Vec::new(),
        }
    }

    fn on_timeout(&mut self) -> Vec<Action> {
        match self.phase {
            Phase::Loading => {
                let actions = self.inner.on_timeout();
                self.pass(actions)
            }
            Phase::Command { opcode } => {
                tracing::warn!("Developer > Debug: command 0x{opcode:04X} sent before reset, no answer");
                self.finish()
            }
            Phase::Writing | Phase::Finished => Vec::new(),
        }
    }

    fn on_control(&mut self, result: Result<Vec<u8>, String>) -> Vec<Action> {
        match self.phase {
            Phase::Writing => {
                match result {
                    Ok(_) => tracing::info!("Developer > Debug: register write done"),
                    Err(e) => tracing::warn!("Developer > Debug: register write failed: {e}"),
                }
                if self.outcome.is_some() {
                    self.send_command()
                } else {
                    // Before "Bluetooth on": the loader goes on
                    self.phase = Phase::Loading;
                    Vec::new()
                }
            }
            Phase::Loading => {
                let actions = self.inner.on_control(result);
                self.pass(actions)
            }
            Phase::Command { .. } | Phase::Finished => Vec::new(),
        }
    }

    fn missing_file_hint(&self, file: &str) -> String {
        self.inner.missing_file_hint(file)
    }

    fn take_report(&mut self) -> Option<crate::probe::ProbeReport> {
        self.inner.take_report()
    }

    fn step(&self) -> &'static str {
        self.inner.step()
    }

    fn progress(&self) -> Option<(usize, usize)> {
        self.inner.progress()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bringup::{ControlData, ControlKind, ControlRecipient};

    /// A loader that switches Bluetooth on (or not) and is done
    struct Fake {
        func_on: bool,
    }

    impl ChipLoader for Fake {
        fn vendor(&self) -> &'static str {
            "Fake"
        }
        fn start(&mut self) -> Vec<Action> {
            if self.func_on {
                let on = crate::mediatek::wmt_command(crate::mediatek::WMT_FUNC_CTRL, 0, &[1]);
                vec![Action::Command(on), Action::Timer(10)]
            } else {
                vec![Action::Done(Ok(Outcome::AlreadyRunning))]
            }
        }
        fn on_event(&mut self, _event: &[u8]) -> Vec<Action> {
            Vec::new()
        }
        fn on_timeout(&mut self) -> Vec<Action> {
            vec![Action::Done(Ok(Outcome::Rom))]
        }
    }

    fn write() -> ControlRequest {
        ControlRequest {
            kind: ControlKind::Vendor,
            recipient: ControlRecipient::Device,
            request: 0x12,
            value: 0x3456,
            index: 0x789A,
            data: ControlData::Out(vec![1, 2, 3, 4]),
        }
    }

    fn kinds(actions: &[Action]) -> Vec<&'static str> {
        actions
            .iter()
            .map(|a| match a {
                Action::Command(c) if crate::mediatek::is_func_on(c) => "on",
                Action::Command(_) => "command",
                Action::UsbControl(_) => "write",
                Action::Timer(_) => "timer",
                Action::Done(Ok(_)) => "done",
                Action::Done(Err(_)) => "failed",
                _ => "other",
            })
            .collect()
    }

    #[test]
    fn nothing_set_leaves_the_loader_alone() {
        let mut l = DebugSteps::wrap(Box::new(Fake { func_on: false }), None, None);
        assert_eq!(kinds(&l.start()), ["done"]);
    }

    #[test]
    fn write_before_bluetooth_on_then_command_before_reset() {
        let command = vec![0x98, 0xFD, 0x02, 0xAA, 0xBB];
        let mut l = DebugSteps::wrap(Box::new(Fake { func_on: true }), Some(write()), Some(command));
        assert_eq!(kinds(&l.start()), ["write", "on", "timer"]);
        assert!(l.on_control(Ok(Vec::new())).is_empty());
        // The loader's end starts the command; its answer ends it
        assert_eq!(kinds(&l.on_timeout()), ["command", "timer"]);
        assert!(l.on_event(&[0x0E, 0x04, 0x01, 0x03, 0x0C, 0x00]).is_empty());
        assert_eq!(kinds(&l.on_event(&[0x0E, 0x04, 0x01, 0x98, 0xFD, 0x01])), ["done"]);
    }

    #[test]
    fn firmware_running_already_writes_at_the_end() {
        let mut l = DebugSteps::wrap(Box::new(Fake { func_on: false }), Some(write()), None);
        assert_eq!(kinds(&l.start()), ["write"]);
        // A failed write is logged; the adapter starts anyway
        assert_eq!(kinds(&l.on_control(Err("stall".into()))), ["done"]);
    }

    #[test]
    fn an_unanswered_command_does_not_stop_the_adapter() {
        let mut l = DebugSteps::wrap(Box::new(Fake { func_on: false }), None, Some(vec![0x98, 0xFD, 0x00]));
        assert_eq!(kinds(&l.start()), ["command", "timer"]);
        assert_eq!(kinds(&l.on_timeout()), ["done"]);
    }
}
