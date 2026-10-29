//! Fixed GPU assets made once a process: every scene builds a renderer of its own (a theatre holds two while it swaps
//! them), but they all draw on the one device, so textures that never change (the combat effects' flipbooks and gas
//! atlas, the ships' paint tiles) are uploaded by the first and handed to the rest.

use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Mutex;

/// One value a device: made by the first caller on that device and shared with every later one until a caller on
/// another device makes its own (tests make several devices in a process; the game makes one). The value lives as long
/// as the process, or until replaced. A wasm32 build (whose wgpu handles cannot be shared between threads, so cannot sit
/// in a `static`) makes it afresh for every caller.
pub struct DeviceCache<T> {
    #[cfg(not(target_arch = "wasm32"))]
    slot: Mutex<Option<(wgpu::Device, Arc<T>)>>,
    #[cfg(target_arch = "wasm32")]
    slot: std::marker::PhantomData<fn() -> T>,
}

impl<T> DeviceCache<T> {
    pub const fn new() -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            slot: Mutex::new(None),
            #[cfg(target_arch = "wasm32")]
            slot: std::marker::PhantomData,
        }
    }

    /// Made afresh for every caller (wasm32).
    #[cfg(target_arch = "wasm32")]
    pub fn get_or_try_make<E>(
        &self,
        _device: &wgpu::Device,
        make: impl FnOnce() -> Result<T, E>,
    ) -> Result<Arc<T>, E> {
        make().map(Arc::new)
    }

    /// The value made on `device`, else `make`'s (kept unless it fails). Callers on other threads wait while it is
    /// made, so it is made once.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn get_or_try_make<E>(
        &self,
        device: &wgpu::Device,
        make: impl FnOnce() -> Result<T, E>,
    ) -> Result<Arc<T>, E> {
        let mut slot = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((held, value)) = slot.as_ref()
            && held == device
        {
            return Ok(value.clone());
        }
        let value = Arc::new(make()?);
        *slot = Some((device.clone(), value.clone()));
        Ok(value)
    }

    /// As [`DeviceCache::get_or_try_make`] with a `make` that cannot fail.
    pub fn get_or_make(&self, device: &wgpu::Device, make: impl FnOnce() -> T) -> Arc<T> {
        match self.get_or_try_make::<std::convert::Infallible>(device, || Ok(make())) {
            Ok(value) => value,
        }
    }
}

impl<T> Default for DeviceCache<T> {
    fn default() -> Self {
        Self::new()
    }
}
