# TUI audit report — after

| field | value |
| --- | --- |
| binary | `/home/dm/pony-agent/target/debug/pony` |
| sha256 | `78bd05bfb9f5655b823e1fce7fc27f8075270d04e00ef043b35315393c694adf` |
| mtime | `2026-10-19T20:51:36+08:00` |
| version | `pony 0.1.127 · linux-x86_64` |
| generated_at | `2026-10-19T12:52:36.248058+00:00` |
| result | **365 pass / 4 fail** across 18 scenarios |

## Failed checks

| scenario | check | evidence |
| --- | --- | --- |
| empty | `empty.command_desc_column` | cols={'/resume': 16, '/provider': 16, '/models': 16, '/clear': None, '/cost': None, '/help': None, '/quit': None} expected=16 missing=['/clear'] |
| slash-menu | `slash.all_commands_listed` | menu_commands=['/resume', '/provider', '/models', '/clear', '/cost', '/help', '/quit'] missing=['/quit'] |
| slash-menu | `slash.each_command_once` | duplicate/missing commands=['/quit'] menu_rows=7 |
| slash-menu | `slash.command_desc_column` | cols={'/resume': 16, '/provider': 16, '/models': 16, '/clear': 16, '/cost': 16, '/help': 16, '/quit': None} expected=16 missing=['/quit'] |

## baseline → after 差异摘要

- 断言：baseline 354 / after 369；失败 48 → 4；场景失败 15 → 2。
- FAIL→PASS（已消除）：39 项。
  - empty：`main.composer_cursor_block`, `empty.placeholder_column`
  - conversation：`main.no_standalone_role_label`, `conversation.user_role_rail`, `conversation.text_columns_aligned`, `conversation.aligned_columns_padded`
  - slash-menu：`slash.resume_once_whole_screen`
  - models-panel：`models.title_counter`, `models.panel_vertical_borders_aligned`, `models.focus_composer_hidden`, `models.focus_chat_hidden`, `models.focus_background_clean`, `models.border_rows_complete`, `models.rows_unique_and_consecutive`
  - provider-panel：`provider.border_box`, `provider.focus_composer_hidden`, `provider.focus_chat_hidden`, `provider.focus_background_clean`, `provider.border_rows_complete`
  - sessions-panel：`main.no_standalone_role_label`, `sessions.panel_width_expected`, `sessions.focus_composer_hidden`, `sessions.focus_background_clean`, `sessions.current_marker`
  - add-provider：`add_provider.panel_vertical_borders_aligned`, `add_provider.focus_composer_hidden`, `add_provider.focus_chat_hidden`, `add_provider.focus_background_clean`, `add_provider.border_rows_complete`
  - streaming：`mid.no_standalone_role_label`, `final.no_standalone_role_label`, `streaming.mid_status_writing`, `streaming.mid_active_elapsed`
  - error：`main.no_standalone_role_label`
  - wide：`conversation.no_standalone_role_label`, `wide.text_columns_aligned`
  - help：`help.commands_listed`, `help.command_desc_column`
  - input-edit：`input_edit.typed_text_column`
- PASS→FAIL（回归）：2 项。
  - slash-menu：`slash.all_commands_listed`, `slash.each_command_once`
- 仅 after 求值的断言（baseline 因边框/证据缺失提前返回，或本相位新增）：29 项。
  - empty：`empty.footer_workspace_only`, `main.separator_single_header_rule`, `main.separator_header_rule_geometry`
  - conversation：`main.separator_single_header_rule`, `main.separator_header_rule_geometry`, `conversation.footer_metrics`
  - models-panel：`models.panel_width_fits`, `models.panel_width_expected`, `models.panel_rows_not_overlapping`
  - provider-panel：`provider.panel_height_fits_screen`, `provider.panel_vertical_borders_aligned`, `provider.panel_width_fits`, `provider.panel_width_expected`, `provider.panel_rows_not_overlapping`, `provider.no_double_border_overlap`
  - add-provider：`add_provider.panel_width_fits`, `add_provider.panel_width_expected`, `add_provider.panel_rows_not_overlapping`
  - error：`error.footer_metrics`
  - narrow：`empty.separator_single_header_rule`, `empty.separator_header_rule_geometry`
  - wide：`empty.separator_single_header_rule`, `empty.separator_header_rule_geometry`
  - scrolled：`scrolled.metrics_footer_before`, `scrolled.metrics_footer_after`
  - help：`main.separator_single_header_rule`, `main.separator_header_rule_geometry`
  - cost：`main.separator_single_header_rule`, `main.separator_header_rule_geometry`
- 仅 baseline 求值的断言：14 项。
  - empty：`empty.no_error_status`, `main.separator_two_rules`
  - conversation：`main.separator_two_rules`, `conversation.footer_ready`
  - models-panel：`models.panel_above_composer`
  - sessions-panel：`sessions.panel_above_composer`
  - add-provider：`add_provider.panel_above_composer`
  - error：`error.footer_failed`
  - narrow：`empty.separator_two_rules`
  - wide：`empty.separator_two_rules`
  - scrolled：`scrolled.hint_absent_before`, `scrolled.hint_present_after`
  - help：`main.separator_two_rules`
  - cost：`main.separator_two_rules`

| scenario | baseline pass/total | after pass/total | fixed | regressed |
| --- | --- | --- | --- | --- |
| empty | 11/15 | 15/16 | 2 | 0 |
| conversation | 14/19 | 20/20 | 4 | 0 |
| slash-menu | 12/14 | 11/14 | 1 | 2 |
| models-panel | 13/20 | 22/22 | 7 | 0 |
| provider-panel | 10/15 | 21/21 | 5 | 0 |
| sessions-panel | 18/23 | 22/22 | 5 | 0 |
| add-provider | 13/18 | 20/20 | 5 | 0 |
| streaming | 14/18 | 18/18 | 4 | 0 |
| error | 13/14 | 14/14 | 1 | 0 |
| narrow | 19/20 | 21/21 | 0 | 0 |
| wide | 18/21 | 22/22 | 2 | 0 |
| scrolled | 18/19 | 19/19 | 0 | 0 |
| help | 9/12 | 13/13 | 2 | 0 |
| cost | 10/11 | 12/12 | 0 | 0 |
| ctrl-c | 15/15 | 15/15 | 0 | 0 |
| input-edit | 86/87 | 87/87 | 1 | 0 |
| non-tty | 4/4 | 4/4 | 0 | 0 |
| cli-modes | 9/9 | 9/9 | 0 | 0 |

