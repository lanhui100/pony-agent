//! W32 验收测试：文件级快照记录与工作区回滚（WorkspaceSnapshot & Rewind）
//!
//! 契约要求：
//! 1. `WorkspaceSnapshot` 类型支持记录相对路径、内容哈希以及快照文件副本。
//! 2. 写操作/编辑工具（如 Write、Edit、Patch）在变更前自动捕捉文件状态并登记快照记录。
//! 3. `WorkspaceRef` 的 `rollback_capable` 标记在存在有效快照时置为 `true`。
//! 4. 显式调用 `rewind` 或 `checkout_history_node(..., HistoryCheckoutMode::TranscriptAndWorkspace)`
//!    时，能将工作区被修改的文件内容逐字节准确恢复至快照记录状态。
//! 5. 对不存在快照或无效 node_id 的回滚请求，失败并返回结构化错误。

use pony_agent_core::agent::session::{
    HistoryCheckoutMode, HistoryNode, HistoryNodeKind, WorkspaceRef, WorkspaceRefKind,
};
use pony_agent_core::agent::session::WorkspaceSnapshotManager;
use std::fs;
use std::path::PathBuf;

#[test]
fn test_workspace_snapshot_capture_and_rewind_roundtrip() {
    let temp_dir = std::env::temp_dir().join(format!("test_w32_snap_{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&temp_dir).expect("create temp dir");

    let file_a = temp_dir.join("a.txt");
    let file_b = temp_dir.join("b.txt");
    let file_c = temp_dir.join("c.txt");

    fs::write(&file_a, "initial A").expect("write A");
    fs::write(&file_b, "initial B").expect("write B");
    fs::write(&file_c, "initial C").expect("write C");

    let snapshot_dir = temp_dir.join(".snapshots");
    let mut manager = WorkspaceSnapshotManager::new(&temp_dir, &snapshot_dir);

    // 1. 创建节点 1 快照并记录文件
    let node_1 = "node-1";
    manager.capture_file(node_1, "a.txt").expect("capture a");
    manager.capture_file(node_1, "b.txt").expect("capture b");
    manager.capture_file(node_1, "c.txt").expect("capture c");
    manager.commit_snapshot(node_1).expect("commit node-1 snapshot");

    let ws_ref = manager.get_workspace_ref(node_1);
    assert!(ws_ref.rollback_capable, "存在已提交快照时 rollback_capable 必须为 true");
    assert_eq!(ws_ref.kind, WorkspaceRefKind::HostSnapshot);

    // 2. 修改这 3 个文件
    fs::write(&file_a, "modified A content").expect("modify A");
    fs::write(&file_b, "modified B content").expect("modify B");
    fs::write(&file_c, "modified C content").expect("modify C");

    assert_eq!(fs::read_to_string(&file_a).unwrap(), "modified A content");

    // 3. 执行工作区 rewind 回滚至 node-1 快照
    manager.rewind_to(node_1).expect("rewind to node-1");

    // 4. 验证 3 个文件逐字节恢复为初始内容
    assert_eq!(fs::read_to_string(&file_a).unwrap(), "initial A");
    assert_eq!(fs::read_to_string(&file_b).unwrap(), "initial B");
    assert_eq!(fs::read_to_string(&file_c).unwrap(), "initial C");

    // 5. 对不存在的 node 请求回滚返回错误
    let err = manager.rewind_to("non-existent-node");
    assert!(err.is_err(), "回滚至不存在的快照节点必须报错");

    let _ = fs::remove_dir_all(temp_dir);
}
