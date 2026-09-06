// Copyright 2016 Amanieu d'Antras
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

//! Hardware lock elision, which this fork does not do.
//!
//! Upstream gates the x86 `xacquire`/`xrelease` prefixes behind an off-by-
//! default `hardware-lock-elision` feature. That feature is gone here, so
//! `have_elision` is `false` and the implementation below is the do-nothing
//! one that upstream compiles everywhere else.
//!
//! The file stays rather than the calls being deleted from `raw_rwlock.rs`,
//! because the lock itself is the one file worth keeping close to upstream,
//! and `have_elision()` folding to `false` costs nothing at run time.

use core::sync::atomic::AtomicUsize;

// Extension trait to add lock elision primitives to atomic types
pub trait AtomicElisionExt {
    type IntType;

    // Perform a compare_exchange and start a transaction
    fn elision_compare_exchange_acquire(
        &self,
        current: Self::IntType,
        new: Self::IntType,
    ) -> Result<Self::IntType, Self::IntType>;

    // Perform a fetch_sub and end a transaction
    fn elision_fetch_sub_release(&self, val: Self::IntType) -> Self::IntType;
}

// Indicates whether the target architecture supports lock elision
#[inline]
pub fn have_elision() -> bool {
    false
}

// This implementation is never actually called because it is guarded by
// have_elision().
impl AtomicElisionExt for AtomicUsize {
    type IntType = usize;

    #[inline]
    fn elision_compare_exchange_acquire(&self, _: usize, _: usize) -> Result<usize, usize> {
        unreachable!();
    }

    #[inline]
    fn elision_fetch_sub_release(&self, _: usize) -> usize {
        unreachable!();
    }
}