## Scenarios

### empty (120x34)

- evidence: `empty/`
- screenshots: [main.png](empty/main.png)
- checks: 15 pass / 1 fail

| result | check | evidence |
| --- | --- | --- |
| PASS | `main.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `main.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `main.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=117 cols=120 :: pony · pony-agent mock-alpha |
| PASS | `main.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `main.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/empty/main.ansi size=3540 |
| PASS | `main.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/empty/main.png size=138735 |
| PASS | `main.header` — header shows brand + workspace | row 0 :: pony · pony-agent mock-alpha |
| PASS | `main.input_padded_from_left_edge` — input prompt has left padding (>0 cols from terminal edge) | row 30 prompt_col=3 :: › 写下任务，或输入 / 查看命令 |
| PASS | `main.input_placeholder` — composer shows placeholder guidance when empty | row 30 :: › 写下任务，或输入 / 查看命令 |
| PASS | `empty.guidance` — empty state shows onboarding guidance | 描述一个任务开始，或输入命令： |
| PASS | `empty.footer_workspace_only` — empty footer shows the workspace only: no metrics segments before the first turn (v3 R11: no value -> no segment, and no '—' placeholder) | footer :: /home/dm/pony-agent |
| PASS | `main.separator_single_header_rule` — exactly one full ▁ rule remains: the header hairline (composer top rule removed) | rule_rows=[1] |
| PASS | `main.separator_header_rule_geometry` — header rule lives on row 1, starts at content column 3 and spans 114 cols | row=1 start=3 expected_start=3 width=114 expected_width=114 |
| PASS | `main.composer_cursor_block` — first placeholder glyph is a reverse-video block (accent bg / canvas-colored glyph) | row=30 col=5 fg=(18, 16, 14) bg=(198, 167, 126) canvas=(18, 16, 14) ink=(232, 229, 224) |
| PASS | `empty.placeholder_column` — placeholder starts at the shared text column 5 | placeholder_col=5 |
| FAIL | `empty.command_desc_column` — command descriptions start at column 16 (same table geometry everywhere) | cols={'/resume': 16, '/provider': 16, '/models': 16, '/clear': None, '/cost': None, '/help': None, '/quit': None} expected=16 missing=['/clear'] |

### conversation (120x34)

- evidence: `conversation/`
- screenshots: [main.png](conversation/main.png)
- checks: 20 pass / 0 fail

| result | check | evidence |
| --- | --- | --- |
| PASS | `main.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `main.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `main.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=117 cols=120 :: pony · pony-agent mock-alpha |
| PASS | `main.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `main.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/conversation/main.ansi size=2971 |
| PASS | `main.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/conversation/main.png size=147380 |
| PASS | `main.header` — header shows brand + workspace | row 0 :: pony · pony-agent mock-alpha |
| PASS | `main.input_padded_from_left_edge` — input prompt has left padding (>0 cols from terminal edge) | row 30 prompt_col=3 :: › 写下任务，或输入 / 查看命令 |
| PASS | `main.input_placeholder` — composer shows placeholder guidance when empty | row 30 :: › 写下任务，或输入 / 查看命令 |
| PASS | `conversation.user_message_visible` — user prompt is rendered in the transcript | row 4 :: ▌ 读取 package.json 并总结 |
| PASS | `conversation.user_role_rail` — user message is distinguished by a rail/block, not a text label | row 3 :: ▌ |
| PASS | `conversation.tool_line_visible` — tool activity line (Read) is visible with a status glyph | row 9 :: ✓ 读取 (Read) · 收集路径 package.json 的上下文。 |
| PASS | `conversation.final_answer_visible` — final assistant answer (mock round 2) is visible | row 11 :: 结论 |
| PASS | `conversation.text_columns_aligned` — user body / assistant body / tool text start on the same column | user_col=5 assistant_col=5 tool_col=5 |
| PASS | `conversation.aligned_columns_padded` — shared text column is inset from the terminal edge | shared_col=5 |
| PASS | `conversation.no_error_rendered` — successful turn shows no error message | no Error:/失败 rows in transcript |
| PASS | `conversation.mock_followup_has_tool_round` — mock receipt proves the final model round carried a tool result | chat_requests=1 tool_round_requests=1 roles=[['system', 'system', 'system', 'system', 'user', 'assistant', 'tool']] scenarios=['final'] |
| PASS | `main.separator_single_header_rule` — exactly one full ▁ rule remains: the header hairline (composer top rule removed) | rule_rows=[1] |
| PASS | `main.separator_header_rule_geometry` — header rule lives on row 1, starts at content column 3 and spans 114 cols | row=1 start=3 expected_start=3 width=114 expected_width=114 |
| PASS | `conversation.footer_metrics` — footer is the composer metrics line after the turn (status phrase removed by wave-39) | footer :: /home/dm/pony-agent ↑32 ↓64 · 缓存 0% · 上下文 0% |

### slash-menu (120x34)

- evidence: `slash-menu/`
- screenshots: [main.png](slash-menu/main.png)
- checks: 11 pass / 3 fail

