# ISSUE-20261111-005 绿相测试收据

## 1. 目标
修改 `REWIND_SCOPE_ROWS` 的行序以及 `RewindState::scope` 的 fallback，使撤回面板默认选中项转为安全模式「仅恢复会话」（`RewindMode::ConversationOnly`），并通过全部单元测试。

## 2. 真实绿相输出记录

```text
running 14 tests
test rewind_tests::esc_arm_expires_outside_window ... ok
test rewind_tests::esc_arm_is_session_scoped ... ok
test rewind_tests::esc_arm_requires_same_session_within_window ... ok
test rewind_tests::esc_arm_reset_drops_pending_press ... ok
test rewind_tests::menu_row_falls_back_to_title_when_message_empty ... ok
test rewind_tests::menu_row_prefers_first_non_empty_message_line ... ok
test rewind_tests::empty_point_list_blocks_entering_scope_stage ... ok
test rewind_tests::missing_overlay_is_treated_as_cancel ... ok
test rewind_tests::scope_rows_map_to_modes_and_clamp ... ok
test rewind_tests::scope_stage_direct_enter_defaults_to_conversation_only ... ok
test rewind_tests::scope_stage_enter_confirms_with_selected_mode ... ok
test rewind_tests::turn_selection_clamps_at_both_ends ... ok
test rewind_tests::turn_stage_enter_moves_to_scope_and_esc_cancels ... ok
test rewind_tests::scope_stage_number_keys_pick_and_confirm ... ok

test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 110 filtered out; finished in 0.00s
```

## 3. 验证项
- `REWIND_SCOPE_ROWS[0]` 验证为 `("仅恢复会话", ConversationOnly)` (PASS)
- 未按键直接回车确认为 `ConversationOnly` (PASS)
- 按数字 1 确认为 `ConversationOnly`，按数字 2 确认为 `ConversationAndFiles` (PASS)
- 按下方向键后回车确认为 `ConversationAndFiles` (PASS)
