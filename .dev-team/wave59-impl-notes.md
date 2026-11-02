# Wave-59 实施记录（L2-E / executor，第二阶段）

契约源（只读）：`.dev-team/contracts/wave-59-desktop-uiux.md`
冻结验收（只读，test-agent 独写）：`tests/acceptance/wave59/wave59-f{1,2,3,4}-*.spec.ts`
开工前失败快照：`.dev-team/wave59-preexisting-failures.txt`（10 files / 24 tests red）
写权：6 个前端文件（未触碰 `docs/observability.yaml`、`src/lib/runtime/types.ts`、`tests/**`、`crates/**`）。

---

## 0. 门禁收据（物理退出码）

| 命令 | 退出码 | 结果 |
| :-- | :-- | :-- |
| `npm run typecheck`（`set -o pipefail` 直取） | **0** | vue-tsc 无报错 |
| 定向门禁（Lead 指定 6 组 spec，见下） | **0** | 9 files / 114 passed / 9 skipped |
| `npx vitest run`（全量） | 1 | 1 failed / 118 passed / 1126 passed / 12 skipped（唯一红 = 开工快照内既有的 `wave47-ob2-enforcement` 外部门禁，非本波） |

定向门禁明细：`tests/acceptance/wave59`(4 files/18 tests) + `HomeWorkspace.spec.ts` + `HomeSidebar.spec.ts` +
`MarkdownRenderer.spec.ts` + `HomeWorkspaceMarkdown.spec.ts` + `workspace-runtime-status-streaming.spec.ts`。

**与开工快照差异**：开工 24 红 → 收工 1 红（`wave47-ob2-enforcement.spec.ts:223` 的 `ponygo observe audit --level ob2`
外部 CLI 退出码，开工快照内既有，未修）。本波 18 条验收全绿；被授权迁移的 5 个既有文件全绿。

---

## 1. F1 —— 移除重复等待动画

| F 编号 | 文件:行 | 变更 |
| :-- | :-- | :-- |
| F1-1 | `src/components/chat/WorkspaceTurnItem.vue:60-65` | `AgentTurnEvent` 联合类型删除 `{ kind: "waiting" }`（编译期清空渲染路径） |
| F1-1 | `src/components/chat/WorkspaceTurnItem.vue`（原 391-407 已删） | 删除 `<Transition name="assistant-waiting-signal">` 整块（`.assistant-waiting-panel/-dots/-dot`、`data-testid="assistant-awaiting-first-signal"`） |
| F1-1 | `src/components/chat/WorkspaceTurnItem.vue`（样式区） | 删除 `.assistant-waiting-panel/-dots/-dot`、`@keyframes assistant-waiting-dot-bounce`、`.assistant-waiting-signal-enter/leave-*`，并精简 `prefers-reduced-motion` 中对应条目 |
| F1-1 | `src/components/HomeWorkspace.vue:827-828` | 删除 `events.push({ kind: "waiting", ... })`（3 行连带 `assistantOrder` 兜底表达式） |
| F1-2 | `src/components/HomeWorkspace.vue:1029-1039` | 该状态由状态条唯一承载：`mode=thinking`（label「思考中...」）+ `data-visible="true"`（hop 三态 `activeStreamKind !== "content"` 时可见） |
| F1-3 | `src/components/HomeWorkspace.vue:972-982` | `shouldShowAgentArticle` 删除 `if (assistantAwaitingFirstSignal(turn)) return true;` ⇒ awaiting-first-signal 不渲染 agent article / `workspace-agent-actions` |
| F1-3 | `src/components/HomeWorkspace.vue`（原 965-1000 已删） | 删除已成孤儿的 `assistantHasReasoningSignal` 与 `assistantAwaitingFirstSignal`（`noUnusedLocals` 强制） |

新增 CSS/testid：无（净删除）。

## 2. F2 —— 计数下移 + 排列（`src/components/HomeStatusPanel.vue`）

