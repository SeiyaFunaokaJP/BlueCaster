//! One bluecaster.exe per user session. A second launch (the tray icon, the
//! Start menu, the core opening the flyout) forwards what it was asked to
//! show (`--flyout`, `--quick-settings`, or nothing: the main window) to
//! the running one and exits.
//!
//! The running instance holds a named mutex (session-local, `<identifier>-sim`:
//! the name the Tauri window of 3.0.x took from its single-instance plugin,
//! so that a 3.0.x window and this one exclude each other, and the installer
//! finds either) and serves a Named Pipe of its own, open only to
//! this user (and SYSTEM), named after the user and the session. A second
//! launch hands the foreground right it got from the user's click to the
//! running one (so that its window can come to the front), sends its
//! arguments over the pipe and waits for the answer.

use std::io::ErrorKind;
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::NamedPipeServer;
use tokio::sync::{mpsc, oneshot};
use win_sys::single_instance::SingleInstance;

/// The installed app's identifier: the AppUserModelID of its windows and of
/// the installer's shortcuts (installer/BlueCaster.iss AppUserModelId)
pub const IDENTIFIER: &str = "io.github.seiyafunaokajp.bluecaster";
/// Development: another identifier, so that a development build runs beside
/// the installed app
pub const IDENTIFIER_ENV: &str = "BLUECASTER_UI_IDENTIFIER";
/// How long a second launch waits for the running one to take its arguments
const FORWARD_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a second launch looks for the running one's pipe (it may be
/// starting, or exiting: then this launch becomes the running one)
const FIND_RUNNING: Duration = Duration::from_secs(5);
/// A forwarded message is at most this long
const MAX_MESSAGE: usize = 4096;
/// All pipe instances busy: try again shortly
const ERROR_PIPE_BUSY: i32 = 231;

/// What a launch asks to show
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Launch {
    /// No argument: the main window
    Main,
    /// `--flyout`: the tray flyout (the core starts the UI this way when
    /// the tray icon is clicked)
    Flyout,
    /// `--quick-settings`: the flyout above Quick Settings, which just opened
    QuickSettings,
}

impl Launch {
    /// From a launch's arguments (without the program's own path);
    /// `--quick-settings` wins over `--flyout`
    pub fn from_args<I, S>(args: I) -> Launch
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut launch = Launch::Main;
        for a in args {
            match a.as_ref() {
                "--quick-settings" => return Launch::QuickSettings,
                "--flyout" => launch = Launch::Flyout,
                _ => {}
            }
        }
        launch
    }

    /// The arguments that ask for it
    pub fn args(self) -> &'static [&'static str] {
        match self {
            Launch::Main => &[],
            Launch::Flyout => &["--flyout"],
            Launch::QuickSettings => &["--quick-settings"],
        }
    }
}

/// The identifier: [`IDENTIFIER_ENV`] when set, else [`IDENTIFIER`]
pub fn identifier() -> String {
    std::env::var(IDENTIFIER_ENV)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| IDENTIFIER.to_string())
}

/// An identifier usable in a mutex and a pipe name
fn sanitized(identifier: &str) -> String {
    identifier.chars().map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '_' }).collect()
}

/// This launch's role
pub enum Instance {
    /// The running instance: keep it for the life of the process
    Primary(Primary),
    /// Another instance runs and has taken this launch's arguments (or did
    /// not answer in time): exit
    Secondary,
}

/// The running instance: holds the mutex and serves the pipe. Dropping it
/// lets both go (before [`crate::recovery::relaunch_if_due`]).
pub struct Primary {
    launches: Option<mpsc::UnboundedReceiver<Launch>>,
    stop: Option<oneshot::Sender<()>>,
    listener: Option<std::thread::JoinHandle<()>>,
    _mutex: Option<SingleInstance>,
}

impl Primary {
    /// What later launches ask to show, in order (once: the receiver is
    /// taken). Its `recv().await` works on any executor.
    pub fn launches(&mut self) -> Option<mpsc::UnboundedReceiver<Launch>> {
        self.launches.take()
    }
}

impl Drop for Primary {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(listener) = self.listener.take() {
            let _ = listener.join();
        }
    }
}