| result | check | evidence |
| --- | --- | --- |
| PASS | `main.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `main.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `main.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=117 cols=120 :: pony · pony-agent mock-alpha |
| PASS | `main.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `main.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/slash-menu/main.ansi size=4466 |
| PASS | `main.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/slash-menu/main.png size=140116 |
| PASS | `main.input_padded_from_left_edge` — input prompt has left padding (>0 cols from terminal edge) | row 30 prompt_col=3 :: › / |
| FAIL | `slash.all_commands_listed` — slash menu lists the full command table | menu_commands=['/resume', '/provider', '/models', '/clear', '/cost', '/help', '/quit'] missing=['/quit'] |
| PASS | `slash.menu_has_top_and_bottom_rules` — menu is framed by two hairline rules | rule_rows=[19, 27] |
| PASS | `slash.menu_above_composer` — menu rows sit above the composer input row | menu_rows=[20, 21]..26 bottom_rule=27 input_row=30 |
| FAIL | `slash.each_command_once` — each command appears exactly once inside the menu region | duplicate/missing commands=['/quit'] menu_rows=7 |
| FAIL | `slash.command_desc_column` — command descriptions start at column 16 (same table geometry everywhere) | cols={'/resume': 16, '/provider': 16, '/models': 16, '/clear': 16, '/cost': 16, '/help': 16, '/quit': None} expected=16 missing=['/quit'] |
| PASS | `slash.resume_once_whole_screen` — '/resume' appears exactly once on the whole screen while the menu is open | /resume occurrences=1 |
| PASS | `slash.selected_row_marker` — selected command row carries the selection marker | row 20 :: ▏ /resume 选择并恢复历史会话 |

### models-panel (120x34)

- evidence: `models-panel/`
- screenshots: [main.png](models-panel/main.png)
- checks: 22 pass / 0 fail

| result | check | evidence |
| --- | --- | --- |
| PASS | `main.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `main.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `main.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=117 cols=120 :: pony · pony-agent mock-alpha |
| PASS | `main.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `main.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/models-panel/main.ansi size=4273 |
| PASS | `main.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/models-panel/main.png size=87513 |
| PASS | `models.title_counter` — model panel title carries the '模型 · 1/N' position counter | title_row :: │ 模型 · 1/20 ↑↓ 选择 · ↵ 确认 · r 刷新 · esc 取消 │ |
| PASS | `models.border_box` — exactly one top and one bottom panel border, top above bottom | top_rows=[9] bottom_rows=[24] |
| PASS | `models.panel_height_fits_screen` — panel top/bottom borders are both on screen and the panel height fits | top=9 bottom=24 height=16 pane_rows=34 |
| PASS | `models.panel_vertical_borders_aligned` — every panel interior row shares the same left/right border column | left_cols=[18] right_cols=[101] rows=14 |
| PASS | `models.panel_width_fits` — panel width fits inside the terminal | panel_cols=18..101 width=84 terminal_cols=120 |
| PASS | `models.panel_width_expected` — panel width matches the design token (84) and is centered | width=84 expected=84 left=18 right_margin=18 |
| PASS | `models.panel_rows_not_overlapping` — panel rows are consecutive (no overlapping/duplicated rows) | interior_rows=[10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23] |
| PASS | `models.no_double_border_overlap` — no row paints more than one panel's worth of vertical borders | max '│' per row=2 |
| PASS | `models.focus_composer_hidden` — focus mode hides the global composer (prompt may only live inside the modal) | prompt_rows_outside_panel=[] |
| PASS | `models.focus_chat_hidden` — focus mode hides chat and splash (no transcript/logo residue) | guidance=False logo_tag=False block_glyphs=False |
| PASS | `models.focus_background_clean` — rows outside header + panel are completely blank (no half-cell residue) | dirty_rows=[] |
| PASS | `models.border_rows_complete` — every panel row carries both border columns at the same display columns | left=18 right=101 broken_rows=[] |
| PASS | `models.rows_rendered` — panel renders at least the capped 12 model rows | model_rows=12 first=['mock-alpha', 'mock-beta'] last=['mock-model-12'] |
| PASS | `models.rows_unique_and_consecutive` — model rows are unique and occupy consecutive screen rows | ids=12 unique=12 rows=[12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23] |
| PASS | `models.no_fetch_failure` — remote /models fetch succeeded (no failure row) | no fetch failure row |
| PASS | `models.selected_marker` — selected model row carries the selection marker | selection marker '▏' present |

### provider-panel (120x34)

- evidence: `provider-panel/`
- screenshots: [main.png](provider-panel/main.png)
- checks: 21 pass / 0 fail

| result | check | evidence |
| --- | --- | --- |
| PASS | `main.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `main.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `main.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=117 cols=120 :: pony · pony-agent mock-alpha |
| PASS | `main.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `main.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/provider-panel/main.ansi size=2210 |
| PASS | `main.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/provider-panel/main.png size=46699 |
| PASS | `provider.title` — provider panel title is visible | │ 提供商 ↑↓ 选择 · ↵ 确认 · a 新增 · esc 取消 │ |
| PASS | `provider.fixture_listed` — fixture provider is listed | │▏ FakeProvider openai · 当前 │ |
| PASS | `provider.add_hint` — panel advertises the 'a 新增' shortcut | │ 提供商 ↑↓ 选择 · ↵ 确认 · a 新增 · esc 取消 │ |
| PASS | `provider.border_box` — exactly one top and one bottom panel border, top above bottom | top_rows=[14] bottom_rows=[19] |
| PASS | `provider.panel_height_fits_screen` — panel top/bottom borders are both on screen and the panel height fits | top=14 bottom=19 height=6 pane_rows=34 |
| PASS | `provider.panel_vertical_borders_aligned` — every panel interior row shares the same left/right border column | left_cols=[18] right_cols=[101] rows=4 |
| PASS | `provider.panel_width_fits` — panel width fits inside the terminal | panel_cols=18..101 width=84 terminal_cols=120 |
| PASS | `provider.panel_width_expected` — panel width matches the design token (84) and is centered | width=84 expected=84 left=18 right_margin=18 |
| PASS | `provider.panel_rows_not_overlapping` — panel rows are consecutive (no overlapping/duplicated rows) | interior_rows=[15, 16, 17, 18] |
| PASS | `provider.no_double_border_overlap` — no row paints more than one panel's worth of vertical borders | max '│' per row=2 |
| PASS | `provider.focus_composer_hidden` — focus mode hides the global composer (prompt may only live inside the modal) | prompt_rows_outside_panel=[] |
| PASS | `provider.focus_chat_hidden` — focus mode hides chat and splash (no transcript/logo residue) | guidance=False logo_tag=False block_glyphs=False |
| PASS | `provider.focus_background_clean` — rows outside header + panel are completely blank (no half-cell residue) | dirty_rows=[] |
| PASS | `provider.border_rows_complete` — every panel row carries both border columns at the same display columns | left=18 right=101 broken_rows=[] |
| PASS | `provider.current_marker` — active provider row is marked as current | │▏ FakeProvider openai · 当前 │ |

