//! What the window shows, without the window: the translations, the display
//! helpers and the rules behind the screens. No UI toolkit here, so all of
//! it is tested with plain `cargo test -p ui-model`.

#![forbid(unsafe_code)]

pub mod chips;
pub mod codecdetails;
pub mod codecs;
pub mod eq;
pub mod flow;
pub mod format;
pub mod i18n;
pub mod links;
pub mod linkstats;
pub mod logview;
pub mod pairing;
pub mod profile;
pub mod radio;
pub mod receiver;
pub mod sbc;
pub mod series;
pub mod store;
pub mod subscription;
pub mod view;