/// Becomes the running instance, or forwards this process' arguments to
/// the running one ([`Instance::Secondary`]: exit then)
pub fn acquire(identifier: &str) -> Instance {
    acquire_with(identifier, Launch::from_args(std::env::args().skip(1)))
}

/// [`acquire`], forwarding `launch`
pub fn acquire_with(identifier: &str, launch: Launch) -> Instance {
    let id = sanitized(identifier);
    let mutex = format!("{id}-sim");
    let pipe = match pipe_name(&id) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("single instance: no pipe name ({:?}); running without", e.kind());
            return Instance::Primary(Primary { launches: None, stop: None, listener: None, _mutex: None });
        }
    };
    let deadline = Instant::now() + FIND_RUNNING;
    loop {
        match SingleInstance::acquire(&mutex) {
            Ok(Some(guard)) => return Instance::Primary(serve(guard, pipe)),
            Ok(None) => {}
            Err(e) => {
                tracing::warn!("single instance: no mutex ({e}); running without");
                return Instance::Primary(Primary { launches: None, stop: None, listener: None, _mutex: None });
            }
        }
        match forward(&pipe, launch) {
            Ok(()) => {
                tracing::info!("another bluecaster runs; {launch:?} forwarded to it");
                return Instance::Secondary;
            }
            // Starting (no pipe yet), busy, or exiting (the mutex goes next)
            Err(e) if retry(&e) && Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => {
                tracing::warn!("another bluecaster runs but did not take {launch:?} ({:?})", e.kind());
                return Instance::Secondary;
            }
        }
    }
}

fn retry(e: &std::io::Error) -> bool {
    e.kind() == ErrorKind::NotFound || e.raw_os_error() == Some(ERROR_PIPE_BUSY)
}

/// The user's own pipe in this session: `\\.\pipe\<id>-sip-<SID>-<session>`
fn pipe_name(id: &str) -> std::io::Result<String> {
    let sid = win_sys::pipe::current_user_sid()?;
    let session = win_sys::pipe::process_session_id(std::process::id())?;
    Ok(format!(r"\\.\pipe\{id}-sip-{sid}-{session}"))
}

fn runtime() -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread().enable_all().build()
}

/// The running instance: the pipe served on a thread of its own
fn serve(mutex: SingleInstance, pipe: String) -> Primary {
    let (tx, rx) = mpsc::unbounded_channel();
    let (stop, stopped) = oneshot::channel();
    let (ready_tx, ready) = std::sync::mpsc::channel();
    let listener = std::thread::Builder::new()
        .name("single-instance".into())
        .spawn(move || listen(&pipe, &tx, stopped, &ready_tx))
        .inspect_err(|e| tracing::warn!("single instance: no listener thread ({e})"))
        .ok();
    if listener.is_some() {
        match ready.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => {}
            Ok(Err(e)) => tracing::warn!("single instance: cannot serve the pipe ({:?})", e.kind()),
            Err(_) => tracing::warn!("single instance: the pipe did not come up"),
        }
    }
    Primary { launches: Some(rx), stop: Some(stop), listener, _mutex: Some(mutex) }
}

