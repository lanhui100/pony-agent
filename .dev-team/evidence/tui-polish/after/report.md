# TUI audit report — after

| field | value |
| --- | --- |
| binary | `/home/dm/pony-agent/target/debug/pony` |
| sha256 | `2897195bd0cd2cc3c49e6ea4bfccb13853ee298b6c04d6662dc06410491f69af` |
| mtime | `2026-11-06T18:48:28+08:00` |
| version | `pony 0.1.154 · linux-x86_64` |
| generated_at | `2026-11-06T10:53:39.014856+00:00` |
| result | **31 pass / 0 fail** across 1 scenarios |

## baseline → after 差异摘要

- 断言：baseline 354 / after 31；失败 56 → 0；场景失败 15 → 0。
- FAIL→PASS（已消除）：0 项。
  - 无
- PASS→FAIL（回归）：0 项。
  - 无
- 仅 after 求值的断言（baseline 因边框/证据缺失提前返回，或本相位新增）：31 项。
  - ask-resume：`panel.no_panic`, `panel.no_standalone_role_label`, `panel.within_terminal_width`, `panel.capture_rows`, `panel.artifact_ansi`, `panel.artifact_png`, `answered.no_panic`, `answered.no_standalone_role_label`, `answered.within_terminal_width`, `answered.capture_rows`, `answered.artifact_ansi`, `answered.artifact_png`, `resumed.no_panic`, `resumed.no_standalone_role_label`, `resumed.within_terminal_width`, `resumed.capture_rows`, `resumed.artifact_ansi`, `resumed.artifact_png`, `askresume.panel_visible`, `askresume.ask_originated_from_model`, `askresume.answer_closes_panel`, `askresume.continuation_assistant_text`, `askresume.streaming_settled`, `askresume.answer_reaches_provider`, `askresume.no_error_copy`, `askresume.exit_code_reported`, `exit.no_panic`, `exit.within_terminal_width`, `exit.capture_rows`, `exit.artifact_ansi`, `exit.artifact_png`
