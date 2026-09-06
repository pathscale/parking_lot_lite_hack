use crate::time::Instant;
use cfg_if::cfg_if;

/// Trait for the platform thread parker implementation.
///
/// All unsafe methods are unsafe because the Unix thread parker is based on
/// pthread mutexes and condvars. Those primitives must not be moved and used
/// from any other memory address than the one they were located at when they
/// were initialized. As such, it's UB to call any unsafe method on
/// `ThreadParkerT` if the implementing instance has moved since the last
/// call to any of the unsafe methods.
pub trait ThreadParkerT {
    type UnparkHandle: UnparkHandleT;

    const IS_CHEAP_TO_CONSTRUCT: bool;

    fn new() -> Self;

    /// Prepares the parker. This should be called before adding it to the queue.
    unsafe fn prepare_park(&self);

    /// Checks if the park timed out. This should be called while holding the
    /// queue lock after `park_until` has returned false.
    unsafe fn timed_out(&self) -> bool;

    /// Parks the thread until it is unparked. This should be called after it has
    /// been added to the queue, after unlocking the queue.
    unsafe fn park(&self);

    /// Parks the thread until it is unparked or the timeout is reached. This
    /// should be called after it has been added to the queue, after unlocking
    /// the queue. Returns true if we were unparked and false if we timed out.
    unsafe fn park_until(&self, timeout: Instant) -> bool;

    /// Locks the parker to prevent the target thread from exiting. This is
    /// necessary to ensure that thread-local `ThreadData` objects remain valid.
    /// This should be called while holding the queue lock.
    unsafe fn unpark_lock(&self) -> Self::UnparkHandle;
}

/// Handle for a thread that is about to be unparked. We need to mark the thread
/// as unparked while holding the queue lock, but we delay the actual unparking
/// until after the queue lock is released.
pub trait UnparkHandleT {
    /// Wakes up the parked thread. This should be called after the queue lock is
    /// released to avoid blocking the queue for too long.
    ///
    /// This method is unsafe for the same reason as the unsafe methods in
    /// `ThreadParkerT`.
    unsafe fn unpark(self);
}

cfg_if! {
    if #[cfg(any(target_os = "linux", target_os = "android"))] {
        #[path = "linux.rs"]
        mod imp;
    } else if #[cfg(unix)] {
        #[path = "unix.rs"]
        mod imp;
    } else if #[cfg(windows)] {
        #[path = "windows/mod.rs"]
        mod imp;
    } else {
        // Upstream falls through to wasm, SGX, Redox and a generic
        // `std::thread::park` backend here. All four are gone: they are the
        // backends that reach for `std`, and none of them is a target this
        // fork builds for.
        compile_error!(
            "parking_lot_core supports linux, unix and Windows. The wasm, SGX, \
             Redox and generic thread parkers are not carried in this fork."
        );
    }
}

pub use self::imp::{thread_yield, ThreadParker};
