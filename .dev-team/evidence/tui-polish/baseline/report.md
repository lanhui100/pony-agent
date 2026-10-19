# TUI audit report — baseline

| field | value |
| --- | --- |
| binary | `/home/dm/pony-agent/tools/tui-audit/bin/pony-baseline` |
| sha256 | `53ede00e3b1cdb93f2316ff85eef6828105d8d0c445be6ffb1c34d688792cfc3` |
| mtime | `2026-10-19T00:03:57+08:00` |
| version | `pony 0.1.125 · linux-x86_64` |
| generated_at | `2026-10-18T19:46:38.418460+00:00` |
| result | **306 pass / 48 fail** across 18 scenarios |

> Baseline is the pre-refactor release binary. Its failing checks are the expected contrast evidence for the UI refactor; they are reported verbatim, never relaxed.

## Failed checks

| scenario | check | evidence |
| --- | --- | --- |
| empty | `main.separator_two_rules` | rule_rows=[] |
| empty | `main.composer_cursor_block` | placeholder not found |
| empty | `empty.placeholder_column` | placeholder_col=None |
| empty | `empty.command_desc_column` | cols={'/resume': 13, '/provider': 13, '/models': 13, '/clear': 13, '/cost': None, '/help': None, '/quit': None} expected=16 missing=[] |
| conversation | `main.no_standalone_role_label` | row 3 = ' 你' |
| conversation | `conversation.user_role_rail` | no '▌' rail found |
| conversation | `conversation.text_columns_aligned` | user_col=2 assistant_col=2 tool_col=6 |
| conversation | `conversation.aligned_columns_padded` | shared_col=2 |
| conversation | `main.separator_two_rules` | rule_rows=[] |
| slash-menu | `slash.command_desc_column` | cols={'/resume': 13, '/provider': 16, '/models': 16, '/clear': 16, '/cost': 16, '/help': 16, '/quit': 16} expected=16 missing=[] |
| slash-menu | `slash.resume_once_whole_screen` | /resume occurrences=2 |
| models-panel | `models.title_counter` | title_row :: ▄▄▄▄▄▄▄▄▄▄ │ 模型 ↑↓ 选择 · 回车 确认 · r 刷新 · esc 取消 │ |
| models-panel | `models.panel_vertical_borders_aligned` | left_cols=[31, 88] right_cols=[88] rows=22 |
| models-panel | `models.focus_composer_hidden` | prompt_rows_outside_panel=[30] |
| models-panel | `models.focus_chat_hidden` | guidance=True logo_tag=True block_glyphs=True |
| models-panel | `models.focus_background_clean` | dirty_rows=[(30, '› ▏ 输入任务，或输入 / 查看命令'), (33, '已加载 20 个模型')] |
| models-panel | `models.border_rows_complete` | left=31 right=88 broken_rows=[(17, ' ', '│')] |
| models-panel | `models.rows_unique_and_consecutive` | ids=19 unique=19 rows=[8, 9, 10, 11, 12, 13, 14, 15, 16, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27] |
| provider-panel | `provider.border_box` | top_rows=[14] bottom_rows=[] |
| provider-panel | `provider.focus_composer_hidden` | prompt_rows_outside_panel=[30] |
| provider-panel | `provider.focus_chat_hidden` | guidance=True logo_tag=True block_glyphs=True |
| provider-panel | `provider.focus_background_clean` | panel bounds missing |
| provider-panel | `provider.border_rows_complete` | missing top/bottom border |
| sessions-panel | `main.no_standalone_role_label` | row 3 = ' 你' |
| sessions-panel | `sessions.panel_width_expected` | width=58 expected=84 left=31 right_margin=31 |
| sessions-panel | `sessions.focus_composer_hidden` | prompt_rows_outside_panel=[30] |
| sessions-panel | `sessions.focus_background_clean` | dirty_rows=[(3, '你'), (4, '读取 package.json 并总结'), (7, 'pony'), (8, '✓ 读取 (Read) · 收集路径 `package.json` 的上下文。')] |
| sessions-panel | `sessions.current_marker` | no 当前 marker |
| add-provider | `add_provider.panel_vertical_borders_aligned` | left_cols=[30, 89] right_cols=[89] rows=9 |
| add-provider | `add_provider.focus_composer_hidden` | prompt_rows_outside_panel=[30] |
| add-provider | `add_provider.focus_chat_hidden` | guidance=True logo_tag=True block_glyphs=True |
| add-provider | `add_provider.focus_background_clean` | dirty_rows=[(6, '▄▄▄▄▄▄▄▄▄▄'), (7, '▀▀▀▀▀▀▀▀▀▀'), (8, '▀▀ ▀▀ pony v0.1.125'), (9, '▀▀▄▄▄▄▄▄▀▀ mock-alpha (fakep)')] |
| add-provider | `add_provider.border_rows_complete` | left=30 right=89 broken_rows=[(19, ' ', '│'), (20, ' ', '│')] |
| streaming | `mid.no_standalone_role_label` | row 3 = ' 你' |
| streaming | `final.no_standalone_role_label` | row 3 = ' 你' |
| streaming | `streaming.mid_status_writing` | footer :: Thinking... |
| streaming | `streaming.mid_active_elapsed` | no elapsed line |
| error | `main.no_standalone_role_label` | row 3 = ' 你' |
| narrow | `empty.separator_two_rules` | rule_rows=[] |
| wide | `conversation.no_standalone_role_label` | row 3 = ' 你' |
| wide | `empty.separator_two_rules` | rule_rows=[] |
| wide | `wide.text_columns_aligned` | user_col=2 assistant_col=2 tool_col=6 |
| scrolled | `scrolled.hint_present_after` | footer :: Turn completed |
| help | `help.commands_listed` | row 3 :: · Commands: /resume (select session), /provider, /models, /clear, /cost, /quit missing=['/help'] |
| help | `main.separator_two_rules` | rule_rows=[] |
| help | `help.command_desc_column` | cols={'/resume': None, '/provider': None, '/models': None, '/clear': None, '/cost': None, '/help': None, '/quit': None} expected=16 missing=['/resume', '/provider', '/models', '/clear', '/cost', '/help', '/quit'] |
| cost | `main.separator_two_rules` | rule_rows=[] |
| input-edit | `input_edit.typed_text_column` | typed_col=4 |

