# Pony Agent Dashboard

## 项目状态

- 项目：`Pony Agent`
- 类型：学习模式重构项目
- 当前主线：`Phase 4 / Graph Runtime Mainline`
- 当前阶段：`Mainline Stabilizing`
- 总体状态：`In Progress`

## 当前主线结论

- `PA-018` 已完成并通过完成态验证
- retrieval 观测与 trace 展示语义已正式拆到 `PA-024`
- `PA-025` 已完成 `Build Context` 与 cache-friendly prompt 边界收口
- `PA-027` 已把 OpenSpec 接入仓库与任务系统，复杂任务现在有正式 spec-first 流程
- `PA-028` 已完成历史节点管理实现、前端交互与定向验证
- `PA-020 / PA-021` 现在不再被 `PA-018` 阻塞
- 当前已正式启动 `PA-031 / PA-032 / PA-033` 三卡串联，用于收口 turn lifecycle、trace persistence/recovery 与 hooks foundation
- `PA-034` 已补入近线主视图，负责把 `checkpointing` 从 contract 名词推进成 runtime / persistence / reload / frontend 共同可见的真实 lifecycle boundary

## 当前重点

1. `PA-018` 已关闭
   已落地 `RetrievedContextState / ContextStateRetriever` contract、runtime / graph / planner / 宿主默认查询面的 retrieval-first 消费链路、`LongTermMemory` 独立边界与项目级稳定事实来源。
2. `PA-029` 已完成并通过定向验证
   `PA-025` 之后缺失的 call-level cache telemetry、首请求 / follow-up 分离、prefix mutation reasons 与第一版 stable prefix 收窄已经落地，缓存命中现在具备可解释的工程口径。
3. `PA-024` 已完成监控读面承接
   `PA-029` 交付的 `provider_call_records`、`requestKind`、`prefixMutationReasons` 与三层 build-context observation 已有正式 monitor read-plane 消费入口。
4. `PA-028` 已完成
   core 历史图、分支游标、历史 checkout/fork/restore/switch branch 与前端历史交互均已落地，且已通过 Rust 与前端定向测试。
5. `PA-020` 已完成并通过定向验证
   capability registry 统一读面、runtime bridge、MCP source snapshot 写面、permission/failure 归一化、monitor capability summary/drilldown 与 `tool / resource / prompt_template` 规范化合同均已落地。
6. `PA-021` 已关闭
   已落地 skill source snapshot ingress、统一 registry、`list_skills / inspect_skill`、tool-only runtime resolution、planner normalized skill facts consumption，以及 monitor summary/drilldown skill lineage 聚合与展示，并已完成 acceptance audit 与 closeout。
7. `PA-030` 已完成并通过前端验收
   已建立并完成 `add-trace-panel-call-model-observability` change，trace 面板 `call_model` 现已补齐 cache hit / TTFT、工具调用与消息输出保真，以及多 hop 输出归因修正。
7. 复杂开发任务治理已升级为 OpenSpec + 任务系统双轨
   `openspec/` 承载 proposal/spec/design/tasks，`management/task-system/` 承载状态、审计、日志与断点续跑。
8. turn lifecycle / recovery / hooks 新主线已完成第一批近线收口
   `PA-031 / PA-032 / PA-033 / PA-034 / PA-035 / PA-036 / PA-037` 已全部完成并关闭，分别收口了 canonical lifecycle/event contract、trace persistence/recovery contract、hooks foundation、checkpoint lifecycle boundary、stable-boundary runtime hook dispatch、terminal truth-source 与 session control UX 闭环。
9. 当前近线主线已从“补 contract / 补闭环”切到“归档首批 / 定下一轮”
   当前 review 队列已清空；`PA-021 / PA-030 / PA-031 / PA-032 / PA-033 / PA-034 / PA-035 / PA-036 / PA-037` 对应的 OpenSpec changes 已同步进 `openspec/specs/` 并归档到 `openspec/changes/archive/`，下一步更适合决定是否把 `PA-022`/后续扩展正式提到近线。
10. `PA-022` 已不再保留为模糊大卡
   当前已把 post-foundation hooks 范围拆成 `PA-038 / PA-039 / PA-040` 三张近线卡，而且三张卡都已完成 closeout：`PA-038` 收口了 run/execution-control boundary evidence，`PA-039` 收口了 memory-write persisted side-effect contract，`PA-040` 收口了 planner/capability mediation hooks。
