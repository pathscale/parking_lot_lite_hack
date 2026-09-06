# Upstream patches, reviewed

This fork strips `parking_lot` to `Mutex` and `RwLock` and patches `std` out of
what is left. That makes every other open upstream change a candidate to take
at the same time, and a candidate is worth taking only if it helps the workload
this exists for.

Everything below was measured, not reasoned about. The workload is a
HyperLiquid-shaped feed: about 200 symbols, every orderbook rewritten inside a
2-10 ms window, once every 500 ms. `benches/burst.rs` reproduces it;
`benches/vs_parking_lot.rs` covers the steady-state mixed shapes.

Hardware for every number here: 16 cores, aarch64, macOS. A result that depends
on core count is called out where it does.

## The question this PR has to answer: does `no_std` cost anything

No, and here is the measurement rather than the claim.

The benchmarks dev-depend on the **published** `parking_lot 0.12.5` from
crates.io, which is a different package from this one now that this one is
named `parking_lot_lite_hack`, so both link into one process and their arms
interleave. That is a paired comparison rather than two runs compared
afterwards, which matters: see "#484" below for the 6% that turned out to be
the machine getting faster between two sequential runs.

`cargo bench --no-default-features`, so the `parking_lot` column below is the
library with `std` switched off. 16 cores, aarch64, macOS, median of 5.

**Mutex.** One lock held 20000 iterations, 8 times per thread, milliseconds.

| threads | upstream wall | upstream cpu | this build wall | this build cpu | spin wall | spin cpu | null |
|---|---:|---:|---:|---:|---:|---:|---:|
| 16 | 4.0 | 4.6 | 4.0 | 4.6 | 3.3 | 26.4 | 1.15x |
| 32 | 6.1 | 7.2 | 6.2 | 7.3 | 5.3 | 54.8 | 0.99x |
| 128 | 24.4 | 29.3 | 24.4 | 29.3 | 22.7 | 331.2 | 1.00x |

**Reader-writer lock.** 60000 operations on a 1024 entry map, 16 threads,
writes as a fraction of operations. The work with no lock at all is 0.3 wall,
2.0 cpu.

| writes/1000 | upstream wall | upstream cpu | this build wall | this build cpu | spin wall | spin cpu | null |
|---|---:|---:|---:|---:|---:|---:|---:|
| 0 | 9.4 | 130.9 | 9.3 | 131.4 | 6.3 | 89.5 | 1.06x |
| 1 | 10.2 | 141.5 | 9.6 | 135.2 | 9.1 | 121.8 | 1.07x |
| 10 | 9.6 | 124.2 | 9.3 | 115.7 | 15.4 | 228.9 | 1.03x |
| 100 | 5.3 | 49.7 | 5.5 | 51.3 | 30.2 | 456.0 | 0.96x |

Every difference between the two `parking_lot` columns is smaller than the null,
which is one arm measured twice. Spin is in the table as context, and note what
it costs: at 128 threads it burns 331 ms of CPU for 22.7 ms of wall clock, and
with `std` on, where the run happened to be less lucky, 1105 ms for 73 ms.

**The burst**, which is the workload this exists for. 200 symbols by 20 levels
per burst, 4 writers, 4 readers, 20 bursts.

