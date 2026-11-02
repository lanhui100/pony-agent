# Wave-Slash-1 & TUI 深度加固交付说明

- **交付日期**：2026-11-02
- **涉及范围**：桌面端（Vue3 + Tauri）斜杠命令系统、TUI 命令表收敛、TUI 核心状态机/异常鲁棒性加固、tmux 真终端审计扩展
- **契约真值源**：[contract-matrix-slash-commands.md](../../../.dev-team/contract-matrix-slash-commands.md)（修订 1 ~ 13）
- **真终端审计报告**：`.dev-team/evidence/tui-polish/after/report.md`（419 PASS / 0 FAIL / 1 SKIPPED across 22 scenarios）

---

## 1. 核心交付成果

### 1.1 桌面端斜杠命令系统（Vue3 + Tauri）
按照契约要求，桌面端仅实现且精确支持以下 3 个斜杠命令，未命中命令保持普通消息发送：
1. `/plan`：展开/切换右侧栏「计划」面板（有计划/任务时切换，无数据时提示「当前会话暂无计划」）。
2. `/tasks`：展开/切换「Team 任务看板」（无任务时给出友好提示）。
3. `/compact`：调用后端真实上下文压缩 `compact_session`，提示归因信息（已压缩 N 条、保留 M 条、前后 token 变化），同时在提示中明确说明「后端上下文已压缩，转录区保留原始历史」。
4. 运行中保护：当 `isSubmitting` 为真时，回车或点击发送斜杠命令统一被拦截并提示「当前正在运行，请等待本轮结束后再执行斜杠命令」，且保留输入框草稿与待发送附件，杜绝中途改写历史。

### 1.2 压缩内核（Rust Core）
- 新增 `compaction_ops.rs`，复用 `agent/compression.rs` 的纯数据变换。
- **严格三段式锁纪律**：短读锁取快照 → 立即释放 sessions 锁 → 无锁调用 LLM 生成结构化摘要 → 短写锁回写，彻底杜绝持有 sessions 锁做网络调用。
- **可靠性硬防线**：
  - 60s 硬超时上界（`COMPACTION_TIMEOUT_SECS`），超时返 `compaction_deadline_exceeded`。
  - 空摘要防线（`empty_summary`）：LLM 返回空串或空白标签时拒绝回写，防止数据丢失。
  - 并发改写防线：`concurrent_suffix` 校验前缀，若会话已被并发修改或清空，安全终止并返回 `compaction_history_changed`，宁可不压缩也不覆盖新数据。

### 1.3 TUI 全面诊断与 8 批收敛加固
针对 TUI 进行了全面架构与代码诊断，实施了 8 批靶向加固与修复：
- **批 1（崩溃与粘贴）**：
  - 修复窄终端（24~33 列）输入 `/` 触发 `need_w.clamp(28, ...)` 导致的 `min <= max` panic 崩溃。
  - 修复命令解析首 token 误吞正文回归：抽离 `parse_command_input`（整串匹配），保证 `/clear 需求拆解` 正常作为消息发送而不误清空上下文。
  - 接入 `EnableBracketedPaste` / `DisableBracketedPaste`，修复多行粘贴被当成首行回车立即提交且后续内容被丢弃的严重缺陷。
- **批 2（生命周期与中断）**：
  - 实现 `TerminalGuard` RAII 守卫：在 `enable_raw_mode()` 成功后立即建立，即使后续 `Terminal::new()` 失败或发生 panic 也能在 `Drop` 中稳定恢复终端（关闭 raw 模式、退出备用屏、显示光标）。
  - 修复输入轮询线程死循环与 100% CPU 空转（`classify_input_poll` 捕获 IO 错误后退出并界面告警）。
  - 修复多线程中 `unsafe { env::remove_var("NO_COLOR") }` 的 UB 数据竞争（移至单线程启动期）。
  - 修复 Esc 假中断：引入 `GoalRoundCancelToken`，续轮前检查取消令牌，每轮动态回写 `current_turn_id`，使 Esc 能真正中断后台多轮执行。
