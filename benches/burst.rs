//! The shape a market-data index actually sees: idle, then a dense write burst.
//!
//! # Why the other benchmark is not this one
//!
//! `vs_parking_lot.rs` sweeps a uniform write ratio over a long run and reports
//! a mean. That models a steady mixed workload, and the workload here is not
//! one. A HyperLiquid feed carries 200-odd symbols and dumps every orderbook
//! inside a 2-10 ms window, once every 500 ms. So the index sits almost idle,
//! takes roughly 4,000 writes in a couple of milliseconds with the queue deep
//! behind them, and goes quiet again.
//!
//! A mean over that is meaningless: it averages an empty period against a
//! saturated one and describes neither. What decides whether the feed keeps up
//! is how fast the burst drains and how bad the worst update in it is. The
//! tail is the product, not the average.
//!
//! # What is measured
//!
//! Per burst: how long until the last of `SYMBOLS * LEVELS` updates lands, the
//! CPU that took, and the slowest single update. Readers run throughout, since
//! in the real system consumers keep querying while the burst is arriving, and
//! they are what a writer has to get past.
//!
//! Reported as a median burst and a worst burst across `BURSTS` of them,
//! because a feed that keeps up on the median and misses one burst in twenty
//! has not kept up.
//!
//! The idle period is deliberately not simulated. Between bursts the index sees
//! reads and no writes, which is the zero-write row of the other benchmark, and
//! there all of these locks are within noise of each other. Nothing is decided
//! there, so nothing is spent measuring it.

use std::collections::BTreeMap;
use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Symbols on the feed.
const SYMBOLS: u64 = 200;
/// Book levels rewritten per symbol per burst.
const LEVELS: u64 = 20;
/// Threads delivering the burst.
///
/// Deliberately short of the core count. At one thread per core the harness
/// has no headroom for the OS, its own threads are descheduled inside critical
/// sections, and one arm drifted 3.55 to 3.04 M/s across six identical rounds.
/// A real feed handler shards across a few threads on a machine that is not
/// otherwise saturated; this is that, and it is measurable.
const WRITERS: usize = 4;
/// Consumers querying throughout.
const READERS: usize = 4;
const BURSTS: usize = 20;

type Book = BTreeMap<u64, u64>;

fn cpu() -> Duration {
    // SAFETY: `getrusage` fills the `rusage` for `RUSAGE_SELF` or returns
    // non-zero without writing.
    let mut usage: libc::rusage = unsafe { core::mem::zeroed() };
    assert_eq!(
        unsafe { libc::getrusage(libc::RUSAGE_SELF, &raw mut usage) },
        0
    );
    let part =
        |t: libc::timeval| Duration::new(t.tv_sec as u64, (t.tv_usec as u32).saturating_mul(1_000));
    part(usage.ru_utime) + part(usage.ru_stime)
}

fn seed() -> Book {
    (0..SYMBOLS * LEVELS).map(|n| (n, n)).collect()
}

