# 绿相收据 · TUI 思考消息 + 消息区底部呼吸间距（thinking-bottom-gap）

- 契约真值源：`.dev-team/contract-matrix-thinking-bottom-gap.md`（C1–C12）
- 验收测试（只读，未改任何断言）：`crates/pony-agent-core/tests/tui_thinking_bottom_gap.rs`
- 实现写域（唯一允许改动的源码）：`crates/pony-agent-core/src/bin/tui.rs`
- 环境：Linux，`cargo` 直跑，全部命令 `set -o pipefail` 取真实退出码
- **未执行任何 git 写操作**（无 add/commit/stash/checkout/reset；仅 `git diff --stat` 只读）

---

## 1. 验收命令与真实结果

### 1.1 唯一验收测试（任务书命令 a）

```bash
cd /home/dm/pony-agent && set -o pipefail && \
cargo test --manifest-path crates/pony-agent-core/Cargo.toml \
  --test tui_thinking_bottom_gap --target-dir target-test 2>&1 | tail -40
```

- **真实退出码：0**
- `test result: ok. 27 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.14s`
- 说明：任务书预估「21 passed」，实际 27 passed——差异来自 `#[path = "../src/bin/tui.rs"] mod tui;`
  会把 `tui.rs` 内的 `#[cfg(test)]` 单元测试一并编进同一个 test harness。本契约 11 条 A 用例
  （A1–A11）全部 ok；其余 16 条为 tui.rs 既有单元测试（含并行会话新落地的
  `wave65_approval_unit_tests`、`splash_gradient_tests`、`wave43_unit_tests`），均 ok。

### 1.2 全量 TUI 回归（排除 `tui_terminal_restore`，任务书命令 b）

```bash
cd /home/dm/pony-agent/crates/pony-agent-core && args=""; \
for f in tests/tui_*.rs; do b=$(basename $f .rs); \
  [ "$b" = "tui_terminal_restore" ] && continue; args="$args --test $b"; done; \
cd /home/dm/pony-agent && set -o pipefail && \
cargo test --manifest-path crates/pony-agent-core/Cargo.toml --target-dir target-test $args 2>&1 | tail -40
```

- **真实退出码：101**（唯一失败目标 = `tui_overlay_cancel_acceptance`，见 §4 既有外部状态）
- 34 个 `tui_*` 目标，cargo 在失败目标处停止后续目标，故分两段取证：

第一段（截至 `tui_overlay_cancel_acceptance`）：

| 目标 | 结果 |
| :-- | :-- |
| tui_acceptance | ok. 51 passed |
| tui_audit_ask_loop_regression | ok |
| tui_audit_hermeticity_regression | ok |
| tui_audit_no_panic_regression | ok |
| tui_audit_scenarios_regression | ok |
| tui_batch1..5_regression | ok |
| tui_batch6_virtualization | ok. 22 passed |
| tui_dialog_wave39 | ok. 29 passed |
| tui_enter_key_wave40 | ok. 20 passed |
| tui_interaction_acceptance | ok. 27 passed |
| tui_interaction_adversarial | ok. 20 passed |
| tui_metrics_snapshot | ok. 20 passed |
| tui_mouse_selection_acceptance | ok. 27 passed |
| tui_mouse_selection_adversarial | ok. 20 passed |
| **tui_overlay_cancel_acceptance** | **FAILED. 30 passed; 2 failed（c9 / j1，既有外部状态，见 §4）** |

第二段（失败目标之后的剩余目标，单独跑，**退出码 0**）：

```bash
cd /home/dm/pony-agent && set -o pipefail && \
cargo test --manifest-path crates/pony-agent-core/Cargo.toml --target-dir target-test \
  --test tui_provider_refactor_test --test tui_render_layout --test tui_resume_cli \
  --test tui_session_resilience_acceptance --test tui_session_resilience_chunk_split \
  --test tui_session_resilience_l3b --test tui_six_fixes_acceptance --test tui_stop_key_feedback \
  --test tui_stop_static_wave60 --test tui_stop_wave60 --test tui_streaming_input_acceptance \
  --test tui_streaming_input_adversarial --test tui_thinking_bottom_gap \
  --test tui_token_context_wave41 --test tui_usage_wave39_delta 2>&1 | grep -E "^test result|Running tests"
```

