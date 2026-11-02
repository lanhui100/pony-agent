# Wave-59 契约矩阵：桌面端 UI/UX 四项优化（冻结，只读）

基线 commit：`d3745a7`（工作区 clean）
回归基线：`npx vitest run` → 114 files / 1100 passed / 12 skipped / exit 0（`.dev-team/regression-baseline-wave59.json`）
分级：B（跨 6 个前端文件 + UI 交互 + 流式状态机；无外部网络/IO 新增）
NFR：N-A（纯前端渲染层，无新增外部调用）；可观测性见 D-59-4。

## 0. 需求 → 契约映射

| 需求 | 契约编号 |
| :-- | :-- |
| ① 去掉与底部"思考中"重复的三点跳跃动画 | F1-1..F1-3 |
| ② 轮次/模型/工具计数下移，与 token 计数同卡排列 | F2-1..F2-4 |
| ③ streaming 时 agent 消息闪跳 诊断+修复 | F3-1..F3-5 |
| ④ 底部状态消息准确性 验证+修复 | F4-1..F4-5 |

## 1. F1 —— 移除重复等待动画

- **F1-1**：`awaiting-first-signal` 状态下，DOM 中不得存在任何 `.assistant-waiting-dot`、`.assistant-waiting-panel`、
  `[data-testid="assistant-awaiting-first-signal"]` 节点。
- **F1-2**：同一状态下底部状态条 `[data-testid="workspace-runtime-status-slot"]` 必须 `data-visible="true"`，
  且 `[data-testid="agent-runtime-status-label"]` 文本非空（承载"思考中/准备就绪中"唯一指示）。
- **F1-3**：不得残留空 agent 壳：`awaiting-first-signal` 时 `[data-testid="workspace-agent-actions"]`
  与 agent 文章容器均不渲染（`shouldShowAgentArticle` 返回 false）。
- 类型契约：`AgentTurnEvent` 联合类型删除 `waiting` 分支（编译期强制清空渲染路径）。

## 2. F2 —— 计数下移 + 排列

- **F2-1**：`[data-testid="status-panel-toggle"]`（头部行）内**不得**再出现 3 个计数承载 `span.inline-flex`；
  头部仅保留「状态」标题 + 复制 Session ID 按钮（`[data-testid="status-copy-session-id"]` 仍在头部）。
- **F2-2**：新增 `[data-testid="status-metrics-counts"]` 行，位于 token 指标卡（`#f6f0e8` 卡）内部、
  token 行**之上**；行内恰好 3 个计数位，顺序为 轮次 → 模型调用 → 工具调用，
  分别保留 testid `status-metric-turn-count` / `status-metric-model-call-count` / `status-metric-tool-call-count`。
- **F2-3**：三个计数位自身或其 ≤2 层祖先必须含 `tabular-nums`；位数变化不推移相邻位点
  （沿用 wave-17 C2 物理口径，定位锚点由头部迁移至 `status-metrics-counts`）。
- **F2-4**：token 行结构不回归：`div.flex.items-center.justify-around` 的直接 `span.inline-flex` 子元素恰为 4 个；
  计数行不得使用 `justify-around` 类（避免既有选择器歧义）。
- 视觉：计数行与 token 行之间允许一条 `border-t border-stone-200/60` 分隔；整体仍在同一张卡内。

## 3. F3 —— streaming 闪跳（已定位根因）

### 根因（Lead 诊断，证据附行号）

- **RC1** `MarkdownRenderer.vue:332-343`：streaming 时把「未渲染尾巴」渲染为**独立块级节点**
  （`.mr-streaming-raw` / `.streaming-unrendered-suffix`）挂在已渲染 `v-html` 块之后。
  块级元素必然新起一行 → 新字符先在段落下方单独成行，下一次 render 后被吸回段落 → 每 tick 位移。
- **RC2** `MarkdownRenderer.vue:334`：`forceMarkdownStreaming`（WorkspaceTurnItem 恒传 true）下尾巴按
  **逐字符 span + `mr-stream-char-fade-in`(含 translateY) + 每次 render 重建**渲染 → 字符反复从
  opacity 0 / 上移状态重播，即肉眼"闪"。
- **RC3** `MarkdownRenderer.vue:152-157`：`executeRender` 在 `await` 之后才写入
  `lastRenderedContentLength = props.content.length`，而 html 由 await 前的旧 content 生成 →
  await 期间到达的字符被标记为"已渲染"却并未出现在 html 中，直到下一次 render 才补上 → 文字出现-消失-再出现。