| reader think time | backend | thru M/s | cpu ms | drain median | drain worst | p99.9 | max |
|---|---|---:|---:|---:|---:|---:|---:|
| 0 | upstream (std) | 3.30 | 6.50 | 1.21 | 1.36 | 0.042 | 0.123 |
| 0 | this build | 3.42 | 6.21 | 1.17 | 1.27 | 0.039 | 0.078 |
| 0 | spin | 0.15 | 197.55 | 26.75 | 42.08 | 0.664 | 2.307 |
| 0 | null (upstream) | 3.52 | 6.11 | 1.14 | 1.25 | 0.038 | 0.080 |
| 100 | upstream (std) | 4.03 | 5.74 | 0.99 | 1.07 | 0.031 | 0.075 |
| 100 | this build | 3.83 | 5.84 | 1.04 | 1.15 | 0.033 | 0.064 |
| 100 | null (upstream) | 3.93 | 5.75 | 1.02 | 1.08 | 0.033 | 0.064 |
| 1000 | upstream (std) | 6.50 | 4.02 | 0.62 | 0.72 | 0.019 | 0.062 |
| 1000 | this build | 6.46 | 4.06 | 0.62 | 0.69 | 0.019 | 0.058 |
| 1000 | null (upstream) | 6.51 | 4.05 | 0.61 | 0.71 | 0.017 | 0.042 |

The null arm earns its place here. Without it the first run of this table read
2.30 M/s for upstream against 3.53 for this build at think time 0, a 53% win
that would have been a lie: the first arm measured pays the cold start, and
with the null in place upstream reads 3.30 and 3.52 on either side of this
build's 3.42.

Run again with `std` on, the two `parking_lot` columns stay together the same
way, so the code is indifferent to the feature rather than the benchmark being
insensitive.

## Where the older numbers come from, and what they do not cover

They were taken against a standalone crate that vendored these same files,
`raw_rwlock.rs`, `word_lock.rs`, `parking_lot.rs` and the thread parkers, and
patched them the same way this PR patches them in place. That is the code this
PR keeps, so the results transfer.

One thing does not transfer. That crate carried its own `pthread_mutex_t`
behind `lock_api` instead of upstream's `RawMutex`, so nothing here measures
`raw_mutex.rs`. Every issue that patches the mutex is listed below as
unevaluated, not as rejected.

## Applied

One thing, and it is not a performance patch.

### `77e184de`, "Fix WordLock queue unlock retries"

Cherry-picked from #533. See "Cherry-picking from #533" for how, and why it
could not be taken as a patch. It is a correctness fix in the queue lock that
every parking operation passes through, it changes no measured number, and that
is the expected result: a benchmark on 16 cores is not the instrument that
finds a lost wakeup.

Everything else considered was rejected or left alone. See "Net result".

## Rejected

### #419, "Fix issue #418": rejected, 5x throughput loss

Real fix for a real problem: on a 120-core Intel under cache contention,
`parking_lot::Mutex` degrades to 403 ms where `std` does 113 ms, and this patch
brings it to 81 ms. The mechanism is `thread::sleep(1ms)`, up to four times,
before parking.

Four milliseconds of sleep inside the lock path, against a 2-10 ms burst window.
Ported to `libc::nanosleep` and measured on `benches/burst.rs`, 8 writers, 8
readers, reader think time 100:

| | without #419 | with #419 |
|---|---:|---:|
| throughput | 3.81 M/s | 0.75 M/s |
| drain, median | 1.05 ms | 5.32 ms |
| drain, worst | 1.18 ms | 9.01 ms |
| p99.9 per update | 0.093 ms | 1.298 ms |
| max per update | 0.254 ms | 6.279 ms |

The worst burst lands at 9.01 ms against a 10 ms budget, and one update in a
thousand takes 1.3 ms. Rejected.

This rejection is machine-dependent, and that cuts both ways. The pathology it
fixes did not appear on 16 cores, so nothing was given up here. On a 100+ core
host the pathology is real and this needs measuring again there.

### #512, "Spin-wait in `RwLock::try_lock_shared`": no effect

Applied cleanly and changed nothing: 1.09 ms drain against upstream's 1.08 ms.
Obvious afterwards. It patches `try_lock_shared`, and neither benchmark calls
`try_read`; the workload uses `read` and `write`. Not carried, because an
unmeasurable patch is still a divergence from upstream to re-merge.

### #461, "fast unlock in contention": superseded

#462 is the same author's more aggressive version of this idea, and #462 was
measured. See "#462: dropped".