- **真实退出码：0**
- 逐目标：provider_refactor 2 / **render_layout 25**（基线 23，本轮文件内新增用例来自并行会话）/
  resume_cli 21 / session_resilience_acceptance 28 / chunk_split 2 / l3b 4 / six_fixes 25 /
  stop_key_feedback 19 / stop_static_wave60 3 / stop_wave60 19 / streaming_input_acceptance 39 /
  streaming_input_adversarial 23 / thinking_bottom_gap 27 / token_context_wave41 21 / usage_wave39_delta 20
  —— **全部 0 failed**。

### 1.3 编译门禁（任务书命令 c）

```bash
cd /home/dm/pony-agent && set -o pipefail && \
cargo check --manifest-path crates/pony-agent-core/Cargo.toml --bin pony --target-dir target-check
```

- **真实退出码：0**（`Finished dev profile in 13.34s`，无 warning/error 增量）

---

## 2. 改动面（`git diff --stat`，只读）

```bash
cd /home/dm/pony-agent && git diff --stat -- crates/pony-agent-core/src/bin/tui.rs
#  crates/pony-agent-core/src/bin/tui.rs | 178 +++++++++++++++++++++++++++++++---
#  1 file changed, 166 insertions(+), 12 deletions(-)
```

> 口径说明：该文件**同时承载另一并行会话（approval-resume）的未提交改动**，故 diffstat 不等于本任务改动量。
> 本任务自己的改动 = 下列 6 处（合计 +110 / −7 行量级），其余增量属并行会话，未触碰。

| # | 位置 | 内容 | 契约 |
| :-- | :-- | :-- | :-- |
| 1 | `tui.rs:3677` `TuiState::apply_stream_reasoning` | 空白 chunk 早退；末条 Thinking 续写否则新开；非 `ExecutingTool` ⇒ `transition_runtime_mode(Thinking)`；不动 `is_streaming`/`scroll_offset`/`auto_scroll` | C4 / C5 |
| 2 | `tui.rs:4624` `stream_delta_ui_payload` | `text` 走 `sanitize_model_protocol_text`，净化后 `trim` 空 ⇒ `None`；`reasoning`（`payload.reasoning_content`）原样透传、仅 `trim` 空 ⇒ `None`；双空 ⇒ `DeltaUiPayload::default()` | C1 / C2 / C3 |
| 3 | `tui.rs:6249` `MessageRole::Thinking` 渲染分支 | 首行 `✻ `（`theme::ACCENT_DIM`）+ 正文 `theme::FAINT` + `Modifier::ITALIC`；续行 `rail_blank()`（RAIL=2）缩进；`push_prefixed(..., width-RAIL)` 折行；前序非 Thinking ⇒ 块首 `push_blank`；块尾 1 行呼吸 | C6 / C7 |
| 4 | `tui.rs:6152` `MessageRole::Assistant` 分支 | 新增 `prev_role == Some(Thinking)` 显式分支：**不再叠加**间距（1 行呼吸由思考块尾部承担，避免 2 行轮间距） | C7 |
| 5 | `tui.rs:6607` `ui()` 布局 | `chat_h = bottom_top - chat_top - CHAT_BOTTOM_GAP`；`chat_rect` 仍 `height = chat_h.max(1)`、`last_chat_rect` 仍写同一 rect；Composer / 指标行几何零改动 | C8 / C11 |
| 6 | `tui.rs:6781` / `tui.rs:6825` `ui()` 滚动锚定 | 逻辑尾部空行（消息块自带下内边距）不计入 `scroll_total`，使「间距行 = 最后一条内容行之后的第一行」成立 | C8 / C9 / C10 |
| 6b | `tui.rs:6619` `ui()` Ask 覆盖层 | Ask 面板下缘仍贴 Composer 色带上缘（允许占用间距行）——覆盖层不是布局项，保住既有 O2 断言 | C8 边界（无回归） |

### 关键实现决定（有据可查）

