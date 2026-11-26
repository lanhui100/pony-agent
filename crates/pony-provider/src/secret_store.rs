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
        Arc::new(CompositeSecretStore::new(vec![SecretBackendSlot::new(
            "file",
            true,
            Box::new(FileSecretStore::new(secret_path)),
        )]))
    }

    #[cfg(not(test))]
    {
        let mut backends: Vec<SecretBackendSlot> = Vec::new();
        if let Some(system_store) = build_system_secret_store() {
            backends.push(SecretBackendSlot::new("system", false, system_store));
        }
        backends.push(SecretBackendSlot::new(
            "file",
            true,
            Box::new(FileSecretStore::new(secret_path)),
        ));
        Arc::new(CompositeSecretStore::new(backends))
    }
}

/// F8b：组合后端的一个槽位。
///
/// `label` 只用于可观测告警定位（绝不参与密钥寻址）；`plaintext_fallback` 标记
/// "非安全介质"后端（当前仅 [`FileSecretStore`]）：一旦写操作由它兜底成功，说明
/// 系统密钥库写入失败、密文已静默落盘为明文，必须显式告警而不是无声降级。
struct SecretBackendSlot {
    label: &'static str,
    plaintext_fallback: bool,
    store: Box<dyn SecretStore>,
}

impl SecretBackendSlot {
    fn new(label: &'static str, plaintext_fallback: bool, store: Box<dyn SecretStore>) -> Self {
        Self {
            label,
            plaintext_fallback,
            store,
        }
    }
}

struct CompositeSecretStore {
    backends: Vec<SecretBackendSlot>,
}

impl CompositeSecretStore {
    fn new(backends: Vec<SecretBackendSlot>) -> Self {
        Self { backends }
    }

    /// F8b：写操作回退到明文文件后端时的可观测告警。
    ///
    /// 只打印 `secret_ref` 名称与失败原因，**永不**打印密文值。
    fn warn_plaintext_fallback(&self, secret_ref: &str, slot: &SecretBackendSlot, failures: &[String]) {
        eprintln!("{}", plaintext_fallback_warning(secret_ref, slot.label, failures));
    }
}

/// F8b：明文回退告警文案（纯函数，便于单测断言"只带 secret_ref、不带密文"）。
fn plaintext_fallback_warning(secret_ref: &str, label: &str, failures: &[String]) -> String {
    format!(
        "[pony-agent] 安全警告：系统密钥库写入失败，密钥已回退为明文文件存储（secret_ref={secret_ref}，落盘后端={label}，失败原因={}）。请修复系统密钥库后重新设置该密钥。",
        failures.join(" | ")
    )
}

impl SecretStore for CompositeSecretStore {
    fn get(&self, secret_ref: &str) -> Result<Option<String>, String> {
        let mut failures = Vec::new();
        for slot in &self.backends {
            match slot.store.get(secret_ref) {
                Ok(Some(value)) => return Ok(Some(value)),
                Ok(None) => continue,
                Err(error) => failures.push(format!("{}: {}", slot.label, error)),
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
        for slot in &self.backends {
            match slot.store.set(secret_ref, value) {
                Ok(()) => {
                    // F8b：首选后端失败、由后续后端兜底成功时不得静默。
                    if !failures.is_empty() {
                        if slot.plaintext_fallback {
                            self.warn_plaintext_fallback(secret_ref, slot, &failures);
                        } else {
                            eprintln!(
                                "[pony-agent] secret store 写入回退：后端 {} 失败（secret_ref={secret_ref}），已由后端 {} 承接：{}",
                                self.backends
                                    .first()
                                    .map(|first| first.label)
                                    .unwrap_or("unknown"),
                                slot.label,
                                failures.join(" | ")
                            );
                        }
                    }
                    return Ok(());
                }
                Err(error) => failures.push(format!("{}: {}", slot.label, error)),
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
        for slot in &self.backends {
            match slot.store.delete(secret_ref) {
                Ok(()) => {
                    deleted = true;
                }
                Err(error) => failures.push(format!("{}: {}", slot.label, error)),
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

pub struct FileSecretStore {
    path: PathBuf,
}

impl FileSecretStore {
    pub fn new(path: PathBuf) -> Self {
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

    /// 永远写失败的假"系统密钥库"，用于验证 F8b 的明文回退路径。
    struct FailingSecretStore;

    impl SecretStore for FailingSecretStore {
        fn get(&self, _secret_ref: &str) -> Result<Option<String>, String> {
            Err("keyring unavailable".to_string())
        }

        fn set(&self, _secret_ref: &str, _value: &str) -> Result<(), String> {
            Err("keyring unavailable".to_string())
        }

        fn delete(&self, _secret_ref: &str) -> Result<(), String> {
            Err("keyring unavailable".to_string())
        }
    }

    /// F8b：系统密钥库写失败后，明文文件后端兜底仍要成功（既有行为不变），
    /// 且回退必须产生带 `secret_ref` 的可观测告警、告警文案不得包含密文。
    #[test]
    fn plaintext_fallback_still_persists_and_warning_names_secret_without_value() {
        let path = temp_secret_path();
        let store = CompositeSecretStore::new(vec![
            SecretBackendSlot::new("system", false, Box::new(FailingSecretStore)),
            SecretBackendSlot::new("file", true, Box::new(FileSecretStore::new(path.clone()))),
        ]);

        let secret_ref = "provider/demo/api-key";
        let secret_value = "sk-mock-plaintext-fallback";
        store
            .set(secret_ref, secret_value)
            .expect("plaintext fallback must still persist the secret");
        assert_eq!(
            store
                .get(secret_ref)
                .expect("fallback value should read back")
                .as_deref(),
            Some(secret_value)
        );

        let warning = plaintext_fallback_warning(
            secret_ref,
            "file",
            &["system: keyring unavailable".to_string()],
        );
        assert!(
            warning.contains(secret_ref),
            "告警必须点名 secret_ref 以便定位：{warning}"
        );
        assert!(
            warning.contains("明文"),
            "告警必须明示已回退明文存储：{warning}"
        );
        assert!(
            !warning.contains(secret_value),
            "告警不得泄漏密文值：{warning}"
        );

        let _ = fs::remove_file(&path);
    }
}
