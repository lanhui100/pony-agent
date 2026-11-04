# wave-64 · 前端实施绿相收据（executor-fe）

真值源：`.dev-team/contracts/wave-64-stop-terminal-status.md` §2 F1/F2、§3 写域、§4 迁移。
分支 HEAD 基线：`06472465`（工作区含他人未提交改动，本人未做任何 git 写操作）。

## 1. 门禁（命令原文 + 机器 Exit Code）

| # | 命令原文 | Exit Code | 结果 |
|---|---|---|---|
| G1 | `npx vitest run tests/acceptance/wave64 tests/stop-stop-turn.spec.ts tests/HomeWorkspace.spec.ts tests/runtime-store.spec.ts tests/runtime-status.spec.ts tests/workspace-runtime-status-integration.spec.ts tests/workspace-turn-item-layout.spec.ts` | **0** | Test Files 9 passed (9) / Tests 225 passed \| 7 skipped (232) |
| G2 | `npm run typecheck` | **0** | `vue-tsc --noEmit` 无输出（无错误） |
| G3（补充） | `npx vitest run tests/`（全量单测回归） | **0** | Test Files 143 passed (143) / Tests 1343 passed \| 12 skipped (1355) |

### 1.1 L3 审查修复轮（L3-B BLOCKING + ADVISORY d/e）后的复跑收据

契约已修订：§2 F2 新增「`turn:failed` wire `text` 归一化」条款（对前台轮与后台会话轮同等生效）。

| # | 命令原文 | Exit Code | 结果 |
|---|---|---|---|
| R1 | `npx vitest run tests/acceptance/wave64 tests/stop-stop-turn.spec.ts tests/HomeWorkspace.spec.ts tests/runtime-store.spec.ts tests/runtime-status.spec.ts tests/workspace-runtime-status-integration.spec.ts tests/workspace-turn-item-layout.spec.ts` | **0** | Test Files 9 passed (9) / Tests 229 passed \| 7 skipped (236) |
| R2 | `npm run typecheck` | **0** | `vue-tsc --noEmit` 无输出（无错误） |
| R3（补充） | `npx vitest run tests/`（全量单测回归） | **0** | Test Files 143 passed (143) / Tests 1347 passed \| 12 skipped (1359) |

修复内容（同一写域内，tests/ 仍只读）：
- **L3-B（BLOCKING）**：`src/stores/runtime.ts:374-400` 新增纯函数 `normalizeFailedTurnContent()`
  （null/空白 ⇒ null；逐字 `"This turn failed."` ⇒ null；`正文 + "\n\n（该轮失败前已产出以上内容）"`
  ⇒ 剥离后缀与前导空行、只返正文，空则 null）。后台会话 `turn:failed` 分支（:4752）由
  `payload.text ?? DEFAULT_FAILED_TURN_MESSAGE` 改为 `normalizeFailedTurnContent(payload.text)`
  （null ⇒ 不写 content，保留已流式正文）；前台 failed 分支（:4838-4849）同函数归一化。
- **L3-B(c)**：`src/stores/runtime.ts` 删除 `DEFAULT_FAILED_TURN_MESSAGE` 导入；
  `src/lib/runtime/constants.ts:12` 该常量已无任何引用者（全仓 grep 仅剩注释/契约文本），随删除。
- **ADVISORY d**：`src/lib/runtime/status.ts:62-65` 注释改为「终态由 `agentRuntimeStatus` 的 `stopped` 模式承载」。
- **ADVISORY e**：`src/components/chat/AgentRuntimeStatusBar.vue:187-193` 的 `Pause` 图标补 `aria-hidden="true"`。


G1 中 `tests/acceptance/wave64`（3 文件 12 例）红相 12 例全转绿；迁移后的
`stop-stop-turn.spec.ts` / `HomeWorkspace.spec.ts` / `runtime-store.spec.ts` 同步转绿，
无新增 skip、无放宽断言、未改任何 tests/ 文件。

## 2. 改动文件与行区（供定向 `git add <具体文件>`）

