use super::backend::SessionBackend;
use super::store::{load_store_from_path, PersistedStore};
use crate::agent::input::TurnInputImage;
use crate::agent::provider::BuildContextObservation;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

pub struct FileSessionBackend {
    pub(super) storage_path: PathBuf,
}

#[cfg(test)]
pub struct MemorySessionBackend {
    pub(super) attachment_root: PathBuf,
}

impl FileSessionBackend {
    pub fn new(storage_path: PathBuf) -> Self {
        Self { storage_path }
    }
}

impl SessionBackend for FileSessionBackend {
    fn load_store(&self) -> Option<PersistedStore> {
        eprintln!(
            "[pony-agent][session] loading sessions from {}",
            self.storage_path.display()
        );
        load_store_from_path(&self.storage_path)
    }

    fn save_store(&self, store: &PersistedStore) {
        let Some(parent) = self.storage_path.parent() else {
            return;
        };
        if fs::create_dir_all(parent).is_err() {
            return;
        }

        let Ok(serialized) = serde_json::to_string_pretty(store) else {
            return;
        };
        eprintln!(
            "[pony-agent][session] saving sessions to {}",
            self.storage_path.display()
        );
        let _ = fs::write(&self.storage_path, serialized);
    }

    fn attachment_root(&self) -> Option<PathBuf> {
        self.storage_path
            .parent()
            .map(|parent| parent.join("attachments"))
    }
}

#[cfg(test)]
impl SessionBackend for MemorySessionBackend {
    fn load_store(&self) -> Option<PersistedStore> {
        None
    }

    fn save_store(&self, _store: &PersistedStore) {}

    fn attachment_root(&self) -> Option<PathBuf> {
        Some(self.attachment_root.clone())
    }
}

#[cfg(test)]
impl Drop for MemorySessionBackend {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.attachment_root);
    }
}