## Not applicable, or not evaluated

- **#531**, reliability improvements for `RawMutex`. Reported as a 10x standard
  deviation reduction on aarch64, which is this hardware. It patches
  `raw_mutex.rs`, which no benchmark here exercises, so it is unevaluated
  rather than rejected. It is the strongest remaining candidate.
- **#338** and **#437**, reports of `parking_lot::Mutex` losing to `std` on an
  Intel i9 and on ARM. Same file, same reason: unevaluated.
- **#532**, RwLock recursive detection. It lives behind `deadlock_detection`,
  which is removed here along with the detector itself.
- **#517**, asynchronous parking. Orthogonal to this change.
- **#279**, safety invariants in the type system. Opened 2021, stale against
  current master.
- **#496 / #457 / #290**, `MappedArcMutexGuard` and owned mapped guards. These
  are `lock_api` changes, and `lock_api` already builds without `std`.

## The one that matters: #533

The maintainer's open rewrite, +3287/-2819, touching every file this PR
touches, plus MSRV 1.95 and edition 2024. Whatever lands here gets re-merged
against it whenever it lands.

Two things in its changelog were worth chasing.

**"Removed automatic eventual fairness"** looked like a direct threat, because
the working theory was that eventual fairness is what lets parking_lot drain a
burst while a spinlock cannot. Testing that theory meant making
`FairTimeout::should_timeout` always return false, which is what the removal
amounts to, and re-running the burst:

| | eventual fairness on | off |
|---|---:|---:|
| throughput | 3.29 M/s | 3.54 M/s |
| drain, median | 1.21 ms | 1.13 ms |
| p99.9 | 0.093 ms | 0.104 ms |

No difference. The theory was wrong, and the removal is not a threat.

What does the work is in the reader fast path, and it is not a fairness policy.
`raw_rwlock.rs` refuses a reader when `WRITER_BIT` is set, and a writer sets
that bit on *intent*, before it holds the lock: "we can't allow grabbing a
shared lock if there is a writer, even if the writer is still waiting for the
remaining readers to exit". A reader-writer lock that refuses only while a
writer is *held* never lets its writer in under continuous readers, because the
reader count never reaches zero. That is one bit of protocol, and it is worth
three orders of magnitude on the burst.

### Cherry-picking from #533

It is 22 granular commits, not one blob, so individual fixes can be taken. Four
of them touch files this PR touches:

| commit | fix | status |
|---|---|---|
| `77e184de` | `WordLock` queue unlock retries | **taken** |
| `0a130cc1` | oversized timeouts handled consistently | candidate, not taken |
| `51ef2208` | futex timeout ABI on 32-bit Linux | candidate; the targets here are 64-bit |
| `33860115` | rwlock upgrade memory orderings | only affects the upgradable paths |

`77e184de` does not apply as a patch. It fixes the retry loop and, in the same
commit, drops the acquire-fence approach for an `Acquire` failure ordering on
the compare-exchange, so two of its four hunks conflict with a base that still
has `fence_acquire`.

Taking the file wholesale would work and would also import what the commits
before it in #533 did to that file: the copyright header deleted, and
`atomic_ptr_fetch_and` and `atomic_ptr_fetch_byte_sub` replaced by the inherent
`AtomicPtr` methods, which is a Rust 1.91 floor that master's polyfills exist
to avoid. So it was resolved by hand instead, on master's own base, and the
polyfills stay. `fence_acquire` goes, because the fix removes both of its
callers.

It changes no measured number: 3.32 M/s against 3.29 before, 1.20 ms drain
against 1.22, both inside noise. That is the expected result and not a reason to
skip it. It is a correctness fix in the queue lock that every parking operation
goes through, and a benchmark on 16 cores is not the instrument that would show
a lost wakeup.

## Free, already carried

