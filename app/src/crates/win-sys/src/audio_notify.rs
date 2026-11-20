//! Windows' audio endpoints changing: one came, went, was enabled or
//! disabled (a virtual cable's driver installed or removed), or a default
//! device changed (IMMNotificationClient).

use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Media::Audio::{
    DEVICE_STATE, EDataFlow, ERole, IMMDeviceEnumerator, IMMNotificationClient, IMMNotificationClient_Impl,
    MMDeviceEnumerator,
};
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
use windows::core::{PCWSTR, implement};

#[implement(IMMNotificationClient)]
struct Client {
    changed: Box<dyn Fn() + Send + Sync>,
}

// Called on Windows' threads, which must not block: `changed` only signals.
impl IMMNotificationClient_Impl for Client_Impl {
    fn OnDeviceStateChanged(&self, _id: &PCWSTR, _state: DEVICE_STATE) -> windows::core::Result<()> {
        (self.changed)();
        Ok(())
    }

    fn OnDeviceAdded(&self, _id: &PCWSTR) -> windows::core::Result<()> {
        (self.changed)();
        Ok(())
    }

    fn OnDeviceRemoved(&self, _id: &PCWSTR) -> windows::core::Result<()> {
        (self.changed)();
        Ok(())
    }

    fn OnDefaultDeviceChanged(&self, _flow: EDataFlow, _role: ERole, _id: &PCWSTR) -> windows::core::Result<()> {
        (self.changed)();
        Ok(())
    }

    // Volume, format, names...: many, and none adds or removes a device
    fn OnPropertyValueChanged(&self, _id: &PCWSTR, _key: &PROPERTYKEY) -> windows::core::Result<()> {
        Ok(())
    }
}

/// Registered while alive. Create and drop it on one thread that has COM
/// initialised ([`crate::audio_policy::ComApartment`]).
pub struct EndpointWatch {
    enumerator: IMMDeviceEnumerator,
    client: IMMNotificationClient,
}

impl EndpointWatch {
    /// `changed` runs on a thread of Windows' for every change; it must
    /// return at once (send on a channel) and never call back into COM
    /// audio interfaces.
    pub fn new(changed: impl Fn() + Send + Sync + 'static) -> windows::core::Result<EndpointWatch> {
        // SAFETY: COM is initialised on this thread (the caller's
        // apartment); plain object creation.
        let enumerator: IMMDeviceEnumerator = unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
        let client: IMMNotificationClient = Client { changed: Box::new(changed) }.into();
        // SAFETY: both are live COM objects; the registration is undone in
        // Drop with the same client.
        unsafe { enumerator.RegisterEndpointNotificationCallback(&client)? };
        Ok(EndpointWatch { enumerator, client })
    }
}

impl Drop for EndpointWatch {
    fn drop(&mut self) {
        // SAFETY: the client registered in new on this enumerator
        if let Err(e) = unsafe { self.enumerator.UnregisterEndpointNotificationCallback(&self.client) } {
            tracing::debug!("audio endpoints: unregister failed: HRESULT 0x{:08x}", e.code().0 as u32);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Windows takes the client and gives it back
    #[test]
    fn registers_and_unregisters() {
        let _com = crate::audio_policy::ComApartment::init_mta().unwrap();
        let watch = EndpointWatch::new(|| {}).unwrap();
        drop(watch);
    }
}
