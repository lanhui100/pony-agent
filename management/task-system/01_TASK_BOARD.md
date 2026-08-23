# Pony Agent Task Board

## Roadmap Layers

- 宿主与入口：`PA-015 / PA-023`
- graph 编排：`PA-012 / PA-013 / PA-014 / PA-019`
- runtime 执行：`PA-010 / PA-025`
- 状态、上下文与记忆：`PA-011 / PA-016 / PA-017 / PA-018`
- 能力接入：`PA-020 / PA-021`
- 生命周期横切：`PA-022`
- 观测与监控：`PA-024`
- core 基础设施边界：`PA-044`

## Backlog

- `PA-022` lifecycle hooks pipeline
  说明：保留为 post-foundation hooks 总入口与分流说明；下一轮已拆成 `PA-038 / PA-039 / PA-040` 三张可执行卡。
- `PA-026` workflow mode 与用户自定义流程编排
  说明：在 agent harness 主线完成并稳定后，基于既有 graph / runtime / checkpoint 底座扩展用户自定义 workflow 模式，支持行业流程节点、条件分支、审批、人机协同、重试与审计恢复；该卡明确属于远期扩展，不进入当前近线主线。

## Ready

- `PA-095` 事件溯源收尾——ADR 0008 承诺缺口关闭（P0）
  说明：**2026-08-21 由 ADR 完成度审核立卡**。PA-091~094 四阶段主体落地后，对照 ADR 原文逐项审核确认 7 项缺口：①同步 `run_turn` 绕过事件流（生产入口零事件）；②`append_turn` 未改造为事件折叠（阶段 2 核心承诺，整包 upsert 写放大未消除）；③blob↔事件折叠对拍测试缺失（豁免清单未固化）；④StepStart/StepEnd 不发射 + AssistantChunk 固定 step=0（多 hop timeline 重建分歧）；⑤`event_schema_version` 仅定义常量从未写入/校验；⑥`cursor_version` 未退役（23 处引用，双版本并存）；⑦trace cache 仍内嵌全量 observation（R4b 违例）。OpenSpec change 待创建（`event-sourcing-closeout`）。

- `PA-084` trace 面板初始化折叠与懒渲染（P0）
  说明：**已完成并收口（2026-08-14）**——`activePanel` 默认 `""`、`HomeTracePanel` body `v-if="open"` 懒挂载（header 常驻）、折叠时 `liveTraceTurn` 返回 null（消除冻结引用污染）。实现后审核 P1 已修复。OpenSpec change 待归档：`trace-panel-collapse-by-default`。

- `PA-085` trace 面板虚拟滚动（P0）
  说明：**已完成并收口（2026-08-14）**——`trace-virtual-scroll.ts` 纯函数（前缀和 + binarySearch + OVERSCAN）、内嵌独立 ScrollArea、turn 级虚拟化（实现偏离：替代单层扁平，理由重构风险）、底部跟随依赖总高度、scroll 原生监听 viewport。实现后审核 P1 已修复。OpenSpec change 待归档：`trace-panel-virtual-scroll`。

- `PA-086` trace 渲染快照投影与节流增强（P0）
  说明：**已完成并收口（2026-08-14）**——`trace-projection.ts` 轻量 memo helper（ref+updatedAt 签名）、15 处写路径收敛 `publishTraceTimeline()`、HomeSidebar/HomeTracePanel 消费投影层、onBeforeUnmount 清理。实现后审核裁决：投影层定位轻量 memo helper（非完整 projection），低频恢复路径不强制收敛（引用已变，memo 自然失效）。OpenSpec change 待归档：`trace-render-snapshot-projection`。

- `PA-087` 输入框优先级隔离（P0）
  说明：**已完成并收口（2026-08-14）**——`handleComposerKeydown` 补 `isComposing`/keyCode 229 守卫（修复中文候选确认误发送）、draft 同步写入确认。实现偏离：调度降级由 084/085/086 覆盖，本卡收敛为 IME 守卫 + 输入路径确认。OpenSpec change 待归档：`composer-input-priority-isolation`。

- `PA-081` 侧边栏 Workspace 树导航（P1）
  说明：`HomeSessionSidebar` 平铺会话列表改造为"项目（二级）→ 对话（三级）"树，折叠/展开、按 workspaceId 归组（孤儿归"未分组"）、Workspace 管理入口；激活 workspace 为前端 localStorage 单一真相源。OpenSpec change：`workspace-sidebar-tree-navigation`（spec 通过，2026-08-09）。



## In Progress

- 暂无

## Review

- 暂无

## Blocked

- 暂无

## Done