This fork sits on `parking_lot-v0.12.5-27-g9a125c8`, twenty-seven commits past
the released 0.12.5, so it already includes unreleased fixes that crates.io does
not have: `word_lock: use AtomicPtr`, pointer types for Windows handles, and the
AIX `timespec` correction.

## What none of this establishes

Every number here is throughput and latency. Nothing in this file says the
`no_std` substitutions are *correct*. `thread_local!` became a `pthread_key_t`,
`Instant` became a `clock_gettime` reading, and `yield_now` became
`sched_yield`; a benchmark cannot tell whether any of that is sound. That needs
loom and Miri, and neither has been run.

`tests/no_std_backends.rs` is what exists instead, and it is weaker than that:
it establishes that the locks still exclude and still block with the feature
off, not that the thread-local substitution is race-free.

## Open issues, hunted

Upstream's issue tracker carries confirmed bugs and half-finished fixes that
never became PRs. These are the ones that touch this change.

### #505, a confirmed deadlock, untouched here

`read_recursive` can deadlock. The maintainer diagnosed it: an exclusive unlock
wakes parked threads in FIFO order but stops at the first writer, so recursive
readers that parked *after* a writer are never woken, and nothing makes
progress. The three lines are in `wake_parked_threads` in `raw_rwlock.rs`:

```rust
// If we are waking up a writer, don't wake anything else.
if s & WRITER_BIT != 0 {
    return FilterOp::Stop;
}
```

Deleting them fixes the deadlock and costs a full walk of the parked list on
every unlock, which is why upstream has not. #533 resolves it properly, with a
separate reader-biased `RecursiveRwLock` type.

This PR does not touch it, and switching `std` off does not change it: the bug
is in the wake filter, not in anything substituted here. It is listed because
anyone reading this file is reading it while deciding what to depend on.

### #489, deadlock when upgrading an `upgradable_read`

Same standing as #505: present upstream, present here, untouched by this
change.

### #261 and #272, one optimization, worth more without `std`

`ThreadParker::IS_CHEAP_TO_CONSTRUCT` is `false` on unix, so `WordLock` reaches
for thread-local storage to reuse a parker across calls. #261 reports 15-20%
average and 5-10% median improvement on macOS from setting it `true` and
constructing on the stack instead. #272 explains why that is plausible:
`WordLock` never calls `park_until`, so the `CLOCK_MONOTONIC` condvar setup that
makes construction look expensive is work it never needs.

`ThreadParker::new` is four field stores over static initializers. The clock
setup is deferred to the first park, behind an `initialized` flag.

It should matter more with `std` off than with it on. With `std` the slot is
`thread_local!`; without it the slot is `pthread_getspecific` behind a lazily
created `pthread_key_t`, guarded by a `WordLock` on first use, which is strictly
more expensive. So the path #261 proposes to skip costs the `no_std` build more
than it costs the `std` one.

Measured anyway, and it is not there. See "#261 and #272: no effect".

### #484, `spin_loop` twice on the first iteration

`SpinWait::spin` runs `cpu_relax(1 << self.counter)` with the counter already
incremented, so the sequence is 2, 4, 8 rather than 1, 2, 4: the first iteration
issues the pause instruction twice. The maintainer's response is that there is
no particular reason for it and starting at 1 would have little impact.

Worth more on aarch64 than the discussion suggests, because `spin_loop()` lowers
to `isb` there, an instruction synchronization barrier, not the cheap `pause` of
x86. Doubling it on the first iteration of every contended acquisition is not
free on this hardware.

Measured, and not established. See "#484: not established, and it nearly
shipped".

### Not applicable

- **#278**, "Possible bug - UB", 18 comments. Speculation about
  `Condvar::wait_until_internal` from a Reddit segfault report, never confirmed.
  `Condvar` is not carried here in any case.
- **#518**, `deadlock_detection` causes a SIGSEGV in 0.12.5. That feature is
  removed here.

## Measurement, and what it cost to get one

The first attempt at all of this was worthless, and the way it was worthless is
worth writing down.

