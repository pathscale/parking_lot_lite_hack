//! What this build of `parking_lot` costs, against the other locks on the shelf.
//!
//! # What this is for
//!
//! Switching `std` off must not cost anything, and the way to know is to
//! measure the same two shapes with it on and with it off. `spin` and Arctic
//! are the other columns, so the result is read against something rather than
//! quoted on its own.
//!
//! It is sized to finish in a few seconds, so it can run on every change
//! rather than being the thing nobody runs. That budget buys relative
//! standing, not absolute throughput: read the columns against each other and
//! against `null`, never as a number to quote on its own.
//!
//! `null` is the `parking_lot` arm measured a second time under a different
//! name. Two identical arms have landed 7.8% apart on this hardware, so
//! anything inside the null's distance from 1.00x is noise. A regression has
//! to clear it.
//!
//! # The two shapes
//!
//! **A contended mutex.** One lock, held long, more threads than cores. This
//! is where a lock has to *block*: a waiter that keeps running holds the core
//! the holder needs.
//!
//! **A read-mostly map.** Every operation takes the lock, does one lookup, and
//! drops it. This is where a reader-writer lock's own bookkeeping costs more
//! than the work it guards, because reader counts and fairness state live on
//! one cache line that every reader writes to.
//!
//! # The floor, and the arm that beats it
//!
//! One line under the reader-writer table is the same lookups with no lock at
//! all, over a `&BTreeMap` nobody mutates. Whatever an arm costs above that is
//! what its synchronisation costs, and at zero writes that is synchronisation
//! against a writer that never arrives.
//!
//! Arctic, a lock-free adaptive radix tree, is in the table because it beats
//! that floor while still taking writes. Read the two `parking_lot` columns
//! against each other to see what `std` costs, which is nothing, and then read
//! the whole pair against Arctic to see what taking a lock costs, which is not
//! nothing.
//!
//! An `ArcSwap` arm used to sit next to it, reaching the floor exactly. It was
//! removed rather than kept as an aspiration, because a snapshot is not a
//! substitute for a lock: a consumer that holds its guard across a compound
//! operation is guaranteed the node it found cannot be split underneath it,
//! and a reader on an older snapshot has no such guarantee. That is a lost
//! write, at any write ratio, so it is not a trade-off to be measured.
//!
//! The write ratios are a fraction of **operations**. An earlier version said
//! "25% writers" while meaning 25% of threads each writing once per 256 steps,
//! which is 0.098% of operations: a soft test presented as a hard one.

use std::collections::BTreeMap;
use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Sized for a few seconds of wall clock, total. See the module header on what
/// that budget does and does not buy.
const ENTRIES: u64 = 1_024;
const OPS: usize = 60_000;
const HELD_ITERS: u64 = 20_000;
const HOLDS: usize = 8;
const REPS: usize = 5;
/// Writes per thousand operations, across all threads.
const RATIOS: [u64; 4] = [0, 1, 10, 100];

/// CPU consumed by this process, user plus system. Wall clock cannot see a
/// burnt core, and burning cores is exactly how a spinlock loses.
fn cpu() -> Duration {
    // SAFETY: `getrusage` either fills the `rusage` for `RUSAGE_SELF` or
    // returns non-zero without writing.
    let mut usage: libc::rusage = unsafe { core::mem::zeroed() };
    assert_eq!(
        unsafe { libc::getrusage(libc::RUSAGE_SELF, &raw mut usage) },
        0,
        "getrusage"
    );
    let part =
        |t: libc::timeval| Duration::new(t.tv_sec as u64, (t.tv_usec as u32).saturating_mul(1_000));
    part(usage.ru_utime) + part(usage.ru_stime)
}

fn seeded(worker: usize) -> u64 {
    0x2545_F491_4F6C_DD1D ^ (worker as u64).wrapping_mul(0x9E37_79B9)
}