- **批 3（状态一致性）**：
  - 修复 `/provider` 切换不生效假象：选择即通过 `save_view` 持久化，`resolve_active_provider_id` 改为 state 优先，发送路径真正跟随选择。
  - 抽离 `reset_session_scope()`：`/clear` 与 `/resume` 统一彻底复位 token 指标、计划、任务、滚动等 15+ 处状态，彻底解决跨会话脏数据串联。
  - 引入覆盖层 `mode_stack`：审批与问答弹窗按 LIFO 栈回退；回退到 `AddingProvider` 时自动清空输入缓冲区，防止敏感 api_key 泄露进 composer。
  - 优化 Ctrl+C：流式中触发取消中断而不退出 TUI；非流式有草稿时先清草稿，空草稿才退出。
  - 新增非流式 `status_notice`（TTL 4s）：解决非流式期状态提示（如复制、审批失败）被完全静默的缺陷。
- **批 4（滚动越界与审计去伪）**：
  - 滚动偏移量全面升级为 `u32`（`scroll_offset`、`max_scroll_offset`），结合 `clamp_render_scroll` 饱和转换，彻底消除超 65535 行时回绕跳顶缺陷。
  - 每帧渲染去重：`workspace_label` 与 `matching_commands` 单帧只算一次；工具执行动效不再每帧 clone 大文本。
  - 抽离 `apply_models_discovered` 生产函数，消除测试中自我克隆的假测试。
  - 审计 `<cap>.no_panic` 改造为双源判定（tmux 屏幕抓取 + 独占运行时日志），并引入 `skipped` 三态与字节偏移过滤，根除读全局历史日志导致的假红。
- **批 5（可用性与诚实化）**：
  - 新增 `gateway_display`：剥离 userinfo/query 凭据，在 `/help` 尾部安全呈现网关 URL，补齐删除 `/cost` 后的网关不可见性。
  - 修复 Ask 交互：移除无选项时伪造的「确认/取消」项；多问（multi-question）场景逐问推进，答完全部才统一提交。
  - `/plan` 界面诚实化：底部常驻提示「计划模式 · 仅界面提示，不限制工具执行」，去掉只读误导。
- **批 6（性能虚拟化）**：
  - 落地 `message_lines_for` 纯函数。
  - 引入 `RenderCache` 消息级布局缓存：未变动消息完全复用，每帧仅重算流式末条。
  - 视口虚拟化：组装给 `Paragraph` 的行数只切取当前可见窗口，`Paragraph` 内部无需全量滚动；`rendered_lines_text` 改为仅在鼠标选区时按需惰性拼接。
- **批 7 & 批 8（tmux 真终端审计闭环）**：
  - 扩展 tmux 真实终端审计用例至 22 个场景、420 条严格断言。
  - 新增 `paste-multiline`（真实 bracketed paste 通路验证）。
  - 新增 `resize-invalidate`（动态窗口尺寸缩放与缓存失效逐行一致性比对）。
  - 新增 `contrast`（ANSI 真实 SGR 转 WCAG 对比度实测计算）。
  - 新增 `ask-loop` 真实闭环：利用 debug-only 引导（`#[cfg(debug_assertions)]`）通过生产管道注入真实 `builtin:ask_user`，完整验证面板出现 → 导航 → 提交 → answer_ask CAS 消费 → 5s 不重开的全链路。

---

## 2. 变更文件清单

### 2.1 桌面端（Vue / TypeScript）
- `src/lib/chat/slash-commands.ts` (新增：斜杠命令纯函数解析与结果格式化)
- `src/stores/workspace-panels.ts` (新增：右侧栏面板状态 Pinia 共享 Store)
- `src/components/HomeSidebar.vue` (消费共享面板 Store，保持原有点击折叠语义)
- `src/components/HomeWorkspace.vue` (回车与按钮发送拦截、运行中忙碌保护、/plan /tasks /compact 分发)
- `src/stores/runtime.ts` (新增 `sessionNotice` 与 `setSessionNotice` 状态通知通道)
- `src/lib/runtime/status.ts` (状态模式增补 `info` 类型)
- `src/components/chat/AgentRuntimeStatusBar.vue` (`info` 模式语义色渲染)

