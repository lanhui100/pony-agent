# wave-39 Protocol Agent 回执（L2-P）

- 任务板：`task-1`（W39-P 冻结契约空桩）
- 契约：`.dev-team/contract-matrix-tui-dialog-wave39.md` §1 R1-R3
- 写域：仅 `crates/pony-agent-core/src/bin/tui.rs`（+ 本回执 + NFR 基线），未触碰 tests/、tools/、前端
- 基线提交：`566e465`（工作树仅新增本波次文件）

## 1. 改动清单（file:line）

| 契约 | 内容 | file:line |
|---|---|---|
| R1 | `UiEvent::StreamUsage { input_tokens, output_tokens, cache_hit_input_tokens, total_tokens }`（纯变体） | `crates/pony-agent-core/src/bin/tui.rs:93-101` |
| R2 | `total_input_tokens` / `total_output_tokens` 补 doc（语义=轮次累加），新增 `total_cache_hit_tokens: u64` / `last_context_tokens: u64` / `context_window_tokens: u32` / 私有 `turn_usage: Option<(u64,u64,u64,u64)>` | `tui.rs:258-270` |
| R2 | `impl Default for TuiState`：`0 / 0 / 0 / None` 初始化 | `tui.rs:318-322` |
| R3 | `pub fn begin_turn_usage(&mut self)`（体仅 `unimplemented!()`） | `tui.rs:364-369` |
| R3 | `pub fn apply_turn_usage(&mut self, input, output, cache_hit, total)`（体仅 `unimplemented!()`） | `tui.rs:371-377` |
| R3 | `pub fn resolve_context_window_tokens(state: &TuiState) -> u32`（体仅 `unimplemented!()`） | `tui.rs:661-668` |
| R1 穷尽匹配 | 主循环 `UiEvent::StreamUsage { .. } => unimplemented!()` | `tui.rs:3690-3692` |

未实现 R4-R8（布局/渲染/接线）——保持空桩，由 Executor 在 task-3 落地。

### 关于 `#[allow(...)]` 的说明（门禁必需，非业务逻辑）
- `turn_usage` 字段：`#[allow(dead_code)]` —— 私有字段在冻结阶段无可读点，否则 `-D warnings` 报 `field is never read`。
- 3 个空桩函数：`#[allow(dead_code)]`；两个带参函数另加 `#[allow(unused_variables)]` —— 体为 `unimplemented!()` 时形参未使用，否则报 `unused variable`。
- 均为编译抑制属性，无任何赋值/分支/计算；执行器实现后可直接删除。

## 2. 命令 / 真实退出码 / 关键输出

### 2.0 基线（改动前）
```
$ cd /home/dm/pony-agent && RUSTFLAGS="-D warnings" cargo check -p pony-agent-core --bin pony
    Finished `dev` profile [unoptimized] target(s) in 0.28s
EXIT=0
```
基线本身干净，无既有 warning 需豁免。

### 2.1 编译门禁（改动后 · 最终）
```
$ cd /home/dm/pony-agent && RUSTFLAGS="-D warnings" cargo check -p pony-agent-core --bin pony
    Finished `dev` profile [unoptimized] target(s) in 0.33s
EXIT_CHECK=0
```
中间态如实记录：首次 check 为 `EXIT=101`，报
`error: unused variable: state`（resolve 函数）与 `error: field turn_usage is never read`；
补上述 `#[allow]` 后复跑为 0。无伪造。

### 2.2 NFR 基线
```
$ python3 /home/dm/.agents/skills/dev-team/scripts/dev-nfr-lint.py .dev-team/nfr-baseline-wave39.json
[PASS] dev-nfr-lint 校验通过: .dev-team/nfr-baseline-wave39.json
EXIT_NFR=0
```
`.dev-team/nfr-baseline-wave39.json` 以 wave38 为模板，`resource_release` 新增：
`no_new_threads_or_handles_introduced_by_composer_metrics_line`、
`tui_usage_state_updates_bound_to_main_event_loop`（本轮只加纯数据字段与空桩，无新增线程/句柄/外部调用）。

### 2.3 空桩纯度（抽取文件）
```
$ python3 /home/dm/.agents/skills/dev-team/scripts/dev-stub-lint.py /tmp/wave39-stubs.rs
[PASS] dev-stub-lint 校验通过 (共检查 1 个文件)
EXIT_STUB=0
```

**为什么整文件不适用（如实披露）**：`dev-stub-lint.py` 只用 Python `ast` 解析 `.py`，并用启发式扫 `.ts/.js`；
对 `.rs` 走 `else: errs = []` 分支，即「文件被计数但不做任何规则校验」。实测：
```
$ python3 .../dev-stub-lint.py crates/pony-agent-core/src/bin/tui.rs
[PASS] dev-stub-lint 校验通过 (共检查 1 个文件)     # 空跑，不能作为纯度证据
```
因此另做机械断言：从 tui.rs 抽取 3 个新空桩函数体 + 主循环匹配臂，逐字校验「有效语句 == `unimplemented!()`」：
```
PASS begin_turn_usage: body=['unimplemented!()']
PASS apply_turn_usage: body=['unimplemented!()']
PASS resolve_context_window_tokens: body=['unimplemented!()']
PASS main-loop arm: ['UiEvent::StreamUsage { .. } => unimplemented!(),']
RESULT: all new stubs contain only unimplemented!()
EXIT_PURITY=0
```
`/tmp/wave39-stubs.rs` 是「只含本波次新增契约条目」的人工抽取件，已去掉既有业务代码，
故它不是整文件误报问题的替代品，仅作为 `unimplemented!()` 纯度的机械凭证。

## 3. 工作树审计
```
$ git status --porcelain
 M crates/pony-agent-core/src/bin/tui.rs
?? .dev-team/contract-matrix-tui-dialog-wave39.md
?? .dev-team/nfr-baseline-wave39.json
?? tools/            # 非本 Agent 产出（Test Agent 写域）
$ git diff --stat
 crates/pony-agent-core/src/bin/tui.rs | 50 +++++++++++++++++++++++++++++++++++
 1 file changed, 50 insertions(+)
```
diff 为纯新增（0 删除）；`tests/` 未改动；未运行任何测试（测试裁决归 Test Agent）。
