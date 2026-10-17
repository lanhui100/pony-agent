//! 工作区文件快照与回滚管理器（W32）。
//!
//! 契约：
//! - 支持按节点记录变更前文件的哈希与备份副本；
//! - 节点绑定 `WorkspaceRef` 时将 `rollback_capable` 置为 `true`，类型置为 `HostSnapshot`；
//! - 执行 `rewind_to` 时能够将该节点快照中的所有文件精准还原到工作区。

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use crate::agent::session::types::{WorkspaceRef, WorkspaceRefKind};

#[derive(Clone, Debug)]
pub struct WorkspaceSnapshotItem {
    pub relative_path: String,
    pub content_hash: String,
    pub backup_path: PathBuf,
}

#[derive(Clone, Debug)]
pub struct NodeWorkspaceSnapshot {
    pub node_id: String,
    pub files: HashMap<String, WorkspaceSnapshotItem>,
}

#[derive(Clone, Debug)]
pub struct WorkspaceSnapshotManager {
    workspace_root: PathBuf,
    snapshot_dir: PathBuf,
    snapshots: HashMap<String, NodeWorkspaceSnapshot>,
}

impl WorkspaceSnapshotManager {
    pub fn new(workspace_root: impl Into<PathBuf>, snapshot_dir: impl Into<PathBuf>) -> Self {
        let ws = workspace_root.into();
        let snap = snapshot_dir.into();
        Self {
            workspace_root: ws,
            snapshot_dir: snap,
            snapshots: HashMap::new(),
        }
    }

    /// 捕获指定相对路径文件在当前状态下的内容，并记录到指定节点的快照中
    pub fn capture_file(&mut self, node_id: &str, relative_path: &str) -> Result<(), String> {
        let source_path = self.workspace_root.join(relative_path);
        if !source_path.exists() {
            return Err(format!("Source file does not exist: {}", source_path.display()));
        }

        let content = fs::read(&source_path)
            .map_err(|e| format!("Failed to read file {}: {e}", source_path.display()))?;
        
        let hash = crate::agent::spill::content_ref_id(&content);
        let node_snap_dir = self.snapshot_dir.join(node_id);
        fs::create_dir_all(&node_snap_dir)
            .map_err(|e| format!("Failed to create snapshot dir: {e}"))?;

        let backup_path = node_snap_dir.join(format!("{}.bak", hash));
        fs::write(&backup_path, &content)
            .map_err(|e| format!("Failed to write backup file: {e}"))?;

        let entry = self.snapshots.entry(node_id.to_string()).or_insert_with(|| NodeWorkspaceSnapshot {
            node_id: node_id.to_string(),
            files: HashMap::new(),
        });

        entry.files.insert(
            relative_path.to_string(),
            WorkspaceSnapshotItem {
                relative_path: relative_path.to_string(),
                content_hash: hash,
                backup_path,
            },
        );

        Ok(())
    }

    /// 提交指定节点的快照
    pub fn commit_snapshot(&mut self, node_id: &str) -> Result<(), String> {
        if !self.snapshots.contains_key(node_id) {
            return Err(format!("No snapshot recorded for node {node_id}"));
        }
        Ok(())
    }

    /// 获取对应节点的 WorkspaceRef
    pub fn get_workspace_ref(&self, node_id: &str) -> WorkspaceRef {
        if let Some(snap) = self.snapshots.get(node_id) {
            if !snap.files.is_empty() {
                return WorkspaceRef {
                    kind: WorkspaceRefKind::HostSnapshot,
                    locator: Some(self.snapshot_dir.join(node_id).to_string_lossy().to_string()),
                    rollback_capable: true,
                };
            }
        }
        WorkspaceRef::default()
    }

    /// 将工作区回滚至指定节点的状态
    pub fn rewind_to(&self, node_id: &str) -> Result<(), String> {
        let snap = self
            .snapshots
            .get(node_id)
            .ok_or_else(|| format!("Snapshot not found for node `{node_id}`"))?;

        for item in snap.files.values() {
            let target_path = self.workspace_root.join(&item.relative_path);
            if let Some(parent) = target_path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let backup_content = fs::read(&item.backup_path)
                .map_err(|e| format!("Failed to read backup {}: {e}", item.backup_path.display()))?;
            fs::write(&target_path, backup_content)
                .map_err(|e| format!("Failed to restore target file {}: {e}", target_path.display()))?;
        }

        Ok(())
    }
}
