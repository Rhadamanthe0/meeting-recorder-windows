//! Check the patched dependency through its public API on stable Rust.
//! Its upstream unit-test crate enables nightly benchmarks and is not part
//! of this workspace; this check runs in the ordinary project/CI test suite.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicIsize, Ordering};
use whisper_rs::{FullParams, SamplingStrategy};

thread_local! {
    static TRACK: Cell<bool> = const { Cell::new(false) };
}
static LIVE_BYTES: AtomicIsize = AtomicIsize::new(0);

struct TrackedAllocator;

fn tracking() -> bool {
    TRACK.try_with(Cell::get).unwrap_or(false)
}

// SAFETY: allocation and deallocation are forwarded unchanged to System.
// The additional counters neither allocate nor alter any returned pointer.
unsafe impl GlobalAlloc for TrackedAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forward the caller's valid layout to the system allocator.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() && tracking() {
            LIVE_BYTES.fetch_add(layout.size() as isize, Ordering::Relaxed);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        if tracking() {
            LIVE_BYTES.fetch_sub(layout.size() as isize, Ordering::Relaxed);
        }
        // SAFETY: forward the original allocation pointer and layout.
        unsafe { System.dealloc(pointer, layout) };
    }
}

#[global_allocator]
static ALLOCATOR: TrackedAllocator = TrackedAllocator;

#[test]
fn language_storage_is_retained_by_clones_and_freed_after_replacement() {
    TRACK.with(|track| track.set(true));
    let retained;
    {
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        for _ in 0..100 {
            params.set_language(Some("en"));
            params.set_language(Some("fr"));
        }
        let cloned = params.clone();
        params.set_language(None);
        retained = LIVE_BYTES.load(Ordering::Relaxed);
        drop(cloned);
    }
    let remaining = LIVE_BYTES.load(Ordering::Relaxed);
    TRACK.with(|track| track.set(false));
    assert!(retained > 0, "a clone must retain its language storage");
    assert_eq!(remaining, 0, "language strings leaked after params dropped");
}