1. **C7 的 1 行呼吸放在思考块尾部**，而不是 Assistant 块首。原因（可机械复现的事实）：
   `message_lines_with_ctx` 对每条消息都返回**独立空 Vec**，块首的 `push_blank`/`push_gap`
   在 `lines.last() == None` 时直接 return（不产生行）；因此只有在思考块内部 `push(blank())`
   才能真出一行。实测确认：若两端都插则 A7 间隔变 2 行（红）。
2. **C9 需要「尾部空行不参与滚动锚定」**：用户消息块以 SURFACE 下内边距行收尾，
   若照旧计入 `total`，贴底时视口末行落在该空白内边距上，`gap_row - 1` 为空白 ⇒ A8 红。
   剔除尾部空行后，视口末行 = 最后一条**内容**行，间距行紧随其后。流式状态行（非空）不受影响，
   故 A9「状态行是内容区最后一行」同样成立。
3. **Ask 覆盖层下缘 +1 行**：`chat_rect` 下缘内收 1 行后，若仍让面板止于 `chat_rect` 下缘，
   `tui_overlay_cancel_acceptance::o2` 会红（期望 row29 / 实际 row28）。覆盖层语义上不是布局项，
   故其下缘锚定 Composer 色带上缘（`chat_rect` 下缘 + `CHAT_BOTTOM_GAP`，再 `min(bottom_top)`），
   消息区几何（O2 Assert 1 的 `chat_rect` 等值断言）不受影响。o2/o1/o3 均绿。

---

## 3. 契约逐条对照（C1–C12）

| 契约 | 落地 | 断言载体 |
| :-- | :-- | :-- |
| C1 取载唯一面 | sink `turn:delta` 分支只调 `stream_delta_ui_payload`（Lead 已接线，本轮补实现） | A1/A2 |
| C2 双通道独立 / 双空 | `DeltaUiPayload { text, reasoning }` 各自独立，双空 = `default()` | A1/A2 |
| C3 text 净化 / reasoning 原样 | 仅 text 走 sanitize；reasoning 仅 trim 判空 | A2 |
| C4 入库口径 | 末条 Thinking 续写，否则新开；空白 chunk 不改 messages | A3/A4 |
| C5 运行态 | 非 `ExecutingTool` ⇒ `Thinking`；三字段（streaming/scroll/auto_scroll）不写 | A5 |
| C6 渲染 | `✻` + FAINT + ITALIC + RAIL 续行 | A6 |
| C7 思考/正文分列 + 1 行呼吸 | 角色序列独立；间距恰 1 行 | A4/A7 |
| C8 底部间距恒定 | `chat_h` 扣 `CHAT_BOTTOM_GAP`；`last_chat_rect.y+height == gap_row` | A8/A9 |
| C9 不靠内容短碰巧 | 40 条消息溢出视口，贴底后间距行之上仍是内容行 | A8 |
| C10 流式状态行同样留距 | 状态行为内容区最后一行，其下 1 行空白 | A9 |
| C11 极小终端 | 24×10：`saturating_sub`、无 panic、下缘 ≤ `prompt-2` | A10 |
| C12 既有角色不变 | `✻` 不泄漏，User/Assistant/ToolCall/System 渲染不变 | A11 + 全量回归 |

---

## 4. 残余风险与既有外部状态（如实记录，未做任何规避性改动）

### 4.1 `tui_overlay_cancel_acceptance::c9` / `::j1` —— **既有失败，与本轮改动无因果**

- 现象：两条 PTY 真二进制用例报「mock 慢速流未开始 / 种子会话未作答」。
- 根因定位（物理取证）：PTY 内启动的 `pony` **进程存活但主线程卡在互斥量**上——
  `/proc/<pid>/task/*` 快照：主线程 `state=S wchan=futex_wait_queue`（utime 3 秒内仅 +1，非热旋），
  输入线程 `pony ... wchan=n_tty_read` 阻塞在终端读；随后向 PTY 写入 `a` / `\x1b[97u`
  **均无任何重绘输出**（输入从未到达事件循环）。
- **独立性证明（关键收据）**：把本轮 6 处改动**全部回退**到 Lead 冻结的空桩基线
  （恢复 3 个 `todo!()`、原 `chat_h`/滚动/Assistant 分支），`cargo build --bin pony --target-dir target-test`
  后跑同一 PTY 探针 → **症状逐字相同**（键入无回显、`streaming=false`）。
  即：该卡死在本轮改动之前就存在，**不是本轮 4 处实现引入的**；Lead 关于「空桩 todo!() panic 导致」
  的推断与观测不符（无 `panicked at`，且实现完成后症状不变）。
