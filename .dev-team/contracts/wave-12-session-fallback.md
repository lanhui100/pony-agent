# Wave-12 冻结契约 — 删除/归档当前对话后的落点（PA-SESSION-FALLBACK）

- 需求（用户原话）：**删除/归档当前对话后，应该是默认进入新建会话，而不是已归档的会话。**
- 用户裁定（`ask_user_question`，2026-10-15）：**方案 A —— 一律进入全新空白会话**（不切换到任何历史会话，含未归档的）。
- 分级：**B-lite**（单模块=会话生命周期状态机；无外部 IO；但存在既有冻结测试编码旧契约，需契约演进）。
- 基线 commit：`4e4942e`（v0.1.144）
- 时序：Test Agent 先落盘验收测试（红）→ Lead 锚定 commit → Executor 才可写 `src/**`。

## 1. 现状（Lead 侦察，代码定位）

| 路径 | 现状 | 位置 |
| :-- | :-- | :-- |
| 归档当前会话 | `remaining = sessionList.filter(id !== 归档id)`；非空则 `switchSession(remaining[0])` → **可能切到另一个已归档会话**（`sessionList` 含 archived 项） | `src/stores/runtime.ts:2183-2196` |
| 删除当前会话 | `fallbackSessionId = sessionList.find(id !== target) ?? sessionList[0] ?? DEFAULT_SESSION_ID` → 单会话时 `sessionList[0]` **就是被删会话本身**（`sessionId` 停留在被删会话）；多会话时切到另一个历史会话 | `src/stores/runtime.ts:2348-2400` |
| 既有冻结契约 | `tests/runtime-store.spec.ts:1422-1447` 断言「删除当前会话 → `sessionId === "session-fallback"` 且载入其历史消息」= **旧契约，本波推翻** | 同上 |

## 2. 目标契约（冻结）

**C1 归档当前会话成功后**（`archiveSession(current)`）：
- `sessionId` 必须是一个**全新会话 id**：≠ 归档 id，且不在调用前的 `sessionList` 任何条目中；
- `phase === "idle"`、`messages.length === 0`、`isSubmitting === false`、`sessionOperation === null`、`sessionError === null`；
- `sessionWorkspaceId === DEFAULT_WORKSPACE_ID`；
- `sessionList[0]` 为该新会话的**瞬态行**（`title === "新对话"`、`turnCount === 0`、`updatedAtMs === 0`），与点击「新对话」后的形态一致；
- 被归档会话**仍在** `sessionList` 中且 `archived === true`（侧栏「显示已归档」可见、可 unarchive）；
- **不得**调用 `switchSession`，**不得**调用 `loadSessionState(历史会话)`。

**C2 删除当前会话成功后**（`deleteSession(current)`）：同 C1 的全新会话要求，另加：
- 被删会话不在 `sessionList`，且 `loadPersistedRuntimeState(current) === null`（不得复活）；
- 不得把被删会话的状态重新落盘（即**禁止**在新会话切换路径上调用 `persistHistory()` 落盘旧会话）。

**C3 非当前会话**：删除/归档**非当前**会话时，`sessionId`、`messages`、`phase` 必须保持不变（无导航）。

**C4 内容保全**：归档会话的持久化内容不得被清空/覆盖（unarchive 后其历史消息仍可恢复）。

**C5 双路径一致**：`isTauriAvailable()` 为 true（invoke + `loadSessionCatalog`）与 false（本地 persisted 分支）两条路径行为一致。

**C6 失败回滚**：归档/删除抛错时，`sessionId`/`messages`/`phase` 回滚到调用前快照（保留现有 snapshot 机制）。

**C7 边界**：当前会话为空（无持久化消息）时执行删除/归档，同样落入全新会话（新 id ≠ 原 id）。

## 3. 非目标（Non-Goals）

- 不改侧栏归档过滤/显示策略（`showArchivedSessions`）、不改「新对话」按钮语义；
- 不改 `switchSession` / `loadSessionState` 既有语义；
- 不改 Rust 后端 `session_archive` / `delete_session` 契约；
- 不做多会话回退策略优化（用户已裁定一律新建）。

## 4. 写域

| 角色 | write_scopes |
| :-- | :-- |
| Test Agent | `tests/`、`.dev-team/evidence/` |
| Executor | `src/stores/runtime.ts`（必要时 `src/lib/runtime/sessions.ts` 仅新增纯函数导出） |
| Lead | `.dev-team/` |

## 5. 验收断言（机器可判定）

- **A1**：`archiveSession(current)` → C1 全部子项（vitest，Tauri mock + 浏览器本地分支各一次）。
- **A2**：`deleteSession(current)` → C2 全部子项。
- **A3**：非当前会话删除/归档 → C3。
- **A4**：归档后 `loadPersistedRuntimeState(归档id)?.messages` 仍含原消息 → C4。
- **A5**：Tauri 可用/不可用两分支 → C5。
- **A6**：invoke 抛错 → C6。
- **A7**：浏览器真实链路（Playwright）：会话 A 有消息 → 归档 A → 工作区显示空态「我能帮你做些什么？」且侧栏不出现 A（未开归档视图）→ 截图 + Console Error = 0。
- 既有 `tests/runtime-store.spec.ts` 中编码旧契约的断言必须按新契约改写（Test Agent 负责，需在收据中逐条列出）。