- `PA-097` pony-agent-core 热点文件重构——P1 测试外移 + P2 session 目录化（P1，Complexity B）
  说明：**已完成并收口（2026-08-23）**——session.rs（11,533 行）→ session/{mod,types,backend,file_backend,store,tests} 六文件（mod.rs 55 行纯声明+定向重导出+cfg(test) 供给区），`crate::agent::session::*` 路径全量保持、调用方零改动；runtime/mod.rs（16,588 行最热文件）测试外移+结构化拆分经用户仲裁交割并行编排 **PA-098** 完成。零行为变化实证：868↔868 `--list` 差分预言机为空、字面量守恒 raw 1/1+普通 1974/1974 字节级一致、生产数据隔离屏障 default_storage_path() 476 字节一致、统一终门禁 cargo:test FULL_EXIT=0 全绿、warning 归因 lib7↔7/test47↔47 零新增。流程：dev-team B 级路径——spec 双路对抗审核 18 条全采纳（含 glob 重导出 E0364 真缺陷、EOL 判据纠正）、Line A/B 各双 code review（Line B 零 P0/P1，P2×3 文档错位按 HEAD 权威回正；两处"疑似多余放开"经证据驳回）；期间处置两次并发编排竞态与一次 CRLF 污染事故（字节级断言+rustc 权威裁决恢复）。OpenSpec change 已归档：`openspec/changes/archive/2026-08-23-core-hotfile-refactor/`。遗留登记：`.gitattributes` EOL 根治建议；runtime 生产代码内聚性深拆由 PA-098 承接。
- `PA-098` runtime/mod.rs 结构化拆分——测试外移 + 自由函数子模块化 + impl 分块门面化（P1，Complexity B）
  说明：**已完成并收口（2026-08-23，并行编排交付）**——16,588 行单体 → mod.rs 4,232 行门面 + 12 个子模块（tests.rs 外移约 9.5k 行含嵌套 parity；自由函数入 types/builder/limits/planner_patches/blocked_records/trace_timeline/tool_recovery/stream_support 八域；impl AgentRuntime 分块迁入 turn_stream/turn_sync/turn_control）；外部路径 `crate::agent::runtime::*` 重导出全量保持。任务卡：`management/task-system/03_TASKS/PA-098-runtime-mod-structured-split.md`，spec 已归档：`openspec/changes/archive/2026-08-23-runtime-mod-structured-split/`。
- `PA-096` 前端工作台布局优化——trace/metrics 二级化 + 配置页 tab 化（P1，Complexity B）
  说明：**已完成并收口（2026-08-22）**——右侧栏瘦身为 状态/Plan/Debug；trace+metrics 合并二级遥测页（左栏一级键：coding 显"遥测"/work 显"指标"；work 仅见指标 tab，tab 可用性收敛替代 App 守卫——用户两项产品决策）；工具目录入配置页；配置页 tab 化（通用/模型/工具，受控态不持久化）；左侧栏工作区置顶 + 激活工作区名入 section 头 + "模型配置"升一级菜单；collapsed rail 六键。核心新增：`useTraceProjection` 共享投影管线（liveTurnEnabled 冻结守卫参数化，状态面板数据供给与 TraceInspector 双消费）、TraceInspector、TelemetryPage、ConfigPage 三件套；删除 SettingsPanel/HomeToolsPanel。**spec 双路对抗审核（架构 2×P0 拆分线修正 + UX 产品决策升级）与实现后双 code review（无 P0/P1；P2×4：页头模式化文案/复制按钮键盘可达/持久化注释/gitignore 卫生）全部采纳修复**。验证：vitest 全量 434 passed / 10 skipped（HomeSidebar.spec 26 条零丢失拆分迁移 + 新增 ConfigPage/TelemetryPage/键盘漫游/冻结聚合守护等用例）+ vue-tsc 0 + vite build ✓。OpenSpec change 已归档：`openspec/changes/archive/2026-08-22-workspace-shell-layout-optimization/`，canonical spec：`openspec/specs/workspace-shell-navigation/spec.md`。后续优化（disclosure aria-expanded、e2e 导航矩阵、embedded IPC 缓存等）登记于任务卡残余节。
- `PA-094` trace 事件化（阶段 4：事件溯源演进）（P0）
  说明：**已完成并收口（2026-08-20）**——trace 表降级为投影缓存（`event_watermark` 水位，清表可重建，事件权威；`SessionStore::with_backend` 加载时缓存空 + 事件存在 → 事件全量折叠重建 + AppendTrace 回写预热）；`TraceProjection` timeline 事件折叠映射表（step/start→call_model、tool/call→call_tool、tool/result→return_result、chunk 文本聚合、turn/end 结算 token、build_context 条目带 ref）；`ProviderCallCacheRecord` 由 `MetricsProjection.by_turn_records` 重建（addReplacing 原位覆盖，豁免 updated_at/event_id/sequence/emitted_at_ms）；大字段外置（`ContextObservation` 事件引用 `bco:<turn_id>:<seq>` + `build_context_observations` 表 + `load_build_context_observation` host command + 前端按需加载 + legacy backfill 派生）；`turn:completed/failed/cancelled` 发射 turn/end + provider/usage（per-call 逐条）+ user/message（turn:started）。**三轮 3 路对抗审核（code-reviewer + consultant + tester + ox-alpha 模型）全部采纳修复**：seq=0 watermark、completed 拆批、ProviderUsage step 坍缩、UserMessage 缺失、多分支重建丢失、事件缓冲跨 session、前端 ref 缓存串数据、prefix reason Debug 格式、顶层 token 非累计、普通 streaming watermark、failed/cancelled usage 缺失、legacy observation 清表丢失、重建回写预热。验证：core 849 + src-tauri 6 + 前端 399 全绿。OpenSpec change 已归档：`openspec/changes/archive/2026-08-20-trace-event-projection/`，canonical spec：`openspec/specs/trace-event-projection/spec.md`。后续架构级事项（同步 run_turn 事件化、事件/缓存事务边界、AssistantChunk 逻辑 step 等）登记为后续任务。
