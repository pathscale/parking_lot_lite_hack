//! That the locks still lock, and still *block*, with `std` switched off.
//!
//! Run this twice: once on default features and once with
//! `--no-default-features`. The second run builds the library without `std`,
//! so it exercises the `pthread_key_t` thread-local, the `clock_gettime`
//! instant and the platform thread identity, while the test harness itself
//! keeps its own `std`.
//!
//! Blocking is the part a type check cannot see, and it is what these
//! substitutions could plausibly break: a lock whose waiters spin passes every
//! correctness test here and then burns tens of times the CPU under
//! contention. `waiters_do_not_burn_cpu` is the one that separates them.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot_lite_hack::Mutex;

#[test]
fn mutual_exclusion_holds() {
    let counter = Arc::new(Mutex::new(0u64));
    let threads = 16;
    let each = 10_000u64;
    std::thread::scope(|scope| {
        for _ in 0..threads {
            let counter = counter.clone();
            scope.spawn(move || {
                for _ in 0..each {
                    *counter.lock() += 1;
                }
            });
        }
    });
    assert_eq!(*counter.lock(), threads as u64 * each);
}

#[test]
fn try_lock_fails_while_held_and_succeeds_after() {
    let lock = Mutex::new(0u64);
    let held = lock.lock();
    assert!(lock.try_lock().is_none(), "try_lock must fail while held");
    drop(held);
    assert!(lock.try_lock().is_some(), "and succeed once released");
}

/// The owned guard a lookup needs to return a reference into a locked value.
#[cfg(feature = "arc_lock")]
#[test]
fn an_owned_guard_outlives_the_call_that_took_it() {
    let lock = Arc::new(Mutex::new(7u64));
    let guard = lock.lock_arc();
    assert!(
        lock.is_locked(),
        "the lock is still held after the call returned"
    );
    assert_eq!(*guard, 7);
    drop(guard);
    assert!(!lock.is_locked(), "and released on drop");
}

/// The two tests that measure CPU hold this for their whole measurement.
///
/// `getrusage` reports the whole process, and the harness runs tests in
/// parallel, so without this each of them bills the other's threads and the
/// result depends on how many tests the enabled features happen to compile in.
static CPU_MEASUREMENT: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// CPU consumed by this process, user plus system.
fn cpu() -> Duration {
    // SAFETY: `getrusage` fully initialises the `rusage` for `RUSAGE_SELF`.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    assert_eq!(
        unsafe { libc::getrusage(libc::RUSAGE_SELF, &raw mut usage) },
        0
    );
    let secs =
        |t: libc::timeval| Duration::new(t.tv_sec as u64, (t.tv_usec as u32).saturating_mul(1_000));
    secs(usage.ru_utime) + secs(usage.ru_stime)
}

/// **The reason this crate exists.** A waiter must consume no CPU.
///
/// One lock, held for a long stretch, with far more threads than cores. A
/// spinlock burns a core per waiter and measures tens of times the CPU of the
/// wall clock. A blocking lock measures close to it, because the waiters are
/// not running.
///
/// Measured at 128 threads on 16 cores: this crate 136.7 ms of CPU against
/// 5513.6 for a spinlock. The bound below is 8x wall clock, which a spinlock
/// misses by an order of magnitude and a blocking lock meets with room to
/// spare, so this catches a regression to spinning without pinning a number.
#[test]
fn waiters_do_not_burn_cpu() {
    let _measuring = CPU_MEASUREMENT.lock().unwrap_or_else(|e| e.into_inner());
    let cores = std::thread::available_parallelism().map_or(8, std::num::NonZeroUsize::get);
    let threads = cores * 8;
    let lock = Arc::new(Mutex::new(0u64));
    let acquisitions = Arc::new(AtomicU64::new(0));

    let before = cpu();
    let now = Instant::now();
    std::thread::scope(|scope| {
        for _ in 0..threads {
            let lock = lock.clone();
            let acquisitions = acquisitions.clone();
            scope.spawn(move || {
                for _ in 0..8 {
                    let mut held = lock.lock();
                    for n in 0..20_000u64 {
                        *held = held.wrapping_mul(0x9e37_79b9).wrapping_add(n);
                    }
                    acquisitions.fetch_add(1, Ordering::Relaxed);
                }
            });
        }
    });
    let wall = now.elapsed();
    let burnt = cpu() - before;

    assert_eq!(acquisitions.load(Ordering::Relaxed), threads as u64 * 8);
    assert!(
        burnt.as_secs_f64() < wall.as_secs_f64() * 8.0,
        "{threads} threads on {cores} cores: {burnt:?} of CPU for {wall:?} of wall clock. \
         Waiters are running instead of blocking, which means this is spinning"
    );
}

/// The reader-writer lock excludes, and its waiters sleep.
///
/// The first half is ordinary. The second needs care to measure: readers on an
/// `RwLock` run *concurrently*, so on sixteen cores sixteen busy readers burn
/// sixteen times the wall clock quite legitimately, and a naive CPU ratio
/// catches parallelism rather than spinning. An earlier version of this test
/// did exactly that and failed a lock that was behaving correctly.
///
/// So the readers here are made to wait rather than to work: one writer takes
/// the lock and holds it, and every reader blocks on `read` behind it. During
/// that window a lock that parks costs almost nothing, and a lock that spins
/// costs a core per waiter.
#[test]
fn rwlock_excludes_and_its_waiters_sleep() {
    let _measuring = CPU_MEASUREMENT.lock().unwrap_or_else(|e| e.into_inner());
    use parking_lot_lite_hack::RwLock;

    let value = Arc::new(RwLock::new(0u64));
    let readers = 16;
    let held = Duration::from_millis(200);

    let write = value.write();
    let before = cpu();
    let started = Instant::now();

    std::thread::scope(|scope| {
        for _ in 0..readers {
            let value = value.clone();
            scope.spawn(move || {
                // Blocks until the writer above lets go.
                assert_eq!(*value.read(), 1);
            });
        }
        // Give every reader time to arrive and park before releasing.
        std::thread::sleep(held);
        let mut write = write;
        *write += 1;
        drop(write);
    });

    let wall = started.elapsed();
    let burnt = cpu() - before;

    assert_eq!(*value.read(), 1, "the write landed and every reader saw it");
    assert!(
        wall >= held,
        "the readers really did wait on the writer, {wall:?}"
    );
    assert!(
        burnt.as_secs_f64() < wall.as_secs_f64() * 2.0,
        "{readers} waiting readers burnt {burnt:?} of CPU over {wall:?} of wall \
         clock; a lock that parks costs almost nothing here, one that spins \
         costs a core per waiter"
    );
}