- 另证：`target-check/debug/pony`（11-03 旧构建，早于本轮全部改动）在同一 PTY 探针下**能正常回显输入**，
  说明回归来自工作区里其它未提交改动（并行 approval-resume 会话 / 启动期键盘协议改动），
  不属于本任务写域；按硬约束未去追改。

### 4.2 `tui_overlay_cancel_acceptance::o1` —— 偶发（环境竞争），非本改动

- 一次全量跑中 o1 红（`header 行/指标行与基线不一致`），单跑与随后整目标重跑均绿。
- 同期真实用户配置 `~/.config/pony-agent/providers.json` 的 mtime 被其它会话的 e2e 用例刷新到
  23:04，`selected_provider_id` 短暂指向注册表里不存在的 `provider-ppx`，
  令 `tui_acceptance::test_resolve_active_provider_id_reads_default_registry` 也偶发红
  （单跑 3/3 绿）。两条同属「真实 HOME 配置被并发写」的测试竞争，与本改动无关。

### 4.3 `tui_terminal_restore` —— 按指令排除

另一并行会话在飞的工作（测试已改、实现未落，当前编译失败），按硬约束**未触碰、未修、未迎合**。

### 4.4 未做 git 写操作

工作区存在并行会话未提交改动；本轮全程仅 `git diff --stat`（只读）。

---

## 5. 变更流水

- 2026-11-06 Executor（L2-E）绿相收据：C1–C12 实现完成，唯一验收测试 exit 0（27 passed，含 A1–A11）；
  全量 TUI 回归除 §4.1 既有外部失败外全绿；`cargo check --bin pony` exit 0。

---

## 6. 复审修复（L3-B F1/F4 + L3-A F6/F7，task-4）

依据：`.dev-team/review-thinking-gap-b.json`（BLOCKING F1 + ADVISORY F4）与
`.dev-team/review-thinking-gap-a.json`（BLOCKING F6 + ADVISORY→修复 F7），Lead 裁决并原地修订契约 C3/C4a/C6/C11。
写域不变：`crates/pony-agent-core/src/bin/tui.rs` + 本收据。**仍未做任何 git 写操作**。

### 6.1 四处改动

| # | 位置 | 改动 | 契约 |
| :-- | :-- | :-- | :-- |
| F1 | `tui.rs:6278` `MessageRole::Thinking` 块首 | 死代码修复：块内 `push_blank` 在**空 Vec** 上恒为 no-op（每消息新建 Vec，`lines.last()==None` 直接 return），改为条件「前驱存在且**不是 Thinking、也不是 User**」时 `lines.push(blank())`。User 前驱保持 0 行叠加（用户块自带 1 行内边距）；块尾 1 行呼吸不动 | C6（修订 R1） |
| F4 | `tui.rs:6692` `ui()` 空态 | `show_banner = (chat_h as usize + CHAT_BOTTOM_GAP as usize) >= 24`，把 C8 扣掉的呼吸行补回阈值，恢复改动前可用性（height=31/35 品牌画回归） | C11 / §2 布局 |
| F6 | `tui.rs:4653` `stream_delta_ui_payload` | 正文判空从 `!cleaned.trim().is_empty()` 改回 HEAD 口径 `!cleaned.is_empty()`：纯空白增量（`"\n\n"`/`" "`）是段落/换行载体，必须原样透传；DSML 分片净化结果仍是空串 ⇒ 语义不变。reasoning 通道维持 `trim().is_empty()` 丢弃 | C3（修订 R3） |
| F7 | `tui.rs:2519 / 2632 / 2983 / 3702 / 3718` | 思考按轮分块：新增私有字段 `reasoning_turn_closed: bool`（`Default` 补 `false`）；`begin_turn_usage()`（`turn:started` 公共复位点）置 `true`；`apply_stream_reasoning` 续写条件加 `&& !turn_closed`（先读局部变量避免借用冲突），函数尾复位 `false` | C4a（修订 R4） |

