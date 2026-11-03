# 契约：wave-60 — 可中断停止（结构化 stop_reason + 立即应答 + 可中断读通道 + 宽限期收口）

> 冻结方：Lead。Executor 与 Test Agent 均**只读**本契约，禁止就地改写；演进走「契约演进门禁」。
> 分级：**B 级**（跨 core/tauri/TUI 多模块 + 事件契约增量 + 并发与阻塞通道；命中 blast 关键字 `thread`/`lock`/`io`）。
> 依据：本轮评审意见（DSH 对照）+ 现状取证 `crates/pony-agent-core/src/agent/execution_control.rs:133`、`provider/mod.rs:2722`、`provider/responses_api.rs:52`、`src-tauri/src/lib.rs:158`、`tauri_adapter.rs:44`、`src/stores/runtime.ts:5351`、`bin/tui.rs:6898`。

## 0. 非目标（Non-Goals，本波不做）

1. **不实现 HTTP/SDK 表面**：仅保证停止生命周期在 core 表面中立（不引用 tauri 类型），HTTP 适配留待后续 backlog。
2. **不改持久化历史语义**：取消轮的哨兵句 `用户终止，发送消息可继续。` 继续留在**落库内容**与 `HistoryNodeKind::TurnCancelled` 分类中；本波只改**实时事件的用户可见呈现**，不做历史迁移、不重编号。
3. **不重构为全异步**：不把 turn 执行链从 `spawn_blocking`/同步迭代器改造成 async；只把"读事件"从不可中断改为可中断。
4. **不做取消竞态的全量覆盖**：只覆盖本契约 §2 列出的判据，其余（disposal 竞速、fork 竞态）留待后续。
5. **不改 TUI 的 Ctrl+C 三态语义**、不改 `/quit`、不改 goal 续轮取消令牌。

## 1. 结构化停止原因（唯一词汇表）

```
StopReason = "user" | "parent" | "hook" | "watchdog" | "legacy"
```

- 缺省 `user`（UI 显式停止）；`parent` 子代理/上游级联；`hook` 钩子触发（附 reason 文本，不单独建枚举值）；`watchdog` 前端看门狗兜底；`legacy` 历史迁移无原因记录。
- Rust：`#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all="snake_case")] pub enum StopReason`，wire 字段 `stopReason`（`Option<String>`，`#[serde(default)]`）。
- TS：`TurnStreamEvent.stopReason?: StopReason | null`；`StopReason` 类型与 Rust 同名同值（`src/types/runtime.ts`）。
- **契约纪律**：新增值只能追加，禁止重命名/删除既有值；`error` 字段语义不变（仍为 `stopped_by_user` 等既有码）。

## 2. 验收判据（black-box，Test Agent 依据本节编写；Executor 只读）

### C1 `stop_turn` 应答语义（core）
| 输入 | 期望 `StopTurnResponse` |
|---|---|
| 运行中 turn，reason 缺省 | `accepted=true, state="stopping", stopReason="user"` |
| 运行中 turn，reason="watchdog" | `accepted=true, state="stopping", stopReason="watchdog"` |
| 未知 turn_id | `accepted=false, state="not_found"` |
| 已终态 turn（cancelled/completed/failed） | `accepted=false, state="already_terminal"` |
| 同一 turn 连续两次 stop | 第 1 次 `accepted=true`；第 2 次 `accepted=false, state="stopping"`（幂等，不重复派发） |

- `state` 的**取值是封闭集合** `{"stopping","not_found","already_terminal"}`；旧的 `"running"` 值**废弃**（TS 侧必须同步消费新值）。
- checkpoint 落盘：`stop_requested_at_ms` 与新增 `stop_reason` 同时写入（`stop_reason` 在 `register_turn` 重注册时与 `stop_requested_at_ms` 一并结转）。