11. hooks post-foundation 近线三卡已完成第一轮工业化闭环
   当前 `run hooks`、`memory-write hooks`、`planner/capability hooks` 都已经具备各自的稳定边界、persisted/read-plane 证据链与 acceptance audit，后续扩展更适合以新卡承接，而不是回灌到同一批 closeout 卡。
12. `PA-038 / PA-039 / PA-040` 对应的 OpenSpec changes 已归档
   三张 change 的 delta specs 已同步到 `openspec/specs/`，并已迁入 `openspec/changes/archive/2026-06-05-*`；当前活跃 `openspec/changes/` 已重新清空，只保留 `archive/`。
13. hooks post-foundation 近线第四卡也已完成
   `PA-041` 已完成 `history checkout / branch restore / branch fork / branch switch` 四类 history-state control boundary 的 hook dispatch、persisted audit chain、reload/read-plane/front-end contract 对齐，以及 degrade truth-source non-regression。
14. `PA-042` 已完成并通过验收审计
   `Session Control Plane audit surface v1` 已完成 history-control summary contract、snapshot/runtime-view/response 统一投影、reload roundtrip、truth-source guardrail 与前端 summary-first explainability，当前这轮近线主线已完成收口。
15. `PA-043` 已完成并通过验收审计
   `Run Control audit surface v1` 已完成 `stop / continue / resume / replay(start)` summary contract、snapshot/runtime-view/response 统一投影、普通首轮 `start_graph_run_stream` 排除、reload/hydration guardrail 与前端 summary-first explainability，当前近线主线已进一步完成 run-control 收口。
16. 新增 `PA-044` 作为下一条基础设施边界加固候选
   本轮 agent core 审核确认：代码内依赖方向总体没有偏成 Tauri-only，但 package 边界、constructor 注入、desktop preset、workspace/storage/secret 默认值仍有回粘桌面端的风险。`PA-044` 已建立 OpenSpec change：`harden-agent-core-infrastructure-boundary`，用于把 Tauri 明确降级为 first host adapter，而不是 core ownership boundary。
17. 工具系统规划主线已正式拆卡
   当前已完成 `PA-045 / PA-046 / PA-047 / PA-048 / PA-049` 五张连续任务卡，分别承接工具协议、workspace 合同、权限审批、首批工具面与观测/前端呈现；五卡均已完成实现、验证、归档与任务系统收口。
18. 工具系统五张 spec 卡已完成首轮文档闭环
   `PA-045 ~ PA-049` 当前都已具备 `proposal / design / tasks / delta spec`，并已同步到 `openspec/specs/` 后归档到 `openspec/changes/archive/2026-06-15-*`；每张卡都完成了独立智能体审核、采纳优化、strict validate 与归档收口。
19. `PA-056` 已完成架构、实现与 6 月 17 日 follow-up 收口
   当前已完成 context layering、memory/project/carry 分层实现、观测字段落点、canonical spec 同步、OpenSpec 归档，以及 3 轮 `opencode / deepseek-v4-flash` 代码审核与一轮 follow-up 采纳调优；后续若继续推进，应转入 instruction scope 真 source 枚举与 continuation/compaction 深化。
20. `PA-056` 已补齐显式 `Coding / Work` 配置与 prompt 收紧闭环
    当前已新增独立 `AppSettings`、Tauri settings 命令、前端设置 store、左侧边栏尾部设置入口、`SettingsPanel` 与 `workspaceMode -> TurnContext -> domain profile` 主链透传，并已补 `BASE_SYSTEM_PROMPT` 中文默认语义恢复、小窗口 domain profile 跳过和 `runtime.ts` teardown 稳定性修复；后续其它全栈配置项可以沿这条配置面继续扩展，而不必再重复改造上下文主路径。
21. `PA-057` 已完成前端 flight recorder 与卡顿诊断体系
    当前已完成前端 `frontend-flight-recorder.ts`（ring buffer、stall 检测、flush 退避）、Rust 端 `frontend_diagnostics.rs`（SQLite 持久化 + async spawn_blocking）、host 层集成（6 个 Tauri command）、主链路埋点激活及`npm run dev`启动冻结分析与修复；已提交 `f6aab80`，OpenSpec change 已归档。
