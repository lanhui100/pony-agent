# 过期 stash 归档收据（2026-11-19）

两条长期滞留在本地 `git stash` 的 WIP 被清理前先导出补丁留痕。结论：**两条 stash 的主题均已在 `main` 落地（且已随 `tui.rs` 模块拆分重构，原补丁无法再直接 apply），故仅作证据归档，不计划复活。**

## 来源元数据

| stash | stash commit | 基线提交 | 创建时间 | 规模 |
|---|---|---|---|---|
| `stash@{0}` `On main: probe-baseline` | `d05b9f2f` | `ebf50d64`（2026-11-06） | 2026-11-06 22:19:56 +0800 | 1 文件 +149/−31 |
| `stash@{1}` `WIP on main: e6983b8` | `5ec6ab66` | `e6983b88`（2026-10-20） | 2026-10-20 11:20:13 +0800 | 5 文件 +133/−28 |

补丁文件：

- `stash0-probe-baseline.patch`（14,165 B）
- `stash1-context-provider.patch`（10,156 B）

## 为何不复活

`git apply --check` 对两份补丁均失败（`patch does not apply`）：

- `stash@{0}`：`tui.rs` 基线 4,300+ 行处上下文已漂移，且 `main` 上 `tui.rs` 已拆出 `tui/terminal.rs`、`tui/input.rs` 子模块，补丁目标函数已迁址。
- `stash@{1}`：`e6983b88`（10-20）到 `main` 的 `tui.rs` 差异达 +10,399/−1,507 行，五个目标文件全部冲突。

## 主题落地核验（符号级）

`stash@{0}`（终端还原可靠性）：

| 补丁中的符号 | `main` 现状 |
|---|---|
| `TERMINAL_RESTORE_INCOMPLETE` | `src/bin/tui/terminal.rs:4` 处命中 |
| `terminal_recovery_hint_line` | `src/bin/tui/terminal.rs` + `tests/tui_terminal_restore.rs` |
| `resume_hint_block` | `src/bin/tui/terminal.rs:5` + 2 个测试文件 |
| `terminal_multiplexer_for_env` | `src/bin/tui/input.rs` + `tests/tui_terminal_restore.rs:5` |
| `keyboard_enhancement_push_flags` | `src/bin/tui.rs`、`tui/input.rs`、`tui/terminal.rs`、测试 |
| `TERMINAL_INPUT_MODES_RESET` 串内容（`<1u`×3 + `<u`） | `src/bin/tui/terminal.rs:45` 一致 |
| `/dev/tty` fd 直写回退 | `src/bin/tui.rs:106/109/161` + `terminal.rs:62/82` |

`stash@{1}`（上下文稳定前缀 / Anthropic 用量解析 / 缓存命中率按单轮）：

| 补丁主题 | `main` 现状 |
|---|---|
| `context.rs` 稳定前缀注入 | `context.rs:656/699` 的 `stable_prefix_text` 已成型，hotloader 走 `resolve_session_prompt_context_with_budget_and_catalog`（`context.rs:954`） |
| `provider/mod.rs` `system` 载荷 | `anthropic_system_payload` 落于 `provider/mod.rs:4717` |
| `anthropic_sse` 用量解析（含 `cache_read_input_tokens`） | `provider/mod.rs` 已含 `extract_anthropic_usage_from_sse_payload` 与 `cache_read_input_tokens` |
| `tests/tui_token_context_wave41.rs` A5 单轮缓存命中率 | `tests/tui_token_context_wave41.rs:196-229` 断言已存在且更严格（含 ≤100% 钳制） |

## 复现核验命令

```bash
# 补丁不可直接应用（预期非零退出）
git stash show -p stash@{0} > /tmp/stash0.patch && git apply --check /tmp/stash0.patch
git stash show -p stash@{1} > /tmp/stash1.patch && git apply --check /tmp/stash1.patch

# 主题已落地（预期有命中）
git grep -n "TERMINAL_RESTORE_INCOMPLETE" main -- 'crates/pony-agent-core/src/bin/tui*'
git grep -n "anthropic_system_payload" main -- 'crates/pony-agent-core/src/agent/provider/mod.rs'
```

## 处置

两条 stash 在本证据入库后 `git stash drop`；如需追溯，用补丁文件或 `git stash` 原始 commit（`d05b9f2f` / `5ec6ab66`，仍可 `git show`，只要未被 `git gc --prune` 回收）。
