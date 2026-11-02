# Wave-59 实施计划（L2-E / executor，阶段一产出，只读阶段）

契约源（只读）：`.dev-team/contracts/wave-59-desktop-uiux.md`
冻结验收（只读，test-agent 独写）：`tests/acceptance/wave59/wave59-f{1,2,3,4}-*.spec.ts` + `wave59-harness.ts`
作者：executor。状态：**待 Lead 放行**。

---

## 0. 事实基线与两条必须先裁决的偏差

### 0.1 工作区**不是** clean（契约 §baseline 与实测不符）
`git status` 实测存在一批**并发在飞**的改动（斜杠命令 C-8/C-9 + test-agent 红相）：
`src/lib/runtime/status.ts`(+`mode:"info"`)、`src/lib/runtime/types.ts`(+`sessionNotice`)、
`src/stores/runtime.ts`(+`sessionNotice` 通道)、`src/components/HomeWorkspace.vue`(+69 行斜杠命令入口)、
`src/components/chat/AgentRuntimeStatusBar.vue`(+info 模式图标)、`src/components/HomeSidebar.vue`、
`?? src/lib/chat/`、`?? src/stores/workspace-panels.ts`、`?? tests/acceptance/wave59/` 等。
**影响**：①契约里的 RC 行号整体已漂移（本计划一律给**当前文件**行号）；②我改 `HomeWorkspace.vue` /
`status.ts` / `runtime.ts` 与 C-8/C-9 写域重叠，存在 edit 冲突（按 FS_STALE_VERSION → 重读再 rebase）；
③门禁会被并发写污染（例：`npm run typecheck` 曾因并发删建 `src/components/common/TempForbiddenBoundary.vue`
报 TS6053）。**请求 Lead 裁决**：门禁是否在并发波次收口后由 Lead 统一跑，或我按"仅本波相关失败"判读。

### 0.2 需要新增 2 个 store state 字段 → 触及 `RuntimeState` 类型（不在我的写权内）
F4 冻结验收 `wave59-f4-...spec.ts:388` 用 `Object.keys(store.$state).filter(/retry/i)` 直接探测 state 键，
F4-2/F4-1 需要 hop 级三态。即必须新增 `activeStreamKind` 与 `retryAttemptInFlight` 两个 state 字段。
`RuntimeState` 定义在 `src/lib/runtime/types.ts`（**不在我的 7 文件写权内**；且该文件正被 C-9 改）。
**两个方案，请 Lead 择一并写明**：
- 方案 A（推荐，零写权变更）：在 `src/stores/runtime.ts` 内定义局部扩展类型
  `type RuntimeStateW59 = RuntimeState & { activeStreamKind?: StreamKind; retryAttemptInFlight: number }`，
  `state: (): RuntimeStateW59 => ({...})`，getters/actions 的 `state`/`this` 自动获得该字段。
- 方案 B：授权我在 `src/lib/runtime/types.ts` 追加 2 行字段声明（与 C-9 的 `sessionNotice` 同处）。

（其余 5 个文件全部在我的写权内，无需变更。`docs/observability.yaml` 按 Lead 指令**不动**。）

---

## 1. F1 —— 移除重复等待动画

| # | 文件:行（当前） | 改法 |
| :-- | :-- | :-- |
| F1-1 | `src/components/chat/WorkspaceTurnItem.vue:60` | 删除 `AgentTurnEvent` 的 `{ kind: "waiting" }` 分支（类型契约，编译期清空渲染路径） |
| F1-1 | `WorkspaceTurnItem.vue:391-407` | 删除 `<Transition v-else-if="event.kind === 'waiting'">` 整块（`.assistant-waiting-panel/-dots/-dot`、`data-testid="assistant-awaiting-first-signal"`） |
| F1-1 | `WorkspaceTurnItem.vue:867-906` | 删除 CSS `.assistant-waiting-panel` / `.assistant-waiting-dots` / `.assistant-waiting-dot` / `@keyframes assistant-waiting-dot-bounce` / `.assistant-waiting-signal-*-active/-from/-to` |
| F1-1 | `WorkspaceTurnItem.vue:925-935` | 精简 `prefers-reduced-motion` 块中的 waiting 条目 |
| F1-1 | `src/components/HomeWorkspace.vue:812-818` | 删除 `if (assistantAwaitingFirstSignal(turn)) events.push({kind:"waiting",...})` |
| F1-2 | 无需改（验证项） | 该状态无 assistant 消息 → 状态条 `data-visible="true"` + label "思考中..."（F4 的 hop 三态同时兜底），由验收 F1-2 断言 |
| F1-3 | `HomeWorkspace.vue:990-992` | 删除 `if (assistantAwaitingFirstSignal(turn)) return true;`（`shouldShowAgentArticle` 返回 false） |
| F1-3 | `HomeWorkspace.vue:950-952, 965-985` | 删除已成孤儿的 `assistantHasReasoningSignal` 与 `assistantAwaitingFirstSignal`（`noUnusedLocals` 会拒绝保留；两者仅服务于被删分支） |

