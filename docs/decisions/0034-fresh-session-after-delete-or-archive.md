# 0034 删除/归档当前对话后一律进入全新空白会话

Status: implemented

## 背景

删除或归档「当前活动对话」后的落点由 `src/stores/runtime.ts` 的两条回退链路决定，二者都会把用户带到历史会话：

- `archiveSession`：取 `sessionList` 中除被归档项外的第一项并 `switchSession(remaining[0])`。`sessionList` 含 `archived: true` 的条目，因此**可能切到一个已归档会话**——在「显示已归档」关闭时它甚至不在侧栏里可见，用户会看到「已归档的对话」被打开。
- `deleteSession`：`fallbackSessionId = sessionList.find(id !== target) ?? sessionList[0] ?? DEFAULT_SESSION_ID`。单会话场景下 `find` 为 undefined，`sessionList[0]` **就是被删会话本身**，`sessionId` 因此停留在被删会话；多会话场景则切到另一个历史会话。

真实浏览器复现（`.dev-team/evidence/session-fallback-after-archive.png`，基线 `4e4942e`）：归档会话 A 后，工作区呈现的是会话 B 的历史消息，侧栏高亮「历史会话BRAVO」。

## 候选方案

- **方案 A（采纳）：一律进入全新空白会话**。删除/归档当前活动会话后，落点等价于点击「新对话」——`resetSessionRuntimeState()` + 新会话 id（`pa_<uuidv7>`）+ 瞬态「新对话」行 + `phase="idle"`，且**不**调用 `switchSession` / `loadSessionState` / `persistHistory`。语义单一、可预期，且顺带修掉「单会话删除落到被删会话自身」的缺陷。
- **方案 B：优先切到最近的未归档会话，无未归档会话时才新建**。改动更小，但仍会把用户带到历史会话，与「默认进入新建会话」的诉求不符；且需要引入「未归档」排序/过滤策略，语义与侧栏过滤耦合，测试面更大。落选。
- **方案 C：仅过滤掉已归档项**（`remaining.filter(s => !s.archived)`）。最小改动，但仍切到历史会话；且 `deleteSession` 的 `sessionList[0]` 落到被删会话自身的缺陷需另行修复。落选：不满足用户诉求且留下第二个缺陷。
- **方案 D：删除/归档后保持原会话不变**。与「会话已不存在/已归档」的状态自相矛盾。落选。

## 决策

新增 store action `enterFreshSession(targetWorkspaceId = DEFAULT_WORKSPACE_ID)`，只做落点重置：`resetSessionRuntimeState()`（内含 `cancelDeferredPersist`，保证旧会话不被延迟落盘）→ 生成全新会话 id → `phase="idle"` / `messages=[]` / `sessionWorkspaceId=DEFAULT_WORKSPACE_ID` → 插入瞬态「新对话」行。`archiveSession` 与 `deleteSession` 在「目标是当前活动会话」时改为直调它，删除 `remaining` / `fallbackSessionId` / `loadSessionState(fallback)` 及其错误分支。**禁止**在该路径上调用 `persistHistory()`（会把被删/归档会话重新落盘）与 `switchSession` / `loadSessionState`（会把历史会话切回来）。

## 影响

- **契约**：`.dev-team/contracts/wave-12-session-fallback.md`（C1–C7）与断言 A1–A7；`tests/runtime-store.spec.ts` 中编码旧契约的 4 个用例按新契约改写（含删除「fallback 载入失败」错误分支覆盖——该分支已随链路删除）。
- **不变项**：非当前会话的删除/归档不导航；归档项仍留在 `sessionList` 且 `archived === true`（unarchive 依赖）；归档内容持久化不被清空；失败仍走 `restoreSessionRuntimeSnapshot` 回滚；侧栏过滤/显示策略、`switchSession`/`loadSessionState` 语义、Rust 后端契约均未改动。
- **行为差异**：删除/归档当前会话后 `sessionId` 变为新 `pa_*` id（不再回退 `DEFAULT_SESSION_ID`）；工作区回到空态「我能帮你做些什么？」；侧栏出现「新对话 / 未保存」行。