### C2 立即应答事件（core → 表面）
- `stop_turn` 被接受时，**同步**（同一次调用内、终态之前）派发恰好一条 `turn:stop_requested`；未被接受时不派发。
- payload：`turnId`（必填）、`sessionId`（checkpoint 有则带）、`stopReason`、`requestedAtMs`、`kind="stop_requested"`、`phase=Some("stopping")`、其余字段缺省/`None`。
- 该事件**不改变** turn 的终态归属：终态仍只有 `turn:completed|failed|cancelled` 三选一。

### C3 终态携带 stop_reason
- `turn:cancelled` 的 `stopReason` 必须等于该 turn 的 checkpoint `stop_reason`（无记录时 `"legacy"`）。
- `turn:completed` / `turn:failed` 的 `stopReason` 为 `None`。
- **wire 兼容**：新字段为**追加**，`#[serde(default)]` + TS 可选字段；旧消费者（不读该字段）行为不变。

### C4 provider 读通道可中断（关键时延判据）
- 现状：`block_on(stream.next())` 在无 chunk 到达时不可中断（仅 30min 总时长上界）。
- 目标：流式读循环在**无新 chunk**期间，仍以 ≤ **`stop_poll_tick_ms = 250`** 的节奏观测取消；`stop_turn` 被接受后，读循环必须在 **≤ 250ms + 250ms** 内退出（本机 CI 判据：≤ 1000ms 硬上界，避免抖动误判）。
- 退出必须以既有取消码语义呈现（沿用 `ProviderStreamGuard::CANCELLED_ERROR_CODE` 的既有行为），**不得**新增对外错误码。
- 回归约束：正常长流（有持续 chunk）不得因 tick 机制被切断；`provider/mod.rs` 既有 stalled-stream 与 long-stream 用例必须继续绿（`mod.rs:7215`、`mod.rs:7238` 一带）。

### C5 宽限期强制收口（supervisor）
- 参数：`stop_grace_ms` 缺省 **2000**，`stop_poll_tick_ms` 缺省 **250**，**必须可注入**（测试用毫秒级小值，禁止 sleep 2s）。
- 语义：stop 被接受后启动监督；若 `stop_grace_ms` 内 checkpoint 仍为 `running`，监督者**幂等地**：① 将 checkpoint 置 `cancelled`（含 `stop_reason`）；② 通过该 turn 注册的终端发射器派发**一条** `turn:cancelled`。
- **幂等键**：每 turn 至多一次"监督者派发终态"；若循环随后自行终止并再派发一条 `turn:cancelled`，表面层必须按 `activeTurnId` 门禁丢弃（已有语义），core 不要求全局去重，但要求**监督者自身**不重复派发。
- 若在宽限期内循环已终止（checkpoint 非 `running`），监督者**必须不派发**任何事件。
- 未注册终端发射器的 turn（如 CLI 无 sink 场景）→ 监督者只落盘 checkpoint，不派发事件（不得 panic）。

### C6 任务注册表按 turn_id 隔离（tauri）
- 注册表 key 从 `session_id` 改为 `turn_id`；同会话两轮并发时，停止 A **不得** abort B 的句柄。
- `stop_turn` 命令的副作用顺序固定为：**① `request_stop`（含 C2 事件）→ ② 取消该 turn 的取消令牌 → ③ abort 该 turn 句柄（仅 L2 兜底）**；顺序反转视为 FAIL。
- `spawn_blocking` 不可抢占是**已知事实**：abort 只是表项回收，禁止把"abort 成功"当作"已停止"呈现给用户。

### C7 桌面端呈现（前端）
- 点击停止后**立即**（RPC ack 后一个微任务内，不等终态）进入 `stopping` 态：`phaseLabel` 新增 `stopping: "正在停止…"`，停止按钮进入禁用/进行中外观，状态区显示"正在停止…"。
- 收到 `turn:cancelled` 且 `stopReason` 存在时：状态消息显示 **"已停止"**（`user` → "你已停止本轮 · 发送新消息可继续"）；**不得**用哨兵句覆盖已流式正文（现状 `runtime.ts:4966` 的 `payload.text` 分支必须走 stopReason 分支）。
- `turn:stop_requested` 事件到达时若本地尚未进入 stopping，也必须切到 stopping（跨窗口/重连路径）。
- `stopping` 态必须有**兜底退出**：终态事件到达或 watchdog 触发后必须回到非 running 态，禁止永久卡在"正在停止…"。