22. session persistence 热路径已完成第一轮性能收口，`PA-058` 已完成并通过全局验收
    当前已完成独立 `session_turn_traces` 表、dual-read / authoritative no-fallback、hot path trace-level mutation、组合写事务、branch/history trace materialization、`NotFound` 自愈与 trace 表 prune，并通过 15 项定向测试与多轮并行智能体审核验收。OpenSpec change 已归档，canonical spec 已同步。
23. `PA-059` 已完成并收口
24. `PA-064` 已完成并收口
    已完成缓存优先会话切换、`list_sessions` 延迟加载、`HomeWorkspace` staged hydration、`createSession()` collision-safe id、前端去重与 3 维度 spec 审核调优。TS 测试全部通过。
25. `PA-065~068` Tokio 异步重构四卡已完成并收口
     - `PA-065` 拆分 runtime ownership：`SessionStore` 提取为 `Arc<RwLock<>>` 独立域，16 个读面方法从 `Mutex<AgentRuntime>` 解耦
     - `PA-066` 异步化 provider IO：`reqwest::blocking` → async `reqwest`（`block_on()` 桥接），provider.rs + tools.rs 双路径
     - `PA-067` blocking 工作收口：`BlockingHelper::spawn` 统一 helper，6 个 Tauri command 迁移，PA-068 过渡标记
     - `PA-068` per-session async turn task：`TurnTaskRegistry` 任务追踪，`spawn_turn_stream`/`spawn_graph_run_stream` 切换 async task 模型
     四卡累计通过 22 次子智能体审核调优，Rust 测试 + 231 项 TS 测试全部通过。OpenSpec changes 已归档：`openspec/changes/archive/2026-06-25-async-refactor-tokio/`。
26. `PA-070` 已完成 provider request retry 与退避边界治理
    当前已完成 `retry.rs` substrate、provider/tool 退避原语收口、前端 whole-turn 自动重试退场、最终严格代码审核与一轮收尾调优。`call model` 的 request-level retry 现在明确收束在 `pony-agent-core`，而不是前端静默 whole-turn retry。

## 远期扩展

- 在当前 agent harness 主线完成并稳定后，路线图将扩展一条 `workflow mode` 支线。
- 该支线目标不是替代 agentic 模式，而是在既有 graph 底座上补用户可定义流程，服务行业 SOP、审批流、分工协作与可审计工作场景。
- 预期复用的底座包括：`graph run`、checkpoint / resume、trace、tool capability registry 与 human-in-the-loop 边界。
- 这一方向当前不进入近线主线范围，待 harness 收口后再正式拆卡。

## 当前代码证据

- retrieval contract 与默认实现：
  [context.rs](crates/pony-agent-core/src/agent/context.rs)
- long-term memory 独立边界与稳定事实来源：
  [session.rs](crates/pony-agent-core/src/agent/session.rs)
- runtime 接入：
  [runtime.rs](crates/pony-agent-core/src/agent/runtime.rs)
- graph handoff 与 planner 收口：
  [graph.rs](crates/pony-agent-core/src/agent/graph.rs)
  [planner.rs](crates/pony-agent-core/src/agent/planner.rs)
- 宿主 retrieval-first 读面：
  [control_plane.rs](crates/pony-agent-core/src/agent/control_plane.rs)
  [lib.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/lib.rs)
- 架构边界文档：
  [context-state-subsystem.md](/C:/Users/HUAWEI/Documents/pony-agent/docs/architecture/context-state-subsystem.md)

## 当前验证

`PA-029` 本轮完成态验证：

```powershell
cargo test --manifest-path src-tauri/Cargo.toml file_backend_roundtrip_restores_turn_trace_history
cargo test --manifest-path src-tauri/Cargo.toml run_turn_accumulates_token_usage_across_tool_followups
cargo test --manifest-path src-tauri/Cargo.toml start_turn_stream_accumulates_token_usage_across_tool_followups
cargo test --manifest-path src-tauri/Cargo.toml build_request_keeps_image_and_truncation_notes_out_of_stable_prefix
cargo test --manifest-path src-tauri/Cargo.toml build_context_observation_fallback_keeps_dynamic_system_and_developer_text_out_of_stable_prefix
npm run test:unit -- --run tests/runtime-store.spec.ts
npm run test:unit -- --run tests/HomeSidebar.spec.ts
```

