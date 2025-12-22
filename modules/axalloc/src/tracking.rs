// Simplified tracking module for demonstration purposes
use core::sync::atomic::{AtomicBool, Ordering};

pub(crate) static TRACKING_ENABLED: AtomicBool = AtomicBool::new(false);
pub(crate) static IN_GLOBAL_ALLOCATOR: bool = false;

/// Enables allocation tracking.
pub fn enable_tracking() {
    TRACKING_ENABLED.store(true, Ordering::SeqCst);
}

/// Disables allocation tracking.
pub fn disable_tracking() {
    TRACKING_ENABLED.store(false, Ordering::SeqCst);
}

/// Returns whether allocation tracking is enabled.
pub fn tracking_enabled() -> bool {
    TRACKING_ENABLED.load(Ordering::SeqCst)
}

pub(crate) fn with_state<R>(_f: impl FnOnce(Option<&mut ()>) -> R) -> R {
    // Simplified implementation
    _f(None)
}

/// Returns current generation of global allocator.
pub fn current_generation() -> u64 {
    0 // Simplified implementation
}

/// Visits all allocations made by global allocator within given generation range.
pub fn allocations_in(_range: core::ops::Range<u64>, _visitor: impl FnMut(&())) {
    // Simplified implementation - no-op
}
