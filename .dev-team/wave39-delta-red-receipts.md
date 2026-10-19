# wave-39-delta 红相收据（L2-T · Test Agent）

- 角色：独立 Test Agent（只写测试/审计工具；未碰 `tui.rs`、未改 A1–A10 冻结文件、未放宽 checks.py）
- 任务板：`task-6`（W39-D2 红相冻结 A11 + run_audit wait 模式适配）
- 契约：`.dev-team/contract-matrix-tui-dialog-wave39.md` §5 v2（R8' / A11）
- 空桩基线：`crates/pony-agent-core/src/bin/tui.rs:1295` `stream_usage_event(...) { unimplemented!() }`（Protocol task-5 落盘）
- 结论：**红相成立 —— 编译 Exit 0，运行 Exit 101，0 passed / 4 failed（4/4 均为 `unimplemented!()` panic）；run_audit.sh wait 模式已改为当前真实信号，`bash -n` Exit 0**

## 1. 交付物（写域内）

| 交付 | 路径 | 说明 |
|---|---|---|
| A11 验收测试（新文件） | `crates/pony-agent-core/tests/tui_usage_wave39_delta.rs`（144 行） | 4 组断言，独立 `--test` 目标 |
| 审计脚本 wait 校正 | `tools/tui-audit/run_audit.sh`（未被 git 跟踪） | 6 处 `完成\|Turn completed` + 1 处 `已上翻` |
| 本收据 | `.dev-team/wave39-delta-red-receipts.md` | — |

写域审计（`git status --porcelain`，排除 E2E 证据目录）：

```
 M crates/pony-agent-core/src/bin/tui.rs     # Protocol/Executor 既有改动（含 task-5 空桩），非本 Agent
 M src/stores/providers.ts                   # 并行前端任务，既有
?? crates/pony-agent-core/tests/tui_usage_wave39_delta.rs   # 本轮新增（唯一业务外新增）
?? tools/                                    # Test Agent 工具目录（未跟踪）
```

A1–A10 冻结文件未被触碰：`tests/tui_dialog_wave39.rs` mtime=`2026-10-19 19:54:55`（task-2 交付时刻，早于本轮），sha256=`66f577f1…d1138e`，且复跑仍 **10 passed / Exit 0**。
`tools/tui-audit/checks.py` mtime=`19:55:52`（task-2 交付时刻）、`src/bin/tui.rs` mtime=`20:11:13`（Protocol task-5 空桩落盘），二者均早于本任务开始（`20:12:45`），本轮零改动。

## 2. 门禁命令与真实退出码

### 2.1 编译门禁（红相必须是「编译通过 + 运行失败」）

```
$ cd /home/dm/pony-agent && cargo test -p pony-agent-core --test tui_usage_wave39_delta --no-run
   Compiling pony-agent-core v0.1.127 (/home/dm/pony-agent/crates/pony-agent-core)
    Finished `test` profile [unoptimized] target(s) in 6.40s
  Executable tests/tui_usage_wave39_delta.rs (target/debug/deps/tui_usage_wave39_delta-8e91daca3268ea85)
EXIT_NO_RUN=0
```

### 2.2 红相门禁

```
$ cd /home/dm/pony-agent && cargo test -p pony-agent-core --test tui_usage_wave39_delta 2>&1 | tail -40
running 4 tests
test a11_missing_fields_default_zero ... FAILED
test a11_completed_yields_none ... FAILED
test a11_other_events_yield_none ... FAILED
test a11_output_end_yields_usage ... FAILED

---- a11_missing_fields_default_zero stdout ----
thread 'a11_missing_fields_default_zero' (3103078) panicked at crates/pony-agent-core/tests/../src/bin/tui.rs:1296:5:
not implemented
[同型 panic ×4，均为 tui.rs:1296:5]

test result: FAILED. 0 passed; 4 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
error: test failed, to rerun pass `-p pony-agent-core --test tui_usage_wave39_delta`
EXIT_TEST=101        # 无管道真实退出码（PIPESTATUS[0]）
```

完整日志：`/tmp/w39d-red-run.log`（会话内临时件，全部失败原因已摘录于此）。

## 3. A11 红相失败分布（4/4，单因）

