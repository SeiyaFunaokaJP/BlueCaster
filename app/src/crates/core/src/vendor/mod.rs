//! What single vendors' devices need beyond the standard profiles, one module
//! per vendor: `channel` is the RFCOMM control channel some of them share.
//!
//! [`VENDORS`] lists them for the rest of the app: how their earbuds are
//! told apart (by the maker's id in their Device ID record, never by their
//! name: the user may choose that), and the options developer mode offers
//! for each. Developer
//! mode's overrides ([`set_overrides`]) are kept by vendor id, so a vendor
//! added here needs nothing more in the settings or the UI than the labels
//! of its options (issue #237).

pub mod apple;
mod channel;
pub mod huawei;
pub mod oppo;

use std::collections::BTreeMap;
use std::sync::RwLock;

use bt_host::{DeviceId, ID_SOURCE_BLUETOOTH, ID_SOURCE_USB};
use serde::{Deserialize, Serialize};

/// A vendor whose earbuds BlueCaster speaks to over their own protocol
pub struct Vendor {
    /// Key of its overrides (settings, IPC)
    pub id: &'static str,
    /// Shown, and in the log
    pub name: &'static str,
    /// Its makers' ids in a Device ID record: (source, vendor id)
    pub ids: &'static [(u16, u16)],
    /// Developer mode's switches of its own (ids; the UI labels them)
    pub options: &'static [&'static str],
}

pub const OPPO: &str = "oppo";
pub const APPLE: &str = "apple";
pub const HUAWEI: &str = "huawei";
/// OPPO: asked for Hi-Res on every connection, the record notwithstanding
pub const OPPO_EVERY_CONNECTION: &str = "every_connection";

pub const VENDORS: &[Vendor] = &[
    // Apple
    Vendor { id: APPLE, name: "AirPods", ids: &[(ID_SOURCE_BLUETOOTH, 0x004C), (ID_SOURCE_USB, 0x05AC)], options: &[] },
    // Huawei, and Honor (the same protocol)
    Vendor {
        id: HUAWEI,
        name: "Huawei",
        ids: &[(ID_SOURCE_BLUETOOTH, 0x027D), (ID_SOURCE_BLUETOOTH, 0x09C6), (ID_SOURCE_USB, 0x12D1)],
        options: &[],
    },
    Vendor {
        id: OPPO,
        name: "OPPO",
        ids: &[(ID_SOURCE_BLUETOOTH, 0x079A), (ID_SOURCE_USB, 0x22D9)],
        options: &[OPPO_EVERY_CONNECTION],
    },
];

pub fn vendor(id: &str) -> Option<&'static Vendor> {
    VENDORS.iter().find(|v| v.id == id)
}

impl Vendor {
    /// Whether a Device ID is one of this vendor's
    pub fn made(&self, d: DeviceId) -> bool {
        self.ids.contains(&(d.source, d.vendor))
    }
}

/// The vendor whose id a Device ID record gives
pub fn of(d: DeviceId) -> Option<&'static Vendor> {
    VENDORS.iter().find(|v| v.made(d))
}

/// When a vendor's protocol is used
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// For the earbuds whose Device ID tells them
    #[default]
    Auto,
    /// For any earbuds, whatever their Device ID
    Always,
    Never,
}

/// Developer mode's override of one vendor
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Override {
    pub mode: Mode,
    /// Its options turned on ([`Vendor::options`])
    pub options: Vec<String>,
}

impl Override {
    pub fn is_default(&self) -> bool {
        self.mode == Mode::Auto && self.options.is_empty()
    }
}

/// By vendor id; absent: the default
pub type Overrides = BTreeMap<String, Override>;

static OVERRIDES: RwLock<Overrides> = RwLock::new(BTreeMap::new());

/// Sets the overrides in force (none while developer mode is off); logs
/// when they change
pub fn set_overrides(mut o: Overrides) {
    o.retain(|id, v| vendor(id).is_some() && !v.is_default());
    let mut cur = OVERRIDES.write().unwrap_or_else(|e| e.into_inner());
    if *cur != o {
        if o.is_empty() {
            tracing::info!("vendor overrides: off");
        } else {
            tracing::warn!("vendor overrides: {o:?}");
        }
        *cur = o;
    }
}

/// When the vendor's protocol is used now
pub fn mode(id: &str) -> Mode {
    OVERRIDES.read().unwrap_or_else(|e| e.into_inner()).get(id).map_or(Mode::Auto, |o| o.mode)
}

/// Whether one of the vendor's options is on now
pub fn option(id: &str, option: &str) -> bool {
    OVERRIDES.read().unwrap_or_else(|e| e.into_inner()).get(id).is_some_and(|o| o.options.iter().any(|x| x == option))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overrides_by_vendor_id() {
        let mut o = Overrides::new();
        o.insert(OPPO.into(), Override { mode: Mode::Always, options: vec![OPPO_EVERY_CONNECTION.into()] });
        o.insert(HUAWEI.into(), Override::default());
        o.insert("acme".into(), Override { mode: Mode::Never, options: Vec::new() });
        set_overrides(o);
        assert_eq!(mode(OPPO), Mode::Always);
        assert!(option(OPPO, OPPO_EVERY_CONNECTION));
        assert_eq!(mode(HUAWEI), Mode::Auto);
        // Unknown vendors and defaults are not kept
        assert_eq!(OVERRIDES.read().unwrap().len(), 1);
        set_overrides(Overrides::new());
        assert_eq!(mode(OPPO), Mode::Auto);
        assert!(!option(OPPO, OPPO_EVERY_CONNECTION));
    }

    #[test]
    fn each_vendor_once() {
        for v in VENDORS {
            assert_eq!(VENDORS.iter().filter(|x| x.id == v.id).count(), 1, "{}", v.id);
        }
        let id = |source, vendor| DeviceId { source, vendor, product: 0 };
        assert_eq!(of(id(1, 0x079A)).map(|v| v.id), Some(OPPO));
        assert_eq!(of(id(1, 0x027D)).map(|v| v.id), Some(HUAWEI));
        assert_eq!(of(id(1, 0x09C6)).map(|v| v.id), Some(HUAWEI));
        assert_eq!(of(id(1, 0x004C)).map(|v| v.id), Some(APPLE));
        assert_eq!(of(id(2, 0x05AC)).map(|v| v.id), Some(APPLE));
        // The source counts: 0x004C of the USB-IF is not Apple
        assert_eq!(of(id(2, 0x004C)).map(|v| v.id), None);
        // A chip maker's id (Qualcomm)
        assert_eq!(of(id(1, 0x000A)).map(|v| v.id), None);
        for v in VENDORS {
            assert_eq!(VENDORS.iter().filter(|x| x.ids.iter().any(|i| v.ids.contains(i))).count(), 1, "{}", v.id);
        }
    }
}