**新增 CSS / testid：无。删除：** `.assistant-waiting-*` 全族、`assistant-awaiting-first-signal`。
**风险**：`HomeWorkspace.spec.ts:1249-1250/1272/1303/3009/4283-4284` 旧三点断言会红 → 契约 §5 已授权迁移（test-agent）。

## 2. F2 —— 计数下移 + 排列（`src/components/HomeStatusPanel.vue`）

| # | 文件:行 | 改法 |
| :-- | :-- | :-- |
| F2-1 | `HomeStatusPanel.vue:144-184` | 头部行删除 162-182（注释 + 3 个 `Tooltip > span.inline-flex` 计数位）；仅留「状态」标题 + `status-copy-session-id` |
| F2-2 | `HomeStatusPanel.vue:187` 后 | token 卡（`bg-[#f6f0e8]`）内、token 行**之上**插入计数行：`<div class="flex items-center justify-between gap-2 text-[11px] leading-5 text-stone-600" data-testid="status-metrics-counts">`，内含 3 个 Tooltip 包裹的计数位，顺序 轮次→模型调用→工具调用，testid 原样保留 |
| F2-3 | 同上 | 每个计数位 `span` 自身保留 `tabular-nums` + 内层 `min-w-[2.25rem] text-right`（沿用 wave-17 C2 物理口径） |
| F2-4 | `HomeStatusPanel.vue:189` | token 行**保持为卡的直接子节点**（F2-2 用 `tokenRow.parentElement.classList.contains("bg-[#f6f0e8]")` 断言）；分隔线加在 token 行自身：`border-t border-stone-200/60 pt-2`，`justify-around` 保留；计数行**不得**用 `justify-around` |

**新增 testid：** `status-metrics-counts`（契约点名）。**新增 CSS：无**（纯 Tailwind class；不得新增选择器）。
**风险**：`tests/acceptance/wave17/tabular-nums-contract.spec.ts:113-119`（头部 3 位点）会红 → §5 授权迁移。
`HomeSidebar.spec.ts:344/487/534/564/608` 以头部 testid 反查 `closest("section")`，迁移后仍成立（计数位仍在该 section 内）。

## 3. F3 —— streaming 闪跳（`MarkdownRenderer.vue` + `WorkspaceTurnItem.vue` + `HomeWorkspace.vue`）

### 3.1 MarkdownRenderer：单一稳定承载节点 + stale-while-render（RC1/RC2/RC3）
| # | 文件:行 | 改法 |
| :-- | :-- | :-- |
| RC1/F3-1 | `MarkdownRenderer.vue:322-350` | 模板收敛为「单一根 + 唯一 `.markdown-body`」；删除 333-342 三类尾巴节点（`.mr-streaming-raw` / `.streaming-unrendered-suffix` / `.mr-streaming-char`）与 `v-if/v-else-if/v-else` 三路分支 |
| F3-2 | 同上 | `.markdown-body` **无条件常驻**（无 `v-if`），只换 `v-html` 内容 → 首帧即存在、增长期与 streaming→done 均不重建 |
| RC2 | `MarkdownRenderer.vue:363-396` | 删除 `.mr-streaming-raw` / `.mr-streaming-char` / `@keyframes mr-stream-char-fade-in` / reduced-motion 覆盖块 |
| RC3 | `MarkdownRenderer.vue:136-173` | `executeRender` **入口即快照**：`const source = props.content`，所有 `renderMarkdown/renderPartialMarkdown/splitMarkdownBlocks` 与 `lastRendered*`/`emit(contentLength)` 一律用 `source`（消除"await 期间到达的字符被标记已渲染却不在 html 里"） |
| RC1 | `MarkdownRenderer.vue:34, 41-42, 119-134, 245-266, 283-299` | 删除 `unrenderedSuffix` / `lastRenderedContentLength` / `shouldRenderNow`；改用单一 `lastRenderedSource: string`（上次渲染完成的输入快照）；`handleContentChange` 流式分支简化为「已排程则 return，否则 schedule」 |
| F3-5② | `MarkdownRenderer.vue:175-212` | 首渲染**立即执行**（`!renderedHtml` 时直接 `executeRender(version)`，不排 16ms）避免「content>0 但正文为空」白屏帧；后续增量仍 16ms 紧凑节流 |
| 纯文本快路径 | `MarkdownRenderer.vue:214-243` | 保留 `plainTextMode` 但改为**同步**写 `renderedHtml = escapeHtml(content)`（新增 6 行局部 `escapeHtml`；`.markdown-body` 加 `whitespace-pre-wrap`）。理由：既有 `MarkdownRenderer.spec.ts:46-61` 要求 nextTick 内即可见 text，且不引入额外节点 |
| 错误兜底 | `MarkdownRenderer.vue:163-172` | catch 分支改为 `renderedHtml = escapeHtml(source)`（失败仍可见文本，不再回填尾巴节点） |

