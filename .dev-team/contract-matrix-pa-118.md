# PA-118 桌面端聊天体验优化 验收契约矩阵（Lead 冻结，Test Agent / Executor 对齐基准）

> 用户需求：
> 1. 用户终止回合时，消息流中不再出现"用户终止，发送消息可继续。"字样。
> 2. 排队消息改为"正常用户气泡"同款灰底样式 + 排队徽标；位置从窗口底部移到对话框内容区顶部上方；
>    折叠态最多显示 1 条，多于 1 条堆叠显示，hover 时平滑铺开展示全部。

## 0. 调研结论（已冻结）

- 哨兵句 `"用户终止，发送消息可继续。"` 由 Rust 后端在 `turn:cancelled` 事件 `text` 字段下发
  （`emit_stream_cancelled`），并作为取消回合历史消息内容持久化（`persist_cancelled_turn_outcome` /
  `store.rs::classify_turn_node_kind` 以字符串相等做 `HistoryNodeKind::TurnCancelled` 分类）。
  **后端哨兵保留**（分类与历史语义依赖它），桌面端仅做**展示层过滤**（范围限定桌面 UI，不动
  Rust、不动历史语义、不动 trace/timeline 文本）。
- 前端哨兵污染点共 2 处：
  - `src/stores/runtime.ts` `handleTurnCancelled`（约 4087 行）：`assistantMessage.content = payload.text ?? "本轮已停止。"`
    用哨兵句**覆盖**已流式内容。
  - `src/lib/runtime/messages.ts` `hydrateMessagesFromHistory`（约 479 行）：历史 assistant 消息 `content` 原样带入哨兵句。
- 取消回合的 trace timeline 文本来自 `completed_model_hops`（流式内容），不含哨兵；`turnTraceHistory` 的
  `return_text` 为 None——trace 面无污染，不改。
- 排队消息数据流：`runtimeStore.pendingQueuedMessages: QueuedMessageItem[]`；queue 模式 push（index 0 =
  最旧/下一条执行），steer 模式 unshift（index 0 = 插队消息）。组件 `QueuedMessagesBubble.vue` 当前在
  `HomeWorkspace.vue` 中渲染于 ScrollArea **之后**（窗口底部、Composer 上方），位置需移到 section 顶部
  （ScrollArea 之前）。
- 正常用户气泡：`WorkspaceTurnItem.vue` `userShellClass()` = `rounded-[0.45rem] bg-stone-900 px-3 py-2
  text-stone-50 ...`（深色）。需求指定排队气泡为**灰色**系、形状/对齐近似用户气泡（右对齐、圆角、紧凑）。
- 现有测试冲突点（Test Agent 负责同步更新）：
  - `tests/runtime-store.spec.ts` 约 7062 行断言 `store.messages[1]?.content === "用户终止，发送消息可继续。"` → 需改为新契约。
  - `tests/QueuedMessagesBubble.spec.ts` 全部用例按新堆叠契约重写/扩展（保留 `queued-messages-container`、
    `queue-steer-btn-N`、`queue-remove-btn-N` 既有 testid 契约）。

## 1. Stage 1 前端验收契约（取消回合消息流净化）

红相文件：`tests/acceptance/stage-1-cancel-sentinel.spec.ts`（vitest，Test Agent 编写）

- **F1-1**：新增模块 `src/lib/runtime/cancelled-turn.ts`，导出：
  - `CANCELLED_TURN_MESSAGE = "用户终止，发送消息可继续。"`
  - `stripCancelledTurnSentinel(content: string): string` —— content 去除首尾空白后等于哨兵句 → 返回 `""`；否则原样返回。
- **F1-2**：`runtime.ts` `handleTurnCancelled`：`payload.text` 为哨兵句时**不覆盖** `assistantMessage.content`
  （保留此前流式内容；未流式则保持空串）；`payload.text` 为其他非空文本时照旧使用；`null` 时仍 fallback
  `"本轮已停止。"`。message `status` 仍置 `"done"`。
  - 测试驱动：沿用 `tests/runtime-store.spec.ts` 的 `eventHandlers.get("turn:cancelled")` 直调缝
    （先 `submitTurn` 经 mock `start_graph_run_stream` 拿 runId，再 `stopTurn`，再派发 `turn:started` /
    `turn:cancelled`）。
  - 行为断言：取消前无流式 → 最终 `messages` 中该 assistant 消息 content 为 `""`（不含哨兵句、不含
    "本轮已停止。"）；取消前已有流式内容（可先派发 `turn:chunk`）→ content 保留流式部分且不含哨兵句。