### sessions-panel (120x34)

- evidence: `sessions-panel/`
- screenshots: [main.png](sessions-panel/main.png)
- checks: 22 pass / 0 fail

| result | check | evidence |
| --- | --- | --- |
| PASS | `main.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `main.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `main.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=117 cols=120 :: pony · pony-agent mock-alpha |
| PASS | `main.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `main.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/sessions-panel/main.ansi size=2211 |
| PASS | `main.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/sessions-panel/main.png size=51216 |
| PASS | `sessions.title` — session panel title is visible | │ 会话 ↑↓ 选择 · ↵ 确认 · esc 取消 │ |
| PASS | `sessions.prior_conversation_listed` — the conversation just completed is offered for resume | │▏ 读取 package.json 并总结 9632973b · 当前 │ |
| PASS | `sessions.session_id_visible` — session row carries a short session id | │▏ 读取 package.json 并总结 9632973b · 当前 │ |
| PASS | `sessions.not_empty` — panel does not show the empty-sessions placeholder | no '暂无历史会话' placeholder |
| PASS | `sessions.border_box` — exactly one top and one bottom panel border, top above bottom | top_rows=[14] bottom_rows=[19] |
| PASS | `sessions.panel_height_fits_screen` — panel top/bottom borders are both on screen and the panel height fits | top=14 bottom=19 height=6 pane_rows=34 |
| PASS | `sessions.panel_vertical_borders_aligned` — every panel interior row shares the same left/right border column | left_cols=[18] right_cols=[101] rows=4 |
| PASS | `sessions.panel_width_fits` — panel width fits inside the terminal | panel_cols=18..101 width=84 terminal_cols=120 |
| PASS | `sessions.panel_width_expected` — panel width matches the design token (84) and is centered | width=84 expected=84 left=18 right_margin=18 |
| PASS | `sessions.panel_rows_not_overlapping` — panel rows are consecutive (no overlapping/duplicated rows) | interior_rows=[15, 16, 17, 18] |
| PASS | `sessions.no_double_border_overlap` — no row paints more than one panel's worth of vertical borders | max '│' per row=2 |
| PASS | `sessions.focus_composer_hidden` — focus mode hides the global composer (prompt may only live inside the modal) | prompt_rows_outside_panel=[] |
| PASS | `sessions.focus_chat_hidden` — focus mode hides chat and splash (no transcript/logo residue) | guidance=False logo_tag=False block_glyphs=False |
| PASS | `sessions.focus_background_clean` — rows outside header + panel are completely blank (no half-cell residue) | dirty_rows=[] |
| PASS | `sessions.border_rows_complete` — every panel row carries both border columns at the same display columns | left=18 right=101 broken_rows=[] |
| PASS | `sessions.current_marker` — sessions panel marks the active session | │▏ 读取 package.json 并总结 9632973b · 当前 │ |

### add-provider (120x34)

- evidence: `add-provider/`
- screenshots: [main.png](add-provider/main.png)
- checks: 20 pass / 0 fail

| result | check | evidence |
| --- | --- | --- |
| PASS | `main.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `main.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `main.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=117 cols=120 :: pony · pony-agent mock-alpha |
| PASS | `main.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `main.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/add-provider/main.ansi size=2518 |
| PASS | `main.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/add-provider/main.png size=43590 |
| PASS | `add_provider.title` — add-provider modal title is visible | │ 新增提供商 回车 下一步 · esc 取消 │ |
| PASS | `add_provider.step_indicator` — 3-step wizard shows the name step | │ 1 / 3 name Provider 名称 │ |
| PASS | `add_provider.input_focused` — modal input row renders the caret prompt | row 16 :: │ › ▏ │ |
| PASS | `add_provider.border_box` — exactly one top and one bottom panel border, top above bottom | top_rows=[12] bottom_rows=[20] |
| PASS | `add_provider.panel_height_fits_screen` — panel top/bottom borders are both on screen and the panel height fits | top=12 bottom=20 height=9 pane_rows=34 |
| PASS | `add_provider.panel_vertical_borders_aligned` — every panel interior row shares the same left/right border column | left_cols=[28] right_cols=[91] rows=7 |
| PASS | `add_provider.panel_width_fits` — panel width fits inside the terminal | panel_cols=28..91 width=64 terminal_cols=120 |
| PASS | `add_provider.panel_width_expected` — panel width matches the design token (64) and is centered | width=64 expected=64 left=28 right_margin=28 |
| PASS | `add_provider.panel_rows_not_overlapping` — panel rows are consecutive (no overlapping/duplicated rows) | interior_rows=[13, 14, 15, 16, 17, 18, 19] |
| PASS | `add_provider.no_double_border_overlap` — no row paints more than one panel's worth of vertical borders | max '│' per row=2 |
| PASS | `add_provider.focus_composer_hidden` — focus mode hides the global composer (prompt may only live inside the modal) | prompt_rows_outside_panel=[] |
| PASS | `add_provider.focus_chat_hidden` — focus mode hides chat and splash (no transcript/logo residue) | guidance=False logo_tag=False block_glyphs=False |
| PASS | `add_provider.focus_background_clean` — rows outside header + panel are completely blank (no half-cell residue) | dirty_rows=[] |
| PASS | `add_provider.border_rows_complete` — every panel row carries both border columns at the same display columns | left=28 right=91 broken_rows=[] |

### streaming (120x34)

- evidence: `streaming/`
- screenshots: [final.png](streaming/final.png), [mid.png](streaming/mid.png)
- checks: 18 pass / 0 fail