| F 编号 | 文件:行 | 变更 |
| :-- | :-- | :-- |
| F2-1 | `HomeStatusPanel.vue:144-161`（头部 `status-panel-toggle` 在 :144） | 头部行（`status-panel-toggle`）删除 3 个计数位及其注释；仅留「状态」标题 + `status-copy-session-id` 按钮（`span.inline-flex` 计数承载数 = 0） |
| F2-2 | `HomeStatusPanel.vue:173-196`（计数行锚点 :176；Lead 真浏览器复核后由 `justify-between` 调整为 `grid grid-cols-3` + 载体 `justify-center` + 数字 `min-w-[1.75rem] text-left`） | token 卡（`bg-[#f6f0e8]`）内、token 行之上新增 `data-testid="status-metrics-counts"` 行（`flex items-center justify-between gap-2`），顺序 轮次→模型调用→工具调用，testid 原样保留 |
| F2-3 | `HomeStatusPanel.vue:178,184,190` | 每个计数承载 `span` 自身 `tabular-nums` + 内层 `min-w-[2.25rem] text-right`（≤2 层祖先口径天然满足） |
| F2-4 | `HomeStatusPanel.vue:199` | token 行保持为卡的**直接子节点**（`div.flex.items-center.justify-around`，4 个直接 `span.inline-flex` 不变），分隔线 `mt-2 border-t border-stone-200/60 pt-2` 加在 token 行自身；计数行不使用 `justify-around` |

新增 testid：`status-metrics-counts`（契约点名）。新增 CSS：无（仅 Tailwind class）。

## 3. F3 —— streaming 闪跳

### 3.1 `src/components/MarkdownRenderer.vue`（RC1/RC2/RC3）
| 项 | 文件:行 | 变更 |
| :-- | :-- | :-- |
| F3-1/RC1 | `MarkdownRenderer.vue:342-359`（模板） | 收敛为「单一根 + 唯一常驻 `.markdown-body`」；删除 `.mr-streaming-raw` / `.streaming-unrendered-suffix` / `.mr-streaming-char` 三类尾巴节点与三路 `v-if` 分支。`.markdown-body` **无条件常驻**（`v-html` 可为空串），仅替换内容 |
| RC2/D-59-2 | `MarkdownRenderer.vue:361-371`（样式区） | 删除 `.mr-streaming-raw`、`.mr-streaming-char`、`@keyframes mr-stream-char-fade-in`、reduced-motion 覆盖；仅保留 `.mr-blocked-link-notice` |
| RC3 | `MarkdownRenderer.vue:149` + `:168-175` + `:180` | `executeRender` **入口快照** `const source = props.content`；渲染输入、`lastRenderedSource`、`emit(contentLength)` 全部用同一 `source`（消除「await 期间的字符被标记已渲染却没进 html」） |
| F3-5② | `MarkdownRenderer.vue:207-230` | 首渲染立即执行（`!renderedHtml` 时不排 16ms），后续增量 16ms 节流；catch 分支降级 `escapeHtml(source)`（不留空白承载节点） |
| 纯文本快路径 | `MarkdownRenderer.vue:70-77`（`escapeHtml`）+ `:257-286` | 保留 `plainTextMode`：同步转义写入同一承载节点（`.markdown-body` 加 `whitespace-pre-wrap`），不解析、不排程、不产生额外节点；`render-complete` 节流口径不变 |
| F3 稳定性 | `MarkdownRenderer.vue:44-59, 189-206, 207-230` | 新增 `renderInFlightVersion` + `renderDirty` 闭环：在飞期间的增量不叠加排程（不呈现尾巴），落地后**补排**一次 —— 修掉「in-flight 期间到达的内容被吞、正文卡在上一次快照」 |

### 3.2 生产路径（F3-3/F3-4，RC4/RC5）
| 项 | 文件:行 | 变更 |
| :-- | :-- | :-- |
| F3-4 | `src/components/chat/WorkspaceTurnItem.vue:151-164`（已删） | 删除 `STREAM_CHAR_STAGGER_*`、`charStaggerDelayMs`、`streamingVisibleChars`、`.assistant-streaming-char` span 与对应 keyframes（D-59-2） |
| F3-4 | `WorkspaceTurnItem.vue:181-186, 435-460` | 内容区收敛为**单一容器** `.assistant-plain-text > .assistant-streaming-content`；其内 markdown 路径用 `MarkdownRenderer`，纯文本路径用**自建 `.markdown-body whitespace-pre-wrap` div** —— 两条路径的正文承载节点都是 `.markdown-body`，且不再按 `streaming` 分叉成两棵子树 |
| F3-3/RC5 | `src/components/HomeWorkspace.vue:1114-1124` | `shouldUseMarkdownAssistantRendering`：非流式时若流式期已锁定纯文本 ⇒ **沿用纯文本路径**（承载节点不替换）；锁不删除，避免 trace 更新时来回切换。流式期保留既有「出现 markdown 语法即可升级」行为 |
| 生命周期 | `src/components/HomeWorkspace.vue:1672-1674` | `onBeforeUnmount` 清理 `streamingRenderPathLock` |