## Scenarios

### empty (120x34)

- evidence: `empty/`
- screenshots: [main.png](empty/main.png)
- checks: 11 pass / 4 fail

| result | check | evidence |
| --- | --- | --- |
| PASS | `main.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `main.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `main.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=118 cols=120 :: pony · pony-agent mock-alpha · 80e9c1 |
| PASS | `main.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `main.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/empty/main.ansi size=3149 |
| PASS | `main.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/empty/main.png size=125642 |
| PASS | `main.header` — header shows brand + workspace | row 0 :: pony · pony-agent mock-alpha · 80e9c1 |
| PASS | `main.input_padded_from_left_edge` — input prompt has left padding (>0 cols from terminal edge) | row 30 prompt_col=2 :: › ▏ 输入任务，或输入 / 查看命令 |
| PASS | `main.input_placeholder` — composer shows placeholder guidance when empty | row 30 :: › ▏ 输入任务，或输入 / 查看命令 |
| PASS | `empty.guidance` — empty state shows onboarding guidance | 描述一个任务开始，或输入命令： |
| PASS | `empty.no_error_status` — empty state has no error status word | footer :: 就绪 |
| FAIL | `main.separator_two_rules` — header rule and composer top rule are both present | rule_rows=[] |
| FAIL | `main.composer_cursor_block` — placeholder cursor cell found | placeholder not found |
| FAIL | `empty.placeholder_column` — placeholder starts at the shared text column 5 | placeholder_col=None |
| FAIL | `empty.command_desc_column` — command descriptions start at column 16 (same table geometry everywhere) | cols={'/resume': 13, '/provider': 13, '/models': 13, '/clear': 13, '/cost': None, '/help': None, '/quit': None} expected=16 missing=[] |

### conversation (120x34)

- evidence: `conversation/`
- screenshots: [main.png](conversation/main.png)
- checks: 14 pass / 5 fail

| result | check | evidence |
| --- | --- | --- |
| PASS | `main.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| FAIL | `main.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | row 3 = ' 你' |
| PASS | `main.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=118 cols=120 :: pony · pony-agent mock-alpha · c087e3 |
| PASS | `main.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `main.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/conversation/main.ansi size=1970 |
| PASS | `main.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/conversation/main.png size=142036 |
| PASS | `main.header` — header shows brand + workspace | row 0 :: pony · pony-agent mock-alpha · c087e3 |
| PASS | `main.input_padded_from_left_edge` — input prompt has left padding (>0 cols from terminal edge) | row 30 prompt_col=2 :: › ▏ 输入任务，或输入 / 查看命令 |
| PASS | `main.input_placeholder` — composer shows placeholder guidance when empty | row 30 :: › ▏ 输入任务，或输入 / 查看命令 |
| PASS | `conversation.user_message_visible` — user prompt is rendered in the transcript | row 4 :: 读取 package.json 并总结 |
| FAIL | `conversation.user_role_rail` — user message is distinguished by a rail/block, not a text label | no '▌' rail found |
| PASS | `conversation.tool_line_visible` — tool activity line (Read) is visible with a status glyph | row 8 :: ✓ 读取 (Read) · 收集路径 `package.json` 的上下文。 |
| PASS | `conversation.final_answer_visible` — final assistant answer (mock round 2) is visible | row 10 :: 结论 |
| FAIL | `conversation.text_columns_aligned` — user body / assistant body / tool text start on the same column | user_col=2 assistant_col=2 tool_col=6 |
| FAIL | `conversation.aligned_columns_padded` — shared text column is inset from the terminal edge | shared_col=2 |
| PASS | `conversation.no_error_rendered` — successful turn shows no error message | no Error:/失败 rows in transcript |
| PASS | `conversation.mock_followup_has_tool_round` — mock receipt proves the final model round carried a tool result | chat_requests=1 tool_round_requests=1 roles=[['system', 'system', 'system', 'system', 'user', 'assistant', 'tool']] scenarios=['final'] |
| FAIL | `main.separator_two_rules` — header rule and composer top rule are both present | rule_rows=[] |
| PASS | `conversation.footer_ready` — footer returns to a ready state after the turn | footer :: Turn completed |

### slash-menu (120x34)

- evidence: `slash-menu/`
- screenshots: [main.png](slash-menu/main.png)
- checks: 12 pass / 2 fail

| result | check | evidence |
| --- | --- | --- |
| PASS | `main.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `main.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `main.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=118 cols=120 :: pony · pony-agent mock-alpha · dd7af6 |
| PASS | `main.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `main.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/slash-menu/main.ansi size=4193 |
| PASS | `main.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/slash-menu/main.png size=160566 |
| PASS | `main.input_padded_from_left_edge` — input prompt has left padding (>0 cols from terminal edge) | row 30 prompt_col=2 :: › /▏ |
| PASS | `slash.all_commands_listed` — slash menu lists the full command table | menu_commands=['/resume', '/provider', '/models', '/clear', '/cost', '/help', '/quit'] missing=[] |
| PASS | `slash.menu_has_top_and_bottom_rules` — menu is framed by two hairline rules | rule_rows=[20, 28] |
| PASS | `slash.menu_above_composer` — menu rows sit above the composer input row | menu_rows=[21, 22]..27 bottom_rule=28 input_row=30 |
| PASS | `slash.each_command_once` — each command appears exactly once inside the menu region | duplicate/missing commands=[] menu_rows=7 |
| FAIL | `slash.command_desc_column` — command descriptions start at column 16 (same table geometry everywhere) | cols={'/resume': 13, '/provider': 16, '/models': 16, '/clear': 16, '/cost': 16, '/help': 16, '/quit': 16} expected=16 missing=[] |
| FAIL | `slash.resume_once_whole_screen` — '/resume' appears exactly once on the whole screen while the menu is open | /resume occurrences=2 |
| PASS | `slash.selected_row_marker` — selected command row carries the selection marker | row 21 :: ▏ /resume 选择并恢复历史会话 |