### 2.2 核心与宿主（Rust Core & Tauri）
- `crates/pony-agent-core/src/bin/tui.rs` (TUI 核心事件循环、渲染、状态机全面加固)
- `crates/pony-agent-core/src/bin/tui/protocol.rs` (TUI 状态机协议定义、多问推进支持)
- `crates/pony-agent-core/src/bin/cli.rs` (移除旧 `/cost`，将 `/compact` 接入真实控制面压缩)
- `crates/pony-agent-core/src/agent/runtime/compaction_ops.rs` (新增：手动上下文压缩核心业务逻辑)
- `crates/pony-agent-core/src/agent/runtime/mod.rs` (注册导出 `compaction_ops`)
- `crates/pony-agent-core/src/agent/control_plane/compaction_commands.rs` (新增：控制面 `compact_session` 实现)
- `crates/pony-agent-core/src/agent/control_plane/mod.rs` (注册导出控制面压缩命令)
- `src-tauri/src/lib.rs` (注册 `compact_session` 异步 Tauri 命令)
- `docs/observability.yaml` (如实声明 `compact_session` 接口及手动压缩不产生独立 span 的结构性理由)

### 2.3 审计与测试工具链
- `tools/tui-audit/run_audit.sh` (新增场景分发、每场景独占日志与 offset、清理陈旧产物)
- `tools/tui-audit/checks.py` (新增对比度计算、双源 no_panic 判定、各场景断言、防崩溃 `capture_or_fail`)
- `tools/tui-audit/mock_llm.py` (增强 mock 工具调用支持)

### 2.4 新增验收与回归测试
- `tests/slash-commands.spec.ts` (前端 14 条单元测试：命令匹配、提交拦截、忙碌保护、IPC 调用)
- `crates/pony-agent-core/tests/slash_commands_acceptance.rs` (Rust 斜杠命令核心契约验收，15 passed)
- `crates/pony-agent-core/tests/tui_batch1_regression.rs` (批 1 窄屏崩溃与命令守卫测试，12 passed)
- `crates/pony-agent-core/tests/tui_batch2_regression.rs` (批 2 RAII 守卫与取消令牌测试，12 passed)
- `crates/pony-agent-core/tests/tui_batch3_regression.rs` (批 3 Provider 切换与覆盖层测试，16 passed)
- `crates/pony-agent-core/tests/tui_batch4_regression.rs` (批 4 滚动 u32 与模型发现映射测试，8 passed)
- `crates/pony-agent-core/tests/tui_batch5_regression.rs` (批 5 网关脱敏与多问推进测试，14 passed)
- `crates/pony-agent-core/tests/tui_batch6_virtualization.rs` (批 6 布局缓存与虚拟化上界测试，9 passed)
- `crates/pony-agent-core/tests/tui_audit_no_panic_regression.rs` (no_panic 判定逻辑测试，4 passed)
- `crates/pony-agent-core/tests/tui_audit_hermeticity_regression.rs` (审计环境隔离性与偏移测试，5 passed)
- `crates/pony-agent-core/tests/tui_audit_scenarios_regression.rs` (审计场景对账与反伪造断言测试，3 passed)
- `crates/pony-agent-core/tests/tui_audit_ask_loop_regression.rs` (Ask 引导与 CAS 闭环测试，8 passed)

---

## 3. 机器物理验收收据

所有测试命令均由机器在当前工作区真实执行并 Exit 0：

### 3.1 前端单元测试（Vitest）
```bash
npx vitest run tests/slash-commands.spec.ts
```
**收据**：
```text
Test Files  1 passed (1)
     Tests  14 passed (14)
  Duration  3.79s
```