节点身份论证：markdown 路径 = 同一 `MarkdownRenderer` 实例内唯一 `.markdown-body`（组件不随 `streaming` 增删）；纯文本路径 = 模板内普通 div + 上述路径锁 ⇒ streaming→done 不替换（冻结验收 F3-2a/2b/3a/3b/4 全绿）。

## 4. F4 —— 底部状态准确性

| F 编号 | 文件:行 | 变更 |
| :-- | :-- | :-- |
| D-59-3 | `src/stores/runtime.ts:331-344` | 新增 `ActiveStreamKind` 类型 + 局部扩展 `RuntimeStateW59 = RuntimeState & { activeStreamKind?: ActiveStreamKind }`（**方案 A**，不改 `types.ts`） |
| D-59-3 | `src/stores/runtime.ts:421` | state 新增 `activeStreamKind`，初值 `undefined`（未接线 ⇒ 回退旧判据） |
| D-59-3 | `src/stores/runtime.ts:3944-3950` | `turn:delta`：有正文 delta ⇒ `"content"`（锁定，后续 reasoning delta 不降级）；仅 reasoning ⇒ `"reasoning"` |
| D-59-3 | `src/stores/runtime.ts:3822` / `:4115` / `:4208` | `turn:started` / `turn:tool` / `turn:hop_complete` 复位为 `null`（本 hop 无输出） |
| F4-1/RC6 | `src/stores/runtime.ts:575-594` | getter：`streamingReasoning`/`hasStreamingContent` 改由 hop 级 `activeStreamKind` 主导（`reasoning`⇒思考中；`content`⇒正在回复；`null`⇒hasContent=false ⇒ 思考中），`undefined` 严格回退旧判据 |
| F4-2/RC7/RC8 | `src/components/HomeWorkspace.vue:1029-1039` | 可见性：运行期 `content` ⇒ 隐藏（F4-3）；`reasoning` ⇒ 可见；`null` ⇒ **可见**（hop 级等待，工具后新 hop 重新显示）；`undefined` ⇒ 回落既有 wave-14 判据（F4-4 三例不变） |
| F4-5/RC9 | `src/stores/runtime.ts:3275-3288` | 新增 action `clearTimeoutRetryPlaceholder(turnId?)`：与 `assistant-${turnId}` id 约定**解耦**地清空「超时重连中...」占位 |
| F4-5/RC9 | `src/stores/runtime.ts:3825` / `:3916` | `turn:started` 与 `turn:delta`（有增量时）调用上述 action ⇒ 回到 calling_model 后 retry 信号清零 |
| F4-5/RC9 | `src/stores/runtime.ts:597-605` | getter：活跃 pending assistant 的 content === `TIMEOUT_RETRY_PENDING_MESSAGE` ⇒ `retryAttempt: 1`；**不传 retryMax** |
| F4-5/RC9 | `src/lib/runtime/status.ts:55-67`（`mode:"retrying"` 在 :62） | retrying 分支接入真实信号；`retryMax` 缺省时 label 降级 `正在重试…`（不虚构分母 1/3）；既有传 `retryMax=3` 的冻结用例语义不变 |

### 4.1 F4-5 定位证据（Lead 要求的 gate 取证）
临时插桩（已删除，未落库）在 getter 打印 `messages.length / content / phase / activeTurnId`，得：
`emit("turn:started")` 之后 `msgCount 2 → 3` 且哨兵 content 未清 ⇒ handler **确实执行**（三个 gate 全部通过），
但 `ensureAssistantMessage` 以 `assistant-${turnId}`（`src/stores/runtime.ts:3289-3297`）查消息，
而冻结夹具用 `id: "assistant-f4"`（`tests/acceptance/wave59/wave59-f4-...spec.ts:127-168`）⇒ 新建了第三条消息，
夹具里那条哨兵消息不会被清理。
**修复取向**：按 Lead 裁定「清零不依赖单个 handler 的 gate/消息身份」，改为 id-无关的
`clearTimeoutRetryPlaceholder(turnId)`，在 `turn:started` 与 `turn:delta` 两处调用；未修改任何测试断言。