| result | check | evidence |
| --- | --- | --- |
| PASS | `mid.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `mid.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `mid.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=117 cols=120 :: pony · pony-agent mock-alpha |
| PASS | `mid.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `mid.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/streaming/mid.ansi size=2249 |
| PASS | `mid.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/streaming/mid.png size=82926 |
| PASS | `final.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `final.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `final.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=117 cols=120 :: pony · pony-agent mock-alpha |
| PASS | `final.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `final.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/streaming/final.ansi size=2700 |
| PASS | `final.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/streaming/final.png size=146346 |
| PASS | `streaming.mid_status_writing` — streaming active line shows the writing state while the footer is the metrics row | active_row :: • 写入中 0秒 \| footer :: /home/dm/pony-agent 上下文 0% |
| PASS | `streaming.mid_active_elapsed` — active line shows elapsed time next to the writing state | active_row :: • 写入中 0秒 |
| PASS | `streaming.mid_partial_body` — partial streamed markdown body is already rendered | 正在分析 |
| PASS | `streaming.mid_not_finished` — mid-stream capture is not the completed answer yet (footer is the metrics row) | footer :: /home/dm/pony-agent 上下文 0% |
| PASS | `streaming.final_complete` — final capture contains the terminal answer and the metrics footer | footer :: /home/dm/pony-agent ↑32 ↓64 · 缓存 0% · 上下文 0% |
| PASS | `streaming.final_not_writing` — writing status disappeared once the stream finished | footer :: /home/dm/pony-agent ↑32 ↓64 · 缓存 0% · 上下文 0% |

### error (120x34)

- evidence: `error/`
- screenshots: [main.png](error/main.png)
- checks: 14 pass / 0 fail