### models-panel (120x34)

- evidence: `models-panel/`
- screenshots: [main.png](models-panel/main.png)
- checks: 13 pass / 7 fail

| result | check | evidence |
| --- | --- | --- |
| PASS | `main.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `main.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `main.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=118 cols=120 :: pony · pony-agent mock-alpha · 3397cc |
| PASS | `main.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `main.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/models-panel/main.ansi size=7098 |
| PASS | `main.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/models-panel/main.png size=278511 |
| FAIL | `models.title_counter` — model panel title carries the '模型 · 1/N' position counter | title_row :: ▄▄▄▄▄▄▄▄▄▄ │ 模型 ↑↓ 选择 · 回车 确认 · r 刷新 · esc 取消 │ |
| PASS | `models.border_box` — exactly one top and one bottom panel border, top above bottom | top_rows=[5] bottom_rows=[28] |
| PASS | `models.panel_height_fits_screen` — panel top/bottom borders are both on screen and the panel height fits | top=5 bottom=28 height=24 pane_rows=34 |
| PASS | `models.panel_above_composer` — panel bottom border sits above the composer prompt | bottom=28 composer_top_row=30 |
| FAIL | `models.panel_vertical_borders_aligned` — every panel interior row shares the same left/right border column | left_cols=[31, 88] right_cols=[88] rows=22 |
| PASS | `models.no_double_border_overlap` — no row paints more than one panel's worth of vertical borders | max '│' per row=2 |
| FAIL | `models.focus_composer_hidden` — focus mode hides the global composer (prompt may only live inside the modal) | prompt_rows_outside_panel=[30] |
| FAIL | `models.focus_chat_hidden` — focus mode hides chat and splash (no transcript/logo residue) | guidance=True logo_tag=True block_glyphs=True |
| FAIL | `models.focus_background_clean` — rows outside header + panel are completely blank (no half-cell residue) | dirty_rows=[(30, '› ▏ 输入任务，或输入 / 查看命令'), (33, '已加载 20 个模型')] |
| FAIL | `models.border_rows_complete` — every panel row carries both border columns at the same display columns | left=31 right=88 broken_rows=[(17, ' ', '│')] |
| PASS | `models.rows_rendered` — panel renders at least the capped 12 model rows | model_rows=19 first=['mock-alpha', 'mock-beta'] last=['mock-model-20'] |
| FAIL | `models.rows_unique_and_consecutive` — model rows are unique and occupy consecutive screen rows | ids=19 unique=19 rows=[8, 9, 10, 11, 12, 13, 14, 15, 16, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27] |
| PASS | `models.no_fetch_failure` — remote /models fetch succeeded (no failure row) | no fetch failure row |
| PASS | `models.selected_marker` — selected model row carries the selection marker | selection marker '▏' present |

### provider-panel (120x34)

- evidence: `provider-panel/`
- screenshots: [main.png](provider-panel/main.png)
- checks: 10 pass / 5 fail

| result | check | evidence |
| --- | --- | --- |
| PASS | `main.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `main.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `main.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=118 cols=120 :: pony · pony-agent mock-alpha · 268cca |
| PASS | `main.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `main.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/provider-panel/main.ansi size=3607 |
| PASS | `main.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/provider-panel/main.png size=143320 |
| PASS | `provider.title` — provider panel title is visible | 终端里的自主智能体。 │ 提供商 ↑↓ 选择 · 回车 确认 · a 新增 · esc 取消 │ |
| PASS | `provider.fixture_listed` — fixture provider is listed | 描述一个任务开始，或输入命令│▏ FakeProvider openaicompletions 当前 │ |
| PASS | `provider.add_hint` — panel advertises the 'a 新增' shortcut | 终端里的自主智能体。 │ 提供商 ↑↓ 选择 · 回车 确认 · a 新增 · esc 取消 │ |
| FAIL | `provider.border_box` — exactly one top and one bottom panel border, top above bottom | top_rows=[14] bottom_rows=[] |
| FAIL | `provider.focus_composer_hidden` — focus mode hides the global composer (prompt may only live inside the modal) | prompt_rows_outside_panel=[30] |
| FAIL | `provider.focus_chat_hidden` — focus mode hides chat and splash (no transcript/logo residue) | guidance=True logo_tag=True block_glyphs=True |
| FAIL | `provider.focus_background_clean` — rows outside the panel are blank | panel bounds missing |
| FAIL | `provider.border_rows_complete` — every panel row carries both border columns | missing top/bottom border |
| PASS | `provider.current_marker` — active provider row is marked as current | 描述一个任务开始，或输入命令│▏ FakeProvider openaicompletions 当前 │ |

### sessions-panel (120x34)

- evidence: `sessions-panel/`
- screenshots: [main.png](sessions-panel/main.png)
- checks: 18 pass / 5 fail