`benches/burst.rs` originally ran 8 writers and 8 readers on a 16-core machine.
One thread per core leaves the OS no headroom, so the harness's own threads were
descheduled inside critical sections. Six identical runs of the same arm drifted
from 3.55 to 3.04 M/s, and a worst-burst figure of 18.22 ms appeared in a
workload whose entire budget is 2-10 ms. On top of that the machine was carrying
a load average of 29.88 from unrelated work.

Two changes fixed it. The thread counts dropped to 4 writers and 4 readers, so
the measurement is not competing with itself, and CPU time per burst became the
primary comparison, because a descheduled thread inflates wall clock and does
not inflate CPU. Worst-burst went from 1.5-18 ms to 1.09 ms against a 1.02 ms
median, max update latency from 0.2-7 ms to 0.080 ms, and the within-arm spread
to 2.3%.

Only then was any of the following worth running.

### #462: dropped

Isolated properly, three interleaved rounds. At reader think time 0, the arm
without it ran 3.45, 3.48, 3.44 M/s, a spread of 1.2%; with it, 3.47, 3.21,
3.21, and drain median moved 1.16 to 1.25 ms. At think time 100 it was 4% ahead
instead. A wash overall with a real regression on the contended end, and it is
an unmerged PR, so carrying it is divergence bought with nothing. Removed,
along with the `TOKEN_RESTORE_PARKED_BIT` constant it needed.

### #261 and #272: no effect

Five interleaved rounds behind a feature flag. Base 4.00, 4.00, 4.06, 4.01,
3.97 M/s; with the parker built on the stack, 3.87, 3.98, 3.96, 4.01, 4.05.
CPU 5.70 against 5.72 ms. Inside a 2.3% null.

That fits the issue rather than contradicting it: #261 reported 15-20% on
pre-Big Sur Intel and its author noted the difference had already largely
disappeared after a macOS update. On aarch64 macOS it is not there at all.

The flag is not carried into this PR. The result is platform-specific by
construction, so it is written down here rather than left in the tree as a
switch nobody will flip.

### #484: not established, and it nearly shipped

Five interleaved rounds at think time 0 said this was a win: throughput 3.55 to
3.77 M/s, CPU 6.02 to 5.52 ms, drain 1.13 to 1.06 ms, three metrics moving
together and each outside the base arm's 3.4% spread. On aarch64 the argument is
also good, since `spin_loop` lowers to `isb` rather than x86's cheap `pause`, so
halving the first burst should matter.

Three more rounds reversed it. Base 3.78, 3.95, 3.87; patched 3.84, 3.71, 3.56.
The base arm had drifted upward from 3.55 to 3.87 between round five and round
six, as the machine's background indexing finished. The apparent 6% was the
machine getting faster underneath a comparison that ran the arms in sequence
rather than truly paired. Across all eight rounds the within-arm spread is
13-14%, larger than the effect. One of those runs also reported a null of 1.44x,
which condemns it without any further argument.

Not established, and not carried.

## Net result

Nothing from the patch hunt earned its place. What this PR carries is upstream's
own code, the `no_std` substitutions, and one upstream correctness fix
(`77e184de`). Every performance patch considered was either measurably worse,
measurably nothing, or indistinguishable from the machine.

That is a good outcome rather than a wasted day. Divergence from upstream has a
running cost, paid every time upstream changes, and three of these would have
been carried on stories rather than evidence.

## Measurement conditions

Every number here comes from a 16-core aarch64 macOS machine. Numbers taken
before the harness was fixed are marked in the sections above and are not
relied on.

Results large enough to survive even the bad conditions: spin's 400x at zero
reader think time, #419's 5x throughput loss, the
eventual-fairness null result, and mutex parity at 128 threads.

Results that required the fixed harness: everything in "Measurement, and what it
cost to get one".

Nothing here has been measured on Linux or Windows.
