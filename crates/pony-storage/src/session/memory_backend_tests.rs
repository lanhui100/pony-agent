//! 内存后端的最小闭环用例（自 `session/tests.rs` 拆分，行为不变；file-size 收口）。
//!
//! 拆分理由：`tests.rs` 已越过 file-size 基线，新增测试一律独立成文件
//! （先例：`host_continuation_goal_round_tests` / `history_compaction_tests`）。

use super::*;

#[test]
fn memory_backend_keeps_turns_in_process() {
    let mut store = SessionStore::memory_only();
    store.append_turn(
        Some("test"),
        "查看 tauri.conf.json",
        "已读取",
        None,
        Vec::new(),
    );

    let snapshot = store.snapshot(Some("test"), &[]);
    assert_eq!(snapshot.title, "查看 tauri.conf.json");
    assert_eq!(snapshot.turn_count, 1);
    assert_eq!(snapshot.history.len(), 2);
    assert_eq!(
        snapshot.last_referenced_file.as_deref(),
        Some("tauri.conf.json")
    );
}