### 3.2 Rust 核心与 TUI 回归全量（14 套集成用例）
```bash
cargo test --manifest-path crates/pony-agent-core/Cargo.toml \
  --test slash_commands_acceptance \
  --test tui_acceptance \
  --test tui_render_layout \
  --test tui_batch1_regression \
  --test tui_batch2_regression \
  --test tui_batch3_regression \
  --test tui_batch4_regression \
  --test tui_batch5_regression \
  --test tui_batch6_virtualization \
  --test tui_audit_no_panic_regression \
  --test tui_audit_hermeticity_regression \
  --test tui_audit_scenarios_regression \
  --test tui_audit_ask_loop_regression \
  --test cli_acceptance
```
**收据**：
```text
test result: ok. 5 passed; 0 failed (cli_acceptance)
test result: ok. 15 passed; 0 failed (slash_commands_acceptance)
test result: ok. 39 passed; 0 failed (tui_acceptance)
test result: ok. 8 passed; 0 failed (tui_audit_ask_loop_regression)
test result: ok. 5 passed; 0 failed (tui_audit_hermeticity_regression)
test result: ok. 4 passed; 0 failed (tui_audit_no_panic_regression)
test result: ok. 3 passed; 0 failed (tui_audit_scenarios_regression)
test result: ok. 12 passed; 0 failed (tui_batch1_regression)
test result: ok. 12 passed; 0 failed (tui_batch2_regression)
test result: ok. 16 passed; 0 failed (tui_batch3_regression)
test result: ok. 8 passed; 0 failed (tui_batch4_regression)
test result: ok. 14 passed; 0 failed (tui_batch5_regression)
test result: ok. 9 passed; 0 failed (tui_batch6_virtualization)
test result: ok. 12 passed; 0 failed (tui_render_layout)
全量合计: 162 条测试全部通过 (0 failed, 0 ignored)
```

### 3.3 tmux 真终端集成审计（E2E）
```bash
bash tools/tui-audit/run_audit.sh --phase after --bin target/debug/pony
```
**收据**：
```text
SUMMARY: 419 pass / 0 fail across 22 scenarios
failed_scenarios=
report: /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/report.json
report: /home/dm/pony-agent/.dev-team/evidence/tui-polish/after/report.md
```
- 22 个交互场景无任何失败。
- 1 个合规记录跳过项：`contrast.aa_gaps_recorded`（次要色在暗色背景下对比度为 3.15~4.22，满足 ≥3.0 底线，未达 4.5 AA 标，按设计作为改进项记录在报告中而不阻断）。

---

## 4. 遗留项与后续建议

1. **终端协议硬件限制（Kitty Keyboard Protocol）**：
   `Shift+Enter` 换行依赖终端支持 Kitty 键盘协议。tmux 与常规终端模拟器（如部分平台默认终端）不支持该扩展协议，故 `Shift+Enter` 会安全降级为普通回车提交（已在 Composer 提供行尾反斜杠 `\` + Enter 换行作为通用保底）。
2. **多终端交互的并发执行规约**：
   在自动化测试或 CI 中跑 `run_audit.sh` 时，必须独占运行同一个 socket/phase 目录，避免并发运行互相清空同名场景产物。
3. **工作区状态声明**：
   当前仓库同时存在另一开发会话（wave-59 桌面流式渲染优化）的在途改动。提交代码时请按文件路径和 hunk 分离暂存。

---

## 5. 复跑与验证指引

### 快速自检（约 15 秒）
```bash
# 1. 验证类型与编译
npm run typecheck
cargo check --manifest-path crates/pony-agent-core/Cargo.toml --all-targets

# 2. 前端斜杠命令测试
npx vitest run tests/slash-commands.spec.ts

# 3. 审计脚本语法与逻辑自检
python3 tools/tui-audit/checks.py selfcheck
```

### 完整回归（约 3 分钟）
```bash
# 编译最新 debug 二进制
cargo build --manifest-path crates/pony-agent-core/Cargo.toml --bin pony

# 运行 14 套 Rust 回归测试
cargo test --manifest-path crates/pony-agent-core/Cargo.toml \
  --test slash_commands_acceptance \
  --test tui_acceptance \
  --test tui_render_layout \
  --test tui_batch1_regression \
  --test tui_batch2_regression \
  --test tui_batch3_regression \
  --test tui_batch4_regression \
  --test tui_batch5_regression \
  --test tui_batch6_virtualization \
  --test tui_audit_no_panic_regression \
  --test tui_audit_hermeticity_regression \
  --test tui_audit_scenarios_regression \
  --test tui_audit_ask_loop_regression \
  --test cli_acceptance

# 运行 tmux 真实终端自动化审计
bash tools/tui-audit/run_audit.sh --phase after --bin target/debug/pony
```