| # | 测试（行号） | 断言目标 | 红相原因 |
|---|---|---|---|
| A11-1 | `a11_output_end_yields_usage`（L46） | `turn:output_end` + 1200/345/600/1545 → `Some(StreamUsage)` 四值一致 | panic `not implemented`（tui.rs:1296 空桩） |
| A11-2 | `a11_completed_yields_none`（L78） | `turn:completed` + 同一 persisted 载荷 → `None`（双发去重根因） | 同上 |
| A11-3 | `a11_other_events_yield_none`（L97） | `turn:delta` / `turn:failed` / `turn:tool` → `None` | 同上（循环三项均 panic） |
| A11-4 | `a11_missing_fields_default_zero`（L114） | 仅 `outputTokens=5` → `Some`，其余三值补 0 | 同上 |

测试构造方式（无假数据旁路）：

- `TurnStreamEvent` 30+ 字段用 `serde_json::from_value` 反序列化构造，只声明契约相关字段，其余为 `None`，与真实 wire 载荷同构；字段名遵循 `#[serde(rename_all = "camelCase")]`（`turnId/kind/inputTokens/outputTokens/cacheHitInputTokens/totalTokens`）。
- `UiEvent` 未实现 `PartialEq`，A11-1/A11-4 用 `match` 逐字段 `assert_eq!`，并断言「不是 StreamUsage 变体即失败」；A11-2/A11-3 用 `is_none()`/`match` 断言。
- 每个测试 AAA 注释齐全；无 `sleep`、无网络、无共享状态。

## 4. `run_audit.sh` wait 模式校正（语义不变，仅换真实信号）

背景：R6 删除了底部状态短语，`完成|Turn completed` 与 `已上翻` 在 wave-39 之后永不出现，`wait_for` 每处空转 8–15s（只耗时不影响断言）。

| 位置 | 改动前 | 改动后 |
|---|---|---|
| L41（新增） | — | `TURN_SETTLED_RE='↑[1-9]'`（注释说明：指标行非零 token = 本轮 usage 回执已 apply，即「本轮落定」；`上下文` 二字恒在，不可作信号） |
| L390 conversation | `"完成\|Turn completed" 15` | `"$TURN_SETTLED_RE" 15`（warn 文案改 `turn usage metrics not seen`） |
| L431 sessions-panel | 同上 | `"$TURN_SETTLED_RE" 15` |
| L458 streaming | 同上 | `"$TURN_SETTLED_RE" 15` |
| L480 narrow | 同上 | `"$TURN_SETTLED_RE" 15` |
| L492 wide | 同上 | `"$TURN_SETTLED_RE" 15` |
| L503 scrolled | 同上 | `"$TURN_SETTLED_RE" 15` |
| L509 scrolled（PageUp） | `"已上翻" 8` | `"第 14 行" 8`（更早正文行进入视口 = 翻页生效，与 `checks.py scrolled.after_shows_earlier_rows` 同一信号） |

- **等待语义保持**：仍是「等到本轮 usage 落定 / 翻页生效」再抓屏，不提前、不放宽；checks.py 一行未改。
- 信号真实性用 task-4 的真实 capture 验证：

```
$ grep -qE '↑[1-9]' .dev-team/evidence/tui-polish/after/conversation/main.txt && echo MATCH
MATCH conversation ↑[1-9]                    # 落定后指标行 ↑32 ↓64
$ grep -qE '↑[1-9]' .dev-team/evidence/tui-polish/after/empty/main.txt || echo "NO MATCH(↑0)"
NO MATCH(↑0)                                # 轮次开始前不会误命中
$ grep -c "第 14 行" .dev-team/evidence/tui-polish/after/scrolled/after.txt
1                                           # PageUp 后视口内确有此行
```

- 语法门禁：

```
$ bash -n tools/tui-audit/run_audit.sh
EXIT_BASH_N=0
$ grep -n "完成|Turn completed\|已上翻" tools/tui-audit/run_audit.sh
507:  # wave-39 R6：footer 的「已上翻」提示已删除；……   # 仅剩注释
```

## 5. 交接给 Executor（task-7）的绿相门禁

1. `cargo test -p pony-agent-core --test tui_usage_wave39_delta` 必须 `4 passed / Exit 0`。
2. R8' 接线：`TuiTurnSink::emit` 的 `"turn:completed" | "turn:output_end"` 分支先
   `if let Some(ev) = stream_usage_event(name, &payload) { send(ev) }` 再发 `StreamCompleted`；
   `turn:completed` 不得再产生 usage（A11-2 冻结这一口径）。
3. 实现仅限 `tui.rs`；不得修改本文件、A1–A10 冻结文件与 checks.py。
4. 复跑 `tui_dialog_wave39`（10/10）与既有回归基线，确认无回退。