- `PA-093` checkpoint 引用化（阶段 3：事件溯源演进）（P0）
  说明：**已完成并收口（2026-08-18）**——`HistoryNode.event_seq_range` + `HistoryCursor.event_watermark` + `SessionState.event_watermark/last_commit_watermark`、checkout/fork 事件化（branch_id 透传）、分支可见性（血缘链 + checkout 重放）、时间线回放 + cursor_version 收敛、`fold_all_with_branches`。验证：core 831 全绿 + 8 项 pa093 定向测试。OpenSpec change：`checkpoint-event-referencing`。
- `PA-092` 投影层（阶段 2：事件溯源演进）（P0）
  说明：**已完成并收口（2026-08-18）**——`Projection` trait（init/apply/view）+ History/Trace/Plan/Metrics 四投影、`append_turn` 重构为事件 + 投影折叠、trace 投影（seq 水位 + higher-seq-wins）、MetricsProjection（addReplacing 四桶）、golden fixture。验证：core 823 全绿 + 12 项投影测试。OpenSpec change：`session-projection-layer`。
- `PA-091` turn 事件日志（阶段 1：事件溯源演进）（P0）
  说明：**已完成并收口（2026-08-18）**——`turn_events` append-only 表（branch_id）+ `turn_events_archive`、`TurnEvent` enum 15 类事件、`TurnEventSink::persist` 事件模型（turn 动态 flush + blob 同事务）、事件映射（8 条路径）、per-session 幂等回填。验证：core 全绿。OpenSpec change：`turn-event-log`。
- `PA-080` Workspace 路径权限边界（P0）
  说明：**已完成并收口（2026-08-13）**——`path_permission.rs` 统一路径判定（组件级前缀比较 + Windows 大小写折叠 + 可注入 canonicalizer）+ 18 项对抗测试、`AuthorizeStore` 授权清单（SQLite store_metadata key=`path_authorizations.v1` + JSON fallback）、写边界（workspace 根递归 + 受控 tmp，`outside_workspace_write_denied`）、读边界（外部需授权，`requires_authorization`）、宿主 `authorize_path`（仅 read）/`revoke_authorization`/`list_authorizations`、锁序登记。**实现后 3 路对抗审核**：@code-reviewer（P0 edit 写判定 / P1-1 Run cwd / P1-2 会话 root）+ @consultant（P0 会话 root 并发串扰 → 方案 A 显式 `ToolExecutionContext`、生产链路按 workspaceId 解析 root、错误码结构化信封、Run sandbox 基准、fallback tmp 覆盖）全部修复。最终验证：core lib 775 + tool_router_regression 13 + session_regression 5 + provider_registry 8 + src-tauri lib 6 + 前端 vitest 377 全绿。提交：`2db0b0e`（实现）+ `a1b75d8`（审核修复）。OpenSpec change 已归档：`openspec/changes/archive/2026-08-13-workspace-path-permission-boundary/`，canonical spec：`openspec/specs/workspace-path-permission/spec.md`。

- `PA-082` 前端流式渲染优化收口（optimize-agent-stream-ui / stabilize-markdown-stream-rendering）
  说明：**已完成并收口（2026-08-13）**——流式部分 markdown 稳定化（未闭合代码围栏专用渲染路径）、缓冲揭示节奏（首屏低阈值/代码块紧节奏）、安全呈现回退、reduced-motion、流式状态指示、终态专属操作、运行时回滚开关；配套 turn 级 timeline memo 与 semantic-event timeline 复用性能优化。OpenSpec changes 已归档：`openspec/changes/archive/2026-08-09-optimize-agent-stream-ui/`、`2026-08-09-stabilize-markdown-stream-rendering/`，canonical spec：`openspec/specs/chat-ui/spec.md`。
- `PA-083` provider registry fallback 回归基线对齐
  说明：**已完成并收口（2026-08-13）**——`resolve_selection_falls_back_to_selected_provider_and_model` 回归测试基线对齐 `a2636da` 模型去重语义（`max_output_tokens` 64000 → 4096），8 passed。OpenSpec change 已归档：`openspec/changes/archive/2026-08-09-fix-provider-registry-fallback-test-baseline/`，canonical spec：`openspec/specs/provider-registry/spec.md`。

