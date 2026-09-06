# parking_lot_lite_hack

`parking_lot`'s `Mutex` and `RwLock`, with `std` patched out and everything else
removed.

This is a fork of [parking_lot](https://github.com/Amanieu/parking_lot), not a
reimplementation. `src/raw_mutex.rs`, `src/raw_rwlock.rs` and everything under
`core/` are upstream's own files.

## Using it

The package is named `parking_lot_lite_hack` so that it cannot collide with the
real `parking_lot` in a dependency graph, and so that taking it is a decision
rather than an accident. Rename it back on the way in:

```toml
[dependencies]
parking_lot = { package = "parking_lot_lite_hack", version = "0.12", default-features = false, features = ["arc_lock", "send_guard"] }
```

Every `use parking_lot::...` in the consumer then keeps working, and a graph
that also contains the real `parking_lot` is fine: they are different packages.

## What `no_std` means here

The crate does not link `std`. It does not mean it cannot make a syscall:
`libc` with default features off is itself `no_std`, so a `no_std` crate on a
hosted target can still block in the kernel. That is the whole premise, and it
is why this matches upstream's numbers rather than losing to them the way a
spinlock does. See [PR-reviewed.md](PR-reviewed.md) for the measurement.

Four things change with `std` off, and nothing else does:

1. `thread_local!` becomes a `pthread_key_t` on unix and an `FlsAlloc` slot on
   Windows. That is what `thread_local!` lowers to for a type with a
   destructor, and `Fls` rather than `Tls` because only `Fls` runs one.
2. `std::time::Instant` becomes `clock_gettime(CLOCK_MONOTONIC)` on unix and
   `QueryPerformanceCounter` on Windows, the same sources `std` uses.
3. `std::thread::yield_now` becomes the `sched_yield` it calls.
4. Nothing else. The locks themselves are upstream's, unedited.

## Features

| feature | default | what it does |
|---|---|---|
| `std` | on | Link `std`. Off, the four substitutions above apply. |
| `arc_lock` | off | `lock_api`'s owned `Arc` guards, which is what a lookup returning a reference into a still-locked node needs. |
| `send_guard` | off | Guards become `Send`. |

That is the whole list. Upstream's `deadlock_detection`, `serde`, `owning_ref`,
`nightly` and `hardware-lock-elision` are gone, along with the code they gated.

## What was removed, and why

| removed | why |
|---|---|
| `Condvar`, `Once`, `FairMutex`, `ReentrantMutex` | not used by the consumer this exists for |
| deadlock detection | the only thing in `parking_lot_core` that needed `HashSet`, `mpsc` and `ThreadId`, which is to say the only thing that needed `std` |
| hardware lock elision | x86 only, off by default upstream, and `have_elision()` folds to `false` here |
| the wasm, SGX, Redox and generic thread parkers | the four backends that reach for `std`; a target that selects one now gets a `compile_error!` naming the restriction |
| `serde`, `owning_ref`, `nightly` | unused |

`src/elision.rs` stays as an inert `have_elision() -> false` rather than having
its call sites deleted from `raw_rwlock.rs`. The lock is the one file worth
keeping close to upstream.

## Platforms

unix, linux and Windows. Anything else fails to compile with a message saying
so, rather than at a missing `slot::create` forty lines into a thread-local.

## Testing

Upstream's own unit tests run against the `no_std` code paths, not only against
the default build: the crate is `no_std` on `all(not(feature = "std"),
not(test))`, so the harness keeps its own prelude while the library under test
is the patched one. `cargo check --no-default-features` is what proves the
library does not link `std`, since a test binary cannot.

`tests/no_std_backends.rs` adds what a type check cannot see: that a waiter
*blocks*. A lock whose waiters spin passes every correctness test and then
burns tens of times the CPU under contention.

## Benchmarks

`benches/vs_parking_lot.rs` and `benches/burst.rs` dev-depend on the published
`parking_lot` from crates.io, so clean upstream and this build link into one
process and their arms interleave. Every table carries a `null` column, which is
one arm measured twice, so noise is visible rather than assumed.

```
cargo bench --no-default-features
```

They are never built or run in CI: they measure CPU with `getrusage`, so they
are unix-only, and a shared runner cannot produce a lock benchmark worth
reading.

## Upstream

Forked at `parking_lot-v0.12.5-27-g9a125c8`, twenty-seven commits past the
released 0.12.5. One further commit is cherry-picked: `77e184de`, "Fix WordLock
queue unlock retries", from the open upstream PR #533.

## Licence

MIT or Apache-2.0, at your option, the same as upstream.