| result | check | evidence |
| --- | --- | --- |
| PASS | `main.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `main.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `main.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=117 cols=120 :: pony · pony-agent mock-alpha |
| PASS | `main.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `main.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/error/main.ansi size=2132 |
| PASS | `main.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/error/main.png size=68175 |
| PASS | `main.input_padded_from_left_edge` — input prompt has left padding (>0 cols from terminal edge) | row 30 prompt_col=3 :: › 写下任务，或输入 / 查看命令 |
| PASS | `main.input_placeholder` — composer shows placeholder guidance when empty | row 30 :: › 写下任务，或输入 / 查看命令 |
| PASS | `error.message_visible` — provider 401 is surfaced as a visible error message | row 7 :: ✕ Error: {"error": {"message": "mock 401 unauthorized: invalid api key (audit scenario)", "type": "invalid_request |
| PASS | `error.error_glyph` — error row carries the failure glyph | row 7 :: ✕ Error: {"error": {"message": "mock 401 unauthorized: invalid api key (audit scenario)", "type": "invalid_request |
| PASS | `error.reason_included` — error text names the 401/unauthorized cause | ▌ 401 认证测试 |
| PASS | `error.footer_metrics` — footer stays the metrics row during a failed turn (no usage -> 上下文 only; failure in transcript) | footer :: /home/dm/pony-agent 上下文 0% |
| PASS | `error.no_fake_answer` — no assistant answer is fabricated for the failed turn | no 结论 row in failed capture |
| PASS | `error.mock_401_receipt` — mock receipt proves the 401 scenario was requested | scenarios=['401', '401'] |

### narrow (60x20)

- evidence: `narrow/`
- screenshots: [conversation.png](narrow/conversation.png), [empty.png](narrow/empty.png)
- checks: 21 pass / 0 fail

| result | check | evidence |
| --- | --- | --- |
| PASS | `empty.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `empty.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `empty.within_terminal_width` — every captured row display width <= 60 cols | widest row 0 width=57 cols=60 :: pony · pony-agent mock-alpha |
| PASS | `empty.capture_rows` — capture has exactly pane height rows (20) | rows=20 pane_rows=20 |
| PASS | `empty.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/narrow/empty.ansi size=1362 |
| PASS | `empty.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/narrow/empty.png size=109072 |
| PASS | `empty.input_padded_from_left_edge` — input prompt has left padding (>0 cols from terminal edge) | row 16 prompt_col=3 :: › 写下任务，或输入 / 查看命令 |
| PASS | `empty.input_placeholder` — composer shows placeholder guidance when empty | row 16 :: › 写下任务，或输入 / 查看命令 |
| PASS | `conversation.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `conversation.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `conversation.within_terminal_width` — every captured row display width <= 60 cols | widest row 0 width=57 cols=60 :: pony · pony-agent mock-alpha |
| PASS | `conversation.capture_rows` — capture has exactly pane height rows (20) | rows=20 pane_rows=20 |
| PASS | `conversation.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/narrow/conversation.ansi size=1711 |
| PASS | `conversation.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/narrow/conversation.png size=99729 |
| PASS | `conversation.input_padded_from_left_edge` — input prompt has left padding (>0 cols from terminal edge) | row 16 prompt_col=3 :: › 写下任务，或输入 / 查看命令 |
| PASS | `conversation.input_placeholder` — composer shows placeholder guidance when empty | row 16 :: › 写下任务，或输入 / 查看命令 |
| PASS | `empty.header` — header shows brand + workspace | row 0 :: pony · pony-agent mock-alpha |
| PASS | `empty.separator_single_header_rule` — exactly one full ▁ rule remains: the header hairline (composer top rule removed) | rule_rows=[1] |
| PASS | `empty.separator_header_rule_geometry` — header rule lives on row 1, starts at content column 3 and spans 54 cols | row=1 start=3 expected_start=3 width=54 expected_width=54 |
| PASS | `narrow.empty_guidance` — narrow empty state keeps onboarding guidance on screen | 描述一个任务开始，或输入命令： |
| PASS | `narrow.conversation_visible` — narrow conversation still shows the tail of the final answer | package.json 显示这是 pony-agent，版本 0.1.125。 |

### wide (160x40)

- evidence: `wide/`
- screenshots: [conversation.png](wide/conversation.png), [empty.png](wide/empty.png)
- checks: 22 pass / 0 fail

| result | check | evidence |
| --- | --- | --- |
| PASS | `empty.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `empty.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `empty.within_terminal_width` — every captured row display width <= 160 cols | widest row 0 width=140 cols=160 :: pony · pony-agent mock-alpha |
| PASS | `empty.capture_rows` — capture has exactly pane height rows (40) | rows=40 pane_rows=40 |
| PASS | `empty.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/wide/empty.ansi size=3945 |
| PASS | `empty.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/wide/empty.png size=147780 |
| PASS | `empty.input_padded_from_left_edge` — input prompt has left padding (>0 cols from terminal edge) | row 36 prompt_col=20 :: › 写下任务，或输入 / 查看命令 |
| PASS | `empty.input_placeholder` — composer shows placeholder guidance when empty | row 36 :: › 写下任务，或输入 / 查看命令 |
| PASS | `conversation.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `conversation.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `conversation.within_terminal_width` — every captured row display width <= 160 cols | widest row 0 width=140 cols=160 :: pony · pony-agent mock-alpha |
| PASS | `conversation.capture_rows` — capture has exactly pane height rows (40) | rows=40 pane_rows=40 |
| PASS | `conversation.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/wide/conversation.ansi size=3400 |
| PASS | `conversation.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/wide/conversation.png size=157369 |
| PASS | `conversation.input_padded_from_left_edge` — input prompt has left padding (>0 cols from terminal edge) | row 36 prompt_col=20 :: › 写下任务，或输入 / 查看命令 |
| PASS | `conversation.input_placeholder` — composer shows placeholder guidance when empty | row 36 :: › 写下任务，或输入 / 查看命令 |
| PASS | `empty.header` — header shows brand + workspace | row 0 :: pony · pony-agent mock-alpha |
| PASS | `empty.separator_single_header_rule` — exactly one full ▁ rule remains: the header hairline (composer top rule removed) | rule_rows=[1] |
| PASS | `empty.separator_header_rule_geometry` — header rule lives on row 1, starts at content column 20 and spans 120 cols | row=1 start=20 expected_start=20 width=120 expected_width=120 |
| PASS | `wide.header_full_metadata` — wide header keeps workspace and model metadata on one line | row 0 :: pony · pony-agent mock-alpha |
| PASS | `wide.conversation_visible` — wide conversation shows the final answer | 结论 |
| PASS | `wide.text_columns_aligned` — at 160x40 the three text streams still share one column | user_col=22 assistant_col=22 tool_col=22 |

### scrolled (120x34)

- evidence: `scrolled/`
- screenshots: [after.png](scrolled/after.png), [before.png](scrolled/before.png)
- checks: 19 pass / 0 fail

| result | check | evidence |
| --- | --- | --- |
| PASS | `before.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `before.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `before.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=117 cols=120 :: pony · pony-agent mock-alpha |
| PASS | `before.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `before.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/scrolled/before.ansi size=7059 |
| PASS | `before.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/scrolled/before.png size=524608 |
| PASS | `after.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `after.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `after.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=117 cols=120 :: pony · pony-agent mock-alpha |
| PASS | `after.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `after.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/scrolled/after.ansi size=9023 |
| PASS | `after.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/scrolled/after.png size=689745 |
| PASS | `after.input_padded_from_left_edge` — input prompt has left padding (>0 cols from terminal edge) | row 30 prompt_col=3 :: › 写下任务，或输入 / 查看命令 |
| PASS | `after.input_placeholder` — composer shows placeholder guidance when empty | row 30 :: › 写下任务，或输入 / 查看命令 |
| PASS | `scrolled.bottom_before` — before scrolling the transcript is pinned to the newest line | 这是最后一行，滚动到底部时应完整可见。 |
| PASS | `scrolled.metrics_footer_before` — footer is the metrics line while pinned to bottom (scroll-up hint removed by wave-39) | footer :: /home/dm/pony-agent ↑32 ↓64 · 缓存 0% · 上下文 0% |
| PASS | `scrolled.after_shows_earlier_rows` — PageUp reveals earlier lines and hides the newest ones | · 第 14 行：这是一段用于验证滚动与折行的中文正文，包含 inline code 与 加粗 片段。 |
| PASS | `scrolled.metrics_footer_after` — footer stays the metrics line after scrolling up (the old '已上翻' hint row is gone) | footer :: /home/dm/pony-agent ↑32 ↓64 · 缓存 0% · 上下文 0% |
| PASS | `scrolled.offset_moved_up` — first visible content row moved up by at least 8 terminal rows | first_visible_before=第23行 first_visible_after=第13行 |

### help (120x34)

- evidence: `help/`
- screenshots: [main.png](help/main.png)
- checks: 13 pass / 0 fail

| result | check | evidence |
| --- | --- | --- |
| PASS | `main.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `main.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `main.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=117 cols=120 :: pony · pony-agent mock-alpha |
| PASS | `main.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `main.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/help/main.ansi size=1874 |
| PASS | `main.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/help/main.png size=136280 |
| PASS | `main.input_padded_from_left_edge` — input prompt has left padding (>0 cols from terminal edge) | row 30 prompt_col=3 :: › 写下任务，或输入 / 查看命令 |
| PASS | `main.input_placeholder` — composer shows placeholder guidance when empty | row 30 :: › 写下任务，或输入 / 查看命令 |
| PASS | `help.commands_listed` — /help renders the command reference in the transcript | row 3 :: · 命令列表 missing=[] |
| PASS | `main.separator_single_header_rule` — exactly one full ▁ rule remains: the header hairline (composer top rule removed) | rule_rows=[1] |
| PASS | `main.separator_header_rule_geometry` — header rule lives on row 1, starts at content column 3 and spans 114 cols | row=1 start=3 expected_start=3 width=114 expected_width=114 |
| PASS | `help.command_desc_column` — command descriptions start at column 16 (same table geometry everywhere) | cols={'/resume': 16, '/provider': 16, '/models': 16, '/clear': 16, '/cost': 16, '/help': 16, '/quit': 16} expected=16 missing=[] |
| PASS | `help.system_message_glyph` — help output is a system row (bullet glyph), not a user message | row 3 :: · 命令列表 |

### cost (120x34)

- evidence: `cost/`
- screenshots: [main.png](cost/main.png)
- checks: 12 pass / 0 fail

| result | check | evidence |
| --- | --- | --- |
| PASS | `main.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `main.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `main.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=117 cols=120 :: pony · pony-agent mock-alpha |
| PASS | `main.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `main.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/cost/main.ansi size=1379 |
| PASS | `main.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/cost/main.png size=53751 |
| PASS | `main.input_padded_from_left_edge` — input prompt has left padding (>0 cols from terminal edge) | row 30 prompt_col=3 :: › 写下任务，或输入 / 查看命令 |
| PASS | `main.input_placeholder` — composer shows placeholder guidance when empty | row 30 :: › 写下任务，或输入 / 查看命令 |
| PASS | `cost.model_line` — /cost shows the active model | row :: · 模型 mock-alpha |
| PASS | `main.separator_single_header_rule` — exactly one full ▁ rule remains: the header hairline (composer top rule removed) | rule_rows=[1] |
| PASS | `main.separator_header_rule_geometry` — header rule lives on row 1, starts at content column 3 and spans 114 cols | row=1 start=3 expected_start=3 width=114 expected_width=114 |
| PASS | `cost.gateway_line` — /cost shows the gateway endpoint | row :: 网关 http://10.42.0.67:8080 |

### ctrl-c (120x34)

- evidence: `ctrl-c/`
- screenshots: [exit.png](ctrl-c/exit.png), [shell.png](ctrl-c/shell.png)
- checks: 15 pass / 0 fail

| result | check | evidence |
| --- | --- | --- |
| PASS | `exit.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `exit.within_terminal_width` — every captured row display width <= 120 cols | widest row 1 width=120 cols=120 :: -u NO_COLOR XDG_CONFIG_HOME=/tmp/tui-audit.t5Z5Yt/iso-ctrl-c/xdg-config XDG_DATA_HOME=/tmp/tui-audit.t5Z5Yt/iso-ctrl-c/x |
| PASS | `exit.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `exit.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/ctrl-c/exit.ansi size=477 |
| PASS | `exit.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/ctrl-c/exit.png size=74478 |
| PASS | `shell.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `shell.within_terminal_width` — every captured row display width <= 120 cols | widest row 1 width=120 cols=120 :: -u NO_COLOR XDG_CONFIG_HOME=/tmp/tui-audit.t5Z5Yt/iso-ctrl-c/xdg-config XDG_DATA_HOME=/tmp/tui-audit.t5Z5Yt/iso-ctrl-c/x |
| PASS | `shell.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `shell.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/ctrl-c/shell.ansi size=532 |
| PASS | `shell.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/ctrl-c/shell.png size=86306 |
| PASS | `ctrl_c.process_exited` — Ctrl-C terminates the TUI process with a normal exit status | row :: UTF-8 /home/dm/pony-agent/target/debug/pony; echo AUDIT_EXIT=$? code=0 |
| PASS | `ctrl_c.terminal_restored` — alternate screen is left and the pane stops painting the TUI | tui_band_present=False placeholder_present=False |
| PASS | `ctrl_c.shell_alive` — the same tmux pane still runs commands after exit | bash-5.2$ echo AUDIT_SHELL_ALIVE=ok |
| PASS | `ctrl_c.pane_command_is_shell` — pane foreground command is a shell again, not pony | pane_current_command='bash' |
| PASS | `ctrl_c.no_panic` — shutdown path prints no panic | no panic tokens in shutdown captures |

### input-edit (120x34)

- evidence: `input-edit/`
- screenshots: [abc.png](input-edit/abc.png), [backspace.png](input-edit/backspace.png), [ctrl_u_cleared.png](input-edit/ctrl_u_cleared.png), [draft.png](input-edit/draft.png), [draft_cleared.png](input-edit/draft_cleared.png), [esc_cleared.png](input-edit/esc_cleared.png), [exit.png](input-edit/exit.png), [inserted.png](input-edit/inserted.png), [multiline.png](input-edit/multiline.png), [shell.png](input-edit/shell.png), [tab_complete.png](input-edit/tab_complete.png), [word_cleared.png](input-edit/word_cleared.png)
- checks: 87 pass / 0 fail

| result | check | evidence |
| --- | --- | --- |
| PASS | `abc.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `abc.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `abc.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=117 cols=120 :: pony · pony-agent mock-alpha |
| PASS | `abc.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `abc.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/input-edit/abc.ansi size=3544 |
| PASS | `abc.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/input-edit/abc.png size=128630 |
| PASS | `input_edit.abc` — typed text lands in the composer | buffer='abc' expected='abc' |
| PASS | `inserted.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `inserted.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `inserted.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=117 cols=120 :: pony · pony-agent mock-alpha |
| PASS | `inserted.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `inserted.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/input-edit/inserted.ansi size=3544 |
| PASS | `inserted.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/input-edit/inserted.png size=129431 |
| PASS | `input_edit.inserted` — Left + X inserts in the middle without losing the tail | buffer='abXc' expected='abXc' |
| PASS | `backspace.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `backspace.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `backspace.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=117 cols=120 :: pony · pony-agent mock-alpha |
| PASS | `backspace.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `backspace.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/input-edit/backspace.ansi size=3544 |
| PASS | `backspace.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/input-edit/backspace.png size=128980 |
| PASS | `input_edit.backspace` — Right + Backspace deletes the character at the end | buffer='abX' expected='abX' |
| PASS | `word_cleared.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `word_cleared.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `word_cleared.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=117 cols=120 :: pony · pony-agent mock-alpha |
| PASS | `word_cleared.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `word_cleared.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/input-edit/word_cleared.ansi size=3540 |
| PASS | `word_cleared.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/input-edit/word_cleared.png size=138735 |
| PASS | `input_edit.word_cleared` — Ctrl+W deletes the whole word | buffer='' expected='' |
| PASS | `tab_complete.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `tab_complete.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `tab_complete.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=117 cols=120 :: pony · pony-agent mock-alpha |
| PASS | `tab_complete.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `tab_complete.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/input-edit/tab_complete.ansi size=3541 |
| PASS | `tab_complete.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/input-edit/tab_complete.png size=68073 |
| PASS | `input_edit.tab_complete` — Tab completes '/mo' to '/models' | buffer='/models' expected='/models' |
| PASS | `ctrl_u_cleared.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `ctrl_u_cleared.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `ctrl_u_cleared.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=117 cols=120 :: pony · pony-agent mock-alpha |
| PASS | `ctrl_u_cleared.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `ctrl_u_cleared.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/input-edit/ctrl_u_cleared.ansi size=3540 |
| PASS | `ctrl_u_cleared.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/input-edit/ctrl_u_cleared.png size=138735 |
| PASS | `input_edit.ctrl_u_cleared` — Ctrl+U clears the composer | buffer='' expected='' |
| PASS | `multiline.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `multiline.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `multiline.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=117 cols=120 :: pony · pony-agent mock-alpha |
| PASS | `multiline.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `multiline.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/input-edit/multiline.ansi size=3717 |
| PASS | `multiline.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/input-edit/multiline.png size=129637 |
| PASS | `input_edit.multiline` — Ctrl+J inserts a newline (two visual rows) | buffer='line1\nline2' expected='line1\nline2' |
| PASS | `esc_cleared.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `esc_cleared.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `esc_cleared.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=117 cols=120 :: pony · pony-agent mock-alpha |
| PASS | `esc_cleared.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `esc_cleared.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/input-edit/esc_cleared.ansi size=3540 |
| PASS | `esc_cleared.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/input-edit/esc_cleared.png size=138735 |
| PASS | `input_edit.esc_cleared` — Esc clears the composer | buffer='' expected='' |
| PASS | `draft.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `draft.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `draft.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=117 cols=120 :: pony · pony-agent mock-alpha |
| PASS | `draft.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `draft.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/input-edit/draft.ansi size=3544 |
| PASS | `draft.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/input-edit/draft.png size=128666 |
| PASS | `input_edit.draft` — draft text is present before Ctrl-C | buffer='draft' expected='draft' |
| PASS | `input_edit.typed_text_column` — typed text starts at the same column as the empty-state placeholder (5) | typed_col=5 |
| PASS | `input_edit.multiline_visual_rows` — multi-line draft occupies two distinct composer rows | composer_rows=2 buffer='line1\nline2' |
| PASS | `draft_cleared.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `draft_cleared.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `draft_cleared.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=117 cols=120 :: pony · pony-agent mock-alpha |
| PASS | `draft_cleared.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `draft_cleared.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/input-edit/draft_cleared.ansi size=3540 |
| PASS | `draft_cleared.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/input-edit/draft_cleared.png size=138735 |
| PASS | `input_edit.ctrl_c_with_draft_clears_only` — Ctrl-C with a draft clears the composer instead of quitting | buffer='' exit_marker=False |
| PASS | `input_edit.ctrl_c_draft_still_running` — TUI is still painting its composer after the first Ctrl-C | › 写下任务，或输入 / 查看命令 |
| PASS | `exit.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `exit.within_terminal_width` — every captured row display width <= 120 cols | widest row 1 width=120 cols=120 :: -u NO_COLOR XDG_CONFIG_HOME=/tmp/tui-audit.t5Z5Yt/iso-input-edit/xdg-config XDG_DATA_HOME=/tmp/tui-audit.t5Z5Yt/iso-inpu |
| PASS | `exit.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `exit.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/input-edit/exit.ansi size=493 |
| PASS | `exit.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/input-edit/exit.png size=75120 |
| PASS | `shell.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `shell.within_terminal_width` — every captured row display width <= 120 cols | widest row 1 width=120 cols=120 :: -u NO_COLOR XDG_CONFIG_HOME=/tmp/tui-audit.t5Z5Yt/iso-input-edit/xdg-config XDG_DATA_HOME=/tmp/tui-audit.t5Z5Yt/iso-inpu |
| PASS | `shell.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `shell.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/input-edit/shell.ansi size=548 |
| PASS | `shell.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/input-edit/shell.png size=86822 |
| PASS | `input_edit.ctrl_c_empty_exits` — a second Ctrl-C on the empty composer exits the process normally | exit_code=0 row :: home LANG=en_US.UTF-8 /home/dm/pony-agent/target/debug/pony; echo AUDIT_EXIT=$? |
| PASS | `input_edit.terminal_restored` — alternate screen is left after exit | tui_band=False |
| PASS | `input_edit.shell_alive` — the pane still runs commands after exit | bash-5.2$ echo AUDIT_SHELL_ALIVE=ok |
| PASS | `input_edit.pane_command_is_shell` — pane foreground command is a shell again | pane_current_command='bash' |

### non-tty (80x24)

- evidence: `non-tty/`
- checks: 4 pass / 0 fail

| result | check | evidence |
| --- | --- | --- |
| PASS | `non_tty.diagnostic` — non-TTY stdin prints an explicit interactive-terminal diagnostic | pony TUI: stdin/stdout 不是交互式终端，TUI 无法启动。 |
| PASS | `non_tty.no_panic` — non-TTY launch does not panic | no panic tokens in stdout/stderr |
| PASS | `non_tty.exit_code_recorded` — exit code is a real process status (not a timeout) | exit_code=1 |
| PASS | `non_tty.tty_hint` — diagnostic points the user at a real terminal / ponycli | 请在真实终端中运行 `pony`，或使用 `ponycli`（非交互 CLI 入口）。 |

### cli-modes (80x24)

- evidence: `cli-modes/`
- checks: 9 pass / 0 fail

| result | check | evidence |
| --- | --- | --- |
| PASS | `cli.version_exit_status` — --version exits with a documented status (0) | exit_code=0 allowed=(0,) |
| PASS | `cli.version_output` — --version prints its documented payload | pony 0.1.127 · linux-x86_64 missing=[] |
| PASS | `cli.version_no_panic` — --version does not panic | no panic tokens in output |
| PASS | `cli.help_exit_status` — --help exits with a documented status (0) | exit_code=0 allowed=(0,) |
| PASS | `cli.help_output` — --help prints its documented payload | pony 0.1.127 — 交互式 TUI Agent missing=[] |
| PASS | `cli.help_no_panic` — --help does not panic | no panic tokens in output |
| PASS | `cli.doctor_exit_status` — --doctor exits with a documented status (0, or 1 when the non-TTY self-check reports a hard failure) | exit_code=1 allowed=(0, 1) |
| PASS | `cli.doctor_output` — --doctor prints its documented payload | [ok] TERM: dumb missing=[] |
| PASS | `cli.doctor_no_panic` — --doctor does not panic | no panic tokens in output |