### 3.2 生产路径：容器与节点身份（RC4/RC5，F3-3/F3-4）
| # | 文件:行 | 改法 |
| :-- | :-- | :-- |
| F3-4 | `WorkspaceTurnItem.vue:151-164` | 删除 `STREAM_CHAR_STAGGER_*` / `charStaggerDelayMs` / `streamingVisibleChars` |
| F3-4 | `WorkspaceTurnItem.vue:458-491` | 内容区收敛为**一个**容器：`.assistant-plain-text > .assistant-streaming-content`；其内 `v-if=markdown 路径` 用 `MarkdownRenderer`，`v-else` 用 **`.markdown-body whitespace-pre-wrap` 纯文本 div**（新增 class，使纯文本路径也有 `.markdown-body` 承载节点）；删除逐字符 `<span class="assistant-streaming-char">` |
| F3-3 | `HomeWorkspace.vue:1109-1135` | `shouldUseMarkdownAssistantRendering` 增补"完成帧沿用流式锁定路径"：`!streaming && !isAssistantStreaming` 分支先读 `streamingRenderPathLock.get(id) === false && isSimpleTextContent(content)` → 返回 `false`（纯文本沿同容器），**不删锁**；否则删锁返回 `true`（原行为）。消除 RC5 的 plain→markdown 子树替换，并使纯文本 `.markdown-body` 节点跨 done 保持 |
| 备注 | `WorkspaceTurnItem.vue:362-368` CSS 新增 | `:deep(.assistant-streaming-content .markdown-body)` 无需新增；仅给纯文本承载 div 复用既有 `.markdown-body` 样式 |

**Node 身份论证**：①markdown 路径：同一 `MarkdownRenderer` 实例（组件不随 `streaming` 增删），唯一 `.markdown-body` 由 Vue patch 复用；②纯文本路径：`.markdown-body` 是模板里的普通 div，`v-if` 分支不切换（3.2 的锁）→ 同一 DOM 引用。两条路径都满足冻结验收 `wave59-f3:274-291/323-338`。

**新增 CSS/testid**：无新增类，仅把既有 `.markdown-body` 复用到纯文本承载 div（契约未禁止）；删除 `.assistant-streaming-char` 及其 keyframes。
**风险**：
1. stale-while-render 使可见文本滞后 ≤1 个渲染周期（D-59-1 已接受）；F3-5①「scrollHeight 不回撤」在"表格/代码块结构替换"帧理论上仍可能 >1px（D-59-1 已登记该代价）。
2. 纯文本路径不再逐字符 → 若 `HomeWorkspace.spec.ts:1596`（"hell" 4 字符释放）依赖 `useStreamingPresentationState` 的释放节奏，仍成立（节奏在 HomeWorkspace 层，未改）；但 `data-streaming`/`.markdown-body` 断言可能需微调 → **只改实现不改断言**前提下我会先跑该 spec 定位。
3. MarkdownRenderer 单测 3 例（`:99-113`、`:141-166`、`:168-194`）与 `HomeWorkspaceMarkdown.spec.ts:323` 依赖尾巴/逐字符节点 → §5 已授权迁移。

## 4. F4 —— 底部状态准确性（`stores/runtime.ts` + `lib/runtime/status.ts` + `HomeWorkspace.vue`）

### 4.1 D-59-3：hop 级三态 `activeStreamKind`
- `stores/runtime.ts:404` 后：state 增加 `activeStreamKind: "reasoning" | "content" | null | undefined`，初值 **`undefined`**（= 未接线/夹具 ⇒ 回退旧判据；`null` = 本 hop 尚无输出）。
- 置位（唯一 e）：`turn:delta` 处理器 `3858-3864`：`deltaReasoning` 非空 → `"reasoning"`；`deltaText` 非空 → `"content"`（同 delta 两者兼具时正文优先，写在 accumulation 之后）。
- 复位为 `null`（契约 D-59-3 三处）：`turn:started`（`3752-3755` 之后）、`turn:tool`（`4042` 之后）、`turn:hop_complete`（`4116` 之后）。