## 5. 决策遵守声明
- D-59-1 stale-while-render：只呈现上次渲染完成 HTML，无尾巴节点；文本滞后 ≤1 个渲染周期。
- D-59-2：markdown 与纯文本两条路径的逐字符淡入全部移除；`useStreamingPresentationState` 释放节奏**未改**（故 `src/lib/useStreamingPresentationState.ts` 本波无 diff）。
- D-59-3：`activeStreamKind` 三态 + `undefined` 回退；复位点 = `turn:started` / `turn:tool` / `turn:hop_complete`。
- D-59-4：`docs/observability.yaml` 未改（Lead 裁决：可观测性 N-A 由 Lead 统一登记）。
- 未夹带重构：6 文件内仅本波因果链变更；`shouldUseOptimizedAssistantStreaming` prop 保留声明并注明不再参与渲染决策（3 处直接挂载用例 + HomeWorkspace 绑定仍传该 prop）。

## 6. 遗留与风险（交 Lead 裁定）
1. 纯文本完成的 assistant 消息沿用纯文本容器（不再转 markdown 渲染）—— 文本等价，但 `<p>` 外边距等样式细节略有差异（F3-3/RC5 必要代价，§4 已批准）。
2. `tests/acceptance/wave47-ob2-enforcement.spec.ts:223` 仍需外部 `ponygo observe audit --level ob2` 绿相；不属本波。
3. F3-5（真浏览器逐帧 `scrollHeight` 单调性）需 e2e `tests/e2e/wave59-stream-stability.spec.ts`（Lead 阶段统一跑）。

## 7. 并发波次事实（避免误归因）
- `docs/observability.yaml` 在本波期间被**并发波次**（斜杠命令，`wave-slash-1 归属登记`）修改
  （mtime 2026-11-01 22:51，diff 内容为 `compact_session` 四维归属）。executor **未写该文件**（Lead 明令不动）。
- `src/lib/runtime/types.ts`、`src/lib/chat/`、`src/stores/workspace-panels.ts`、`crates/**` 的改动同样来自并发波次，
  本波写权 6 文件内除去上述情况外无越界改动。

## 8. Lead 真浏览器复核后的排版微调（F2 计数行，2026-11-01 二次提交）
- `src/components/HomeStatusPanel.vue:174` 行容器 `flex items-center justify-between` → `grid grid-cols-3 items-center gap-2`
  （仍不含 `justify-around`，F2-4 不回归；三列等宽天然等距，与 token 行成组感一致）。