- `PA-079` Workspace 数据模型与注册表（P1）
  说明：**已完成并收口（2026-08-09）**——workspace 注册表（`workspace.rs`，默认保留字 `"default"`）、`PersistedStore.workspaces` 持久化（SQLite store_metadata + File/Memory 自动）、`SessionState.workspace_id` + 投影、`TurnInput.workspaceId` 传输 + 首次盖章（修复首轮丢失）、宿主 `workspace_list`/`workspace_create`、`import_attachment` 注册表解析。spec 3 路审核 + 实现后审核（@tester 通过 + 自审）通过；验证 core lib 754 + 回归 8/5/13 + 前端 377 全绿。审核产物：`02_REVIEWS/2026-08-09-pa079-implementation-review.md`。OpenSpec change 已归档：`openspec/changes/archive/2026-08-09-workspace-data-model-and-registry/`，canonical spec：`openspec/specs/workspace-data-model/spec.md`。
- `PA-078` 对话文件附件入口与文件类型注册表（P0）
  说明：**已完成并收口（2026-08-09）**——composer 别针入口 + 待发送附件条、类型注册表（扩展名优先/MIME 兜底 + 图片魔数嗅探）、文本内容注入（≤64KiB 截断）/二进制文档引用附着（path+mime）、宿主 `import_attachment`/`get_workspace_root`（name 净化 + `.tmp/imports/` 受控导入）、附件-only 自动摘要与生命周期清空。spec 3 路对抗审核 + 实现后 3 路对抗审核均通过，无未解决 P0/P1。验证：前端 vitest 377 + build + cargo:check:shared + core lib 740 全绿。审核产物：`02_REVIEWS/2026-08-09-pa078-081-spec-review.md`、`2026-08-09-pa078-implementation-review.md`。OpenSpec change 已归档：`openspec/changes/archive/2026-08-09-add-chat-file-attachment-entry/`，canonical spec：`openspec/specs/chat-file-attachment-entry/spec.md`。
- `PA-076` 加固并扩展 Agent Tool Runtime (P0)
    说明：已完成并收口（2026-08-05）。OpenSpec change `harden-and-expand-agent-tool-runtime` 已归档：`openspec/changes/archive/2026-08-05-harden-and-expand-agent-tool-runtime/`。阶段 1–7 核心 + runtime 默认切换（`build_governed_executor`）+ Ask 真实接线（P1-1 端到端：sync + stream 双路径挂起/恢复 + 共享 dispatcher + resume 注入唯一终态 + 前端 graph_resume_ask）+ pinned connector（P1-5，连接 pin 到已校验地址 + 链级总 deadline）+ 阶段 7 工具注册（P2-6，plan_control/view_image 进 builtin registry）+ Sandbox 裁决（2026-08-04，fail-closed 为合规终态，Job Object 拆 PA-077）。4 份阶段独立审核 + 8.4 收口（consultant PASS，无未解决 P0/P1）。最终验证：core lib 723 + matrix 27 + tool_router_regression 13 + session_regression 5 + 前端 vitest 332 全绿。审核产物：`02_REVIEWS/2026-08-05-pa076-phase{4,5,6,7}-review.md`、`2026-08-05-pa076-84-closeout.md`、`2026-08-04-pa076-sandbox-backend-evaluation.md`。

- `PA-072` 决议 checkpoint message controls change (P1)
    说明：已归档。`add-checkpoint-message-controls-and-bottom-menu` 的 checkpoint UX 核心能力已在代码中基本实现，剩余前端打磨与当前基础设施优先级不匹配。归档位置：`openspec/changes/archive/2026-06-27-add-checkpoint-message-controls-and-bottom-menu/`。

- `PA-071` 为基础设施变更补充架构文档 (P1)
    说明：4 份架构文档已写入 `docs/architecture/`（runtime-ownership-split, async-provider-io-migration, blocking-helper-unification, per-session-async-turn-task-model），INDEX.md 已更新。

- `PA-073` 推进统一 retry 边界到 design/spec 阶段 (P2)
    说明：已完成 design/spec 推进，后续 PA-070 已继续完成实现、审核、canonical spec 同步与 archive 收口。

- `PA-070` 统一 provider request retry 与退避边界 (P1)
    说明：已完成 `retry.rs` substrate、provider/tool 退避原语收口、前端 whole-turn auto retry 退场、canonical spec 同步与 archive 收口。归档位置：`openspec/changes/archive/2026-06-27-unify-provider-retry-and-backoff-boundary/`。

- `PA-074` 重写 Rust 智能体开发指南 (P2)
    说明：`docs/guides/rust-agent.md` 已全面重写，覆盖 22 模块、关键 trait、开发工作流、测试策略。