### 4.2 `agentRuntimeStatus` getter（`stores/runtime.ts:543-571`）
```
kind = state.activeStreamKind
isStreamingReasoning = kind === "reasoning" ? true : kind === "content" ? false : <旧判据>
hasStreamingContent   = kind === "content" ? true : kind === "reasoning" ? false
                        : kind === null ? false : <旧判据>   // null ⇒ 本 hop 无输出（F4-2/RC7）
retryAttempt = state.retryAttemptInFlight > 0 ? state.retryAttemptInFlight : undefined
retryMax     = 同上（单次在飞，无后端上限信号，不虚构上限）
```
- **F4-1**：kind 由 reasoning→content 切换 ⇒ RC6「整轮永久 thinking」消除。
- **F4-2**：`turn:tool`/`turn:started` 后 kind=null ⇒ mode=thinking 且状态条可见（RC7/RC8）。
- **F4-4**：夹具不 patch `activeStreamKind`（保持 `undefined`）⇒ 逐字节沿用旧判据 ⇒ 三例全绿。

### 4.3 可见性（`HomeWorkspace.vue:1019-1058`，插在既有白名单之后、旧判据之前）
```
if (isSubmitting.value) {
  if (kind === "content") return false;   // F4-3
  if (kind === "reasoning") return true;  // 思考中可见
  if (kind === null)       return true;   // F4-2：hop 级等待必须重新可见（RC8）
}
…既有 "pending assistant 有正文则隐藏" 判据保留（kind===undefined 时仍生效）…
```
`isSubmitting` 守卫是必需的：`mode:"info"`（C-9 并发新增）时不得被上一轮残留的 `kind==="content"` 误隐藏。

### 4.4 F4-5：接真实信号（**路线 A**），不删分支
- 新增 state `retryAttemptInFlight: number`（初值 0）；`submitTurn`：`retryingAfterTimeout` 时置 1，否则置 0（`stores/runtime.ts:5286/5386` 附近）；`turn:started` 处理器末尾清零（"回到 calling_model 后清零"）。
- **为何不选路线 B（删分支）**：`tests/runtime-status.spec.ts:158-173` 与
  `tests/acceptance/status-flow-contract.spec.ts:456-490` 两处**冻结**用例显式断言 resolver 返回
  `mode:"retrying"`；契约 §5 的迁移清单**未包含**它们 ⇒ 删分支即引入被迫红。
- 冻结验收 `wave59-f4:388` 用 `/retry/i` 遍历 `store.$state`，故该字段**必须**是 retry 命名的 state 键（命名即契约证据）。
- `status.ts` `retrying` 分支（`:55-64`）**保留**（首个分支，优先级不变）。

**风险**：新增字段触碰 `RuntimeState`（见 §0.2）；标签显示"（1/1）"而非"（1/3）"——不虚构上限，需 Lead 认可这一诚实表述。

---

## 5. 会暂时变红、且只允许 test-agent 迁移的既有用例（我只读）
1. `tests/HomeWorkspace.spec.ts:1249-1250, 1272, 1303, 3009, 4283-4284` — F1 三点（§5 授权）
2. `tests/MarkdownRenderer.spec.ts:99-113, 141-166, 168-194`（尾巴/逐字符）+ `:43/60/153/269` 仍应绿
3. `tests/HomeWorkspaceMarkdown.spec.ts:323`（`.assistant-streaming-char`）
4. `tests/acceptance/wave17/tabular-nums-contract.spec.ts:113-119`；`tests/e2e/wave17-browser-gate.spec.ts:248-262`（e2e 不进 vitest run）
5. 待观察（不属授权迁移，若红我改实现）：`HomeWorkspace.spec.ts:1358/1386/1420/1597/2543`（`.markdown-stub` 不存在）、`:1419`（纯文本裸露 markdown 文案）。

## 6. 门禁执行计划（第二期，每次改完必跑）
`set -o pipefail` 直取 npm 原生退出码（**禁止 `| tail` 吞码**）：
```
npm run typecheck                      # vue-tsc --noEmit，退出码 0
npx vitest run --reporter=dot          # 基线 114 files / 1100 passed / 12 skipped；+4 wave59 spec
```
验收冻结件 `tests/acceptance/wave59/*.spec.ts` 单独跑，判读只看本波 4 个 spec + 既有基线文件；
并发波次（C-8/C-9）造成的红我会单独标注证据，不修改其文件、不改任何测试。
变更登记写入 `.dev-team/wave59-impl-notes.md`（file:line + F 编号 + 门禁收据）。