- **RC4** `WorkspaceTurnItem.vue:481-489`：纯文本路径逐字符 span，`charStaggerDelayMs` 以**绝对下标**
  计算且封顶 480ms → 长文本新到达字符恒延迟 480ms 才可见，成批后置浮现。
- **RC5** `WorkspaceTurnItem.vue:459-490` + `HomeWorkspace.vue:1109-1135`：streaming 走纯文本分支、
  完成态走 Markdown 分支，`v-if/v-else` 切换**替换整棵 DOM 子树** → 完成瞬间闪一下。

### 契约

- **F3-1**：streaming 期间，`MarkdownRenderer` 根元素下不得出现 `.mr-streaming-raw`、
  `.streaming-unrendered-suffix`、`.mr-streaming-char` 节点（未渲染尾巴不再以独立节点呈现）。
- **F3-2**：streaming 期间，`.markdown-body` 承载节点在内容增长过程中**DOM 节点身份不变**（不得重建）。
- **F3-3**：streaming→完成态切换时，承载正文的 `.markdown-body` 节点身份不变（不替换子树），
  且完成帧的可见文本包含完整内容（无"先清空再重绘"）。
- **F3-4**：`WorkspaceTurnItem` 内容区不得出现 `.assistant-streaming-char` 逐字符节点；
  流式与完成态共用同一容器结构。
- **F3-5**（真浏览器）：合成 markdown 长流下逐帧采样 agent 内容列，
  ① `scrollHeight` 不得回撤 > 1px（单调不减）；② 任一帧在 content 长度 > 0 时文本不得为空（无白屏闪）；
  ③ Console Error = 0；④ 落盘前后对照截图。

## 4. F4 —— 底部状态准确性

### 缺陷（Lead 诊断）

- **RC6** `stores/runtime.ts:546-549` + `lib/runtime/status.ts:115-122`：`isStreamingReasoning` 取
  "本轮 reasoningContent 是否非空"→ 一旦出现过 reasoning 就**整轮永久为真**，
  于是 `streamingReasoning` 分支（先于 calling_model 判定）优先级压过正文流式 → 正文已在输出仍显示"思考中"。
- **RC7** `stores/runtime.ts:550-553`：`hasStreamingContent` 同样取累计 content → 多 hop（工具后第二轮模型调用）
  开始思考时仍被判为"正在回复"。
- **RC8** `HomeWorkspace.vue:1046-1055`：可见性判据取"本轮任意 content 非空"→ 工具结束后的思考 hop 里
  状态条被隐藏，用户看不到任何进行中指示（与 wave-14「运行期始终可见」相悖）。
- **RC9** `lib/runtime/status.ts:53-63`：`retrying` 分支为死代码（store 从不传 `retryAttempt`），
  超时重试时用户只看到"思考中"。

### 契约

- **F4-1**：hop 内 reasoning delta 后 → `mode="thinking"`；紧随 text delta 后 → 必须切到 `mode="responding"`
  （不得因本轮曾有 reasoning 而停留 thinking）。
- **F4-2**：工具事件后的新 hop（尚无 reasoning/text delta）→ `mode="thinking"`，且
  `[data-visible]="true"`（状态条重新可见，承载 hop 级等待）。
- **F4-3**：正文流式（当前 hop 有 text delta 且正文可见）→ 状态条 `data-visible="false"`（沿用优化3）。
- **F4-4**：既有 `tests/workspace-runtime-status-streaming.spec.ts` 三例（仅 patch messages/phase 的夹具）必须保持绿。
- **F4-5**：`retrying` 要么接入真实超时重试信号（store 传 `retryAttempt`/`retryMax` 并回到 `calling_model` 后清零），
  要么显式删除该分支并在契约登记「N-A：无重试信号源」。**不得保留不可达分支冒充能力**。

## 5. 测试冻结与分权

- 本次为**契约演进**（用户显式要求改动布局与动画），Lead 授权迁移以下既有断言（原意图不变，仅迁移定位锚点）：
  - `tests/acceptance/wave17/tabular-nums-contract.spec.ts` C2-1 计数位定位：头部 → `status-metrics-counts`
  - `tests/e2e/wave17-browser-gate.spec.ts` C2-2 同上迁移
  - `tests/HomeWorkspace.spec.ts` 等待三点断言 → 改为 F1-1/F1-2 断言
  - `tests/MarkdownRenderer.spec.ts` / `tests/HomeWorkspaceMarkdown.spec.ts` 尾巴节点与逐字符断言 → F3-1/F3-4
