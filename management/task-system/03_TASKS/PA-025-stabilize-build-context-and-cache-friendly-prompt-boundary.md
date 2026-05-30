# PA-025 收口 Build Context 与 cache-friendly prompt 边界

## 状态
- Status: `Backlog`
- Priority: `P2`
- Owner: `Codex`

## 目标
在 `PA-018` 已建立统一 retrieval boundary 的基础上，把 `RetrievedContextState -> prompt/request` 的映射边界单独收口，明确：

- 哪些 retrieval 字段属于稳定前缀
- 哪些字段只属于易变层
- `Build Context` 作为“本轮实际送给模型的请求观察”应该暴露什么

让本卡负责处理 prompt 组装稳定性、cache-friendly request 组织和 `Build Context` 的可理解性，而不是继续把这些问题留在 `PA-018` 的 retrieval boundary 本体里。

## 输出
- `RetrievedContextState -> prompt/request` 第一版分层映射规则
- `BuildContextObservation` 的正式语义说明：它观测的是“本轮实际入模请求”，不是“当前统一上下文状态”
- prompt 稳定前缀 / 半稳定层 / 易变层的最小边界文档
- cache-friendly prompt 组装的第一版工程约束
- 对应前端与 Rust 回归测试补齐

## 验收标准
- retrieval 与 build-context 的语义边界清晰分离：
  - retrieval 表达当前统一上下文状态
  - build context 表达本轮最终 request
- `BuildContextObservation` 至少能稳定表达：
  - request format
  - message count
  - image count
  - tool count
  - request message preview
  - tool definition preview
- prompt 组装层明确区分稳定前缀、半稳定层和易变层
- 高时间波动字段不会默认混入稳定前缀
- 文档明确本卡不负责 retrieval 监控面产品化，不负责 provider 侧长期 telemetry 聚合

## 当前进展
- `PA-018` 已提供 `RetrievedContextState / ContextStateRetriever`
- `DefaultTurnContextBuilder.build_request()` 已开始消费 retrieval 结果
- `BuildContextObservation` 已有基础字段：
  - `requestFormat`
  - `messageCount`
  - `imageCount`
  - `toolCount`
  - `requestMessagesText`
  - `toolDefinitionsText`
- 当前 `Trace` 中的 `Build Context` 展示仍偏弱，尚不足以解释“本轮到底送了什么给模型”
- 当前 cache-friendly 收口只完成了第一步：
  - session summary 不再默认带每轮递增 `turn_count`
  - run 提示不再默认注入 `runId / phase / checkpointStatus`
  - `LongTermMemory.entries` 已按稳定键排序

## 下一步动作
1. 固化 `RetrievedContextState -> prompt/request` 的稳定字段映射规则
2. 收口 `BuildContextObservation` 的 UI/trace 展示，确保它真正解释“本轮实际请求”
3. 梳理哪些字段必须留在易变层，避免打碎 provider cache 命中
4. 为 prompt 组装稳定性补最小回归验证

## 当前卡点
- 当前 retrieval 和 build-context 在右侧 Trace 中语义靠得过近，用户容易把“当前上下文状态”和“本轮实际请求”混为一谈
- 目前还没有 provider cache hit / miss 的直接指标，因此 cache-friendly 收口更多依赖结构与边界约束
- 如果不先定义清楚稳定前缀边界，后续继续优化 prompt 组装容易再次和 retrieval boundary、本地监控面耦合

## 断点续跑提示
继续前先看：
- `src-tauri/src/agent/context.rs`
- `src-tauri/src/agent/provider.rs`
- `src-tauri/src/agent/runtime.rs`
- `src/components/HomeSidebar.vue`
- `src/types/runtime.ts`
- `docs/learning/0015-prompt-caching-as-runtime-design-constraint.md`
