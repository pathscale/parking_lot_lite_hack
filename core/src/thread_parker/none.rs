use crate::time::Instant;
use core::sync::atomic::{AtomicU32, Ordering};
use libc_rs::time::Timespec;

pub struct ThreadParker {
    futex: AtomicU32,
}

impl super::ThreadParkerT for ThreadParker {
    type UnparkHandle = UnparkHandle;

    const IS_CHEAP_TO_CONSTRUCT: bool = true;

    #[inline]
    fn new() -> ThreadParker {
        ThreadParker {
            futex: AtomicU32::new(0),
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
            libc_rs::sync::futex_wait(&self.futex, 1, None);
        }
    }

    #[inline]
    unsafe fn park_until(&self, timeout: Instant) -> bool {
        while self.futex.load(Ordering::Acquire) != 0 {
            let now = Instant::now();
            if timeout <= now {
                return false;
            }
            let diff = timeout.saturating_duration_since(now);
            let Ok(tv_sec) = i64::try_from(diff.as_secs()) else {
                unsafe { self.park() };
                return true;
            };
            let ts = Timespec {
                tv_sec,
                tv_nsec: i64::from(diff.subsec_nanos()),
            };
            libc_rs::sync::futex_wait(&self.futex, 1, Some(&ts));
        }
        true
    }

    #[inline]
    unsafe fn unpark_lock(&self) -> UnparkHandle {
        self.futex.store(0, Ordering::Release);
        UnparkHandle { futex: &self.futex }
    }
}

pub struct UnparkHandle {
    futex: *const AtomicU32,
}

impl super::UnparkHandleT for UnparkHandle {
    #[inline]
    unsafe fn unpark(self) {
        // SAFETY: `self.futex` points to the live `AtomicU32` inside `ThreadData`,
        // or if unmapped after wake, the kernel futex syscall returns `EFAULT`.
        let word = unsafe { &*self.futex };
        let _ = libc_rs::sync::futex_wake_one(word);
    }
}

#[inline]
pub fn thread_yield() {
    libc_rs::process::sched_yield();
}
