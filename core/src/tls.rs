// Copyright 2016 Amanieu d'Antras
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

//! `thread_local!` for a crate that does not link `std`.
//!
//! `thread_local!` on a type with a destructor lowers to a `pthread_key_t` on
//! unix, so this reaches for that directly. On Windows it is `FlsAlloc` rather
//! than `TlsAlloc`, because only fibre-local storage runs a destructor on
//! thread exit, and the values here are boxed.
//!
//! Each `Tls` owns its own key. That is not tidiness: this crate declares two
//! different per-thread types, and one shared key would hand a `ThreadData`
//! back as the wrong one.

use core::marker::PhantomData;
use core::sync::atomic::{AtomicUsize, Ordering};

use alloc::boxed::Box;

/// One thread-local slot holding a `T` per thread.
pub struct Tls<T> {
    /// The platform key, biased by one, so that zero means "not yet created"
    /// and `new` stays a `const fn`. Windows keys are `u32` and unix keys are
    /// `pthread_key_t`; a `usize` holds either.
    key: AtomicUsize,
    owns: PhantomData<fn() -> T>,
}

// SAFETY: the key is atomic and the values behind it are per-thread, so
// nothing is shared between threads except the key itself.
unsafe impl<T> Sync for Tls<T> {}

/// The destructor the slot runs on thread exit. Split only because the two
/// platforms want different ABIs for it.
#[cfg(unix)]
unsafe extern "C" fn drop_value<T>(value: *mut core::ffi::c_void) {
    if !value.is_null() {
        // SAFETY: the only pointer stored under this key came from
        // `Box::into_raw` in `with`, for this same `T`.
        drop(unsafe { Box::from_raw(value.cast::<T>()) });
    }
}

#[cfg(windows)]
unsafe extern "system" fn drop_value<T>(value: *const core::ffi::c_void) {
    if !value.is_null() {
        // SAFETY: as above. `FlsAlloc` types its callback argument as const;
        // the pointer under it is the one `Box::into_raw` handed over, and
        // this is the only thing that ever reclaims it.
        drop(unsafe { Box::from_raw(value.cast::<T>().cast_mut()) });
    }
}

/// The platform's own slot, under one name.
mod slot {
    use core::ffi::c_void;

    #[cfg(unix)]
    pub fn create(drop: unsafe extern "C" fn(*mut c_void)) -> usize {
        let mut fresh: libc::pthread_key_t = 0;
        // SAFETY: `fresh` is a live local and `drop` is a valid callback.
        assert_eq!(
            unsafe { libc::pthread_key_create(&raw mut fresh, Some(drop)) },
            0,
            "pthread_key_create"
        );
        fresh as usize
    }

    #[cfg(windows)]
    pub fn create(drop: unsafe extern "system" fn(*const c_void)) -> usize {
        // SAFETY: `drop` is valid for the lifetime of the process.
        let fresh = unsafe { windows_sys::Win32::System::Threading::FlsAlloc(Some(drop)) };
        assert_ne!(fresh, u32::MAX, "FlsAlloc");
        fresh as usize
    }

    /// Give back a key that lost the race to install itself.
    #[cfg(unix)]
    pub fn destroy(key: usize) {
        // SAFETY: `key` came from `create` and no thread has stored a value
        // under it.
        unsafe { libc::pthread_key_delete(key as libc::pthread_key_t) };
    }

    #[cfg(windows)]
    pub fn destroy(key: usize) {
        // SAFETY: as above.
        unsafe { windows_sys::Win32::System::Threading::FlsFree(key as u32) };
    }

    #[cfg(unix)]
    pub fn get(key: usize) -> *mut c_void {
        // SAFETY: `key` came from `create` and is valid process-wide.
        unsafe { libc::pthread_getspecific(key as libc::pthread_key_t) }
    }

    #[cfg(windows)]
    pub fn get(key: usize) -> *mut c_void {
        // SAFETY: as above.
        unsafe { windows_sys::Win32::System::Threading::FlsGetValue(key as u32) }
    }

    /// Whether the value was stored.
    #[cfg(unix)]
    pub fn set(key: usize, value: *mut c_void) -> bool {
        // SAFETY: as above; `value` is live and uniquely owned.
        unsafe { libc::pthread_setspecific(key as libc::pthread_key_t, value) == 0 }
    }

    #[cfg(windows)]
    pub fn set(key: usize, value: *mut c_void) -> bool {
        // SAFETY: as above.
        unsafe { windows_sys::Win32::System::Threading::FlsSetValue(key as u32, value) != 0 }
    }
}

impl<T> Tls<T> {
    pub const fn new() -> Self {
        Self {
            key: AtomicUsize::new(0),
            owns: PhantomData,
        }
    }

    /// Racing threads each create a key and one of them wins the swap; the
    /// losers hand theirs straight back.
    ///
    /// A lock would be the obvious way to do this once instead, and it cannot
    /// be used: `WordLock` reaches for a `Tls` of its own when it has to park,
    /// so a `Tls` that locks to create its key can re-enter that same lock
    /// through the parking path and recurse until the stack runs out. The
    /// window is a thread creating the very first key while another contends
    /// for it, which is exactly the kind of thing a benchmark never finds.
    fn key(&self) -> usize {
        if let Some(key) = self.existing_key() {
            return key;
        }
        let fresh = slot::create(drop_value::<T>);
        match self
            .key
            .compare_exchange(0, fresh + 1, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) => fresh,
            Err(biased) => {
                // Another thread won. This key holds no value on any thread,
                // since nothing has been handed it yet.
                slot::destroy(fresh);
                biased - 1
            }
        }
    }

    fn existing_key(&self) -> Option<usize> {
        match self.key.load(Ordering::Acquire) {
            0 => None,
            biased => Some(biased - 1),
        }
    }

    /// Run `body` against this thread's value, creating it on first touch.
    ///
    /// The caller must tolerate the value being absent, which is also what
    /// `thread_local!`'s `try_with` reports during thread teardown, and what
    /// the callers here already fall back from.
    pub fn with<R>(&self, init: impl FnOnce() -> T, body: impl FnOnce(&T) -> R) -> Option<R> {
        let key = self.key();
        let existing = slot::get(key);
        if !existing.is_null() {
            // SAFETY: stored below for this same `T`, and owned by this thread
            // until its destructor runs.
            return Some(body(unsafe { &*existing.cast::<T>() }));
        }
        let fresh = Box::into_raw(Box::new(init()));
        if !slot::set(key, fresh.cast()) {
            // SAFETY: nothing took ownership, so this reclaims it.
            drop(unsafe { Box::from_raw(fresh) });
            return None;
        }
        // SAFETY: just stored, and not reachable from any other thread.
        Some(body(unsafe { &*fresh }))
    }
}