- `PA-075` 清理会话日志中的过期路径引用 (P3)
    说明：63 个文件、200+ 处 `src-tauri/src/agent/` → `crates/pony-agent-core/src/agent/` 路径替换完成。98_IMPORTS/ 中 20 处绝对路径已改为相对路径。

- `PA-069` Tokio 异步后优化治理（Spec + 5 张子卡）
    说明：已完成并收口（2026-08-08）。五张子卡 A~E 全部实现完毕并通过收尾验证：`cargo check`（shared）通过、core lib 733 全绿、回归集 13 项全绿、src-tauri lib 6 项全绿、前端 vitest 332 全绿。收尾时修复 1 个历史 flaky 附件测试（同一毫秒 asset_id 碰撞）。

- `PA-069-A` 工具文件 IO 从 runtime 锁内移出 (P1)
    说明：已完成并收口（2026-08-08）。`TurnContext`（`runtime/turn_runner.rs:19-27`）携带 `tool_executor: Arc<dyn ToolExecutor + Send>`，工具执行落在独立模块 `runtime/tool_exec.rs`，不持 runtime 锁。`grep runtime\.lock()` 在生产路径无匹配。验证通过。

- `PA-069-B` capability_registry Mutex → RwLock (P2)
    说明：已完成并收口（2026-08-08）。`control_plane/mod.rs:846` 与 `turn_runner.rs:24` 均已使用 `RwLock<CapabilityRegistry>`，`apply_skill_source_snapshot` TOCTOU 已按「读→释放→应用→写回重验证」修复。验证通过。

- `PA-069-C` Mutex 中毒容错修复 (P2)
    说明：已完成并收口（2026-08-08）。`control_plane.rs` 全量使用 `unwrap_or_else(|e| { eprintln!(...); e.into_inner() })`（仅 `runtime` 保留 `expect()`——有意之举），`execution_control.rs` 同理。验证通过。

- `PA-069-D` SQLite 写路径优化 (P2)
    说明：已完成并收口（2026-08-08）。`write_full_store` 仅用于 JSON→SQLite 迁移（`sqlite_session.rs:149`），写路径走 `upsert_session()` 逐行 upsert，含 `PRAGMA wal_checkpoint(PASSIVE)`（`sqlite_session.rs:603`），有专用测试 `upsert_session_updates_one_row_without_rewriting_other_sessions`。验证通过。

- `PA-069-E` 锁序规范文档化 (P2)
    说明：已完成并收口（2026-08-08）。`docs/concurrency/lock-ordering.md` 73 行，包含规范锁序（5 级）、获取规则、已知异常和锁清单。验证通过。

- `PA-068` 接入 per-session async turn task 模型
    说明：已完成 `TurnTaskRegistry` per-session 任务追踪、`spawn_turn_stream`/`spawn_graph_run_stream` 从 `spawn_blocking` 切换到 `tauri::async_runtime::spawn` + 内层 `spawn_blocking` 的 async task 模型、`TaskCleanupGuard` 自动反注册、`abort_all` 挂钩窗口关闭事件。多 session 通过 `TurnTaskRegistry` 实现独立任务身份。Rust 测试 30 项 + TS 测试 231 项全部通过，3 轮并行智能体审核验收。

- `PA-067` 收口 blocking 工作并建立统一执行 helper
    说明：已完成 `BlockingHelper::spawn` 统一 helper、6 个前端诊断 Tauri command 从 `tauri::async_runtime::spawn_blocking` 迁移到 `BlockingHelper::spawn`、`tauri_adapter.rs` 添加 PA-068 transition target 标记与 `TaskCleanupGuard`。前/后端全量测试通过，3 轮并行审核验收。

- `PA-066` 异步化 provider IO 与 streaming 边界
    说明：已完成 `provider.rs` + `tools.rs` 从 `reqwest::blocking` 迁移到 async `reqwest`（通过 `block_on()` 桥接）、`crates/pony-agent-core/Cargo.toml` 移除 `blocking` feature、新增 `runtime_helper::block_on` 共享函数消除 provider/tools 重复。Provider 测试 25 项全部通过，3 轮并行智能体审核验收。

- `PA-065` 拆分 runtime ownership 并解除 turn 与读路径互锁
    说明：已完成 `SessionStore` 提取为 `Arc<RwLock<SessionStore>>` 独立所有权域、`HostControlPlane` 新增 `sessions_rwlock` 字段、16 个读面方法从 `Mutex<AgentRuntime>` 迁移到 `sessions_rwlock`、`SessionBackend` trait 增加 `Sync` 约束。全量编译通过，3 轮并行智能体审核验收。

- `PA-064` 降低 session 切换与 init 宿主读取压力
    说明：已完成缓存优先会话切换、`list_sessions` 延迟加载、`HomeWorkspace` staged hydration、`createSession()` collision-safe id、前端去重与 spec 3 维度审核调优。TypeScript 测试全部通过。