- 仅 baseline 求值的断言：354 项。
  - add-provider：`main.no_panic`, `main.no_standalone_role_label`, `main.within_terminal_width`, `main.capture_rows`, `main.artifact_ansi`, `main.artifact_png`, `add_provider.title`, `add_provider.step_indicator`, `add_provider.input_focused`, `add_provider.border_box`, `add_provider.panel_height_fits_screen`, `add_provider.panel_above_composer`, `add_provider.panel_vertical_borders_aligned`, `add_provider.no_double_border_overlap`, `add_provider.focus_composer_hidden`, `add_provider.focus_chat_hidden`, `add_provider.focus_background_clean`, `add_provider.border_rows_complete`
  - cli-modes：`cli.version_exit_status`, `cli.version_output`, `cli.version_no_panic`, `cli.help_exit_status`, `cli.help_output`, `cli.help_no_panic`, `cli.doctor_exit_status`, `cli.doctor_output`, `cli.doctor_no_panic`
  - conversation：`main.no_panic`, `main.no_standalone_role_label`, `main.within_terminal_width`, `main.capture_rows`, `main.artifact_ansi`, `main.artifact_png`, `main.header`, `main.input_padded_from_left_edge`, `main.input_placeholder`, `conversation.user_message_visible`, `conversation.user_role_rail`, `conversation.tool_line_visible`, `conversation.final_answer_visible`, `conversation.text_columns_aligned`, `conversation.aligned_columns_padded`, `conversation.no_error_rendered`, `conversation.mock_followup_has_tool_round`, `main.separator_single_header_rule`, `conversation.footer_metrics`
  - cost：`main.no_panic`, `main.no_standalone_role_label`, `main.within_terminal_width`, `main.capture_rows`, `main.artifact_ansi`, `main.artifact_png`, `main.input_padded_from_left_edge`, `main.input_placeholder`, `cost.model_line`, `main.separator_single_header_rule`, `cost.gateway_line`
  - ctrl-c：`exit.no_panic`, `exit.within_terminal_width`, `exit.capture_rows`, `exit.artifact_ansi`, `exit.artifact_png`, `shell.no_panic`, `shell.within_terminal_width`, `shell.capture_rows`, `shell.artifact_ansi`, `shell.artifact_png`, `ctrl_c.process_exited`, `ctrl_c.terminal_restored`, `ctrl_c.shell_alive`, `ctrl_c.pane_command_is_shell`, `ctrl_c.no_panic`
  - empty：`main.no_panic`, `main.no_standalone_role_label`, `main.within_terminal_width`, `main.capture_rows`, `main.artifact_ansi`, `main.artifact_png`, `main.header`, `main.input_padded_from_left_edge`, `main.input_placeholder`, `empty.guidance`, `empty.footer_workspace_only`, `main.separator_single_header_rule`, `main.composer_cursor_block`, `empty.placeholder_column`, `empty.command_desc_column`
  - error：`main.no_panic`, `main.no_standalone_role_label`, `main.within_terminal_width`, `main.capture_rows`, `main.artifact_ansi`, `main.artifact_png`, `main.input_padded_from_left_edge`, `main.input_placeholder`, `error.message_visible`, `error.error_glyph`, `error.reason_included`, `error.footer_metrics`, `error.no_fake_answer`, `error.mock_401_receipt`
  - help：`main.no_panic`, `main.no_standalone_role_label`, `main.within_terminal_width`, `main.capture_rows`, `main.artifact_ansi`, `main.artifact_png`, `main.input_padded_from_left_edge`, `main.input_placeholder`, `help.commands_listed`, `main.separator_single_header_rule`, `help.command_desc_column`, `help.system_message_glyph`
  - input-edit：`abc.no_panic`, `abc.no_standalone_role_label`, `abc.within_terminal_width`, `abc.capture_rows`, `abc.artifact_ansi`, `abc.artifact_png`, `input_edit.abc`, `inserted.no_panic`, `inserted.no_standalone_role_label`, `inserted.within_terminal_width`, `inserted.capture_rows`, `inserted.artifact_ansi`, `inserted.artifact_png`, `input_edit.inserted`, `backspace.no_panic`, `backspace.no_standalone_role_label`, `backspace.within_terminal_width`, `backspace.capture_rows`, `backspace.artifact_ansi`, `backspace.artifact_png`, `input_edit.backspace`, `word_cleared.no_panic`, `word_cleared.no_standalone_role_label`, `word_cleared.within_terminal_width`, `word_cleared.capture_rows`, `word_cleared.artifact_ansi`, `word_cleared.artifact_png`, `input_edit.word_cleared`, `tab_complete.no_panic`, `tab_complete.no_standalone_role_label`, `tab_complete.within_terminal_width`, `tab_complete.capture_rows`, `tab_complete.artifact_ansi`, `tab_complete.artifact_png`, `input_edit.tab_complete`, `ctrl_u_cleared.no_panic`, `ctrl_u_cleared.no_standalone_role_label`, `ctrl_u_cleared.within_terminal_width`, `ctrl_u_cleared.capture_rows`, `ctrl_u_cleared.artifact_ansi`, `ctrl_u_cleared.artifact_png`, `input_edit.ctrl_u_cleared`, `multiline.no_panic`, `multiline.no_standalone_role_label`, `multiline.within_terminal_width`, `multiline.capture_rows`, `multiline.artifact_ansi`, `multiline.artifact_png`, `input_edit.multiline`, `esc_cleared.no_panic`, `esc_cleared.no_standalone_role_label`, `esc_cleared.within_terminal_width`, `esc_cleared.capture_rows`, `esc_cleared.artifact_ansi`, `esc_cleared.artifact_png`, `input_edit.esc_cleared`, `draft.no_panic`, `draft.no_standalone_role_label`, `draft.within_terminal_width`, `draft.capture_rows`, `draft.artifact_ansi`, `draft.artifact_png`, `input_edit.draft`, `input_edit.typed_text_column`, `input_edit.multiline_visual_rows`, `draft_cleared.no_panic`, `draft_cleared.no_standalone_role_label`, `draft_cleared.within_terminal_width`, `draft_cleared.capture_rows`, `draft_cleared.artifact_ansi`, `draft_cleared.artifact_png`, `input_edit.ctrl_c_with_draft_clears_only`, `input_edit.ctrl_c_draft_still_running`, `exit.no_panic`, `exit.within_terminal_width`, `exit.capture_rows`, `exit.artifact_ansi`, `exit.artifact_png`, `shell.no_panic`, `shell.within_terminal_width`, `shell.capture_rows`, `shell.artifact_ansi`, `shell.artifact_png`, `input_edit.ctrl_c_empty_exits`, `input_edit.terminal_restored`, `input_edit.shell_alive`, `input_edit.pane_command_is_shell`
  - models-panel：`main.no_panic`, `main.no_standalone_role_label`, `main.within_terminal_width`, `main.capture_rows`, `main.artifact_ansi`, `main.artifact_png`, `models.title_counter`, `models.border_box`, `models.panel_height_fits_screen`, `models.panel_above_composer`, `models.panel_vertical_borders_aligned`, `models.no_double_border_overlap`, `models.focus_composer_hidden`, `models.focus_chat_hidden`, `models.focus_background_clean`, `models.border_rows_complete`, `models.rows_rendered`, `models.rows_unique_and_consecutive`, `models.no_fetch_failure`, `models.selected_marker`
  - narrow：`empty.no_panic`, `empty.no_standalone_role_label`, `empty.within_terminal_width`, `empty.capture_rows`, `empty.artifact_ansi`, `empty.artifact_png`, `empty.input_padded_from_left_edge`, `empty.input_placeholder`, `conversation.no_panic`, `conversation.no_standalone_role_label`, `conversation.within_terminal_width`, `conversation.capture_rows`, `conversation.artifact_ansi`, `conversation.artifact_png`, `conversation.input_padded_from_left_edge`, `conversation.input_placeholder`, `empty.header`, `empty.separator_single_header_rule`, `narrow.empty_guidance`, `narrow.conversation_visible`
  - non-tty：`non_tty.diagnostic`, `non_tty.no_panic`, `non_tty.exit_code_recorded`, `non_tty.tty_hint`
  - provider-panel：`main.no_panic`, `main.no_standalone_role_label`, `main.within_terminal_width`, `main.capture_rows`, `main.artifact_ansi`, `main.artifact_png`, `provider.title`, `provider.fixture_listed`, `provider.add_hint`, `provider.border_box`, `provider.focus_composer_hidden`, `provider.focus_chat_hidden`, `provider.focus_background_clean`, `provider.border_rows_complete`, `provider.current_marker`
  - scrolled：`before.no_panic`, `before.no_standalone_role_label`, `before.within_terminal_width`, `before.capture_rows`, `before.artifact_ansi`, `before.artifact_png`, `after.no_panic`, `after.no_standalone_role_label`, `after.within_terminal_width`, `after.capture_rows`, `after.artifact_ansi`, `after.artifact_png`, `after.input_padded_from_left_edge`, `after.input_placeholder`, `scrolled.bottom_before`, `scrolled.metrics_footer_before`, `scrolled.after_shows_earlier_rows`, `scrolled.metrics_footer_after`, `scrolled.offset_moved_up`
  - sessions-panel：`main.no_panic`, `main.no_standalone_role_label`, `main.within_terminal_width`, `main.capture_rows`, `main.artifact_ansi`, `main.artifact_png`, `sessions.title`, `sessions.prior_conversation_listed`, `sessions.session_id_visible`, `sessions.not_empty`, `sessions.border_box`, `sessions.panel_height_fits_screen`, `sessions.panel_above_composer`, `sessions.panel_vertical_borders_aligned`, `sessions.panel_width_fits`, `sessions.panel_width_expected`, `sessions.panel_rows_not_overlapping`, `sessions.no_double_border_overlap`, `sessions.focus_composer_hidden`, `sessions.focus_chat_hidden`, `sessions.focus_background_clean`, `sessions.border_rows_complete`, `sessions.current_marker`
  - slash-menu：`main.no_panic`, `main.no_standalone_role_label`, `main.within_terminal_width`, `main.capture_rows`, `main.artifact_ansi`, `main.artifact_png`, `main.input_padded_from_left_edge`, `slash.all_commands_listed`, `slash.menu_has_top_and_bottom_rules`, `slash.menu_above_composer`, `slash.each_command_once`, `slash.command_desc_column`, `slash.resume_once_whole_screen`, `slash.selected_row_marker`
  - streaming：`mid.no_panic`, `mid.no_standalone_role_label`, `mid.within_terminal_width`, `mid.capture_rows`, `mid.artifact_ansi`, `mid.artifact_png`, `final.no_panic`, `final.no_standalone_role_label`, `final.within_terminal_width`, `final.capture_rows`, `final.artifact_ansi`, `final.artifact_png`, `streaming.mid_status_generating`, `streaming.mid_active_elapsed`, `streaming.mid_partial_body`, `streaming.mid_not_finished`, `streaming.final_complete`, `streaming.final_not_generating`
  - wide：`empty.no_panic`, `empty.no_standalone_role_label`, `empty.within_terminal_width`, `empty.capture_rows`, `empty.artifact_ansi`, `empty.artifact_png`, `empty.input_padded_from_left_edge`, `empty.input_placeholder`, `conversation.no_panic`, `conversation.no_standalone_role_label`, `conversation.within_terminal_width`, `conversation.capture_rows`, `conversation.artifact_ansi`, `conversation.artifact_png`, `conversation.input_padded_from_left_edge`, `conversation.input_placeholder`, `empty.header`, `empty.separator_single_header_rule`, `wide.header_full_metadata`, `wide.conversation_visible`, `wide.text_columns_aligned`