### C8 双击 Esc 快捷键（桌面端 + TUI，共享判定语义）
- 判定窗口 `stop_sequence_ms = 500`（可注入）；**同一身份** = (sessionId, turnId, generation) 全等。
- 第 1 次 Esc（流式中）：仅"上膛"，**不**发停止请求；窗口内第 2 次 Esc：**恰好一次** stop 请求。
- 超出窗口、身份变化、turn 结束、会话切换 → 上膛失效。
- 非流式态 Esc 行为**保持不变**（TUI 清输入/关弹窗；桌面端现有语义不变）。
- 桌面端与 TUI 各有一份实现，但**语义必须逐条对齐**（同一判据表覆盖两端）。

### C9 TUI 停止呈现
- 流式中双击 Esc → 状态行"正在停止…"（发请求后），收到 `turn:cancelled` → 状态行"已停止"，并**不再**在按键处理里本地伪造终态（删除现有"当场 `is_streaming=false` + 立即打印已中断"的双写路径；改为由事件驱动）。
- 流式中 Ctrl+C 单按**保留**立即中断语义（不要求双击），但同样必须走"请求→事件驱动终态"路径，不得本地伪造。
- 未收到终态事件时不得打印"已中断本轮生成。"（防止假终态）。

## 3. 写域（write_scopes，路径段边界）

| 任务 | write_scopes |
|---|---|
| wave60-T1 core-contract | `crates/pony-agent-core/src/agent/execution_control.rs`、`crates/pony-agent-core/src/agent/control_plane/`、`crates/pony-agent-core/src/agent/turn_flow.rs`、`crates/pony-agent-core/src/agent/runtime/types.rs`、`crates/pony-agent-core/src/agent/stop_escalation.rs`(新) |
| wave60-T2 provider-abort | `crates/pony-agent-core/src/agent/provider/` |
| wave60-T3 tauri-wiring | `src-tauri/src/` |
| wave60-T4 desktop-ui | `src/stores/runtime.ts`、`src/types/runtime.ts`、`src/components/HomeWorkspace.vue`、`src/lib/runtime/`、`src/components/ui/`(如需) |
| wave60-T5 tui-stop | `crates/pony-agent-core/src/bin/tui.rs`、`crates/pony-agent-core/src/agent/stop_sequence.rs`(新) |
| 测试（Test Agent 独占） | `crates/pony-agent-core/tests/`、`tests/acceptance/stop/`、`tests/stop-*.spec.ts` |

**衍生同步豁免**：因 `ExecutionCheckpoint`/`TurnStreamEvent`/`StopTurnResponse` 增字段导致的既有测试/夹具纯字段填充，允许 Executor 改，须在任务认领时声明 `allow_derivative_scopes`，禁止夹带逻辑。

## 4. NFR（本波增量，写入 `.dev-team/nfr-baseline.json` 的 wave 扩展）

| key | 值 | 判据 |
|---|---|---|
| `stop_poll_tick_ms` | 250 | 读循环取消观测节奏；实测 ≤ 500 视为达标 |
| `stop_grace_ms` | 2000 | 宽限期；测试须可注入为 ≤50ms |
| `stop_sequence_ms` | 500 | 双击 Esc 窗口 |
| `stop_ack_latency_ms` | 100 | `stop_turn` RPC 返回到前端呈现 stopping 的上界（人工观测 + 前端单测断言状态翻转同一微任务内） |
| `external_call_timeout_ms` | 3000 | 沿用既有基线，本波不得新增无超时外部调用 |
| `logging` | 沿用既有（json + trace_id，新增行不得裸 eprintln 承载业务日志；锁毒化恢复 eprintln 沿用豁免） |

## 5. 验收收据要求（门禁）

