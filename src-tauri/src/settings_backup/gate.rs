//! Ordinary operations share leases; snapshots/restores take exclusive maintenance.
//! Ordinary entry never waits while a runtime-store mutex could be held.
use parking_lot::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::{
    cell::Cell,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::Duration,
};

static GATE: RwLock<()> = RwLock::new(());
static BLOCKED: AtomicBool = AtomicBool::new(false);
static REVISION: AtomicU64 = AtomicU64::new(0);
thread_local! { static MAINTENANCE_DEPTH: Cell<u32> = const { Cell::new(0) }; }

pub enum SettingsGuard {
    Shared {
        _guard: RwLockReadGuard<'static, ()>,
    },
    MaintenanceOwner,
}
pub struct MaintenanceGuard {
    _guard: Option<RwLockWriteGuard<'static, ()>>,
}
impl Drop for MaintenanceGuard {
    fn drop(&mut self) {
        MAINTENANCE_DEPTH.with(|depth| depth.set(depth.get() - 1));
    }
}
fn check_recovery() -> Result<(), String> {
    if BLOCKED.load(Ordering::SeqCst) {
        return Err("Settings recovery is pending. Unlock the OS keyring and restart to recover the journal.".into());
    }
    Ok(())
}
pub fn enter() -> Result<SettingsGuard, String> {
    check_recovery()?;
    if MAINTENANCE_DEPTH.with(|depth| depth.get() != 0) {
        return Ok(SettingsGuard::MaintenanceOwner);
    }
    let guard = GATE
        .try_read_recursive()
        .ok_or("Settings maintenance is busy; retry when backup/import finishes")?;
    check_recovery()?;
    Ok(SettingsGuard::Shared { _guard: guard })
}
/// Called only by backup workers before taking runtime-store locks.
pub fn maintain() -> Result<MaintenanceGuard, String> {
    check_recovery()?;
    if MAINTENANCE_DEPTH.with(|depth| depth.get() != 0) {
        MAINTENANCE_DEPTH.with(|depth| depth.set(depth.get() + 1));
        return Ok(MaintenanceGuard { _guard: None });
    }
    let guard = GATE
        .try_write_for(Duration::from_secs(5))
        .ok_or("Settings are in use; retry backup/import after the current operation finishes")?;
    check_recovery()?;
    MAINTENANCE_DEPTH.with(|depth| depth.set(1));
    Ok(MaintenanceGuard {
        _guard: Some(guard),
    })
}
pub fn changed() {
    REVISION.fetch_add(1, Ordering::SeqCst);
}
pub fn revision() -> u64 {
    REVISION.load(Ordering::SeqCst)
}
pub fn block() {
    BLOCKED.store(true, Ordering::SeqCst);
}

static CC_STARTED: AtomicBool = AtomicBool::new(false);
pub fn mark_cc_started() {
    CC_STARTED.store(true, Ordering::SeqCst);
}
pub fn cc_started() -> bool {
    CC_STARTED.load(Ordering::SeqCst)
}

static ASYNC_CHANGES: AtomicU64 = AtomicU64::new(0);
pub struct AsyncSettingsChange;
pub fn begin_async_change() -> Result<AsyncSettingsChange, String> {
    let _guard = enter()?;
    ASYNC_CHANGES.fetch_add(1, Ordering::SeqCst);
    Ok(AsyncSettingsChange)
}
pub fn has_async_changes() -> bool {
    ASYNC_CHANGES.load(Ordering::SeqCst) != 0
}
impl Drop for AsyncSettingsChange {
    fn drop(&mut self) {
        ASYNC_CHANGES.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Only call before any runtime-store locks. Recheck the retry generation while the
/// lease is held, and keep the lease through process publication.
pub fn wait_for_maintenance() -> Result<SettingsGuard, String> {
    let guard = GATE.read_recursive();
    check_recovery()?;
    Ok(SettingsGuard::Shared { _guard: guard })
}