| 文件 | 行区（改后） | 内容 |
|---|---|---|
| `src/lib/runtime/status.ts` | 13-17 | `AgentRuntimeMode` 新增 `"stopped"`（与进行中态 `stopping` 语义分离） |
| `src/components/chat/AgentRuntimeStatusBar.vue` | 3、98-111、186-192、233 | 引入 lucide `Pause`；`isStoppedMode` + stone 中性 tone（无 shimmer/emerald）；`Pause` 图标分支带 `data-testid`/`data-mode`；stopped 不渲染 elapsed |
| `src/stores/runtime.ts` | 346-373 | 新增 `resolveTerminalFailureLabel()`（真实错误文案解析，不读 `state.error`，禁止回落占位正文） |
| 同上 | 598-618 | `agentRuntimeStatus` getter：`!isSubmitting` 分支按 `phase` 派生终态（`cancelled` ⇒ stopped/「已停止」/startTime=null；`failed` + 有轮次消息 ⇒ error + 真实错误文案）。优先序：sessionOperation > sessionError > error > sessionNotice > 终态兜底 |
| 同上 | 976-997 | `cancelBrowserPreviewTurn`：删除 `content = "本轮已停止。"` |
| 同上 | 4808-4815（第一轮）→ 现 4838-4849 | `turn:failed`：占位正文写入移除；**L3-B 修复轮**起改用 `normalizeFailedTurnContent()` 归一化 wire `text`，归一为 null 时不写 content（保留已产出正文）；`errorDetail` 保留 |
| 同上 | 374-400（L3 修复轮新增） | `normalizeFailedTurnContent()` + 两个 wire 文案常量（零正文兼容锁 / 失败说明后缀） |
| 同上 | 4752（L3 修复轮） | 后台会话 `turn:failed` 分支：`payload.text ?? DEFAULT_FAILED_TURN_MESSAGE` ⇒ `normalizeFailedTurnContent(payload.text)` |
| 同上 | 5018-5046 | `turn:cancelled`：删除占位正文写入；删除 `sessionNotice` 停止文案写入（notice 仅留 C-9 斜杠命令） |
| 同上 | 5767-5772 | `submitTurn` 异常分支：删除占位正文写入，错误明细落 `errorDetail` |
| 同上 | 87（L3 修复轮） | 删除 `DEFAULT_FAILED_TURN_MESSAGE` 导入（已无运行时引用） |
| `src/components/HomeWorkspace.vue` | 794、857 | 正文渲染不再按错误态抑制（失败轮正文照常渲染） |
| 同上 | 882-883 | 删除 `error` 事件注入（消息区不再有「错误详情」面板源） |
| 同上 | 998-1012 | `shouldShowAgentArticle` 移除错误态条件（空正文不再渲染空 agent 壳） |
| 同上 | 1034-1052 | `isRuntimeStatusVisible`：`stopped` 与 `error` 显式短路返回 true（终态常显，不被「正文流式隐藏」误吞） |
| 同上 | 1193-1195 | 删除死函数 `assistantErrorDetail` / `shouldRenderAssistantAsError` |
| `src/components/chat/WorkspaceTurnItem.vue` | 71-78 | `AgentTurnEvent` 删除 `"error"` 变体（编译期封死面板渲染路径）— **仅此块与下述两块为本波改动** |
| 同上 | 154-158、426-428、715-716 | 删除错误详情 `details` 面板、`copyErrorDetail`/`copiedErrorDetail`/`assistantErrorDetail`/`latestTraceForTurn`；相应收敛 `storeToRefs` 解构 |
| `src/lib/runtime/constants.ts` | 11-13（L3 修复轮） | 删除已无引用者的 `DEFAULT_FAILED_TURN_MESSAGE`（占位正文随 F2 废除；保留 `DEFAULT_FAILED_TURN_ERROR`） |

> 说明：`WorkspaceTurnItem.vue` 的工作区版本另含**他人未提交**的 P1-2 退出码徽标改动
> （imports `exitCodeClassLabel`/`parseToolExitMeta`、`exitCodeBadge()`、工具行徽标模板）。
> 本次仅做定点最小编辑，未触碰上述行；该文件按块暂存时请留意分离。

## 3. 遗留风险

1. `resolveAgentRuntimeStatus()` 本身的 `!isSubmitting ⇒ null` 语义未改（wave-59 F4 契约锁），
   stopped/error 终态由 store getter 的 `phase` 派生分支承载 —— 两条通道互不冒充。
2. 恢复路径（刷新后）`phase` 由 `resolveRestoredPersistedPhase(canonicalTerminalPhase)` 还原，
   失败终态的错误文案来源退化为「助手消息 `errorDetail` → 失败 trace `error` → `本轮执行失败。`」，
   与实时事件路径（`state.error`）文案可能不同（同一 `payload.error` 时一致）。
3. `fetchHealth` 失败亦置 `phase="failed"`：派生分支已按「存在轮次消息」门控，
   启动期无消息时不新增状态行（与既有 `state.error` 分支口径一致）。
4. 未做真实浏览器（Playwright）取证与 `docs/observability.yaml` 四维归属 —— 属契约 §5 的其他门禁，
   由 Lead 分派；本收据只覆盖前端单测 + 类型门禁。
5. wire 归一化按契约给定的**逐字**后缀匹配（`"\n\n（该轮失败前已产出以上内容）"`）。core 侧该后缀
   定义在 `crates/pony-agent-core/src/agent/turn_flow.rs`（`FAILED_PARTIAL_TEXT_SUFFIX`），
   两侧为跨语言重复字面量：任一侧改字即静默失配（后果=后缀漏进正文）。已加常量注释指回 Rust 真值源；
   如需长期护栏，建议另起"前后端字面量同步"契约用例（本波不扩范围）。
6. `src/lib/runtime/constants.ts` 的 `DEFAULT_FAILED_TURN_MESSAGE` 已删除（L3-B(c) 收口）。
   若后续有分支需要"失败占位文案"，请重新引入并显式声明用途，不得再被终态路径使用。
