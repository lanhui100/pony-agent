# ISSUE-20261111-005 红相测试收据

## 1. 目标
在修改实现代码前，先通过单元测试断言 `REWIND_SCOPE_ROWS` 的安全行序与 `RewindState` 的默认行为，确保测试精准捕获到既有缺陷并失败（Exit != 0）。

## 2. 真实红相输出记录

```text
running 14 tests
test rewind_tests::esc_arm_expires_outside_window ... ok
test rewind_tests::esc_arm_is_session_scoped ... ok
test rewind_tests::esc_arm_requires_same_session_within_window ... ok
test rewind_tests::esc_arm_reset_drops_pending_press ... ok
test rewind_tests::empty_point_list_blocks_entering_scope_stage ... ok
test rewind_tests::menu_row_falls_back_to_title_when_message_empty ... ok
test rewind_tests::menu_row_prefers_first_non_empty_message_line ... ok
test rewind_tests::missing_overlay_is_treated_as_cancel ... ok
test rewind_tests::scope_rows_map_to_modes_and_clamp ... FAILED
test rewind_tests::scope_stage_direct_enter_defaults_to_conversation_only ... FAILED
test rewind_tests::turn_selection_clamps_at_both_ends ... ok
test rewind_tests::scope_stage_enter_confirms_with_selected_mode ... FAILED
test rewind_tests::scope_stage_number_keys_pick_and_confirm ... FAILED
test rewind_tests::turn_stage_enter_moves_to_scope_and_esc_cancels ... ok

failures:

---- rewind_tests::scope_rows_map_to_modes_and_clamp stdout ----
thread 'rewind_tests::scope_rows_map_to_modes_and_clamp' panicked at crates/pony-agent-core/src/bin/tui/tests/rewind_tests.rs:89:5:
assertion `left == right` failed: 第一项必须是安全默认（仅恢复会话）
  left: ("恢复代码与会话", ConversationAndFiles)
 right: ("仅恢复会话", ConversationOnly)

---- rewind_tests::scope_stage_direct_enter_defaults_to_conversation_only stdout ----
thread 'rewind_tests::scope_stage_direct_enter_defaults_to_conversation_only' panicked at crates/pony-agent-core/src/bin/tui/tests/rewind_tests.rs:166:5:
assertion `left == right` failed: 未切换范围直接回车必须默认为仅恢复会话
  left: Some(ConversationAndFiles)
 right: Some(ConversationOnly)

---- rewind_tests::scope_stage_enter_confirms_with_selected_mode stdout ----
thread 'rewind_tests::scope_stage_enter_confirms_with_selected_mode' panicked at crates/pony-agent-core/src/bin/tui/tests/rewind_tests.rs:182:5:
assertion `left == right` failed: 按 Down 选中破坏性模式并确认
  left: Some(ConversationOnly)
 right: Some(ConversationAndFiles)

---- rewind_tests::scope_stage_number_keys_pick_and_confirm stdout ----
thread 'rewind_tests::scope_stage_number_keys_pick_and_confirm' panicked at crates/pony-agent-core/src/bin/tui/tests/rewind_tests.rs:197:5:
assertion `left == right` failed: 数字 1 选择仅恢复会话
  left: Some(ConversationAndFiles)
 right: Some(ConversationOnly)

failures:
    rewind_tests::scope_rows_map_to_modes_and_clamp
    rewind_tests::scope_stage_direct_enter_defaults_to_conversation_only
    rewind_tests::scope_stage_enter_confirms_with_selected_mode
    rewind_tests::scope_stage_number_keys_pick_and_confirm

test result: FAILED. 10 passed; 4 failed; 0 ignored; 0 measured; 110 filtered out; finished in 0.00s
error: test failed, to rerun pass `-p pony-agent-core --bin pony`
```

## 3. 结论
红相测试完全证实了缺陷：
1. 默认首项为破坏性项 `("恢复代码与会话", ConversationAndFiles)`；
2. 直接回车直接采用破坏性项；
3. 数字 1 映射到破坏性项。
红相锚定成功。
