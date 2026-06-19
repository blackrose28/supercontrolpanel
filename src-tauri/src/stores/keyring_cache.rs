//! In-memory cache for OS keyring values.
//!
//! On Linux the OS credential store (Secret Service / GNOME Keyring) can
//! trigger an "unlock keyring" popup every time the app accesses a secret.
//! This module loads all known secrets once at startup and serves all
//! subsequent reads from memory, while writes go to both the OS keyring and
//! the cache so the two stay in sync.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// Global in-memory cache of keyring values.
/// Initialised once during app startup by `initialize_keyring_cache`.
static KEYRING_CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

/// Return a reference to the global cache, creating an empty one if it has not
/// been initialised yet (defensive fallback — normal path goes through
/// `initialize_keyring_cache`).
pub fn keyring_cache() -> &'static Mutex<HashMap<String, String>> {
    KEYRING_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Retrieve a cached keyring value.  Returns `None` when the account has
/// never been stored or was deleted.
pub fn keyring_cache_get(account: &str) -> Option<String> {
    let guard = keyring_cache().lock().expect("keyring cache lock poisoned");
    guard.get(account).cloned()
}

/// Store or delete a cached keyring value.
/// Empty string / `None` semantics: an empty value removes the entry (mirrors
/// the `keyring_set_or_delete` contract).
pub fn keyring_cache_set(account: &str, value: &str) {
    let mut guard = keyring_cache().lock().expect("keyring cache lock poisoned");
    if value.is_empty() {
        guard.remove(account);
    } else {
        guard.insert(account.to_string(), value.to_string());
    }
}

/// Seed the global cache with a pre-built map (called once at startup).
/// If the cache has already been initialised this is a no-op — the first
/// caller wins.
pub fn seed_keyring_cache(entries: HashMap<String, String>) {
    // Try to set the OnceLock; if it was already set (e.g. by a test or a
    // concurrent startup path) we merge into the existing map instead.
    match KEYRING_CACHE.set(Mutex::new(entries.clone())) {
        Ok(()) => {} // first init, done
        Err(_already_set) => {
            // Cache already exists — merge the new entries in.
            let guard = KEYRING_CACHE.get().expect("just checked it exists");
            let mut map = guard.lock().expect("keyring cache lock poisoned");
            for (k, v) in entries {
                map.insert(k, v);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Since OnceLock is global/static, tests share the same cache.
    /// We use unique account names per test to avoid interference.

    #[test]
    fn keyring_cache_get_returns_none_for_missing() {
        // Ensure cache is initialised (may already be by another test).
        let _ = keyring_cache();
        assert!(keyring_cache_get("__test_nonexistent_account__").is_none());
    }

    #[test]
    fn keyring_cache_set_and_get_round_trips() {
        keyring_cache_set("__test_roundtrip__", "secret123");
        assert_eq!(
            keyring_cache_get("__test_roundtrip__"),
            Some("secret123".to_string())
        );
    }

    #[test]
    fn keyring_cache_set_empty_removes_entry() {
        keyring_cache_set("__test_delete__", "to_be_deleted");
        assert!(keyring_cache_get("__test_delete__").is_some());

        keyring_cache_set("__test_delete__", "");
        assert!(keyring_cache_get("__test_delete__").is_none());
    }

    #[test]
    fn keyring_cache_set_overwrites_existing() {
        keyring_cache_set("__test_overwrite__", "old_value");
        keyring_cache_set("__test_overwrite__", "new_value");
        assert_eq!(
            keyring_cache_get("__test_overwrite__"),
            Some("new_value".to_string())
        );
    }

    #[test]
    fn seed_keyring_cache_populates_entries() {
        let mut map = HashMap::new();
        map.insert("__test_seed_a__".to_string(), "val_a".to_string());
        map.insert("__test_seed_b__".to_string(), "val_b".to_string());
        seed_keyring_cache(map);

        assert_eq!(
            keyring_cache_get("__test_seed_a__"),
            Some("val_a".to_string())
        );
        assert_eq!(
            keyring_cache_get("__test_seed_b__"),
            Some("val_b".to_string())
        );
    }
}