fn listen(
    pipe: &str,
    tx: &mpsc::UnboundedSender<Launch>,
    mut stopped: oneshot::Receiver<()>,
    ready: &std::sync::mpsc::Sender<std::io::Result<()>>,
) {
    let rt = match runtime() {
        Ok(rt) => rt,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    rt.block_on(async {
        let mut server = match win_sys::pipe::create_server(pipe, true) {
            Ok(s) => {
                let _ = ready.send(Ok(()));
                s
            }
            Err(e) => {
                let _ = ready.send(Err(e));
                return;
            }
        };
        loop {
            tokio::select! {
                _ = &mut stopped => return,
                connected = server.connect() => {
                    // The next instance first, so that another launch
                    // meanwhile finds the pipe
                    let next = loop {
                        match win_sys::pipe::create_server(pipe, false) {
                            Ok(s) => break s,
                            Err(e) => {
                                tracing::warn!("single instance: no next pipe instance ({:?})", e.kind());
                                tokio::time::sleep(Duration::from_secs(1)).await;
                            }
                        }
                    };
                    let client = std::mem::replace(&mut server, next);
                    if connected.is_ok() {
                        // A launch that does not finish in time is dropped
                        let _ = tokio::time::timeout(FORWARD_TIMEOUT, take(client, tx)).await;
                    }
                }
            }
        }
    });
}

/// Reads one forwarded launch (its arguments as a JSON array, one line)
/// and answers it
async fn take(mut pipe: NamedPipeServer, tx: &mpsc::UnboundedSender<Launch>) {
    let mut message = Vec::new();
    let mut buf = [0u8; 512];
    while !message.contains(&b'\n') {
        match pipe.read(&mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(n) => message.extend_from_slice(&buf[..n]),
        }
        if message.len() > MAX_MESSAGE {
            return;
        }
    }
    let line = message.split(|b| *b == b'\n').next().unwrap_or_default();
    let Ok(args) = serde_json::from_slice::<Vec<String>>(line) else { return };
    let launch = Launch::from_args(&args);
    tracing::info!("another launch asks for {launch:?}");
    let _ = tx.send(launch);
    let _ = pipe.write_all(b"ok\n").await;
    let _ = pipe.flush().await;
}

/// Sends `launch` to the running instance over its pipe and waits for its
/// answer
fn forward(pipe: &str, launch: Launch) -> std::io::Result<()> {
    runtime()?.block_on(async {
        let mut client = win_sys::pipe_peer::open_client(pipe)?;
        // Windows lets this process bring a window to the front (the
        // user's click started it), not the running one: hand that right
        // over before it shows its window
        {
            use std::os::windows::io::AsHandle;
            if let Ok(server) = win_sys::pipe_peer::server_process(client.as_handle()) {
                win_sys::shell::allow_set_foreground(server.pid);
            }
        }
        let mut message = serde_json::to_vec(launch.args()).map_err(std::io::Error::other)?;
        message.push(b'\n');
        let exchange = async {
            client.write_all(&message).await?;
            client.flush().await?;
            let mut answer = [0u8; 3];
            client.read_exact(&mut answer).await?;
            if &answer == b"ok\n" { Ok(()) } else { Err(std::io::Error::other("unexpected answer")) }
        };
        tokio::time::timeout(FORWARD_TIMEOUT, exchange)
            .await
            .map_err(|_| std::io::Error::new(ErrorKind::TimedOut, "no answer"))?
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launches_from_their_arguments() {
        assert_eq!(Launch::from_args(Vec::<String>::new()), Launch::Main);
        assert_eq!(Launch::from_args(["--flyout"]), Launch::Flyout);
        assert_eq!(Launch::from_args(["--flyout", "--quick-settings"]), Launch::QuickSettings);
        assert_eq!(Launch::from_args(["--quick-settings", "--flyout"]), Launch::QuickSettings);
        assert_eq!(Launch::from_args(["--other"]), Launch::Main);
        for l in [Launch::Main, Launch::Flyout, Launch::QuickSettings] {
            assert_eq!(Launch::from_args(l.args()), l);
        }
    }

    #[test]
    fn identifiers_fit_in_kernel_object_names() {
        assert_eq!(sanitized(IDENTIFIER), IDENTIFIER);
        assert_eq!(sanitized(r"dev\build one"), "dev_build_one");
    }

    /// A second launch forwards its arguments to the first and is told to
    /// exit; once the first has gone, the next launch runs itself
    #[test]
    fn a_second_launch_forwards_to_the_first() {
        let id = format!("bluecaster-test-ui-host-si-{}", std::process::id());
        let Instance::Primary(mut first) = acquire_with(&id, Launch::Main) else {
            panic!("the first launch runs");
        };
        let mut launches = first.launches().expect("the receiver");
        assert!(first.launches().is_none(), "taken once");
        let id2 = id.clone();
        let second = std::thread::spawn(move || matches!(acquire_with(&id2, Launch::Flyout), Instance::Secondary));
        assert!(second.join().unwrap(), "the second launch exits");
        assert_eq!(launches.blocking_recv(), Some(Launch::Flyout));
        let id3 = id.clone();
        let third =
            std::thread::spawn(move || matches!(acquire_with(&id3, Launch::QuickSettings), Instance::Secondary));
        assert!(third.join().unwrap());
        assert_eq!(launches.blocking_recv(), Some(Launch::QuickSettings));
        drop(first);
        assert!(matches!(acquire_with(&id, Launch::Main), Instance::Primary(_)), "the first has gone");
    }
}
