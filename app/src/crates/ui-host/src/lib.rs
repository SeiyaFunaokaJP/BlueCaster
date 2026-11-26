//! bluecaster.exe's side without the window: the connection to
//! bluecaster-core.exe ([`connection`]), the service add-on ([`service`]),
//! in-app updates ([`update`]), the process' log ([`logging`]), opening
//! pages, folders and files ([`shell`]), starting again after a fatal
//! failure ([`recovery`]), the one running instance ([`single_instance`])
//! and the removal of what the 3.0.x window left ([`webview_data`]).
//!
//! No window toolkit here: the host runs its own tokio runtime, and what
//! happens reaches the window as [`HostEvent`]s on a channel whose
//! receiver works on any executor. Requests answer on oneshot channels in
//! the same way.

#![forbid(unsafe_code)]

pub mod connection;
pub mod logging;
pub mod recovery;
pub mod service;
pub mod shell;
pub mod single_instance;
pub mod update;
pub mod webview_data;

pub use connection::{Host, HostEvent, Status, start};

/// This program's version; the service add-on must be exactly this one
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
