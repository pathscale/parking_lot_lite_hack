// Copyright 2016 Amanieu d'Antras
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

//! `parking_lot`'s `Mutex` and `RwLock`, with `std` patched out and everything
//! else removed.
//!
//! This is a fork, not a reimplementation: `raw_mutex.rs` and `raw_rwlock.rs`
//! are upstream's own files. What is gone is `Condvar`, `Once`,
//! `ReentrantMutex`, deadlock detection, hardware lock elision, serde, and
//! every thread parker except unix, linux and Windows.
//!
//! `no_std` here means the crate does not link `std`. It does not mean it
//! cannot make a syscall: `libc` with default features off is itself `no_std`,
//! so a `no_std` crate on a hosted target can still block in the kernel. That
//! is why this matches upstream's numbers rather than losing to them the way a
//! spinlock does.
//!
//! The package is named `parking_lot_lite_hack` so it cannot collide with the
//! real `parking_lot`. Opt in by renaming it back:
//!
//! ```toml
//! parking_lot = { package = "parking_lot_lite_hack", version = "0.12", default-features = false, features = ["arc_lock", "send_guard"] }
//! ```

// `not(test)` so the crate keeps the `std` prelude when the harness compiles
// it. The substitutions below are selected on `feature = "std"` rather than on
// this attribute, so the unit tests exercise the `no_std` code paths either
// way, and `cargo check --no-default-features` is what proves the library
// really does not link `std`.
#![cfg_attr(all(not(feature = "std"), not(test)), no_std)]
#![warn(missing_docs)]
#![warn(rust_2018_idioms)]

mod elision;
mod fair_mutex;
mod mutex;
mod raw_fair_mutex;
mod raw_mutex;
mod raw_rwlock;
mod rwlock;
mod util;

#[cfg(feature = "send_guard")]
type GuardMarker = lock_api::GuardSend;
#[cfg(not(feature = "send_guard"))]
type GuardMarker = lock_api::GuardNoSend;

pub use self::fair_mutex::{const_fair_mutex, FairMutex, FairMutexGuard, MappedFairMutexGuard};
pub use self::mutex::{const_mutex, MappedMutexGuard, Mutex, MutexGuard};
pub use self::raw_fair_mutex::RawFairMutex;
pub use self::raw_mutex::RawMutex;
pub use self::raw_rwlock::RawRwLock;
pub use self::rwlock::{
    const_rwlock, MappedRwLockReadGuard, MappedRwLockWriteGuard, RwLock, RwLockReadGuard,
    RwLockUpgradableReadGuard, RwLockWriteGuard,
};
pub use ::lock_api;

#[cfg(feature = "arc_lock")]
pub use self::lock_api::{ArcMutexGuard, ArcRwLockReadGuard, ArcRwLockWriteGuard};