- `PA-059` 扩展 TurnHistoryMessage 携带消息元数据
    说明：已完成 `MessageStatus` 枚举、`TurnHistoryMessage` 扩展 5 字段 + `stable_id()`、`PartialEq,Eq`、末端对齐投影 enrich（live + node 双路径）、前端 TS 类型更新、`hydrateMessagesFromHistory` 优先级重构、`buildTurnHistory` 传播、`isPersistedMessageShapeCompatible` 简化，已通过 5 个新增 Rust 测试 + 1 个前端测试 + 3 轮并行智能体审核 + 最终代码审核三轮调优。审核记录：`02_REVIEWS/2026-06-22-pa059-spec-review.md`。

- `PA-060` 统一 checkpoint / cursor / view 合同以支持多端宿主
     说明：已完成 `session-cursor-view-contract` canonical spec、任务系统接入、3 路 spec 审核采纳、2 路代码审核采纳，以及 authority/read-model 最小合同接线（`authorityMode / resolvedVisibleNodeId / activeBranchHeadNodeId / isAtBranchHead`）。审核记录：`02_REVIEWS/2026-06-22-pa060-spec-review.md`。OpenSpec change 已归档：`openspec/changes/archive/2026-06-22-unify-session-cursor-view-contract/`。

- `PA-061` 宿主权威 session view 硬切与旧读面安全清理
     说明：已完成正式宿主链路硬切，host-backed `loadSessionState()` 不再从 persisted local state 猜测历史节点，旧 `previousHistoryState` host 补偿分支已删除，并补齐"host authoritative 清空 stale local historical state"定向测试。`npm run verify` 通过验证。OpenSpec change 已归档：`openspec/changes/archive/2026-06-22-unify-session-cursor-view-contract/`。

- `PA-062` browser preview fallback 退场与安全降级收口
     说明：已完成 browser preview / local preview 的能力收口：保留最小可用 preview 体验，但禁用 `restore branch head / fork / switch branch` 这类正式宿主历史控制动作，避免继续伪装成 host-authoritative 恢复能力。相关定向测试已通过。OpenSpec change 已归档：`openspec/changes/archive/2026-06-22-unify-session-cursor-view-contract/`。

- `PA-063` cursor versioning 与多端冲突保护
     说明：已完成 `HistoryCursor.cursor_version`、四类 history-control command 的 `expected_cursor_version`、stale revision 显式冲突错误，以及前端 store 的 `cursorVersion` 透传与 `sessionError` 提示。Rust 回归集已恢复并通过。OpenSpec change 已归档：`openspec/changes/archive/2026-06-22-unify-session-cursor-view-contract/`。

- `PA-058` 拆分 session trace 存储并扩展定向持久化
    说明：已完成独立 `session_turn_traces` 表、dual-read / authoritative no-fallback、hot path trace-level mutation、组合写事务、branch/history trace materialization、`NotFound` 自愈与 trace 表 prune，并通过全局并行审核验收。OpenSpec change 已归档。

- `PA-057` 前端飞行记录仪与卡顿诊断体系
  说明：已完成前端 flight recorder、stall 检测、Tauri/SQLite 持久化与导出、主链路埋点、启动冻结分析与修复。已提交 `f6aab80`。OpenSpec change 已归档。
  
- `PA-054` 桥接能力入口：`MCP Resource / ToolSearch`
  说明：已完成 `MCP Resource / ToolSearch` 的最小 builtin 入口、capability mediation 接线、registry alias/limit/空结果/错误路径回归、前端 capability fallback 对齐，以及 `tool_router_regression` / core 全量 / E2E / smoke 验证。
- `PA-053` 外部读取与知识获取：`WebFetch / WebSearch`
  说明：已完成 `WebFetch / WebSearch` 的真实 builtin 实现、HTTP 成功/非 2xx 结构化结果、解析测试、宿主层回归、前端能力目录同步，以及 core / regression / E2E / smoke 验证。
- `PA-052` 代码库探索增强：`Glob / Grep`
  说明：已完成 `workspace_glob_files` 与 `workspace_search_text.regex` 增强、前端 fallback 对齐、边界回归与宿主层验证，并与第二波工具面保持一致。
- `PA-051` 工具缺口收口：`Edit / Write / Run`
  说明：已完成 `workspace_write_file / workspace_edit_file / workspace_run_command`，补齐权限事实、高风险命令拒绝、结构化错误与宿主层回归，并通过全量前端、core、regression、smoke 与 E2E 验证。
- `PA-050` 第二批基础工具能力与实现顺序
  说明：已完成第二波工具面 spec、任务拆分、`opencode / deepseek-v4-flash` 审核采纳、canonical spec 同步与 OpenSpec 归档；对应实现卡 `PA-051 ~ PA-054` 已全部完成并验证。
