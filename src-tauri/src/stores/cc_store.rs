use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

#[derive(Default)]
pub struct CcRuntimeState {
    connected: AtomicBool,
    generation: AtomicU64,
}

impl CcRuntimeState {
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    pub fn set_connected(&self, connected: bool) {
        self.connected.store(connected, Ordering::Relaxed);
    }

    pub fn begin_generation(&self) -> u64 {
        self.connected.store(false, Ordering::Relaxed);
        self.generation.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub fn is_generation_current(&self, generation: u64) -> bool {
        self.generation.load(Ordering::Relaxed) == generation
    }
}