### 6.2 验收命令与真实结果

```bash
cd /home/dm/pony-agent && set -o pipefail && \
cargo test --manifest-path crates/pony-agent-core/Cargo.toml --target-dir target-test \
  --test tui_thinking_bottom_gap --test tui_render_layout \
  --test tui_batch6_virtualization --test tui_dialog_wave39 --test tui_acceptance 2>&1 \
  | grep -E "^test (a1[234])|^test result|Running tests|FAILED"
```

- **真实退出码：0**
  - `tui_thinking_bottom_gap`：**33 passed**（含 `a12_thinking_keeps_one_breath_against_previous_non_user_blocks`、
    `a13_delta_payload_preserves_whitespace_only_text`、`a14_thinking_block_is_bounded_by_turn_started_reset` 均 ok）
  - `tui_render_layout`：28 passed
  - `tui_batch6_virtualization`：25 passed
  - `tui_dialog_wave39`：32 passed
  - `tui_acceptance`：55 passed

```bash
cd /home/dm/pony-agent && set -o pipefail && \
cargo check --manifest-path crates/pony-agent-core/Cargo.toml --bin pony --target-dir target-check
```

- **真实退出码：0**

### 6.3 F1 / F4 的物理取证（探针式，非主观判断）

- **F1**：A12 四条序列即原生判据 —— Assistant→Thinking 行差 2、ToolResult→Thinking 行差 2、
  User→Thinking 行差 2（不叠加成 3）、Thinking→User 行差 3（跨轮沿用 2 行空白，不撑成 4）。修复后全绿。
- **F4（含反向突变探针）**：临时在 `tui.rs` 内加一次性 `#[cfg(test)] mod tmp_f4_probe`（TestBackend 渲染空态，
  读品牌画哨兵），跑 `cargo test --bin pony probe_tmp_banner -- --nocapture`：
  - 修复态：`height=31 banner=true / 34 true / 35 true / 36 true` → **exit 0**；
  - 反向突变（阈值改回 `chat_h as usize >= 24 // MUTATION-PROBE`）：`height=31 banner=false` → **exit 101**
    （复审 F4 的反例被原生复现）。
  - 探针模块用完**已整块删除**（`grep -c tmp_f4_probe` = 0，文件回到 12240 行），不留在交付物里。

### 6.4 `git diff --stat`（只读，仍含并行会话改动）

```bash
cd /home/dm/pony-agent && git diff --stat -- crates/pony-agent-core/src/bin/tui.rs
#  crates/pony-agent-core/src/bin/tui.rs | 208 +++++++++++++++++++++++++++++++---
#  1 file changed, 195 insertions(+), 13 deletions(-)
```

### 6.5 残余说明

- `tui_acceptance::test_resolve_active_provider_id_reads_default_registry` 在多目标合并跑时偶发红一次
  （报 `provider-ppx` 不在默认注册表），单跑与随后合并重跑均绿；同期观测到真实用户配置
  `~/.config/pony-agent/providers.json` 被并行会话的 e2e 反复重写（mtime 23:31），属**进程级 HOME 配置竞态**，
  与本改动无关（该用例不经过 tui.rs 本轮任何改动路径）。
- L3-B 的 F2（`scroll_total` 剔尾随空白）/ F3（极小终端让位）/ F5（Thinking 打散「末条即运行中 ToolCall」判据）
  经 Lead 裁决已分别登记为契约修订（C11 修订 R2 等）与待观测残余；本轮**未改动**其语义。

---

## 7. 底部间距 1 → 2（契约 C8 修订 R5 / C11，task-6）

依据：用户实测反馈「1 行偏挤」，契约 C8 原地修订为 `CHAT_BOTTOM_GAP = 2`，C11 判据同步 `prompt_row - 3`。

### 7.1 改动（唯一一处常量 + 注释同步）

```diff
-/// 之间**恒定保留**的空白行数（阅读呼吸，防止最后一行正文/状态行贴住输入框）。
-pub const CHAT_BOTTOM_GAP: u16 = 1;
+/// 之间**恒定保留**的空白行数（阅读呼吸，防止最后一行正文/状态行贴住输入框）。
+///
+/// 修订 R5（2026-11-06 用户实测）：1 行偏挤 ⇒ 2 行。所有几何推导（`chat_h` 的扣除、Ask 覆盖层
+/// 的下缘补偿、`show_banner` 的阈值补偿）都必须继续引用本常量，禁止在别处写死 1/2。
+pub const CHAT_BOTTOM_GAP: u16 = 2;
```