- `PA-056` 上下文构建与缓存命中策略重构
  说明：已完成 layered context 实现、`context_refresh_reason / instruction_scope_sources / conversation_carry_mode` 观测落点、显式 `Coding / Work` 配置闭环、`BASE_SYSTEM_PROMPT` 中文语义恢复、小窗口 domain profile 跳过、runtime teardown 稳定性修复、canonical spec 同步、OpenSpec 归档，以及 3 轮 `opencode / deepseek-v4-flash` 代码审核与 follow-up 调优。

- `PA-070` 统一 provider request retry 与退避边界
    说明：已完成 `retry.rs` 核心 substrate、provider/tool 退避对齐、前端 whole-turn auto retry 退场、OpenSpec canonical spec 同步与 archive 收口，并通过 3 路 spec 审核 + 2 路严格代码审核 + 1 轮最终代码调优。Canonical spec：`openspec/specs/provider-retry-and-backoff-boundary/spec.md`。

- `PA-049` 工具观测读面、前端呈现与迁移验收
  说明：已完成实现、单测/e2e/tauri smoke、OpenSpec validate 与收口同步。
- `PA-048` 首批基础工具面与旧工具映射收口
  说明：已完成实现、单测/e2e/tauri smoke、OpenSpec validate 与收口同步。
- `PA-047` 工具权限事实、审批语义与失败归一化
  说明：已完成实现、单测/e2e/tauri smoke、OpenSpec validate 与收口同步。
- `PA-046` agent workspace 合同与路径边界
  说明：已完成实现、单测/e2e/tauri smoke、OpenSpec validate 与收口同步。
- `PA-045` 工具系统协议、暴露策略与结果合同
  说明：已完成实现、单测/e2e/tauri smoke、OpenSpec validate 与收口同步。

- `PA-036` terminal trace envelope 与 monitor 真相源
  说明：已完成 sync failed / streamed cancelled terminal envelope 对齐、reload evidence 保真、monitor canonical truth-source 收紧与前端 raw-trace 防误读约束，并已通过 acceptance audit 与完成态裁定。
- `PA-033` agent hooks pipeline foundation
  说明：已完成 foundation/no-op contract、binding、traceability 与 persisted roundtrip 基础，并已通过独立 acceptance audit；runtime hook dispatch integration 已由 `PA-035` 单独承接。
- `PA-037` session 控制交互面与反馈闭环
  说明：已完成 stop/resume/continue/replay 显式入口、history degrade feedback、统一状态语言与 disabled reason，并已通过 acceptance audit 与前端完成态验证。
- `PA-035` runtime hook dispatch stable-boundary integration
  说明：已完成 stable-boundary runtime hook dispatch、runtime-produced hook trace realtime/persisted/read-plane 闭环，以及 ordering/failure/reload/front-end 验收，并已通过 acceptance audit 与 closeout。
- `PA-034` checkpoint lifecycle boundary implementation
  说明：已完成 runtime checkpoint boundary、persisted evidence、reload/control-plane 投影与前端 runtime store 消费闭环，并已通过 acceptance audit 与完成态验证。
- `PA-032` trace persistence 与 recovery contract
  说明：已完成 recovery contract、submission plan 仲裁、reload/hydration 收口、history degrade 合同与后端/前端恢复仲裁闭环，并已通过 acceptance audit 与完成态裁定。
- `PA-031` turn lifecycle 与 event contract
  说明：已完成 canonical lifecycle/event vocabulary、SSE/event envelope、multi-hop/failed/cancelled 终态语义与前端 canonical 消费收口，并已通过 acceptance audit 与完成态裁定。
- `PA-021` skills registry 与 bridge
  说明：已完成 skill source snapshot ingress、统一 registry、`list_skills / inspect_skill`、tool-only runtime execution、planner normalized skill facts consumption 与 monitor skill lineage 聚合/下钻展示，并已通过 acceptance audit 与完成态 closeout。
- `PA-020` MCP capability bridge
  说明：已完成 capability registry 统一读面、runtime capability bridge、MCP source snapshot 写面、permission/failure 归一化、capability telemetry summary/drilldown，以及 `tool / resource / prompt_template` 的规范化合同与定向验证。
- `PA-040` planner 与 capability-mediation hooks
  说明：已完成 planner `preflight / tool selection / graph decision`、capability `resolve / skill mediation` 与 source ingress 的真实 hook dispatch、白名单 transform、session snapshot / control-plane drilldown 读面闭环，并已通过 acceptance audit 与 closeout。
- `PA-041` history-state hooks 与 restore-boundary contract
  说明：已完成 `history checkout / branch restore / branch fork / branch switch` 四类 boundary 的真实 hook dispatch、persisted audit chain、reload/control-plane/runtime-view/frontend contract 对齐，以及 degrade/non-regression 验收，并已通过 acceptance audit。
- `PA-042` session control audit surface 与 history evidence summary
  说明：已完成 `Session Control Plane audit surface v1`、history-control summary read-model、snapshot/runtime-view/response 统一投影、frontend summary-first explainability 与 acceptance audit/closeout；后续 run-control summary 扩展应由新卡承接。