- 新增验收：`tests/acceptance/wave59/`（test-agent 独写）+ `tests/e2e/wave59-stream-stability.spec.ts`
- **执行者禁止改测试**（test-agent 独有测试写权）；测试冻结后 executor 只读。

## 6. 决策登记

- **D-59-1**：流式渲染改为「stale-while-render」：只呈现上次渲染完成的 HTML，未渲染尾巴不单独成块；
  代价是文字滞后 ≤ 1 个 render 周期（16ms + parse），换取零位移。
- **D-59-2**：移除逐字符淡入（markdown 与纯文本两条路径），保留 `useStreamingPresentationState` 的释放节奏（不属本波范围）。
- **D-59-3**：hop 级流式类型由 store 记录 `activeStreamKind`（`reasoning` | `content` | null），
  `turn:started` / `turn:tool` / `hop_complete` 时复位为 null；缺省时回退旧判据（保证既有夹具兼容）。
- **D-59-4**：可观测性 N-A 理由 —— 本波仅改前端渲染层 DOM 结构与展示态判据，无新增对外接口/网络 IO/后台任务/LLM 调用；
  `docs/observability.yaml` 四维归属按 N-A 登记结构性理由（审计 `audit-diff.sh` 与 `check-declaration.sh` 需 exit 0）。

## 7. 独立审查裁定（2026-11-02 追加，`.dev-team/review-join-wave59.json` / `review-plan-wave59.json`）

- **B-59-1（BLOCKING，必修）**：`activeStreamKind` 必须在**全部终态**（`turn:completed` / `turn:failed` /
  `turn:cancelled` / `turn:suspended`）与 `submitTurn` 起点复位。否则每轮结束后残留 `"content"` →
  下一轮 `submitTurn` 到 `turn:started` 之间的 awaiting-first-signal 窗口被误判 `responding` 且状态条隐藏，
  而三点动画已删 ⇒ 该窗口零进行中指示（违反 F1-2 / F4-2）。
  验收：新增 F1-4 —— 「上一轮以 content 结束 → 立即 submitTurn（未收到 turn:started）」窗口内
  状态条 `data-visible="true"` 且 `mode ∈ {connecting, thinking}`。
- **A-59-1（必修）**：`MarkdownRenderer.scheduleNonStreamingRender` 在内容空白时提前 return 却已写入
  `renderInFlightVersion` ⇒ 该实例后续流式更新被 dirty 永久吞掉。改为提前 return 前不置在飞标记（或立即清）。
- **A-59-2（必修，附取舍）**：`shouldUseMarkdownAssistantRendering` 的 `streamingRenderPathLock=false`
  不得延续到完成态；完成态若 `!isSimpleTextContent(content)` 必须走 markdown 渲染
  （避免把 `**` / `|` 原样显示）。**取舍**：该窄场景下完成帧允许一次子树替换（正确性优先于零替换），
  在交付说明登记为 RC5 的已知例外。
- **D-59-7（契约修订）**：F3-5 ② 采用**收窄读法**并正式落契约 —— 「正文首次非空帧之后不得再出现空文本帧」
  + 「首个内容帧后 ≤4 帧内出现首个非空正文帧」。原「content>0 任一帧不得为空」被本裁定取代（D-59-1 允许 ≤1 渲染周期滞后）。
- **D-59-8（契约补强）**：F3-5 必须覆盖 **streaming→完成态**：驱动消息从 pending 转 done，
  断言完成帧前后 `.markdown-body` 节点身份不变、文本不回撤、无空白帧。
- **D-59-9（契约补强）**：F4-5 的可达性必须由**生产路径**驱动（`submitTurn` 遇可重试错误的超时重试），
  不得由夹具直接写入 `TIMEOUT_RETRY_PENDING_MESSAGE` 自证。
- **A-59-3（修订）**：F4-2 不得钉死 label 文案，只断言 `mode` 与 `data-visible`（文案属展示层，允许后续措辞调整）。
- **A-59-4（补测）**：F4 追加 error / sessionError / sessionOperation 与 ≥3 连续 hop 的覆盖（验证这些态强制 `visible=true` 不被 F4-3 压过）。