结果：

- Rust / session trace roundtrip 定向测试通过
- Rust / runtime sync 与 stream cache telemetry 定向测试通过
- Rust / context stable-prefix fallback 定向测试通过
- 前端 `runtime-store` 定向测试通过：`39 passed`
- 前端 `HomeSidebar` 定向测试通过：`7 passed`

## 下一步最小动作

1. 优先对 `PA-044 / harden-agent-core-infrastructure-boundary` 做一轮独立 spec 审核，确认 core/package/builder/preset/harness 边界足以防止 agent core 回粘 Tauri。
2. 后续若继续扩展工具系统，应以新 change 承接，不再回灌已归档的 `PA-045 ~ PA-049`。
3. 在 validate 通过后，为工具系统五卡确定实现顺序与首批落地范围，继续保持“spec 审核 -> 实现 -> acceptance -> 归档”的整批闭环节奏。
4. 如继续扩展全栈配置项，优先复用本轮 `AppSettings + settings store + settings panel + runtime pass-through` 这条主链，而不是把新配置散落到 provider 配置或单轮 prompt 推断里。
5. 前端卡顿定位现已具备 flight recorder 证据链（`PA-057` 已完成），后续性能优化可以基于 `frontend-diagnostics.db` 中的 stall 快照和 trace 事件进行数据分析，而不是再靠手动复现。
6. `PA-058` 已完成，不再处于 Ready/Dashboard 主文中作为下一步目标。
## 新近线候选

1. 基于 `Session Control Plane` 的 monitor / drilldown 读面扩展
   在 `PA-042 / PA-043` 已完成 summary family 收口的前提下，评估是否需要新增更细的 run-control / history-control 审计下钻，而不是回灌已关闭任务。
2. `PA-044` agent core infrastructure boundary hardening
   在 harness 基础已成立的前提下，把 core 明确拆成可被 Tauri / HTTP-SSE / CLI / service 多端复用的基础设施边界，优先解决 package 边界、构造注入、desktop preset 与 non-Tauri harness 证明。
3. `PA-045` tool system contract and exposure boundary
   在 core 多端边界继续稳定的同时，优先把工具系统的模型可见协议、结果合同、暴露策略与分类体系正式写成 spec，避免后续 workspace / permission / first-wave tools 各自发明字段与命名。
4. `PA-056` context assembly and cache strategy
   在 `PA-025 / PA-029` 已经提供第一版上下文观测与缓存 telemetry 的基础上，把 system prompt、runtime facts、project instructions、conversation carry 与长期记忆扩展点正式统一到同一套上下文分层架构中。
5. 前端卡顿根因定位
     PA-057 已交付完整的前端 flight recorder 与 stall 诊断体系。后续可通过分析 frontend-diagnostics.db 中的 rAF gap / timer drift / longtask 数据定位剩余卡顿。
6. `PA-070` 已完成，不再作为近线候选
    当前 canonical spec 已同步到 `openspec/specs/provider-retry-and-backoff-boundary/spec.md`，archive 已完成。后续若继续推进显式 turn-level retry，应以新 change 承接，而不是回灌已完成变更。

## 关联入口

- 任务板：[01_TASK_BOARD.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/01_TASK_BOARD.md)
- OpenSpec 目录：[openspec](/C:/Users/HUAWEI/Documents/pony-agent/openspec)
- 已完成任务卡：[PA-018](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-018-build-context-state-subsystem-and-retrieval-boundary.md>)
- 已完成任务卡：[PA-027](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-027-integrate-openspec-into-task-system.md>)
- 已完成任务卡：[PA-028](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-028-build-history-node-management-and-branching.md>)
- 正式验收审计：[PA-018 Acceptance Audit](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-05-28-pa018-acceptance-audit.md>)
- 文档索引：[docs/INDEX.md](/C:/Users/HUAWEI/Documents/pony-agent/docs/INDEX.md)
- 会话日志目录：[99_LOGS](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/99_LOGS)
