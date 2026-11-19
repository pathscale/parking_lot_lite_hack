// Copyright 2016 Amanieu d'Antras
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

use crate::time::Instant;
use core::{
    ffi::{c_int, c_long},
    ptr,
    sync::atomic::{AtomicI32, Ordering},
};
use eko::libc_lite as libc;

const FUTEX_WAIT: c_int = 0;
const FUTEX_WAKE: c_int = 1;
const FUTEX_PRIVATE_FLAG: c_int = 128;

#[cfg(target_arch = "x86_64")]
const SYS_FUTEX: c_long = 202;
#[cfg(target_arch = "aarch64")]
const SYS_FUTEX: c_long = 98;
#[cfg(target_arch = "riscv64")]
const SYS_FUTEX: c_long = 98;
#[cfg(target_arch = "x86")]
const SYS_FUTEX: c_long = 240;
#[cfg(target_arch = "arm")]
const SYS_FUTEX: c_long = 240;

unsafe extern "C" {
    fn syscall(num: c_long, ...) -> c_long;
}

// x32 Linux uses a non-standard type for tv_nsec in timespec.
// See https://sourceware.org/bugzilla/show_bug.cgi?id=16437
#[cfg(all(target_arch = "x86_64", target_pointer_width = "32"))]
#[allow(non_camel_case_types)]
type tv_nsec_t = i64;
#[cfg(not(all(target_arch = "x86_64", target_pointer_width = "32")))]
#[allow(non_camel_case_types)]
type tv_nsec_t = c_long;

fn errno() -> c_int {
    eko::Errno::current().0
}

// Helper type for putting a thread to sleep until some other thread wakes it up
pub struct ThreadParker {
    futex: AtomicI32,
}

impl super::ThreadParkerT for ThreadParker {
    type UnparkHandle = UnparkHandle;

    const IS_CHEAP_TO_CONSTRUCT: bool = true;

    #[inline]
    fn new() -> ThreadParker {
        ThreadParker {
            futex: AtomicI32::new(0),
        }
    }

    #[inline]
    unsafe fn prepare_park(&self) {
        self.futex.store(1, Ordering::Relaxed);
    }

    #[inline]
    unsafe fn timed_out(&self) -> bool {
        self.futex.load(Ordering::Relaxed) != 0
    }

    #[inline]
    unsafe fn park(&self) {
        while self.futex.load(Ordering::Acquire) != 0 {
            self.futex_wait(None);
        }
    }

    #[inline]
    unsafe fn park_until(&self, timeout: Instant) -> bool {
        while self.futex.load(Ordering::Acquire) != 0 {
            let now = Instant::now();
            if timeout <= now {
                return false;
            }
            let diff = timeout - now;
            if diff.as_secs() > i64::MAX as u64 {
                self.park();
                return true;
            }
            // SAFETY: libc::timespec is zero initializable.
            let mut ts: libc::timespec = core::mem::zeroed();
            ts.tv_sec = diff.as_secs() as _;
            ts.tv_nsec = diff.subsec_nanos() as tv_nsec_t;
            self.futex_wait(Some(ts));
        }
        true
    }

    #[inline]
    unsafe fn unpark_lock(&self) -> UnparkHandle {
        self.futex.store(0, Ordering::Release);

        UnparkHandle { futex: &self.futex }
    }
}

impl ThreadParker {
    #[inline]
    fn futex_wait(&self, ts: Option<libc::timespec>) {
        let ts_ptr = ts
            .as_ref()
            .map(|ts_ref| ts_ref as *const _)
            .unwrap_or(ptr::null());
        let r = unsafe {
            syscall(
                SYS_FUTEX,
                &self.futex,
                FUTEX_WAIT | FUTEX_PRIVATE_FLAG,
                1,
                ts_ptr,
            )
        };
        debug_assert!(r == 0 || r == -1);
        if r == -1 {
            debug_assert!(
                errno() == libc::EINTR
                    || errno() == libc::EAGAIN
                    || (ts.is_some() && errno() == libc::ETIMEDOUT)
            );
        }
    }
}

pub struct UnparkHandle {
    futex: *const AtomicI32,
}

impl super::UnparkHandleT for UnparkHandle {
    #[inline]
    unsafe fn unpark(self) {
        let r = syscall(
            SYS_FUTEX,
            self.futex,
            FUTEX_WAKE | FUTEX_PRIVATE_FLAG,
            1,
        );
        debug_assert!(r == 0 || r == 1 || r == -1);
        if r == -1 {
            debug_assert_eq!(errno(), libc::EFAULT);
        }
    }
}

#[inline]
pub fn thread_yield() {
    unsafe {
        libc::sched_yield();
    }
}
