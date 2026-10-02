# PA-044 加固 agent core 多端基础设施边界

## 状态
- Status: `Ready`
- Delivery state: `Frozen / Not started`（本轮不实施；无代码影响、无提交）
- Priority: `P1`
- Owner: `Codex`

## OpenSpec Change
- 稳定归档路径：
  `openspec/changes/archive/2026-06-09-harden-agent-core-infrastructure-boundary/`

## Delta Spec
- 归档路径：
  `openspec/changes/archive/2026-06-09-harden-agent-core-infrastructure-boundary/specs/agent-core-infrastructure-boundary/spec.md`

## Canonical Spec
- `openspec/specs/agent-core-infrastructure-boundary/spec.md`（若当前仓库未单独落 canonical 文件，须在重新启动实现前补齐并复核；本轮不启动）

## Spec 状态
- Proposal: `ready`
- Spec: `ready`
- Design: `ready`
- Tasks: `ready`

## 背景
当前 Pony Agent 已经具备 agent harness 的基础，`runtime / session / graph / planner / provider / tool / control plane` 的主体逻辑没有明显偏成 Tauri-only；`TurnEventSink`、`tauri_adapter.rs`、`sse_adapter.rs` 也已经证明事件投递可以从 core 中分离。

但本轮 agent core 审核发现，当前边界仍有几个会让方向逐渐偏向“只为 Tauri 桌面端构建”的风险：

- core 主体仍在 `src-tauri` crate 中，外部宿主无法只依赖一个独立 core package
- `AgentRuntime::with_dependencies`、`HostControlPlane::with_runtime`、`ToolRouter::with_workspace_root` 等关键注入入口对非测试/非 crate 内宿主不可用或受限
- 默认构造隐含桌面单用户本机假设，包括本地 data path、keyring/file secret、`current_dir` workspace root
- `sse_adapter` 证明了 event sink 可复用，但还不能证明外部 HTTP/SSE/CLI harness 可以脱离 Tauri crate 构造同一套 core
- provider transport 当前以 blocking reqwest 为默认实现，未来服务端宿主需要更明确的 transport/concurrency seam

## 目标
把 agent core 明确加固为“多端可复用基础设施”，让 Tauri 成为第一个宿主 adapter，而不是 core 的所有权边界。

## 输出
- `agent-core-infrastructure-boundary` OpenSpec change
- core / adapter / host preset / builder 的正式边界合同
- 可被非 Tauri 宿主使用的 runtime/control-plane 构造入口
- 桌面默认值收口为 desktop preset 或等价 host preset
- 至少一个 non-Tauri harness 证明 core 可脱离 Tauri 构造、运行和消费事件
- core-only / desktop adapter / non-Tauri harness 的验证矩阵

## 范围边界
- 本卡不重写 provider 协议、模型选择策略或前端 UI
- 本卡不要求一次性交付完整 HTTP server、WebSocket server 或远程多租户产品
- 本卡不改变现有 Tauri command/event 用户可见行为
- 本卡不把所有未来宿主能力提前抽象完，只抽当前多端边界必须存在的 dependency seams
- 本卡不否定桌面默认路径和 keyring；只要求它们属于 desktop preset，而不是 core 不可替换默认

## 验收标准
- core package 或等价 Tauri-free target SHALL 不依赖 `tauri` / `tauri-build`
- `rg "tauri::|AppHandle|State<|Emitter|Manager"` 在 core package / target 内无命中
- 非 Tauri 宿主 SHALL 能通过 builder 或等价 API 构造 `AgentRuntime / HostControlPlane`
- builder SHALL 允许注入 session backend、graph store、provider resolver、tool executor、workspace root、secret store 与 event delivery policy
- `ToolRouter` 或等价 tool executor SHALL 支持非测试可用的 workspace root 注入
- session/graph/provider registry/secret 默认路径 SHALL 能通过 desktop preset 注入，不作为 core 唯一默认
- Tauri adapter SHALL 只负责 command 翻译、event 投递和 desktop preset，不复制 provider/session/tool/graph 语义
- 至少一个 non-Tauri harness SHALL 覆盖：
  - sync turn
  - stream turn
  - injected workspace root tool execution
  - injected session/graph persistence roundtrip
  - non-Tauri `TurnEventSink` event consumption
- 现有 Tauri desktop command/event contract SHALL 通过回归验证
- 架构文档 SHALL 明确“Tauri 是 first host adapter，不是 core ownership boundary”

## 当前进展
- **历史 spec 阶段快照（2026-06-09）**：完成 agent core 边界审核、proposal/design/tasks/delta spec 草案与独立 spec review；当时的 active change 后续已归档。
- **当前状态（2026-09-30）**：保持 `Ready / Frozen / Not started`。本轮不启动实现，不把历史 acceptance audit 当作当前 PA-077 或本轮交付证明。
- PA-044 的历史验收/架构记录需按其对应提交与归档快照理解；若重新启动，先核对当前 HEAD 的 core boundary、canonical spec 与 non-Tauri harness 证据，再重新走三路计划/spec 审核。

## 下一步动作

PA-044 本轮不实施。重新启动前必须先完成 PA-077 closeout，并重新核对：

- archived OpenSpec change 与 canonical spec 是否可复核；
- 当前 core/Tauri-free target 与历史 acceptance audit 是否仍一致；
- 新一轮三路 plan/spec 审核是否通过。

## 当前卡点
- 冻结等待 PA-077 closeout；不属于本轮实现阻塞。

## 断点续跑提示

继续前先看：

- `openspec/changes/archive/2026-06-09-harden-agent-core-infrastructure-boundary/`
- `management/task-system/02_REVIEWS/2026-06-09-pa044-acceptance-audit.md`（历史验收快照）
- `management/task-system/03_TASKS/PA-044-harden-agent-core-infrastructure-boundary.md`
