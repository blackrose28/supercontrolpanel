//! Instance-scoped settings maintenance gate for standalone runtimes.
use crate::error::CoreError;
use parking_lot::{Mutex, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::{
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    thread::ThreadId,
    time::Duration,
};

pub struct MaintenanceGate {
    lock: RwLock<()>,
    blocked: AtomicBool,
    blocked_reason: Mutex<Option<String>>,
    revision: AtomicU64,
    async_changes: AtomicU64,
    owner: Mutex<Option<(ThreadId, u32)>>,
}

impl Default for MaintenanceGate {
    fn default() -> Self {
        Self::new()
    }
}

pub enum SettingsGuard<'a> {
    Shared { _guard: RwLockReadGuard<'a, ()> },
    MaintenanceOwner,
}

impl std::fmt::Debug for SettingsGuard<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Shared { .. } => f.write_str("SettingsGuard::Shared"),
            Self::MaintenanceOwner => f.write_str("SettingsGuard::MaintenanceOwner"),
        }
    }
}

pub struct MaintenanceGuard<'a> {
    gate: &'a MaintenanceGate,
    _guard: Option<RwLockWriteGuard<'a, ()>>,
}

impl std::fmt::Debug for MaintenanceGuard<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MaintenanceGuard")
    }
}

impl Drop for MaintenanceGuard<'_> {
    fn drop(&mut self) {
        let mut owner = self.gate.owner.lock();
        if let Some((tid, depth)) = *owner {
            if depth <= 1 {
                *owner = None;
            } else {
                *owner = Some((tid, depth - 1));
            }
        }
    }
}

pub struct AsyncChangeGuard<'a> {
    gate: &'a MaintenanceGate,
}

impl Drop for AsyncChangeGuard<'_> {
    fn drop(&mut self) {
        self.gate.async_changes.fetch_sub(1, Ordering::SeqCst);
    }
}

impl MaintenanceGate {
    pub fn new() -> Self {
        Self {
            lock: RwLock::new(()),
            blocked: AtomicBool::new(false),
            blocked_reason: Mutex::new(None),
            revision: AtomicU64::new(0),
            async_changes: AtomicU64::new(0),
            owner: Mutex::new(None),
        }
    }

    fn check_recovery(&self) -> Result<(), CoreError> {
        if self.blocked.load(Ordering::SeqCst) {
            let reason = self
                .blocked_reason
                .lock()
                .clone()
                .unwrap_or_else(|| {
                    "Settings recovery is pending. Unlock the OS keyring and restart to recover the journal."
                        .to_string()
                });
            return Err(CoreError::conflict(
                "recovery_blocked",
                reason,
                false,
                "unlock_keyring_and_retry",
            ));
        }
        Ok(())
    }

    fn is_current_owner(&self) -> bool {
        let current = std::thread::current().id();
        self.owner
            .lock()
            .is_some_and(|(tid, depth)| tid == current && depth > 0)
    }

    pub fn enter(&self) -> Result<SettingsGuard<'_>, CoreError> {
        self.check_recovery()?;
        if self.is_current_owner() {
            return Ok(SettingsGuard::MaintenanceOwner);
        }
        let guard = self.lock.try_read_recursive().ok_or_else(|| {
            CoreError::conflict(
                "maintenance_busy",
                "Settings maintenance is busy; retry when backup/import finishes",
                true,
                "retry_later",
            )
        })?;
        self.check_recovery()?;
        Ok(SettingsGuard::Shared { _guard: guard })
    }

    pub fn maintain(&self) -> Result<MaintenanceGuard<'_>, CoreError> {
        self.check_recovery()?;
        let current = std::thread::current().id();
        {
            let mut owner = self.owner.lock();
            if let Some((tid, depth)) = *owner {
                if tid == current && depth > 0 {
                    *owner = Some((tid, depth + 1));
                    return Ok(MaintenanceGuard {
                        gate: self,
                        _guard: None,
                    });
                }
            }
        }
        let guard = self
            .lock
            .try_write_for(Duration::from_secs(5))
            .ok_or_else(|| {
                CoreError::conflict(
                    "maintenance_busy",
                    "Settings are in use; retry backup/import after the current operation finishes",
                    true,
                    "retry_later",
                )
            })?;
        self.check_recovery()?;
        *self.owner.lock() = Some((current, 1));
        Ok(MaintenanceGuard {
            gate: self,
            _guard: Some(guard),
        })
    }

    pub fn wait_for_maintenance(&self) -> Result<SettingsGuard<'_>, CoreError> {
        let guard = self.lock.read_recursive();
        self.check_recovery()?;
        Ok(SettingsGuard::Shared { _guard: guard })
    }

    pub fn changed(&self) {
        self.revision.fetch_add(1, Ordering::SeqCst);
    }

    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::SeqCst)
    }

    pub fn block(&self, reason: impl Into<String>) {
        *self.blocked_reason.lock() = Some(reason.into());
        self.blocked.store(true, Ordering::SeqCst);
    }

    pub fn is_blocked(&self) -> Option<String> {
        if self.blocked.load(Ordering::SeqCst) {
            self.blocked_reason.lock().clone()
        } else {
            None
        }
    }

    pub fn begin_async_change(&self) -> Result<AsyncChangeGuard<'_>, CoreError> {
        let _guard = self.enter()?;
        self.async_changes.fetch_add(1, Ordering::SeqCst);
        Ok(AsyncChangeGuard { gate: self })
    }

    pub fn has_async_changes(&self) -> bool {
        self.async_changes.load(Ordering::SeqCst) != 0
    }
}