fn roll(rng: &mut u64) -> u64 {
    *rng ^= *rng << 13;
    *rng ^= *rng >> 7;
    *rng ^= *rng << 17;
    *rng
}

// ---- shape one: a contended mutex ---------------------------------------

macro_rules! held_arm {
    ($name:ident, $mx:ty) => {
        fn $name(threads: usize) -> (Duration, Duration) {
            let lock: Arc<$mx> = Arc::new(<$mx>::new(0u64));
            let before = cpu();
            let now = Instant::now();
            std::thread::scope(|scope| {
                for _ in 0..threads {
                    let lock = Arc::clone(&lock);
                    scope.spawn(move || {
                        for _ in 0..HOLDS {
                            let mut held = lock.lock();
                            for n in 0..HELD_ITERS {
                                *held = held.wrapping_mul(0x9e37_79b9).wrapping_add(n);
                            }
                        }
                    });
                }
            });
            (now.elapsed(), cpu() - before)
        }
    };
}

held_arm!(held_upstream, parking_lot::Mutex<u64>);
held_arm!(held_a, parking_lot_lite_hack::Mutex<u64>);
held_arm!(held_spin, spin::Mutex<u64>);

// ---- shape two: a read-mostly map ---------------------------------------

type Map = BTreeMap<u64, u64>;

fn seed() -> Map {
    (0..ENTRIES).map(|n| (n, n.wrapping_mul(31))).collect()
}

macro_rules! map_arm {
    ($name:ident, $rw:ty) => {
        fn $name(threads: usize, writes_per_1000: u64) -> (Duration, Duration) {
            let map: Arc<$rw> = Arc::new(<$rw>::new(seed()));
            let before = cpu();
            let now = Instant::now();
            std::thread::scope(|scope| {
                for worker in 0..threads {
                    let map = Arc::clone(&map);
                    scope.spawn(move || {
                        let mut acc = 0u64;
                        let mut rng = seeded(worker);
                        for _ in 0..OPS / threads {
                            let r = roll(&mut rng);
                            if r % 1000 < writes_per_1000 {
                                map.write().insert(r % ENTRIES, r);
                            } else if let Some(found) = map.read().get(&(r % ENTRIES)) {
                                acc ^= *found;
                            }
                        }
                        black_box(acc);
                    });
                }
            });
            (now.elapsed(), cpu() - before)
        }
    };
}

map_arm!(map_upstream, parking_lot::RwLock<Map>);
map_arm!(map_a, parking_lot_lite_hack::RwLock<Map>);

/// Arctic, a lock-free adaptive radix tree, doing the same work with no lock.
///
/// This arm is not a competitor to the two `parking_lot` columns, it is the
/// question they should be read against. A `BTreeMap` behind a reader-writer
/// lock is the *shape* this crate exists to serve, not a structure anyone
/// should reach for when a lock-free ordered map is available: the locked arms
/// pay coordination on every operation and this one does not.
fn map_arctic(threads: usize, writes_per_1000: u64) -> (Duration, Duration) {
    let map = arctic::ConcurrentMap::<u64, u64>::default();
    for key in 0..ENTRIES {
        map.upsert(key, key.wrapping_mul(31));
    }
    let map = &map;
    let before = cpu();
    let now = Instant::now();
    std::thread::scope(|scope| {
        for worker in 0..threads {
            scope.spawn(move || {
                let mut acc = 0u64;
                let mut rng = seeded(worker);
                for _ in 0..OPS / threads {
                    let r = roll(&mut rng);
                    if r % 1000 < writes_per_1000 {
                        map.upsert(r % ENTRIES, r);
                    } else if let Some(found) = map.get(&(r % ENTRIES)) {
                        acc ^= *found;
                    }
                }
                black_box(acc);
            });
        }
    });
    (now.elapsed(), cpu() - before)
}