1. Rust：`cargo check --manifest-path crates/pony-agent-core/Cargo.toml --all-targets` Exit 0；`cargo test --manifest-path crates/pony-agent-core/Cargo.toml` 相关用例 Exit 0。
2. 前端：`npx vitest run tests/stop-*.spec.ts` 与全量 `npx vitest run` Exit 0（既有基线失败项须举证为既有）。
3. TUI：`cargo test --manifest-path crates/pony-agent-core/Cargo.toml --bin pony` 中新增判据 Exit 0（若 bin 测试不可分离，允许在 `#[cfg(test)]` 纯函数层断言）。
4. 红相：Test Agent 先写测试并**必须**在未实现时失败；Lead 冻结红相 commit 后才解锁实现。
5. 可观测性：本波触及 UI 交互 + 后台任务 → 必跑 `audit-diff.sh`；`docs/observability.yaml` 存在则并跑 `check-declaration.sh`。

## 6-bis. 补遗 v2（Lead 一票裁决，2026-11-03，红相前生效）

| # | 事项 | 裁决 |
|---|---|---|
| A1 | `src-tauri` 注册表测试缝 | `lib.rs` 的 `mod turn_task_registry;` → **`pub mod`**；API 固定为 `register(turn_id, handle)` / `abort_turn(turn_id)->bool` / `unregister(turn_id)`；`abort_session` 删除或改名 |
| A2 | 终态文案通道 | "已停止"**不得**依赖 `phaseLabel`（零消费者）或 `resolveAgentRuntimeStatus`（`!isSubmitting` 时返回 null）；终态落 `sessionNotice`，进行中落 `phase="stopping"` + `agentRuntimeStatus` |
| A3 | C9 可测性 | TUI 必须提供公开纯状态迁移 `TuiState::apply_stop_requested/apply_stop_terminal` 且 `TuiTurnSink` 增 `turn:stop_requested`/`turn:cancelled` 分支；源码级 `include_str!` 静态断言仅作补充证据 |
| A4 | wire 契约快照 | 新增 `stopReason` 触发 W57-A 漂移门禁：`crates/pony-agent-core/tests/ts_contract_snapshot.rs` 的 sample 补 `stop_reason: Some("user")`，并重生成 `docs/contracts/ts-contract.snapshot.json`（`UPDATE_CONTRACT_SNAPSHOT=1`），`npm run contract:check` 必须 Exit 0。属 Protocol 写域 |
| A5 | 既有断言迁移 | `control_plane/mod.rs` 内联单测 `stop_turn_and_checkpoint_queries_share_same_registry_surface` 断言 `state=="running"`，随 C1 改为 `"stopping"` —— 允许 Executor 做**纯取值迁移**并登记为 derivative；`tests/` 下既有断言不得改（如需迁移须 Lead 批准） |
| A6 | T4 写域扩容 | 增 `src/lib/runtime/status.ts`、`src/lib/runtime/stop-sequence.ts`、`src/lib/runtime/cancelled-turn.ts`、`src/components/chat/WorkspaceComposer.vue`、`src/components/chat/AgentRuntimeStatusBar.vue` |
| A7 | 已知事实（免重复侦察） | `phaseLabel` 在 `src/` 中**零消费者**；`stopRequested`（HomeWorkspace.vue:102）为**死变量**；TUI sink 的 `_ => {}`（tui.rs:2453）丢弃 `turn:cancelled`；`should_render_status_notice` 在流式期间抑制 notice（tui.rs:3426） |

## 6. 熔断与回滚

- 每任务连续失败 3 次触发隔离（`quarantine/wave60-<task>-<ts>`），定向 `git stash`/`git checkout -- <scope>`，**禁止** `git reset --hard`/`git clean -fd`。
- C4（读通道）若导致既有 provider 长流用例红相不可修复 → 允许**降级为「仅在已有 cancel_tick 观测点上加密检查」**，但必须由 Lead 一票核准并在交付说明登记（NFR-DEGRADED）。
