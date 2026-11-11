# PA-057 Code Review

## 审核对象

- [frontend-flight-recorder.ts](/C:/Users/HUAWEI/Documents/pony-agent/src/lib/frontend-flight-recorder.ts)
- [HomeWorkspace.vue](/C:/Users/HUAWEI/Documents/pony-agent/src/components/HomeWorkspace.vue)
- [HomeSidebar.vue](/C:/Users/HUAWEI/Documents/pony-agent/src/components/HomeSidebar.vue)
- [runtime.ts](/C:/Users/HUAWEI/Documents/pony-agent/src/stores/runtime.ts)
- [frontend_diagnostics.rs](/C:/Users/HUAWEI/Documents/pony-agent/crates/pony-agent-core/src/agent/frontend_diagnostics.rs)

## 审核背景

- 时间：`2026-06-18`
- 目标：对本轮 flight recorder / stall diagnostics 正式实现做代码级复审
- 前置实现状态：
  - 前端 recorder、stall detector、snapshot、导出入口已完成
  - Tauri / SQLite 持久化、查询、JSON / Chrome Trace 导出已完成
  - `HomeWorkspace` 热路径埋点与手动 stall smoke 注入入口已补齐

## 已完成验证

1. `npx vitest run tests/frontend-flight-recorder.spec.ts tests/HomeWorkspace.spec.ts tests/HomeSidebar.spec.ts`：通过，`3` 个文件、`61` 条测试
2. `npx vue-tsc --noEmit`：通过
3. `npm run build`：通过
4. `cargo check --manifest-path src-tauri/Cargo.toml -p pony-agent-core --target-dir target-check-core`：通过
5. `npx vitest run`：通过，`13` 个文件、`197` 条测试

## 本轮 `opencode` 并行审核采纳情况

### 已采纳

1. recorder reset 需要恢复默认配置，避免测试间配置泄漏。
   - 已在 `__resetFrontendFlightRecorderForTests()` 中补 `config = { ...DEFAULT_CONFIG }`。
2. 手动 stall 注入需要 wall-clock 兜底，避免 `performance.now()` 异常时忙等失控。
   - 已在 `injectFrontendDiagnosticStall()` 中补 wall clock 逃生条件。
3. `syncStreamingPresentationState` 的完成态埋点不应继续使用高频 counter。
   - 已把该点改为 `recordFrontendSample(...)`，降低 recorder 自身对热路径的扰动。
4. Chrome Trace 导出时间戳需要相对化，提升 Trace Viewer 可读性。
   - 已将导出 `ts` 归一化到本次 trace 首事件时间，并额外保留 `traceOriginWallMs` 元数据。
5. recorder 持久化能力不能只在初始化时拍一次快照。
   - 已在 `flush()` 前刷新 capability；当 Tauri 暂时不可用时，不再清空缓冲，而是保留待恢复后继续写入。

### 暂缓

1. `requeueFlushBatch` 的“优先保旧还是优先保新”需要明确 retention 策略。
   - 这是数据保真策略问题，不适合在本轮无产品约束下直接修改。
2. `latestTurnSignature` / watcher 结构性重排。
   - 这属于后续真实长对话取证后再决定的热路径优化，不在 recorder 能力建设这一卡里扩散。
3. `query_window()` / `query_stall_snapshots()` 的 keyset 分页、`delete_session` 级联清理、`VACUUM`、retention 配置化。
   - 都是合理的存储层增强，但改动面已超出“先建立稳定取证链路”的范围。

## 本轮人工复审结论

1. 已收口 `HomeWorkspace` 首批正式埋点缺口。
   - 现在已覆盖 `turns`、`latestTurnSignature`、`syncStreamingPresentationState`、滚动跟随与用户滚动意图。
2. 已收口 recorder flush 失败时的“逐条重入放大”风险。
   - `flush()` 失败后改为批量前置回排，而不是逐条重新 `enqueue`。
3. 已补最小运维消费与复现路径。
   - 侧边栏和 workspace 都有手动 `stall smoke` 注入入口。
   - Sidebar 已支持 JSON / Chrome Trace 导出。
4. 当前未再发现会阻塞本卡收口的实现问题。
   - 剩余工作应转入“真实长对话采样 -> 导出证据 -> 根因分析”。

## `opencode` 代码审核说明

- 本轮计划继续使用 `opencode / deepseek-v4-flash-free` 做 3 路并行只读代码审核。
- 由于此前已出现 `session continue` 跑偏、最终导出内容被后续上下文污染的问题，本轮必须：
  1. 每路使用独立 prompt
  2. 使用 `opencode run --format json` 保留原始事件流
  3. 不复用旧 session，不做 continue
  4. 仅在拿到稳定最终结论后，才将 findings 记入“已采纳项”

## 当前状态

- 本轮实现、补测、验证已完成
- 前端取证链路已具备：
  - 热路径 sample / span / stall snapshot 记录
  - 手动 stall smoke 复现入口
  - SQLite 持久化
  - JSON / Chrome Trace 导出
- 下一阶段应回到真实长对话复现与证据采样，使用本轮 recorder 数据继续做根因分析

## 调优后复审补充

本轮实现完成后，已再次调用 3 路 `opencode / deepseek-v4-flash-free` 做并行代码复审：

1. `recorder` 路：
   - 仍提示后续可增强项：`flush()` 失败重试缺少退避 / 熔断、页面关闭前缺少 unload flush、detector 健康监控和更多测试覆盖。
   - 本轮未继续采纳，原因：这些属于 recorder 容灾增强，不影响当前“先稳定取证”的主目标。
2. `frontend` 路：
   - 有效指出：`span/counter/instant` 仍然没有统一限流，`latestTurnSignature` 仍是 O(n) 热路径，`MarkdownRenderer` 在长 streaming 中仍有进一步节流空间。
   - 本轮已确认这批属于下一阶段“基于真实采样继续压热路径”的主攻方向。
3. `backend` 路：
   - 仍提示：`delete_session` 未级联清理 diagnostics、retention cleanup 仍在 append 热路径、缺少 vacuum / auto_vacuum、stall snapshots 无分页。
   - 本轮未继续采纳，原因：这已经超出 PA-057 当前卡片“建立诊断能力”的收口范围。

结论：

- 本轮代码审核未发现阻塞当前交付的问题。
- 新增 findings 已形成下一阶段优化 backlog，优先级建议为：
  1. 前端热路径统一限流
  2. `latestTurnSignature` / watcher 结构优化
  3. recorder flush 失败退避与 unload flush
  4. diagnostics 存储层清理与分页增强