| result | check | evidence |
| --- | --- | --- |
| PASS | `main.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| FAIL | `main.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | row 3 = ' 你' |
| PASS | `main.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=118 cols=120 :: pony · pony-agent mock-alpha · 952e16 |
| PASS | `main.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `main.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/sessions-panel/main.ansi size=3077 |
| PASS | `main.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/sessions-panel/main.png size=159880 |
| PASS | `sessions.title` — session panel title is visible | ▏ { │ 会话 ↑↓ 选择 · 回车 确认 · esc 取消 │ |
| PASS | `sessions.prior_conversation_listed` — the conversation just completed is offered for resume | 读取 package.json 并总结 |
| PASS | `sessions.session_id_visible` — session row carries a short session id | ▏ "version": "0.1.125" │▏ 读取 package.json 并总结 952e16ca │ |
| PASS | `sessions.not_empty` — panel does not show the empty-sessions placeholder | no '暂无历史会话' placeholder |
| PASS | `sessions.border_box` — exactly one top and one bottom panel border, top above bottom | top_rows=[14] bottom_rows=[19] |
| PASS | `sessions.panel_height_fits_screen` — panel top/bottom borders are both on screen and the panel height fits | top=14 bottom=19 height=6 pane_rows=34 |
| PASS | `sessions.panel_above_composer` — panel bottom border sits above the composer prompt | bottom=19 composer_top_row=30 |
| PASS | `sessions.panel_vertical_borders_aligned` — every panel interior row shares the same left/right border column | left_cols=[31] right_cols=[88] rows=4 |
| PASS | `sessions.panel_width_fits` — panel width fits inside the terminal | panel_cols=31..88 width=58 terminal_cols=120 |
| FAIL | `sessions.panel_width_expected` — panel width matches the design token (84) and is centered | width=58 expected=84 left=31 right_margin=31 |
| PASS | `sessions.panel_rows_not_overlapping` — panel rows are consecutive (no overlapping/duplicated rows) | interior_rows=[15, 16, 17, 18] |
| PASS | `sessions.no_double_border_overlap` — no row paints more than one panel's worth of vertical borders | max '│' per row=2 |
| FAIL | `sessions.focus_composer_hidden` — focus mode hides the global composer (prompt may only live inside the modal) | prompt_rows_outside_panel=[30] |
| PASS | `sessions.focus_chat_hidden` — focus mode hides chat and splash (no transcript/logo residue) | guidance=False logo_tag=False block_glyphs=False |
| FAIL | `sessions.focus_background_clean` — rows outside header + panel are completely blank (no half-cell residue) | dirty_rows=[(3, '你'), (4, '读取 package.json 并总结'), (7, 'pony'), (8, '✓ 读取 (Read) · 收集路径 `package.json` 的上下文。')] |
| PASS | `sessions.border_rows_complete` — every panel row carries both border columns at the same display columns | left=31 right=88 broken_rows=[] |
| FAIL | `sessions.current_marker` — sessions panel marks the active session | no 当前 marker |

### add-provider (120x34)

- evidence: `add-provider/`
- screenshots: [main.png](add-provider/main.png)
- checks: 13 pass / 5 fail

| result | check | evidence |
| --- | --- | --- |
| PASS | `main.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `main.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `main.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=118 cols=120 :: pony · pony-agent mock-alpha · a0fad6 |
| PASS | `main.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `main.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/add-provider/main.ansi size=4236 |
| PASS | `main.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/add-provider/main.png size=137982 |
| PASS | `add_provider.title` — add-provider modal title is visible | ▀▀ │ 新增提供商 回车 下一步 · esc 取消 │ |
| PASS | `add_provider.step_indicator` — 3-step wizard shows the name step | │ 1 / 3 name Provider 名称 │ |
| PASS | `add_provider.input_focused` — modal input row renders the caret prompt | row 30 :: › ▏ 输入任务，或输入 / 查看命令 |
| PASS | `add_provider.border_box` — exactly one top and one bottom panel border, top above bottom | top_rows=[11] bottom_rows=[21] |
| PASS | `add_provider.panel_height_fits_screen` — panel top/bottom borders are both on screen and the panel height fits | top=11 bottom=21 height=11 pane_rows=34 |
| PASS | `add_provider.panel_above_composer` — panel bottom border sits above the composer prompt | bottom=21 composer_top_row=30 |
| FAIL | `add_provider.panel_vertical_borders_aligned` — every panel interior row shares the same left/right border column | left_cols=[30, 89] right_cols=[89] rows=9 |
| PASS | `add_provider.no_double_border_overlap` — no row paints more than one panel's worth of vertical borders | max '│' per row=2 |
| FAIL | `add_provider.focus_composer_hidden` — focus mode hides the global composer (prompt may only live inside the modal) | prompt_rows_outside_panel=[30] |
| FAIL | `add_provider.focus_chat_hidden` — focus mode hides chat and splash (no transcript/logo residue) | guidance=True logo_tag=True block_glyphs=True |
| FAIL | `add_provider.focus_background_clean` — rows outside header + panel are completely blank (no half-cell residue) | dirty_rows=[(6, '▄▄▄▄▄▄▄▄▄▄'), (7, '▀▀▀▀▀▀▀▀▀▀'), (8, '▀▀ ▀▀ pony v0.1.125'), (9, '▀▀▄▄▄▄▄▄▀▀ mock-alpha (fakep)')] |
| FAIL | `add_provider.border_rows_complete` — every panel row carries both border columns at the same display columns | left=30 right=89 broken_rows=[(19, ' ', '│'), (20, ' ', '│')] |

### streaming (120x34)

- evidence: `streaming/`
- screenshots: [final.png](streaming/final.png), [mid.png](streaming/mid.png)
- checks: 14 pass / 4 fail

| result | check | evidence |
| --- | --- | --- |
| PASS | `mid.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| FAIL | `mid.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | row 3 = ' 你' |
| PASS | `mid.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=118 cols=120 :: pony · pony-agent mock-alpha · 61e1b0 |
| PASS | `mid.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `mid.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/streaming/mid.ansi size=1293 |
| PASS | `mid.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/streaming/mid.png size=77907 |
| PASS | `final.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| FAIL | `final.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | row 3 = ' 你' |
| PASS | `final.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=118 cols=120 :: pony · pony-agent mock-alpha · 61e1b0 |
| PASS | `final.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `final.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/streaming/final.ansi size=1739 |
| PASS | `final.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/streaming/final.png size=140257 |
| FAIL | `streaming.mid_status_writing` — footer shows the writing state while the stream is still arriving | footer :: Thinking... |
| FAIL | `streaming.mid_active_elapsed` — active line shows elapsed time next to the writing state | no elapsed line |
| PASS | `streaming.mid_partial_body` — partial streamed markdown body is already rendered | 正在分析 |
| PASS | `streaming.mid_not_finished` — mid-stream capture is not the completed answer yet | footer :: Thinking... |
| PASS | `streaming.final_complete` — final capture contains the terminal answer and ready footer | footer :: Turn completed |
| PASS | `streaming.final_not_writing` — writing status disappeared once the stream finished | footer :: Turn completed |

### error (120x34)

- evidence: `error/`
- screenshots: [main.png](error/main.png)
- checks: 13 pass / 1 fail

| result | check | evidence |
| --- | --- | --- |
| PASS | `main.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| FAIL | `main.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | row 3 = ' 你' |
| PASS | `main.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=118 cols=120 :: pony · pony-agent mock-alpha · 70aec9 |
| PASS | `main.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `main.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/error/main.ansi size=1218 |
| PASS | `main.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/error/main.png size=74218 |
| PASS | `main.input_padded_from_left_edge` — input prompt has left padding (>0 cols from terminal edge) | row 30 prompt_col=2 :: › ▏ 输入任务，或输入 / 查看命令 |
| PASS | `main.input_placeholder` — composer shows placeholder guidance when empty | row 30 :: › ▏ 输入任务，或输入 / 查看命令 |
| PASS | `error.message_visible` — provider 401 is surfaced as a visible error message | row 6 :: ✕ Error: {"error": {"message": "mock 401 unauthorized: invalid api key (audit scenario)", "type": "invalid_request_e |
| PASS | `error.error_glyph` — error row carries the failure glyph | row 6 :: ✕ Error: {"error": {"message": "mock 401 unauthorized: invalid api key (audit scenario)", "type": "invalid_request_e |
| PASS | `error.reason_included` — error text names the 401/unauthorized cause | 401 认证测试 |
| PASS | `error.footer_failed` — footer reports the failed turn | footer :: Turn failed: {"error": {"message": "mock 401 unauthorized: invalid api key (audit scenario)", "type": "invalid_requ |
| PASS | `error.no_fake_answer` — no assistant answer is fabricated for the failed turn | no 结论 row in failed capture |
| PASS | `error.mock_401_receipt` — mock receipt proves the 401 scenario was requested | scenarios=['401', '401'] |

### narrow (60x20)

- evidence: `narrow/`
- screenshots: [conversation.png](narrow/conversation.png), [empty.png](narrow/empty.png)
- checks: 19 pass / 1 fail

| result | check | evidence |
| --- | --- | --- |
| PASS | `empty.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `empty.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `empty.within_terminal_width` — every captured row display width <= 60 cols | widest row 0 width=58 cols=60 :: pony · pony-agent mock-alpha · db05d2 |
| PASS | `empty.capture_rows` — capture has exactly pane height rows (20) | rows=20 pane_rows=20 |
| PASS | `empty.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/narrow/empty.ansi size=1341 |
| PASS | `empty.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/narrow/empty.png size=96820 |
| PASS | `empty.input_padded_from_left_edge` — input prompt has left padding (>0 cols from terminal edge) | row 16 prompt_col=2 :: › ▏ 输入任务，或输入 / 查看命令 |
| PASS | `empty.input_placeholder` — composer shows placeholder guidance when empty | row 16 :: › ▏ 输入任务，或输入 / 查看命令 |
| PASS | `conversation.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `conversation.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `conversation.within_terminal_width` — every captured row display width <= 60 cols | widest row 0 width=58 cols=60 :: pony · pony-agent mock-alpha · db05d2 |
| PASS | `conversation.capture_rows` — capture has exactly pane height rows (20) | rows=20 pane_rows=20 |
| PASS | `conversation.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/narrow/conversation.ansi size=1380 |
| PASS | `conversation.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/narrow/conversation.png size=92432 |
| PASS | `conversation.input_padded_from_left_edge` — input prompt has left padding (>0 cols from terminal edge) | row 16 prompt_col=2 :: › ▏ 输入任务，或输入 / 查看命令 |
| PASS | `conversation.input_placeholder` — composer shows placeholder guidance when empty | row 16 :: › ▏ 输入任务，或输入 / 查看命令 |
| PASS | `empty.header` — header shows brand + workspace | row 0 :: pony · pony-agent mock-alpha · db05d2 |
| FAIL | `empty.separator_two_rules` — header rule and composer top rule are both present | rule_rows=[] |
| PASS | `narrow.empty_guidance` — narrow empty state keeps onboarding guidance on screen | 描述一个任务开始，或输入命令： |
| PASS | `narrow.conversation_visible` — narrow conversation still shows the tail of the final answer | `package.json` 显示这是 pony-agent，版本 0.1.125。 |

### wide (160x40)

- evidence: `wide/`
- screenshots: [conversation.png](wide/conversation.png), [empty.png](wide/empty.png)
- checks: 18 pass / 3 fail

| result | check | evidence |
| --- | --- | --- |
| PASS | `empty.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `empty.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `empty.within_terminal_width` — every captured row display width <= 160 cols | widest row 0 width=158 cols=160 :: pony · pony-agent mock-alpha · 461faa |
| PASS | `empty.capture_rows` — capture has exactly pane height rows (40) | rows=40 pane_rows=40 |
| PASS | `empty.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/wide/empty.ansi size=3355 |
| PASS | `empty.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/wide/empty.png size=132685 |
| PASS | `empty.input_padded_from_left_edge` — input prompt has left padding (>0 cols from terminal edge) | row 36 prompt_col=2 :: › ▏ 输入任务，或输入 / 查看命令 |
| PASS | `empty.input_placeholder` — composer shows placeholder guidance when empty | row 36 :: › ▏ 输入任务，或输入 / 查看命令 |
| PASS | `conversation.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| FAIL | `conversation.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | row 3 = ' 你' |
| PASS | `conversation.within_terminal_width` — every captured row display width <= 160 cols | widest row 0 width=158 cols=160 :: pony · pony-agent mock-alpha · 461faa |
| PASS | `conversation.capture_rows` — capture has exactly pane height rows (40) | rows=40 pane_rows=40 |
| PASS | `conversation.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/wide/conversation.ansi size=2176 |
| PASS | `conversation.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/wide/conversation.png size=149340 |
| PASS | `conversation.input_padded_from_left_edge` — input prompt has left padding (>0 cols from terminal edge) | row 36 prompt_col=2 :: › ▏ 输入任务，或输入 / 查看命令 |
| PASS | `conversation.input_placeholder` — composer shows placeholder guidance when empty | row 36 :: › ▏ 输入任务，或输入 / 查看命令 |
| PASS | `empty.header` — header shows brand + workspace | row 0 :: pony · pony-agent mock-alpha · 461faa |
| FAIL | `empty.separator_two_rules` — header rule and composer top rule are both present | rule_rows=[] |
| PASS | `wide.header_full_metadata` — wide header keeps workspace and model metadata on one line | row 0 :: pony · pony-agent mock-alpha · 461faa |
| PASS | `wide.conversation_visible` — wide conversation shows the final answer | 结论 |
| FAIL | `wide.text_columns_aligned` — at 160x40 the three text streams still share one column | user_col=2 assistant_col=2 tool_col=6 |

### scrolled (120x34)

- evidence: `scrolled/`
- screenshots: [after.png](scrolled/after.png), [before.png](scrolled/before.png)
- checks: 18 pass / 1 fail

| result | check | evidence |
| --- | --- | --- |
| PASS | `before.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `before.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `before.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=118 cols=120 :: pony · pony-agent mock-alpha · 46e13f |
| PASS | `before.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `before.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/scrolled/before.ansi size=5328 |
| PASS | `before.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/scrolled/before.png size=518562 |
| PASS | `after.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `after.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `after.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=118 cols=120 :: pony · pony-agent mock-alpha · 46e13f |
| PASS | `after.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `after.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/scrolled/after.ansi size=6742 |
| PASS | `after.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/scrolled/after.png size=684667 |
| PASS | `after.input_padded_from_left_edge` — input prompt has left padding (>0 cols from terminal edge) | row 30 prompt_col=2 :: › ▏ 输入任务，或输入 / 查看命令 |
| PASS | `after.input_placeholder` — composer shows placeholder guidance when empty | row 30 :: › ▏ 输入任务，或输入 / 查看命令 |
| PASS | `scrolled.bottom_before` — before scrolling the transcript is pinned to the newest line | 这是最后一行，滚动到底部时应完整可见。 |
| PASS | `scrolled.hint_absent_before` — scroll-up hint is absent while pinned to bottom | footer :: Turn completed |
| PASS | `scrolled.after_shows_earlier_rows` — PageUp reveals earlier lines and hides the newest ones | · 第 14 行：这是一段用于验证滚动与折行的中文正文，包含 `inline code` 与 加粗 片段。 |
| FAIL | `scrolled.hint_present_after` — footer advertises the scrolled-up state | footer :: Turn completed |
| PASS | `scrolled.offset_moved_up` — first visible content row moved up by at least 8 terminal rows | first_visible_before=第23行 first_visible_after=第13行 |

### help (120x34)

- evidence: `help/`
- screenshots: [main.png](help/main.png)
- checks: 9 pass / 3 fail

| result | check | evidence |
| --- | --- | --- |
| PASS | `main.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `main.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `main.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=118 cols=120 :: pony · pony-agent mock-alpha · 277d25 |
| PASS | `main.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `main.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/help/main.ansi size=1070 |
| PASS | `main.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/help/main.png size=48338 |
| PASS | `main.input_padded_from_left_edge` — input prompt has left padding (>0 cols from terminal edge) | row 30 prompt_col=2 :: › ▏ 输入任务，或输入 / 查看命令 |
| PASS | `main.input_placeholder` — composer shows placeholder guidance when empty | row 30 :: › ▏ 输入任务，或输入 / 查看命令 |
| FAIL | `help.commands_listed` — /help renders the command reference in the transcript | row 3 :: · Commands: /resume (select session), /provider, /models, /clear, /cost, /quit missing=['/help'] |
| FAIL | `main.separator_two_rules` — header rule and composer top rule are both present | rule_rows=[] |
| FAIL | `help.command_desc_column` — command descriptions start at column 16 (same table geometry everywhere) | cols={'/resume': None, '/provider': None, '/models': None, '/clear': None, '/cost': None, '/help': None, '/quit': None} expected=16 missing=['/resume', '/provider', '/models', '/clear', '/cost', '/help', '/quit'] |
| PASS | `help.system_message_glyph` — help output is a system row (bullet glyph), not a user message | row 3 :: · Commands: /resume (select session), /provider, /models, /clear, /cost, /quit |

### cost (120x34)

- evidence: `cost/`
- screenshots: [main.png](cost/main.png)
- checks: 10 pass / 1 fail

| result | check | evidence |
| --- | --- | --- |
| PASS | `main.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `main.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `main.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=118 cols=120 :: pony · pony-agent mock-alpha · d42b81 |
| PASS | `main.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `main.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/cost/main.ansi size=1045 |
| PASS | `main.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/cost/main.png size=50365 |
| PASS | `main.input_padded_from_left_edge` — input prompt has left padding (>0 cols from terminal edge) | row 30 prompt_col=2 :: › ▏ 输入任务，或输入 / 查看命令 |
| PASS | `main.input_placeholder` — composer shows placeholder guidance when empty | row 30 :: › ▏ 输入任务，或输入 / 查看命令 |
| PASS | `cost.model_line` — /cost shows the active model | row :: · Model: mock-alpha \| Gateway: http://10.42.0.67:8080 |
| FAIL | `main.separator_two_rules` — header rule and composer top rule are both present | rule_rows=[] |
| PASS | `cost.gateway_line` — /cost shows the gateway endpoint | row :: · Model: mock-alpha \| Gateway: http://10.42.0.67:8080 |

### ctrl-c (120x34)

- evidence: `ctrl-c/`
- screenshots: [exit.png](ctrl-c/exit.png), [shell.png](ctrl-c/shell.png)
- checks: 15 pass / 0 fail

| result | check | evidence |
| --- | --- | --- |
| PASS | `exit.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `exit.within_terminal_width` — every captured row display width <= 120 cols | widest row 1 width=120 cols=120 :: -u NO_COLOR XDG_CONFIG_HOME=/tmp/tui-audit.9w7sM0/iso-ctrl-c/xdg-config XDG_DATA_HOME=/tmp/tui-audit.9w7sM0/iso-ctrl-c/x |
| PASS | `exit.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `exit.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/ctrl-c/exit.ansi size=493 |
| PASS | `exit.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/ctrl-c/exit.png size=77687 |
| PASS | `shell.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `shell.within_terminal_width` — every captured row display width <= 120 cols | widest row 1 width=120 cols=120 :: -u NO_COLOR XDG_CONFIG_HOME=/tmp/tui-audit.9w7sM0/iso-ctrl-c/xdg-config XDG_DATA_HOME=/tmp/tui-audit.9w7sM0/iso-ctrl-c/x |
| PASS | `shell.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `shell.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/ctrl-c/shell.ansi size=548 |
| PASS | `shell.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/ctrl-c/shell.png size=89087 |
| PASS | `ctrl_c.process_exited` — Ctrl-C terminates the TUI process with a normal exit status | row :: UTF-8 /home/dm/pony-agent/tools/tui-audit/bin/pony-baseline; echo AUDIT_EXIT=$? code=0 |
| PASS | `ctrl_c.terminal_restored` — alternate screen is left and the pane stops painting the TUI | tui_band_present=False placeholder_present=False |
| PASS | `ctrl_c.shell_alive` — the same tmux pane still runs commands after exit | bash-5.2$ echo AUDIT_SHELL_ALIVE=ok |
| PASS | `ctrl_c.pane_command_is_shell` — pane foreground command is a shell again, not pony | pane_current_command='bash' |
| PASS | `ctrl_c.no_panic` — shutdown path prints no panic | no panic tokens in shutdown captures |

### input-edit (120x34)

- evidence: `input-edit/`
- screenshots: [abc.png](input-edit/abc.png), [backspace.png](input-edit/backspace.png), [ctrl_u_cleared.png](input-edit/ctrl_u_cleared.png), [draft.png](input-edit/draft.png), [draft_cleared.png](input-edit/draft_cleared.png), [esc_cleared.png](input-edit/esc_cleared.png), [exit.png](input-edit/exit.png), [inserted.png](input-edit/inserted.png), [multiline.png](input-edit/multiline.png), [shell.png](input-edit/shell.png), [tab_complete.png](input-edit/tab_complete.png), [word_cleared.png](input-edit/word_cleared.png)
- checks: 86 pass / 1 fail

| result | check | evidence |
| --- | --- | --- |
| PASS | `abc.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `abc.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `abc.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=118 cols=120 :: pony · pony-agent mock-alpha · 2550f1 |
| PASS | `abc.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `abc.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/input-edit/abc.ansi size=3112 |
| PASS | `abc.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/input-edit/abc.png size=115706 |
| PASS | `input_edit.abc` — typed text lands in the composer | buffer='abc' expected='abc' |
| PASS | `inserted.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `inserted.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `inserted.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=118 cols=120 :: pony · pony-agent mock-alpha · 2550f1 |
| PASS | `inserted.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `inserted.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/input-edit/inserted.ansi size=3132 |
| PASS | `inserted.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/input-edit/inserted.png size=116546 |
| PASS | `input_edit.inserted` — Left + X inserts in the middle without losing the tail | buffer='abXc' expected='abXc' |
| PASS | `backspace.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `backspace.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `backspace.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=118 cols=120 :: pony · pony-agent mock-alpha · 2550f1 |
| PASS | `backspace.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `backspace.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/input-edit/backspace.ansi size=3112 |
| PASS | `backspace.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/input-edit/backspace.png size=116088 |
| PASS | `input_edit.backspace` — Right + Backspace deletes the character at the end | buffer='abX' expected='abX' |
| PASS | `word_cleared.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `word_cleared.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `word_cleared.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=118 cols=120 :: pony · pony-agent mock-alpha · 2550f1 |
| PASS | `word_cleared.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `word_cleared.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/input-edit/word_cleared.ansi size=3149 |
| PASS | `word_cleared.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/input-edit/word_cleared.png size=124696 |
| PASS | `input_edit.word_cleared` — Ctrl+W deletes the whole word | buffer='' expected='' |
| PASS | `tab_complete.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `tab_complete.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `tab_complete.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=118 cols=120 :: pony · pony-agent mock-alpha · 2550f1 |
| PASS | `tab_complete.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `tab_complete.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/input-edit/tab_complete.ansi size=3594 |
| PASS | `tab_complete.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/input-edit/tab_complete.png size=129437 |
| PASS | `input_edit.tab_complete` — Tab completes '/mo' to '/models' | buffer='/models' expected='/models' |
| PASS | `ctrl_u_cleared.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `ctrl_u_cleared.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `ctrl_u_cleared.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=118 cols=120 :: pony · pony-agent mock-alpha · 2550f1 |
| PASS | `ctrl_u_cleared.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `ctrl_u_cleared.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/input-edit/ctrl_u_cleared.ansi size=3149 |
| PASS | `ctrl_u_cleared.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/input-edit/ctrl_u_cleared.png size=124696 |
| PASS | `input_edit.ctrl_u_cleared` — Ctrl+U clears the composer | buffer='' expected='' |
| PASS | `multiline.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `multiline.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `multiline.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=118 cols=120 :: pony · pony-agent mock-alpha · 2550f1 |
| PASS | `multiline.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `multiline.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/input-edit/multiline.ansi size=3147 |
| PASS | `multiline.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/input-edit/multiline.png size=117020 |
| PASS | `input_edit.multiline` — Ctrl+J inserts a newline (two visual rows) | buffer='line1\nline2' expected='line1\nline2' |
| PASS | `esc_cleared.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `esc_cleared.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `esc_cleared.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=118 cols=120 :: pony · pony-agent mock-alpha · 2550f1 |
| PASS | `esc_cleared.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `esc_cleared.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/input-edit/esc_cleared.ansi size=3149 |
| PASS | `esc_cleared.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/input-edit/esc_cleared.png size=124696 |
| PASS | `input_edit.esc_cleared` — Esc clears the composer | buffer='' expected='' |
| PASS | `draft.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `draft.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `draft.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=118 cols=120 :: pony · pony-agent mock-alpha · 2550f1 |
| PASS | `draft.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `draft.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/input-edit/draft.ansi size=3114 |
| PASS | `draft.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/input-edit/draft.png size=115774 |
| PASS | `input_edit.draft` — draft text is present before Ctrl-C | buffer='draft' expected='draft' |
| FAIL | `input_edit.typed_text_column` — typed text starts at the same column as the empty-state placeholder (5) | typed_col=4 |
| PASS | `input_edit.multiline_visual_rows` — multi-line draft occupies two distinct composer rows | composer_rows=2 buffer='line1\nline2' |
| PASS | `draft_cleared.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `draft_cleared.no_standalone_role_label` — no standalone role label line "你" (role shown by material, not label) | no line equals "你" |
| PASS | `draft_cleared.within_terminal_width` — every captured row display width <= 120 cols | widest row 0 width=118 cols=120 :: pony · pony-agent mock-alpha · 2550f1 |
| PASS | `draft_cleared.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `draft_cleared.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/input-edit/draft_cleared.ansi size=3149 |
| PASS | `draft_cleared.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/input-edit/draft_cleared.png size=124696 |
| PASS | `input_edit.ctrl_c_with_draft_clears_only` — Ctrl-C with a draft clears the composer instead of quitting | buffer='' exit_marker=False |
| PASS | `input_edit.ctrl_c_draft_still_running` — TUI is still painting its composer after the first Ctrl-C | › ▏ 输入任务，或输入 / 查看命令 |
| PASS | `exit.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `exit.within_terminal_width` — every captured row display width <= 120 cols | widest row 1 width=120 cols=120 :: -u NO_COLOR XDG_CONFIG_HOME=/tmp/tui-audit.9w7sM0/iso-input-edit/xdg-config XDG_DATA_HOME=/tmp/tui-audit.9w7sM0/iso-inpu |
| PASS | `exit.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `exit.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/input-edit/exit.ansi size=509 |
| PASS | `exit.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/input-edit/exit.png size=78141 |
| PASS | `shell.no_panic` — capture contains no Rust panic/backtrace | no panic tokens in capture |
| PASS | `shell.within_terminal_width` — every captured row display width <= 120 cols | widest row 1 width=120 cols=120 :: -u NO_COLOR XDG_CONFIG_HOME=/tmp/tui-audit.9w7sM0/iso-input-edit/xdg-config XDG_DATA_HOME=/tmp/tui-audit.9w7sM0/iso-inpu |
| PASS | `shell.capture_rows` — capture has exactly pane height rows (34) | rows=34 pane_rows=34 |
| PASS | `shell.artifact_ansi` — ansi artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/input-edit/shell.ansi size=564 |
| PASS | `shell.artifact_png` — png artifact exists for screenshot review | /home/dm/pony-agent/.dev-team/evidence/tui-polish/baseline/input-edit/shell.png size=89497 |
| PASS | `input_edit.ctrl_c_empty_exits` — a second Ctrl-C on the empty composer exits the process normally | exit_code=0 row :: home LANG=en_US.UTF-8 /home/dm/pony-agent/tools/tui-audit/bin/pony-baseline; echo AUDIT_EXIT=$? |
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
| PASS | `cli.version_output` — --version prints its documented payload | pony 0.1.125 · linux-x86_64 missing=[] |
| PASS | `cli.version_no_panic` — --version does not panic | no panic tokens in output |
| PASS | `cli.help_exit_status` — --help exits with a documented status (0) | exit_code=0 allowed=(0,) |
| PASS | `cli.help_output` — --help prints its documented payload | pony 0.1.125 — 交互式 TUI Agent missing=[] |
| PASS | `cli.help_no_panic` — --help does not panic | no panic tokens in output |
| PASS | `cli.doctor_exit_status` — --doctor exits with a documented status (0, or 1 when the non-TTY self-check reports a hard failure) | exit_code=1 allowed=(0, 1) |
| PASS | `cli.doctor_output` — --doctor prints its documented payload | [ok] TERM: dumb missing=[] |
| PASS | `cli.doctor_no_panic` — --doctor does not panic | no panic tokens in output |

