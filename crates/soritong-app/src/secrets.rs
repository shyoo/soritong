//! API keys and other secrets. The app uses the Windows Credential Manager (service `soritong`);
//! tests use [`MemoryStore`] so they never touch the real one.
//!
//! Values are never logged and never returned to the frontend: the UI only learns whether one
//! is set.

use std::collections::HashMap;
use std::sync::Mutex;

pub const SERVICE: &str = "soritong";

pub trait SecretStore: Send + Sync {
    fn get(&self, account: &str) -> Result<Option<String>, String>;
    fn set(&self, account: &str, value: &str) -> Result<(), String>;
    /// Deleting a secret that does not exist is not an error.
    fn delete(&self, account: &str) -> Result<(), String>;
}

/// The credential account for an engine's secret field: the engine id for its `api_key` (the
/// common case, and what a user sees in Credential Manager), `engine/field` for anything else.
pub fn account(engine_id: &str, field: &str) -> String {
    if field == "api_key" {
        engine_id.to_string()
    } else {
        format!("{engine_id}/{field}")
    }
}

/// OS keychain via the `keyring` crate (Credential Manager on Windows).
pub struct KeyringStore;

impl KeyringStore {
    fn entry(account: &str) -> Result<keyring::Entry, String> {
        keyring::Entry::new(SERVICE, account).map_err(|e| e.to_string())
    }
}

impl SecretStore for KeyringStore {
    fn get(&self, account: &str) -> Result<Option<String>, String> {
        match Self::entry(account)?.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }
    fn set(&self, account: &str, value: &str) -> Result<(), String> {
        Self::entry(account)?
            .set_password(value)
            .map_err(|e| e.to_string())
    }
    fn delete(&self, account: &str) -> Result<(), String> {
        match Self::entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

#[derive(Default)]
pub struct MemoryStore(Mutex<HashMap<String, String>>);

impl SecretStore for MemoryStore {
    fn get(&self, account: &str) -> Result<Option<String>, String> {
        Ok(self.0.lock().unwrap().get(account).cloned())
    }
    fn set(&self, account: &str, value: &str) -> Result<(), String> {
        self.0
            .lock()
            .unwrap()
            .insert(account.to_string(), value.to_string());
        Ok(())
    }
    fn delete(&self, account: &str) -> Result<(), String> {
        self.0.lock().unwrap().remove(account);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_key_uses_the_bare_engine_id() {
        assert_eq!(account("elevenlabs", "api_key"), "elevenlabs");
        assert_eq!(account("openai", "org_token"), "openai/org_token");
    }

    #[test]
    fn memory_store_round_trips_and_deletes_idempotently() {
        let s = MemoryStore::default();
        assert_eq!(s.get("x").unwrap(), None);
        s.set("x", "k").unwrap();
        assert_eq!(s.get("x").unwrap().as_deref(), Some("k"));
        s.delete("x").unwrap();
        s.delete("x").unwrap();
        assert_eq!(s.get("x").unwrap(), None);
    }
}