| scenario | baseline pass/total | after pass/total | fixed | regressed |
| --- | --- | --- | --- | --- |
| ask-resume | 0/0 | 31/31 | 0 | 0 |
| add-provider | 13/18 | 0/0 | 0 | 0 |
| cli-modes | 9/9 | 0/0 | 0 | 0 |
| conversation | 13/19 | 0/0 | 0 | 0 |
| cost | 10/11 | 0/0 | 0 | 0 |
| ctrl-c | 15/15 | 0/0 | 0 | 0 |
| empty | 10/15 | 0/0 | 0 | 0 |
| error | 12/14 | 0/0 | 0 | 0 |
| help | 9/12 | 0/0 | 0 | 0 |
| input-edit | 86/87 | 0/0 | 0 | 0 |
| models-panel | 13/20 | 0/0 | 0 | 0 |
| narrow | 19/20 | 0/0 | 0 | 0 |
| non-tty | 4/4 | 0/0 | 0 | 0 |
| provider-panel | 10/15 | 0/0 | 0 | 0 |
| scrolled | 17/19 | 0/0 | 0 | 0 |
| sessions-panel | 18/23 | 0/0 | 0 | 0 |
| slash-menu | 10/14 | 0/0 | 0 | 0 |
| streaming | 12/18 | 0/0 | 0 | 0 |
| wide | 18/21 | 0/0 | 0 | 0 |