/// One burst: every symbol's book rewritten, readers querying throughout.
///
/// Returns how long the burst took to drain, the CPU it cost, and the slowest
/// single update in it.
macro_rules! burst_arm {
    ($name:ident, $rw:ty) => {
        fn $name(think: u32) -> (Duration, Duration, Vec<Duration>) {
            let book: Arc<$rw> = Arc::new(<$rw>::new(seed()));
            let stop = Arc::new(core::sync::atomic::AtomicBool::new(false));
            let samples = Arc::new(std::sync::Mutex::new(Vec::<Duration>::new()));

            let before = cpu();
            let started = Instant::now();
            std::thread::scope(|scope| {
                for _ in 0..READERS {
                    let book = Arc::clone(&book);
                    let stop = Arc::clone(&stop);
                    scope.spawn(move || {
                        let mut acc = 0u64;
                        let mut key = 0u64;
                        while !stop.load(core::sync::atomic::Ordering::Relaxed) {
                            key = key.wrapping_add(2_654_435_761) % (SYMBOLS * LEVELS);
                            if let Some(found) = book.read().get(&key) {
                                acc ^= *found;
                            }
                            // Think time. A real consumer does not re-query
                            // the instant its last answer lands.
                            for _ in 0..think {
                                core::hint::spin_loop();
                            }
                        }
                        black_box(acc);
                    });
                }
                let handles: Vec<_> = (0..WRITERS)
                    .map(|w| {
                        let book = Arc::clone(&book);
                        let samples = Arc::clone(&samples);
                        scope.spawn(move || {
                            let mut mine =
                                Vec::with_capacity((SYMBOLS / WRITERS as u64 * LEVELS) as usize);
                            // Each writer owns a contiguous slice of symbols,
                            // as a feed handler shard would.
                            let per = SYMBOLS / WRITERS as u64;
                            let from = w as u64 * per;
                            for symbol in from..from + per {
                                for level in 0..LEVELS {
                                    let key = symbol * LEVELS + level;
                                    let at = Instant::now();
                                    book.write().insert(key, key.wrapping_mul(31));
                                    mine.push(at.elapsed());
                                }
                            }
                            samples.lock().expect("not poisoned").extend(mine);
                        })
                    })
                    .collect();
                for handle in handles {
                    handle.join().expect("writer");
                }
                stop.store(true, core::sync::atomic::Ordering::Relaxed);
            });
            let drained = started.elapsed();
            let latencies = core::mem::take(&mut *samples.lock().expect("not poisoned"));
            (drained, cpu() - before, latencies)
        }
    };
}

burst_arm!(burst_upstream, parking_lot::RwLock<Book>);
burst_arm!(burst_parking, parking_lot_lite_hack::RwLock<Book>);
burst_arm!(burst_spin, spin::RwLock<Book>);

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

/// The value at `fraction` through a sorted sample.
fn at(sorted: &[Duration], fraction: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let index = ((sorted.len() - 1) as f64 * fraction) as usize;
    sorted[index]
}

fn main() {
    let updates = SYMBOLS * LEVELS;
    println!(
        "\n{SYMBOLS} symbols x {LEVELS} levels = {updates} updates per burst, \
         {WRITERS} writers, {READERS} readers, {BURSTS} bursts"
    );
    println!(
        "\n  thru  = updates per second while a burst is draining, in millions.\n  \
           drain = time for a whole burst to land; the feed sends one every 500 ms.\n  \
           p50..max = latency of one update, across every update of every burst."
    );

    for think in [0u32, 100, 1_000, 10_000] {
        println!("\n  reader think time: {think}");
        println!("                 thru   cpu ms    drain ms          update latency ms");
        println!("  backend        M/s   median   median   worst    p50     p99   p99.9     max");
        for (name, arm) in [
            (
                "upstream (std)",
                burst_upstream as fn(u32) -> (Duration, Duration, Vec<Duration>),
            ),
            (
                "this build",
                burst_parking as fn(u32) -> (Duration, Duration, Vec<Duration>),
            ),
            ("spin", burst_spin),
            // The first arm again, last, so the table carries its own noise
            // floor: whatever this differs from `upstream (std)` by is drift,
            // not a result.
            ("null (upstream)", burst_upstream),
        ] {
            let mut drains = Vec::with_capacity(BURSTS);
            let mut cpus = Vec::with_capacity(BURSTS);
            let mut latencies = Vec::with_capacity(BURSTS * updates as usize);
            for _ in 0..BURSTS {
                let (drain, burst_cpu, mut samples) = arm(think);
                drains.push(drain);
                cpus.push(burst_cpu);
                latencies.append(&mut samples);
            }
            drains.sort_unstable();
            cpus.sort_unstable();
            latencies.sort_unstable();
            let median_drain = drains[BURSTS / 2];
            let worst_drain = drains[BURSTS - 1];
            let throughput = updates as f64 / median_drain.as_secs_f64() / 1e6;
            println!(
                "  {name:<12} {throughput:>5.2} {:>7.2}  {:>7.2} {:>7.2}  {:>6.3} {:>7.3} {:>7.3} {:>7.3}",
                ms(cpus[BURSTS / 2]),
                ms(median_drain),
                ms(worst_drain),
                ms(at(&latencies, 0.50)),
                ms(at(&latencies, 0.99)),
                ms(at(&latencies, 0.999)),
                ms(at(&latencies, 1.0)),
            );
        }
    }
}