- **F1-3**：`messages.ts` `hydrateMessagesFromHistory`：assistant 消息 `content` 经 `stripCancelledTurnSentinel`
  清洗（历史取消回合 → 空内容）。
- **F1-4**：`HomeWorkspace.vue` `shouldShowAgentArticle`：agent 文章仅在以下任一成立时渲染——
  `turn.assistant` 存在且有可见内容（`assistantHasVisibleContent`）或有可见推理（`shouldShowReasoningBlock`）
  或是错误态（`shouldRenderAssistantAsError`）；或 `turn.tools.length > 0`；或 `assistantAwaitingFirstSignal(turn)`。
  取消回合（内容空、无推理、无工具）→ 不渲染空 agent 壳/复制按钮行。
- **F1-5**：回归：正常回合（非空内容）与错误回合的渲染行为与现状一致（不可见退化）；`turn:cancelled`
  的 trace/timeline/phase 语义（`phase === "cancelled"`、`error === "stopped_by_user"`、traceSteps 末态
  cancelled）不得改变。
- **门禁**：`npm test` 全绿（含本 spec 与同步更新后的 runtime-store.spec.ts）；`npm run build`（vue-tsc + vite）通过。
  后端零改动（`cargo` 门禁跳过）。

## 2. Stage 2 前端验收契约（排队气泡顶部堆叠 + hover 铺开）

红相文件：`tests/acceptance/stage-2-queued-stack.spec.ts`（vitest，Test Agent 编写；可复挂/替换
`tests/QueuedMessagesBubble.spec.ts`）

- **F2-1**：`QueuedMessagesBubble.vue` 根容器保留 `data-testid="queued-messages-container"`；空队列不渲染。
  单条排队消息（n===1）渲染 1 个气泡：灰底、圆角、右对齐、紧凑（形似用户气泡但为灰色系），带排队徽标：
  queue 模式徽标文案含"排队"；steer 模式徽标文案含"插队"；**配色统一灰色系（禁用 amber）**。
- **F2-2**：n > 1 且未 hover（折叠态）：最多 1 条消息内容可见（index 0，即下一条将执行的消息），其余消息
  以堆叠边缘/占位呈现（不展示其文本、不可操作）；容器呈现堆叠数量提示（`data-testid="queue-stack-count"`，
  显示形如 `+N` / `N 排队` 的数量徽标）。
- **F2-3**：hover 容器（`@mouseenter`）：平滑铺开展示全部消息——每条消息为完整气泡（内容可见 + 各自
  插队/删除操作按钮），展开动画为 CSS transition（transform/opacity/height 之一或组合，时长 200~300ms，
  ease-out）；`@mouseleave` 平滑收拢回折叠态。
- **F2-4**：操作不回归：每条展开消息保留 `queue-steer-btn-${index}`（emit `steer(msg.id)`）与
  `queue-remove-btn-${index}`（emit `remove(msg.id)`）；折叠态堆叠边缘不触发操作。
- **F2-5**：位置迁移：`HomeWorkspace.vue` 中 `<QueuedMessagesBubble>` 从 ScrollArea 之后移到 section 顶部
  （ScrollArea 之前、内容列上方），成为对话框顶部上方的常驻条；宽度对齐内容列（`max-w-[46.4rem]` 居中）。
- **F2-6**：jsdom 可测性：折叠/展开状态通过可断言类名或 testid 暴露（如展开态行带 `data-testid=
  "queued-message-expanded"` 或折叠行内容容器类名切换），避免依赖 computed style 数值断言。
- **门禁**：`npm test` 全绿（含本 spec 与重写后的 QueuedMessagesBubble.spec.ts）；`npm run build` 通过。

## 3. 角色与红线

- Test Agent：编写/更新红相验收测试（上列文件），红相验证失败（断言/NotImplemented），供 Lead 红相锚定提交；
  对业务代码只读。
- Executor：仅实现业务代码（cancelled-turn.ts、runtime.ts、messages.ts、HomeWorkspace.vue、
  QueuedMessagesBubble.vue）；**严禁修改任何测试文件**。
- 门禁顺序：红相锚定 commit → Executor 实现 → `npm test` 全绿 → `npm run build` 通过 → Lead 逐项核对契约 →
  审查（≥2 路，跨模块 Diff 触发）→ 阶段提交 + state.json 更新。
- 超出 3 次机器失败 → 熔断隔离分支 + hard reset 回红相锚定点 + `.dev-team/circuit-breaker-report.json`。
