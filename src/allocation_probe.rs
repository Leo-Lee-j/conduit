//! Opt-in test instrumentation. Counts requested Rust allocations, not heap commit,
//! allocator metadata, GPU memory, or allocations made directly by native code.
use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicUsize, Ordering::Relaxed},
};

#[derive(Clone, Copy, Debug, serde::Serialize)]
pub(crate) struct AllocationSample {
    pub live_bytes: usize,
    pub live_blocks: usize,
}

struct TrackingAllocator {
    bytes: AtomicUsize,
    blocks: AtomicUsize,
}

impl TrackingAllocator {
    const fn new() -> Self {
        Self {
            bytes: AtomicUsize::new(0),
            blocks: AtomicUsize::new(0),
        }
    }
    fn sample(&self) -> AllocationSample {
        // Process-wide counters: concurrent activity may occur between the loads.
        AllocationSample {
            live_bytes: self.bytes.load(Relaxed),
            live_blocks: self.blocks.load(Relaxed),
        }
    }
    fn allocated(&self, pointer: *mut u8, size: usize) {
        if !pointer.is_null() {
            self.bytes.fetch_add(size, Relaxed);
            self.blocks.fetch_add(1, Relaxed);
        }
    }
}

// SAFETY: all operations delegate unchanged pointers/layouts to System. Counter
// updates cannot allocate, panic, or change the allocator's ownership contract.
unsafe impl GlobalAlloc for TrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        self.allocated(pointer, layout.size());
        pointer
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        self.allocated(pointer, layout.size());
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
        self.bytes.fetch_sub(layout.size(), Relaxed);
        self.blocks.fetch_sub(1, Relaxed);
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let replacement = unsafe { System.realloc(pointer, layout, new_size) };
        if !replacement.is_null() {
            if new_size >= layout.size() {
                self.bytes.fetch_add(new_size - layout.size(), Relaxed);
            } else {
                self.bytes.fetch_sub(layout.size() - new_size, Relaxed);
            }
        }
        replacement
    }
}

#[global_allocator]
static ALLOCATOR: TrackingAllocator = TrackingAllocator::new();

#[cfg(conduit_shell_desktop)]
pub(crate) fn sample() -> AllocationSample {
    ALLOCATOR.sample()
}

#[test]
fn counts_follow_zeroed_growth_shrink_and_free() {
    let _locale = crate::test_support::chinese();
    // Independent counters avoid interference from the test harness allocator.
    let allocator = TrackingAllocator::new();
    unsafe {
        let layout = Layout::from_size_align(64, 32).unwrap();
        let pointer = allocator.alloc_zeroed(layout);
        assert!(!pointer.is_null());
        assert_eq!(allocator.sample().live_bytes, 64);
        assert_eq!(allocator.sample().live_blocks, 1);
        assert!((0..64).all(|i| *pointer.add(i) == 0));
        *pointer = 73;
        let pointer = allocator.realloc(pointer, layout, 4096);
        assert!(!pointer.is_null());
        assert_eq!(*pointer, 73);
        assert_eq!(allocator.sample().live_bytes, 4096);
        assert_eq!(allocator.sample().live_blocks, 1);
        let pointer = allocator.realloc(pointer, Layout::from_size_align(4096, 32).unwrap(), 32);
        assert!(!pointer.is_null());
        assert_eq!(*pointer, 73);
        assert_eq!(allocator.sample().live_bytes, 32);
        allocator.dealloc(pointer, Layout::from_size_align(32, 32).unwrap());
        assert_eq!(allocator.sample().live_bytes, 0);
        assert_eq!(allocator.sample().live_blocks, 0);
        let pointer = allocator.alloc(layout);
        assert!(!pointer.is_null());
        assert_eq!(allocator.sample().live_bytes, 64);
        allocator.dealloc(pointer, layout);
        assert_eq!(allocator.sample().live_bytes, 0);
        assert_eq!(allocator.sample().live_blocks, 0);
    }
}