- `:178,:184,:190` 三个载体加 `justify-center`（保持自身 `tabular-nums`，F2-3 不回归）。
- `:180,:186,:192` 数字 span `min-w-[2.25rem] text-right` → `min-w-[1.75rem] text-left`（数字紧跟图标；固定预留宽度 ⇒ 位数变化不推移相邻位点，wave-17 C2 口径不变）。
- 未动 token 行、未动 tests/**、未动其余文件。
- 门禁：`npx vitest run tests/acceptance/wave59 tests/acceptance/wave17/tabular-nums-contract.spec.ts tests/HomeSidebar.spec.ts` → **EXIT 0**（6 files / 31 passed / 2 skipped）；`npm run typecheck` → **EXIT 0**。

## 9. L3 合入对抗审查修复（B-59-1 / A-59-1 / A-59-2，契约 §7）

### B-59-1（BLOCKING）activeStreamKind 终态残留
- `src/stores/runtime.ts:4275-4279`（turn:completed）/ `:4683-4687`（turn:failed）/ `:4905-4909`（turn:cancelled）/ `:5085-5089`（turn:suspended）：
  在 `activeTurnId/session` 守卫之后、`shouldProcessTurnEvent` 之前**无条件**置 `this.activeStreamKind = null`
  （末次模型调用按协议不保证发 `hop_complete`）。
- `src/stores/runtime.ts:5483-5487`：`submitTurn` 进入提交态时同样置 `null`（不等 `turn:started`）。
- 复现验证：把审查反例 `residue.spec.ts:123` 的「缺陷取证」断言改判为 `null` 后整例通过
  （`/tmp/w59-verify/residue-fixed.spec.ts` → EXIT 0），其契约断言 `{mode:"thinking", visible:"true", dots:0}` 成立。

### A-59-1 非流式早退留下在飞标记
- `src/components/MarkdownRenderer.vue:236-256`：`renderInFlightVersion = version` 移到「内容非空且确实排程」之后；
  早退分支显式 `renderInFlightVersion = null`（原 `:216/:249` 位置调整），流式更新不再被 `renderDirty` 永久吞掉。

### A-59-2 纯文本锁延续到完成态
- `src/components/HomeWorkspace.vue:1119-1127`：锁只在流式期有效；完成态仅当 `锁定纯文本 && isSimpleTextContent(content)`
  才沿用纯文本路径，否则切回 markdown。
  **已批准例外**：若 markdown 语法只在最后一个 delta 出现，完成帧允许**一次**子树替换（正确性优先，
  「`**` / `|` 原样显示」是缺陷）；markdown 路径的 F3-3 节点身份仍成立。

### 附：retry 清零与 turnId 脱钩（A-59-5 的必然推论，同一 BLOCKING 的次生面）
- `src/stores/runtime.ts:3806-3812`（turn:started）/ `:4157-4163`（output_end）/ `:4233-4239`（completed）/
  `:4644-4650`（failed）/ `:4886-4892`（cancelled）/ `:5066-5072`（suspended）/ `:3920`（delta）：
  `clearTimeoutRetryPlaceholder()` 改为**仅凭会话归属**触发，不再按 `payload.turnId` 过滤。
  依据：生产路径 `submitTurn` 用本地 requestId 建消息，而后端 `turn:started` 携带自己的 turnId，
  按 turnId 过滤会把清零挡在 `activeTurnId !== payload.turnId` 的守卫外（实测：F4-5 生产路径用例恒为 retrying）。

### 门禁（二次复核，均 `set -o pipefail` 直取原生码）
- `npm run typecheck` → **0**
- `npx vitest run tests/acceptance/wave59 tests/HomeWorkspace.spec.ts tests/HomeWorkspaceMarkdown.spec.ts tests/MarkdownRenderer.spec.ts tests/workspace-runtime-status-streaming.spec.ts tests/acceptance/status-flow-contract.spec.ts`
  → **0**（9 files / 124 passed / 7 skipped；wave59 自身 22 tests 全绿）
- `npx vitest run`（全量）→ 1 failed / 118 passed（1137 passed / 12 skipped）；唯一红 = 开工快照内既有的 `wave47-ob2` 外部门禁
- 审查反例复跑：`renderer.spec.ts` ✓、`plainlock.spec.ts` ✓、`residue.spec.ts` 仅在「缺陷取证」那一行失败
  （它断言缺陷存在 `=== "content"`，修复后为 `null`），其契约断言部分已用改判副本证明通过。

## 10. 第二类同类缺口修复（A-59-5：activeStreamKind 不随会话切换复位）

- `src/stores/runtime.ts:785-786`：`resetSessionRuntimeState` 用 `blankFields.activeStreamKind` 复位
  （hop 级瞬时态不得跨会话继承）。
- `src/stores/runtime.ts:1717-1719`：`applySessionSnapshot` 恢复目标会话时同样复位，
  随后由后台轮恢复分支重新推导。
- `src/stores/runtime.ts:2276-2284`：`switchSession` 后台运行轮恢复分支按真实缓冲推导 ——
  `textBuffer` 非空 ⇒ `"content"`；否则 `reasoningBuffer` 非空 ⇒ `"reasoning"`；都空 ⇒ `null`。
- `src/stores/runtime.ts:2285-2286`：目标会话无后台轮 ⇒ `undefined`（不携带任何 hop 级瞬时态）。
- `src/lib/runtime/sessions.ts:104-110`：`createBlankSessionRuntimeFields` 纳入 `activeStreamKind`
  （局部别名 `ActiveStreamKindValue`，不改 `types.ts`）；`:234`：`restoreSessionRuntimeSnapshot`
  恢复快照时清除该字段（快照不携带 hop 级瞬时态）。

### 门禁（均 `set -o pipefail` 直取原生码）
- `npm run typecheck` → **0**
- `npx vitest run tests/acceptance/wave59 tests/HomeWorkspace.spec.ts tests/workspace-runtime-status-streaming.spec.ts`
  → **0**（6 files / 98 passed / 7 skipped）
- 复审反例 `/tmp/w59r2/leak.spec.ts` → EXIT 0，`[FACTS]` 由
  `{"leaked":"content","submitting":true,"mode":"responding","visible":"false"}`
  变为 `{"leaked":null,"submitting":true,"mode":"thinking","visible":"true"}`（切到会话 C 后状态条可见、文案「思考中」）。
- 复审其余探针 `/tmp/w59r2/{a591,a592,b591}.spec.ts` → EXIT 0（14 passed）
- 全量 `npx vitest run` → 1 failed / 118 passed（1138 passed / 12 skipped）；唯一红仍为开工快照内既有的 `wave47-ob2`
