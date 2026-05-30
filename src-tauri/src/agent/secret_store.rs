use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[cfg(not(test))]
const APP_SECRET_SERVICE: &str = "pony-agent";

pub trait SecretStore: Send + Sync {
    fn get(&self, secret_ref: &str) -> Result<Option<String>, String>;
    fn set(&self, secret_ref: &str, value: &str) -> Result<(), String>;
    fn delete(&self, secret_ref: &str) -> Result<(), String>;
}

pub fn default_secret_store(secret_path: PathBuf) -> Arc<dyn SecretStore> {
    #[cfg(test)]
    {
        Arc::new(CompositeSecretStore::new(vec![Box::new(
            FileSecretStore::new(secret_path),
        )]))
    }

    #[cfg(not(test))]
    {
        let mut backends: Vec<Box<dyn SecretStore>> = Vec::new();
        if let Some(system_store) = build_system_secret_store() {
            backends.push(system_store);
        }
        backends.push(Box::new(FileSecretStore::new(secret_path)));
        Arc::new(CompositeSecretStore::new(backends))
    }
}

struct CompositeSecretStore {
    backends: Vec<Box<dyn SecretStore>>,
}

impl CompositeSecretStore {
    fn new(backends: Vec<Box<dyn SecretStore>>) -> Self {
        Self { backends }
    }
}

impl SecretStore for CompositeSecretStore {
    fn get(&self, secret_ref: &str) -> Result<Option<String>, String> {
        let mut failures = Vec::new();
        for backend in &self.backends {
            match backend.get(secret_ref) {
                Ok(Some(value)) => return Ok(Some(value)),
                Ok(None) => continue,
                Err(error) => failures.push(error),
            }
        }

        if failures.is_empty() {
            Ok(None)
        } else {
            Err(format!(
                "secret store read failed for {}: {}",
                secret_ref,
                failures.join(" | ")
            ))
        }
    }

    fn set(&self, secret_ref: &str, value: &str) -> Result<(), String> {
        let mut failures = Vec::new();
        for backend in &self.backends {
            match backend.set(secret_ref, value) {
                Ok(()) => return Ok(()),
                Err(error) => failures.push(error),
            }
        }

        Err(format!(
            "secret store write failed for {}: {}",
            secret_ref,
            failures.join(" | ")
        ))
    }

    fn delete(&self, secret_ref: &str) -> Result<(), String> {
        let mut deleted = false;
        let mut failures = Vec::new();
        for backend in &self.backends {
            match backend.delete(secret_ref) {
                Ok(()) => {
                    deleted = true;
                }
                Err(error) => failures.push(error),
            }
        }

        if deleted || failures.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "secret store delete failed for {}: {}",
                secret_ref,
                failures.join(" | ")
            ))
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
struct FileSecretStorage {
    secrets: BTreeMap<String, String>,
}

struct FileSecretStore {
    path: PathBuf,
}

impl FileSecretStore {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn load_storage(&self) -> Result<FileSecretStorage, String> {
        if !self.path.exists() {
            return Ok(FileSecretStorage::default());
        }

        let content = fs::read_to_string(&self.path)
            .map_err(|error| format!("read secret file failed: {}", error))?;
        serde_json::from_str::<FileSecretStorage>(&content)
            .map_err(|error| format!("parse secret file failed: {}", error))
    }

    fn persist_storage(&self, storage: &FileSecretStorage) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("create secret directory failed: {}", error))?;
        }

        let json = serde_json::to_string_pretty(storage)
            .map_err(|error| format!("serialize secret file failed: {}", error))?;
        fs::write(&self.path, json)
            .map_err(|error| format!("write secret file failed: {}", error))?;
        restrict_secret_file_permissions(&self.path);
        Ok(())
    }
}

impl SecretStore for FileSecretStore {
    fn get(&self, secret_ref: &str) -> Result<Option<String>, String> {
        let storage = self.load_storage()?;
        Ok(storage.secrets.get(secret_ref).cloned())
    }

    fn set(&self, secret_ref: &str, value: &str) -> Result<(), String> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Ok(());
        }

        let mut storage = self.load_storage()?;
        storage
            .secrets
            .insert(secret_ref.to_string(), trimmed.to_string());
        self.persist_storage(&storage)
    }

    fn delete(&self, secret_ref: &str) -> Result<(), String> {
        let mut storage = self.load_storage()?;
        storage.secrets.remove(secret_ref);
        self.persist_storage(&storage)
    }
}

#[cfg(unix)]
fn restrict_secret_file_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict_secret_file_permissions(_path: &Path) {}

#[cfg(not(test))]
#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
fn build_system_secret_store() -> Option<Box<dyn SecretStore>> {
    Some(Box::new(KeyringSecretStore::new(APP_SECRET_SERVICE)))
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
fn build_system_secret_store() -> Option<Box<dyn SecretStore>> {
    None
}

#[cfg(not(test))]
#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
struct KeyringSecretStore {
    service: String,
}

#[cfg(not(test))]
#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
impl KeyringSecretStore {
    fn new(service: &str) -> Self {
        Self {
            service: service.to_string(),
        }
    }

    fn entry(&self, secret_ref: &str) -> Result<keyring::Entry, String> {
        keyring::Entry::new(&self.service, secret_ref)
            .map_err(|error| format!("create system secret entry failed: {}", error))
    }
}

#[cfg(not(test))]
#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
impl SecretStore for KeyringSecretStore {
    fn get(&self, secret_ref: &str) -> Result<Option<String>, String> {
        let entry = self.entry(secret_ref)?;
        match entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(error) => {
                let message = error.to_string();
                if message.to_lowercase().contains("no entry") {
                    Ok(None)
                } else {
                    Err(format!("read system secret failed: {}", message))
                }
            }
        }
    }

    fn set(&self, secret_ref: &str, value: &str) -> Result<(), String> {
        self.entry(secret_ref)?
            .set_password(value)
            .map_err(|error| format!("write system secret failed: {}", error))
    }

    fn delete(&self, secret_ref: &str) -> Result<(), String> {
        let entry = self.entry(secret_ref)?;
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(error) => {
                let message = error.to_string();
                if message.to_lowercase().contains("no entry") {
                    Ok(())
                } else {
                    Err(format!("delete system secret failed: {}", message))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_secret_path() -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("pony-agent-secret-store-{stamp}.json"))
    }

    #[test]
    fn file_secret_store_round_trips_values() {
        let store = FileSecretStore::new(temp_secret_path());

        store
            .set("provider/test/api-key", "secret-value")
            .expect("secret should save");
        assert_eq!(
            store
                .get("provider/test/api-key")
                .expect("secret should load")
                .as_deref(),
            Some("secret-value")
        );

        store
            .delete("provider/test/api-key")
            .expect("secret should delete");
        assert_eq!(
            store
                .get("provider/test/api-key")
                .expect("secret should reload"),
            None
        );
    }
}