/// The same lookups with no lock at all: the floor.
///
/// A shared `&Map` that nobody mutates, so this is the work the other arms are
/// doing plus nothing. Whatever an arm costs above this is what its
/// synchronisation costs, and at zero writes that is synchronisation against a
/// writer that never arrives.
///
/// Only valid at zero writes, which is why it takes no ratio.
fn map_nolock(threads: usize) -> (Duration, Duration) {
    let map = seed();
    let map = &map;
    let before = cpu();
    let now = Instant::now();
    std::thread::scope(|scope| {
        for worker in 0..threads {
            scope.spawn(move || {
                let mut acc = 0u64;
                let mut rng = seeded(worker);
                for _ in 0..OPS / threads {
                    let r = roll(&mut rng);
                    if let Some(found) = map.get(&(r % ENTRIES)) {
                        acc ^= *found;
                    }
                }
                black_box(acc);
            });
        }
    });
    (now.elapsed(), cpu() - before)
}

// ---- reporting -----------------------------------------------------------

fn median(mut runs: Vec<(Duration, Duration)>) -> (Duration, Duration) {
    runs.sort_by_key(|(wall, _)| *wall);
    runs[REPS / 2]
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

fn main() {
    let started = Instant::now();
    let cores = std::thread::available_parallelism().map_or(8, std::num::NonZeroUsize::get);
    println!("\n{cores} cores, median of {REPS}, milliseconds.");

    println!("\nMUTEX  one lock held {HELD_ITERS} iterations, {HOLDS} times per thread");
    println!("        upstream (std)       this build            spin (context)");
    println!("  threads    wall      cpu      wall      cpu      wall      cpu    null");
    for &threads in &[cores, cores * 2, cores * 8] {
        let mut runs: Vec<Vec<(Duration, Duration)>> = vec![Vec::new(); 4];
        for _ in 0..REPS {
            runs[0].push(held_upstream(threads));
            runs[1].push(held_a(threads));
            runs[2].push(held_spin(threads));
            runs[3].push(held_upstream(threads));
        }
        let m: Vec<_> = runs.into_iter().map(median).collect();
        println!(
            "  {threads:>7} {:>7.1}  {:>7.1}   {:>7.1}  {:>7.1}   {:>7.1}  {:>7.1}  {:>5.2}x",
            ms(m[0].0),
            ms(m[0].1),
            ms(m[1].0),
            ms(m[1].1),
            ms(m[2].0),
            ms(m[2].1),
            m[0].0.as_secs_f64() / m[3].0.as_secs_f64(),
        );
    }

    println!("\nRWLOCK  {OPS} operations on a {ENTRIES} entry map, {cores} threads");
    println!("  writes are a fraction of operations, not of threads");
    let floor = median((0..REPS).map(|_| map_nolock(cores)).collect());
    println!(
        "  no lock at all (the work alone): {:.1} wall, {:.1} cpu",
        ms(floor.0),
        ms(floor.1)
    );
    println!("       upstream (std)       this build           arctic (lock-free)");
    println!("  wr/1000    wall      cpu      wall      cpu      wall      cpu    null");
    for &writes in &RATIOS {
        let mut runs: Vec<Vec<(Duration, Duration)>> = vec![Vec::new(); 4];
        for _ in 0..REPS {
            runs[0].push(map_upstream(cores, writes));
            runs[1].push(map_a(cores, writes));
            runs[2].push(map_arctic(cores, writes));
            runs[3].push(map_upstream(cores, writes));
        }
        let m: Vec<_> = runs.into_iter().map(median).collect();
        println!(
            "  {writes:>7} {:>7.1}  {:>7.1}   {:>7.1}  {:>7.1}   {:>7.1}  {:>7.1}  {:>5.2}x",
            ms(m[0].0),
            ms(m[0].1),
            ms(m[1].0),
            ms(m[1].1),
            ms(m[2].0),
            ms(m[2].1),
            m[0].0.as_secs_f64() / m[3].0.as_secs_f64(),
        );
    }

    println!(
        "\n  harness wall clock: {:.1}s",
        started.elapsed().as_secs_f64()
    );
}