- `PA-043` run-control audit surface 与 summary-first explainability
  说明：已完成 `Run Control audit surface v1`、`stop / continue / resume / replay(start)` summary contract、snapshot/runtime-view/response 统一投影、frontend summary-first explainability 与 acceptance audit/closeout；普通首轮 `start_graph_run_stream` 排除、truth-source guardrail 与 reload/hydration 回归已收口。
- `PA-039` memory-write hooks 与 persisted side-effect contract
  说明：已完成 `long-term memory write` 的 `intent -> hook -> evidence -> recovery` 真闭环，`memory_write_evidence` 与 `memory_write_hook_trace_records` 已通过 snapshot / history checkout / file roundtrip / checkpoint recovery 验证，并已通过 acceptance audit 与 closeout。
- `PA-038` run hooks 与 execution-control boundary
  说明：已完成 `submission_plan / wait_user / stop_requested / run_resume` 的 canonical boundary persisted evidence、runtime view/session control 统一读面与前端回归，并已通过 acceptance audit 与 closeout。

- `PA-024` 模型监控与 telemetry 聚合面
  说明：已完成 monitor summary / session drill-down 的 Tauri 聚合读面、`ModelMonitorPage` 真实数据页、trace / build-context 下钻展示与前后端定向测试。
- `PA-029` 缓存命中 telemetry 与第一版前缀稳定化
  说明：已完成 call-level cache telemetry、`initial_request / tool_followup` request kind、`PrefixMutationReason` 与第一版 stable prefix 收窄；后端持久化、前端 store 保真与定向测试均已完成，当前可作为 `PA-024` 的底层监控输入。
- `PA-030` trace 面板 call model 可观测性补强
  说明：已完成 `call_model` 的 cache hit / TTFT 展示补齐、工具调用与消息输出保真、多 hop 归因修正，以及完整前端测试与构建验证。
- `PA-028` 历史节点管理、撤销恢复与分支化运行
  说明：已完成 core 历史图、checkout/restore/fork/switch branch、`nodeId` 历史读面与 Tauri 前端历史管理交互，并通过 Rust 与前端定向测试验证。
- `PA-027` OpenSpec 接入任务系统
  说明：已引入 `@fission-ai/openspec`、初始化 `openspec/` 与 Codex workflow skills，并把“复杂开发任务默认先走 OpenSpec”正式写入仓库规范与任务系统规则。
- `PA-025` Build Context 与 cache-friendly prompt 边界
  说明：已完成 `RetrievedContextState -> prompt/request` 三层观测收口；`BuildContextObservation` 现可区分 stable prefix / semi-stable context / volatile input，前后端 trace 展示与回归测试已补齐，并已通过定向 `cargo test`、`session_regression`、前端单测与 `npm run build` 验证。
- `PA-018` 分层 context/state subsystem 与 retrieval boundary
  说明：已完成 retrieval boundary contract、runtime / graph / planner / 宿主默认查询面的 retrieval-first 消费链路、`LongTermMemory` 独立边界与项目级稳定事实来源，并已通过 `cargo test --lib` 与 `npm run verify` 完成态验证。
- `PA-023` 统一 run-stream 正式入口与前端主提交链路
  说明：后端 `start/continue/resume_graph_run_stream`、Tauri run-stream 入口、前端 `submitTurn()` 统一提交流程与 `Trace -> Run -> Turn` 归组链路已经收口。
- `PA-019` graph planner 与计划决策策略
  说明：已落地 `GraphPlanner / GraphPlanningContext / DefaultGraphPlanner`，graph run 可以基于稳定 handoff 输出 `continue / wait_user`。
- `PA-017` 附件生命周期、检索与管理面
  说明：已落地附件生命周期状态、最小检索、显式 cleanup 边界与前端附件中心集成。
- `PA-016` 附件中心索引与资产目录底座
  说明：已落地 `AttachmentAsset / AttachmentReference` 与跨会话附件索引。
- `PA-015` 宿主控制面与统一控制命令
  说明：已落地 `HostControlPlane`、`inspect_host` 与统一控制入口。
- `PA-014` graph stop / resume / checkpoint 与 stop-condition 矩阵
  说明：已落地 `GraphRunStopReason / GraphRunCheckpoint / stop_graph_run / resume_graph_run / load_graph_run_checkpoint`。
- `PA-013` 最小 graph run orchestrator
  说明：已落地 `GraphRunStore / GraphRunner / GraphRunEvent / GraphRunTurnResponse`。
- `PA-012` graph run contract 与 runtime handoff 边界
  说明：已落地 `GraphRun / GraphDecision / GraphTurnHandoff` 第一版 contract。
- `PA-011` 多模态会话记忆与附件生命周期
  说明：已完成附件元数据持久化、recent-image recall、历史附件展示与 review 收口。
- `PA-010` runtime execution control substrate
  说明：已完成 `stop_turn / load_execution_checkpoint / cooperative cancel / turn:cancelled`。

## Dropped

- 暂无
