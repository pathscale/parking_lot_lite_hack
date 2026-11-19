// Copyright 2016 Amanieu d'Antras
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

//! The monotonic clock the parking lot uses, over `ekostd` on Unix / `target_os = "none"`
//! and `QueryPerformanceCounter` on Windows.

pub use self::monotonic::Instant;

mod monotonic {
    use core::ops::Add;
    use core::time::Duration;

    /// A point on the system's monotonic clock.
    #[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
    pub struct Instant {
        nanos: u128,
    }

    impl Instant {
        /// Read the clock.
        #[cfg(unix)]
        pub fn now() -> Self {
            use eko::libc_lite as libc;
            // SAFETY: `clock_gettime` either fills the `timespec` or returns
            // non-zero without writing to it.
            let mut now = unsafe { core::mem::zeroed::<libc::timespec>() };
            assert_eq!(
                unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &raw mut now) },
                0,
                "CLOCK_MONOTONIC"
            );
            Self {
                nanos: now.tv_sec as u128 * 1_000_000_000 + now.tv_nsec as u128,
            }
        }

        /// Read the clock.
        #[cfg(target_os = "none")]
        pub fn now() -> Self {
            let now = libc_rs::time::clock_gettime(libc_rs::time::CLOCK_MONOTONIC)
                .expect("CLOCK_MONOTONIC");
            Self {
                nanos: now.tv_sec.max(0) as u128 * 1_000_000_000 + now.tv_nsec.max(0) as u128,
            }
        }

        /// Read the clock.
        #[cfg(windows)]
        pub fn now() -> Self {
            use core::sync::atomic::{AtomicU64, Ordering};
            use windows_sys::Win32::System::Performance::{
                QueryPerformanceCounter, QueryPerformanceFrequency,
            };

            static TICKS_PER_SECOND: AtomicU64 = AtomicU64::new(0);
            let mut frequency = TICKS_PER_SECOND.load(Ordering::Relaxed);
            if frequency == 0 {
                let mut read = 0i64;
                assert!(unsafe { QueryPerformanceFrequency(&raw mut read) } != 0);
                frequency = read as u64;
                TICKS_PER_SECOND.store(frequency, Ordering::Relaxed);
            }
            let mut ticks = 0i64;
            assert!(unsafe { QueryPerformanceCounter(&raw mut ticks) } != 0);
            Self {
                nanos: ticks as u128 * 1_000_000_000 / frequency as u128,
            }
        }

        /// Saturating rather than wrapping, if a caller has the two the wrong
        /// way round.
        pub fn saturating_duration_since(self, earlier: Self) -> Duration {
            let nanos = self.nanos.saturating_sub(earlier.nanos);
            Duration::new(
                (nanos / 1_000_000_000) as u64,
                (nanos % 1_000_000_000) as u32,
            )
        }

        /// `None` if `earlier` is later, rather than a zero `Duration`.
        pub fn checked_duration_since(self, earlier: Self) -> Option<Duration> {
            (self >= earlier).then(|| self.saturating_duration_since(earlier))
        }

        /// Always `Some`: the counter is 128 bits wide and starts at boot.
        pub fn checked_add(self, duration: Duration) -> Option<Self> {
            Some(self + duration)
        }
    }

    impl Add<Duration> for Instant {
        type Output = Self;

        fn add(self, duration: Duration) -> Self {
            Self {
                nanos: self.nanos + duration.as_nanos(),
            }
        }
    }

    impl core::ops::Sub for Instant {
        type Output = Duration;

        fn sub(self, earlier: Self) -> Duration {
            self.saturating_duration_since(earlier)
        }
    }
}
