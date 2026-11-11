# Agent Note: 流式正文渲染收敛（stale-while-render）与运行状态 hop 级判定

Status: implemented

> **归档冻结说明**：本文档为原历史决策 `docs/archive/decisions/0056-streaming-render-stale-while-render-and-hop-status.md` 之归档快照，仅供溯源。

## 背景

桌面端四项 UI/UX 缺陷（wave-59）根因诊断（证据见 `.dev-team/wave59-delivery.md` §2，全部经真浏览器逐帧采样与 DOM 节点身份断言复核）：

1. **等待首信号时三点跳跃动画与底部状态条「思考中」重复**：`AgentTurnEvent` 的 `waiting` 分支与底部状态条各自播报同一件事。
2. **状态面板计数位挤压可用空间**：轮次/模型/工具三个计数位占据头部行，与 token 指标卡分离。
3. **流式期间 agent 消息「闪跳」**，五条独立根因：
   - RC1 未渲染尾巴以**块级节点**挂在已渲染 HTML 之后 → 每次内容增长新起一行、下一帧又被吸回段落；
   - RC2 尾巴按**逐字符 span + translateY 动画**渲染，且每次 render 重建 → 字符反复从 `opacity: 0` 重播；
   - RC3 `executeRender` 在 `await` 之后才把「本次渲染消费的 content 长度」写回，却用 `await` 前的旧 content 生成 HTML → await 期间到达的字符被标记已渲染但未出现（出现-消失-再出现）；
   - RC4 纯文本路径逐字符动画以**绝对下标**算延迟且封顶 480ms → 长文本新字符恒定延迟后成批浮现；
   - RC5 流式走纯文本分支、完成态走 markdown 分支，`v-if/v-else` 在完成瞬间**替换整棵子树**。
4. **底部状态消息不准确**：`isStreamingReasoning` 取「本轮是否出现过 reasoning」⇒ 出现过即整轮 `thinking`（RC6）；`hasStreamingContent` 取**累计** content ⇒ 工具后的新 hop 被判「正在回复」（RC7）；状态条可见性同样取「本轮任意 content 非空」⇒ 工具后思考 hop 状态条被隐藏（RC8）；`retrying` 分支无 store 信号源，是不可达死代码（RC9）。

## 候选方案

- **方案 A：保留尾巴节点，改为 `display: inline` 内联到已渲染 HTML 尾部**。落选：`v-html` 是整块替换，无法把尾巴插进最后一个 `<p>`；且仍会与逐字符动画耦合（RC2 不解决）。
- **方案 B：把未渲染尾巴写进 `renderedHtml` 同一个 `v-html`**。落选：需要每次内容增长重新解析全文（当前已按块提交 + 缓存，全量重解析会把成本从 O(tail) 抬到 O(text)），且解析期间的中间态仍是"半个块"。
- **方案 C：逐字符动画改为 `animation-delay` 相对批次计算**。落选：仅缓解 RC4，RC1/RC3/RC5 全部残留。
- **方案 D（采纳）「stale-while-render」**：流式期间**只呈现上一次渲染完成的 HTML**，未渲染尾巴不以任何节点呈现；渲染输入在 `executeRender` 入口快照（同一 source 既生成 HTML 也标记已渲染）；`.markdown-body` 作为**唯一常驻承载节点**，streaming↔done 只替换 `v-html` 内容；在飞渲染用 `renderInFlightVersion` + `renderDirty` 闭环，落地后补排一次。
- **方案 E（采纳，状态侧）**：store 新增 hop 级 `activeStreamKind`（`undefined` | `null` | `reasoning` | `content`），在 `turn:started` / 工具事件 / hop 完成时复位、在 delta 到达时置位；`undefined` 严格回退旧判据以保持既有夹具与 wave-14 语义。

## 决策

1. **正文渲染**：语义上等价于「滞后 ≤ 1 个渲染周期（16ms + 解析）换取零位移」。逐字符淡入（markdown 与纯文本两条路径）整体移除；`useStreamingPresentationState` 的字符释放节奏**保留**（不属本波范围）。
2. **DOM 契约**：`.markdown-body` 从首帧起无条件常驻（`v-html` 可为空串）；内容增长与 streaming→done 只改 `v-html`，不重建节点。纯文本内容在完成后**继续走纯文本容器**（自建 `.markdown-body`），以消除完成帧子树替换。
3. **状态判定**：`mode` 与状态条可见性均由 hop 级 `activeStreamKind` 驱动 —— 本 hop 仅推理流式 ⇒ `thinking` 且可见；本 hop 正文流式且正文可见 ⇒ `responding` 且隐藏；本 hop 尚无输出（如工具后的新 hop）⇒ `thinking` 且**重新可见**。
4. **retrying**：接真实信号 —— 超时重试提交时 store 把 `TIMEOUT_RETRY_PENDING_MESSAGE` 写入 pending assistant，getter 据此传 `retryAttempt`；**无后端最大重试次数信号，故不传 `retryMax`**，label 降级为「正在重试…」，不虚构分母。清零由 id 无关的 `clearTimeoutRetryPlaceholder(turnId)` 在 `turn:started` / `turn:delta` 执行。
5. **等待指示唯一化**：删除 `waiting` 事件类型与三点动画；`awaiting-first-signal` 不再渲染空 agent 壳，指示只由底部状态条承担。
6. **状态卡布局**：计数位下移至 token 指标卡内的 `status-metrics-counts` 行（`grid grid-cols-3`，载体 `justify-center`，数字 `min-w-[1.75rem] text-left`，保留 `tabular-nums` 与既有 testid）；token 行结构不变。

## 影响

- **用户可见变化**：等待期不再有两点指示并存；流式文字以 ≤1 帧粒度追赶（肉眼不可辨），且**不再出现行位移与完成瞬间重绘**；工具执行后的第二个思考 hop 重新显示状态条；超时重试显示「正在重试…」而非「思考中」。
- **测试策略演进**：wave-17 的 tabular-nums 物理口径（0.5px / 单调不减）保留，但计数位定位锚点从头部行迁移到 `status-metrics-counts`；`MarkdownRenderer` 的尾巴节点断言（`.mr-streaming-raw` / `.mr-streaming-char` / `.streaming-unrendered-suffix`）改为「不得出现任何尾巴节点 + `.markdown-body` 节点身份不变」；新增真浏览器逐帧门禁 F3-5（scrollHeight 不回撤 >1px、无空白帧、Console Error = 0）。
- **性能**：每次渲染仍需按块解析；已渲染 HTML 在解析在飞期间保持可见（不再有"先清空再重绘"）。
- **可观测性**：纯前端渲染层，四维归属 N-A（无新增接口/网络 IO/后台任务/LLM 调用）；`audit-diff.sh` 与 `check-declaration.sh` 均 exit 0。
- **测试与交付**：契约 `.dev-team/contracts/wave-59-desktop-uiux.md`；红相在 `git archive d3745a7` 纯净快照采集（`.dev-team/wave59-red-receipts.md`）；绿相 `.dev-team/wave59-green-receipts.md`；证据图 `docs/release/evidence/wave-59/`。