## Scenarios

### ask-resume (120x34)

- evidence: `ask-resume/`
- screenshots: [answered.png](ask-resume/answered.png), [exit.png](ask-resume/exit.png), [panel.png](ask-resume/panel.png), [resumed.png](ask-resume/resumed.png)
- checks: 31 pass / 0 fail

| result | check | evidence |
| --- | --- | --- |
| PASS | `panel.no_panic` — capture and tui runtime log (/home/dm/pony-agent/.dev-team/evidence/tui-polish/after/ask-resume/tui-runtime.log from byte 0) contain no Rust panic/backtrace | no panic tokens in capture or tui runtime log |
| PASS | `panel.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `panel.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=117 cols=120 :: pony · pony-agent mock-alpha |
| PASS | `panel.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `panel.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/ask-resume/panel.ansi size=3343 |
| PASS | `panel.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/ask-resume/panel.png size=134526 |
| PASS | `answered.no_panic` — capture and tui runtime log (/home/dm/pony-agent/.dev-team/evidence/tui-polish/after/ask-resume/tui-runtime.log from byte 0) contain no Rust panic/backtrace | no panic tokens in capture or tui runtime log |
| PASS | `answered.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `answered.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=117 cols=120 :: pony · pony-agent mock-alpha |
| PASS | `answered.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `answered.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/ask-resume/answered.ansi size=3064 |
| PASS | `answered.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/ask-resume/answered.png size=129589 |
| PASS | `resumed.no_panic` — capture and tui runtime log (/home/dm/pony-agent/.dev-team/evidence/tui-polish/after/ask-resume/tui-runtime.log from byte 0) contain no Rust panic/backtrace | no panic tokens in capture or tui runtime log |
| PASS | `resumed.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `resumed.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=117 cols=120 :: pony · pony-agent mock-alpha |
| PASS | `resumed.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `resumed.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/ask-resume/resumed.ansi size=4223 |
| PASS | `resumed.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/ask-resume/resumed.png size=176326 |
| PASS | `askresume.panel_visible` — the model-initiated Ask raised a real panel (question + both options + esc hint) | ◇ 提问 继续吗？ |
| PASS | `askresume.ask_originated_from_model` — the first provider round declared tools and carried zero tool results (the Ask came from the model, not from a bootstrap injection) | chat_requests=2 first_tools=38 first_tool_msgs=0 first_scenario='ask' |
| PASS | `askresume.answer_closes_panel` — Enter submits the answer and the panel closes (question + esc hint gone) | panel closed |
| PASS | `askresume.continuation_assistant_text` — after answering, the same turn continues and renders the second assistant text (mock FINAL_TEXT fingerprint becomes visible) | markers_resumed=['0.1.125', '包管理器：npm'] markers_before_answer=[] tail=['如果需要，我可以继续列出依赖清单。', '› 写下任务，或输入 / 查看命令', '/home/dm/pony-agent ↑32 ↓64 · 上下文 0% · 187 t/s · 首token 63ms'] |
| PASS | `askresume.streaming_settled` — the resumed turn settles: no Thinking.../思考中/生成中 residue and the footer shows non-zero turn usage | residue=[] footer=/home/dm/pony-agent ↑32 ↓64 · 上下文 0% · 187 t/s · 首token 63ms |
| PASS | `askresume.answer_reaches_provider` — a follow-up provider round carries the user's answer as a tool result and never the suspended control_outcome_pending marker | followup_rounds=1 tool_msgs=['{"output":{"answer":{"answer":"继续"},"kind":"waiting_user","ok":true,"requestId":"control-…'] answer_present=True pending_marker_hits=0 |
| PASS | `askresume.no_error_copy` — no stale/expired/resume-unavailable/submit-failure copy after answering | no error/stale copy on the answered/resumed frames |
| PASS | `askresume.exit_code_reported` — the scenario ends with the TUI's real process exit status (AUDIT_EXIT=…) | code=0 row :: e.log /home/dm/pony-agent/target/debug/pony; echo AUDIT_EXIT=$? |
| PASS | `exit.no_panic` — capture and tui runtime log (/home/dm/pony-agent/.dev-team/evidence/tui-polish/after/ask-resume/tui-runtime.log from byte 0) contain no Rust panic/backtrace | no panic tokens in capture or tui runtime log |
| PASS | `exit.within_terminal_width` — every captured row display width <= 120 cols | widest row 1 width=120 cols=120 :: -u NO_COLOR XDG_CONFIG_HOME=/tmp/tui-audit.snQsWL/iso-ask-resume/xdg-config XDG_DATA_HOME=/tmp/tui-audit.snQsWL/iso-ask- |
| PASS | `exit.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `exit.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/ask-resume/exit.ansi size=983 |
| PASS | `exit.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/ask-resume/exit.png size=166612 |