- 位置：`crates/pony-agent-core/src/bin/tui.rs:4918`
- **未写死**：三处几何全部照旧引用常量（复核 grep 结果）
  - `tui.rs:6661` `chat_h = bottom_top.saturating_sub(chat_top).saturating_sub(CHAT_BOTTOM_GAP)`
  - `tui.rs:6676` Ask 覆盖层 `ask_bottom = chat_rect.y + chat_rect.height + CHAT_BOTTOM_GAP`（再 `.min(bottom_top)`）
  - `tui.rs:6710` `show_banner = (chat_h as usize + CHAT_BOTTOM_GAP as usize) >= 24`
- Composer / 指标行几何、`last_chat_rect` 写入口径零改动。

### 7.2 验收命令与真实结果

```bash
cd /home/dm/pony-agent && set -o pipefail && \
cargo test --manifest-path crates/pony-agent-core/Cargo.toml --target-dir target-test \
  --test tui_thinking_bottom_gap --test tui_render_layout \
  --test tui_batch6_virtualization --test tui_dialog_wave39 --test tui_acceptance 2>&1 \
  | grep -E "^test a|^test result|Running tests|FAILED"
```

- **真实退出码：0**
  - `tui_thinking_bottom_gap`：**35 passed**（A1–A14 全绿；A8 已随动断言 `CHAT_BOTTOM_GAP == 2`、
    间距行 = `[prompt-3, prompt-2]` 且两行均为 CANVAS 底色，`band_top` 仍为 SURFACE；A9 状态行下同为 2 行；
    A10 判据改为 `prompt_row - 3`）
  - `tui_render_layout`：30 passed
  - `tui_batch6_virtualization`：27 passed
  - `tui_dialog_wave39`：34 passed
  - `tui_acceptance`：57 passed

```bash
cd /home/dm/pony-agent && set -o pipefail && \
cargo check --manifest-path crates/pony-agent-core/Cargo.toml --bin pony --target-dir target-check
```

- **真实退出码：0**

### 7.3 24×10 极小终端边界说明（`chat_h = 1`）

24×10、空输入（`visual_lines = 1` ⇒ `input_h = 2`、`bottom_h = 4`）的实际几何：

| 量 | 值 | 推导 |
| :-- | :-- | :-- |
| `bottom_top`（Composer 色带上缘） | 6 | `area.height - bottom_h = 10 - 4` |
| `chat_top` | 3 | `content.y + 3`（header + hairline + 1 行呼吸） |
| `chat_h` | **1** | `6 - 3 - CHAT_BOTTOM_GAP(2)`（未触发饱和） |
| `chat_rect` | `{y:3, h:1}` | `height = chat_h.max(1)`，下缘 4 |
| 间距行 | y=4、y=5 | 2 行，均在 Composer 色带之外、CANVAS 底色 |
| `prompt_row` | 7 | `bottom_top + 1` |
| C11 判据 | `4 ≤ 7 - 3 = 4` | 恰好取等，未挤压 Composer |

要点：GAP 由 1→2 后，24×10 下消息区从 2 行收为 **1 行**（仍 ≥ 契约下限，未触发 `saturating` 兜底、
未降级为「终端窗口过小」、未 panic）。这正是契约 C11 修订 R2 登记的 best-effort 边界：若 Composer 多行输入
再把高度压到 ≤1 行，`chat_h.max(1)` 会让位（间距行退化为 1 行或贴边），属已裁决的边界行为，A10 覆盖空输入档位。

### 7.4 `git diff --stat`（只读，仍含并行会话改动）

```bash
cd /home/dm/pony-agent && git diff --stat -- crates/pony-agent-core/src/bin/tui.rs
#  crates/pony-agent-core/src/bin/tui.rs | 210 +++++++++++++++++++++++++++++++---
#  1 file changed, 197 insertions(+), 13 deletions(-)
```
