// D1 会话目录与生命周期（11 键 / 2 getters / 9 actions）
//
// 实现自 `2f11dc2f:src/stores/runtime.ts` **逐字搬迁**（唯一改动：import 路径按新层级调整、
// action 签名补 `this: RuntimeStoreThis` 形参），语义零改动。键清单与签名真源见 `./types.ts`；
// 跨域读写仍直接落在同一 store 上（不引中间层）。
import { isTauriAvailable, safeInvoke } from "@/lib/tauri";
import { debugLog, measureHostRead } from "@/lib/runtime/utils";
import {
  archiveSession as invokeArchiveSession,
  renameSession as invokeRenameSession,
  unarchiveSession as invokeUnarchiveSession
} from "@/lib/runtime/workspace-api";
import {
  archivePersistedSessionState,
  buildRuntimeViewFromPersistedState,
  buildSessionOverviewFromPersistedState,
  createSessionRuntimeSnapshot,
  filterDeletingSessions,
  hasPersistableMessages,
  loadPersistedRuntimeCache,
  loadPersistedRuntimeState,
  persistSessionState,
  removePersistedSessionState,
  restoreSessionRuntimeSnapshot,
  unarchivePersistedSessionState
} from "@/lib/runtime/sessions";
import {
  normalizeHistoryBranchSwitchResult,
  normalizeHistoryForkResult,
  previewHistoryMutationUnavailable
} from "@/lib/runtime/history";
import type { SessionOverview, SessionRuntimeView } from "@/types/runtime";
import type {
  RuntimeStoreState,
  RuntimeStoreThis,
  SessionCatalogActions,
  SessionCatalogGetters
} from "./types";

/** D1 会话目录与生命周期 getters（键清单见 `types.ts` 的 `SessionCatalogGetters` 与事实图 §2）。 */
export const sessionCatalogGetters = {
  /** :600 */
  isSessionDeleting(this: RuntimeStoreThis, state: RuntimeStoreState) {
    return (sessionId: string) => sessionId in state.deletingSessionSet;
  },
  /** :655 */
  isSessionRunning(this: RuntimeStoreThis, state: RuntimeStoreState): (sessionId: string) => boolean {
    return (sessionId: string) => sessionId in state.runningSessionMap;
  }
} as SessionCatalogGetters;

/** D1 会话目录与生命周期 actions（键清单见 `types.ts` 的 `SessionCatalogActions` 与事实图 §3）。 */
export const sessionCatalogActions = {
    async loadSessionCatalog(this: RuntimeStoreThis) {
      if (isTauriAvailable()) {
        const catalog = await measureHostRead("list_sessions", {
            deletingCount: Object.keys(this.deletingSessionSet).length
          }, () => safeInvoke<SessionOverview[]>("list_sessions"));
        // 后端 list_sessions 返回所有会话（含 archived: bool）；
        // sessionList 保留后端全量列表，由前端派生单一树（deriveSidebarTree）根据 showArchived 进行过滤与展示。
        const filteredCatalog = filterDeletingSessions(
          catalog,
          this.deletingSessionSet
        );
        // 如果当前活跃会话是正在进行中或未落盘的瞬态会话，且未在后端返回列表中，则必须保留在前端列表中，防止消失
        const currentActiveSession = this.sessionList.find(
          (session) => session.conversationId === this.sessionId
        );
        if (currentActiveSession && !filteredCatalog.some((s) => s.conversationId === this.sessionId)) {
          this.sessionList = [currentActiveSession, ...filteredCatalog];
        } else {
          this.sessionList = filteredCatalog;
        }
        return;
      }

      const cache = loadPersistedRuntimeCache();
      this.sessionList = filterDeletingSessions(
        Object.entries(cache.sessions)
          .map(([conversationId, state]) => buildSessionOverviewFromPersistedState(conversationId, state))
          .sort((left, right) => right.updatedAtMs - left.updatedAtMs),
        this.deletingSessionSet
      );
    },
    async loadSessionRuntimeViewState(this: RuntimeStoreThis, sessionId: string, nodeId?: string | null) {
      if (isTauriAvailable()) {
        const payload: Record<string, unknown> = {
          turnId: null,
          sessionId,
          runId: null
        };
        if (nodeId) {
          payload.nodeId = nodeId;
        }
        return await measureHostRead("load_session_runtime_view", {
          sessionId,
          nodeId: nodeId ?? null
        }, () => safeInvoke<SessionRuntimeView>("load_session_runtime_view", {
          ...payload
        }));
      }

      return buildRuntimeViewFromPersistedState(sessionId, loadPersistedRuntimeState(sessionId), nodeId);
    },
    async forkHistoryNode(this: RuntimeStoreThis, nodeId?: string | null) {
      const targetNodeId = nodeId?.trim() || this.visibleNodeId;
      if (!this.sessionId || !targetNodeId) {
        return null;
      }

      if (!isTauriAvailable()) {
        this.sessionError = previewHistoryMutationUnavailable();
        return null;
      }

      return this.performHistoryOperation({
        cmd: "fork_from_history_node",
        invokeArgs: { nodeId: targetNodeId },
        normalize: (payload) => normalizeHistoryForkResult(payload, this.historyNodes, this.historyBranches),
        selectNodeId: (payload) => payload.cursor.visibleNodeId ?? payload.cursor.branchHeadNodeId ?? null,
        errorMessagePrefix: "创建 branch"
      });
    },
    async switchHistoryBranch(this: RuntimeStoreThis, branchId: string) {
      if (!this.sessionId || !branchId.trim()) {
        return null;
      }

      if (!isTauriAvailable()) {
        this.sessionError = previewHistoryMutationUnavailable();
        return null;
      }

      return this.performHistoryOperation({
        cmd: "switch_history_branch",
        invokeArgs: { branchId },
        normalize: (payload) => normalizeHistoryBranchSwitchResult(payload, this.historyNodes, this.historyBranches),
        selectNodeId: (payload) => payload.cursor.visibleNodeId ?? payload.cursor.branchHeadNodeId ?? null,
        errorMessagePrefix: "切换 branch"
      });
    },
    async renameSession(this: RuntimeStoreThis, sessionId: string, title: string): Promise<{ ok: boolean; error?: string }> {
      const key = `s-rename:${sessionId}`;
      if (this.sidebarOpInflightSet[key]) {
        return { ok: false };
      }
      const trimmed = title.trim();
      if (!trimmed) {
        return { ok: false, error: "名称不能为空" };
      }
      if ([...trimmed].length > 64) {
        return { ok: false, error: "名称不能超过 64 个字符" };
      }
      if (!this.sessionList.some((session) => session.conversationId === sessionId)) {
        return { ok: false, error: "会话不存在" };
      }
      this.sidebarOpInflightSet[key] = true;
      try {
        await invokeRenameSession(sessionId, trimmed);
        await this.loadSessionCatalog();
        return { ok: true };
      } catch (error) {
        debugLog("session:rename:failed", { error: String(error) });
        return { ok: false, error: String(error) };
      } finally {
        delete this.sidebarOpInflightSet[key];
      }
    },
    async archiveSession(this: RuntimeStoreThis, sessionId: string): Promise<{ ok: boolean; error?: string }> {
      const key = `s-archive:${sessionId}`;
      if (this.sidebarOpInflightSet[key] || this.deletingSessionSet[sessionId]) {
        return { ok: false };
      }
      // 三级树纵深：当前会话提交期禁归档；后台运行中的会话同样拒绝
      // （菜单禁用是第一道防线，此处为 store 前哨）。
      if (this.isSubmitting && sessionId === this.sessionId) {
        return { ok: false, error: "当前对话正在提交，请稍候后再试" };
      }
      if (this.runningSessionMap[sessionId]) {
        return { ok: false, error: "对话运行中，暂不能归档" };
      }
      if (!this.sessionList.some((session) => session.conversationId === sessionId)) {
        return { ok: false, error: "会话不存在" };
      }
      this.sidebarOpInflightSet[key] = true;
      try {
        if (isTauriAvailable()) {
          await invokeArchiveSession(sessionId);
        } else {
          archivePersistedSessionState(sessionId);
        }
        delete this.completedSessionSet[sessionId];
        delete this.failedSessionSet[sessionId];
        await this.loadSessionCatalog();
        if (sessionId === this.sessionId) {
          // Wave-12 契约 C1（方案 A）：归档当前会话后一律进入全新空白会话；
          // 不切换任何历史会话（含已归档的），也不回退 DEFAULT_SESSION_ID。
          this.enterFreshSession();
        }
        return { ok: true };
      } catch (error) {
        debugLog("session:archive:failed", { error: String(error) });
        return { ok: false, error: String(error) };
      } finally {
        delete this.sidebarOpInflightSet[key];
      }
    },
    async unarchiveSession(this: RuntimeStoreThis, sessionId: string): Promise<{ ok: boolean; error?: string }> {
      const key = `s-unarchive:${sessionId}`;
      if (this.sidebarOpInflightSet[key] || this.deletingSessionSet[sessionId]) {
        return { ok: false };
      }
      this.sidebarOpInflightSet[key] = true;
      try {
        if (isTauriAvailable()) {
          await invokeUnarchiveSession(sessionId);
        } else {
          unarchivePersistedSessionState(sessionId);
        }
        await this.loadSessionCatalog();
        return { ok: true };
      } catch (error) {
        debugLog("session:unarchive:failed", { error: String(error) });
        return { ok: false, error: String(error) };
      } finally {
        delete this.sidebarOpInflightSet[key];
      }
    },
    async deleteSession(this: RuntimeStoreThis, targetSessionId: string): Promise<{ ok: boolean; error?: string }> {
      if (this.isSubmitting || this.deletingSessionSet[targetSessionId]) {
        return { ok: false, error: "当前对话正在提交，请稍候后再试" };
      }

      const deletingActiveEmptySession =
        targetSessionId === this.sessionId && !hasPersistableMessages(this.messages);
      const requiresGlobalSessionOperation =
        deletingActiveEmptySession ||
        targetSessionId === this.sessionId ||
        !this.sessionList.some((session) => session.conversationId === this.sessionId);
      if (requiresGlobalSessionOperation && this.sessionOperation) {
        return { ok: false, error: "当前操作进行中，请稍候" };
      }
      const previousSnapshot = createSessionRuntimeSnapshot(this);
      const persistedStateToRestore = loadPersistedRuntimeState(targetSessionId);
      this.deletingSessionSet[targetSessionId] = true;
      if (requiresGlobalSessionOperation) {
        this.sessionOperation = "deleting";
      } else {
        this.sessionList = this.sessionList.filter((session) => session.conversationId !== targetSessionId);
      }
      this.sessionError = null;

      try {
        removePersistedSessionState(targetSessionId);
        delete this.runningSessionMap[targetSessionId];
        delete this.completedSessionSet[targetSessionId];
        delete this.failedSessionSet[targetSessionId];

        if (isTauriAvailable() && !deletingActiveEmptySession) {
          this.sessionList = filterDeletingSessions(
            await safeInvoke<SessionOverview[]>("delete_session", {
              sessionId: targetSessionId
            }),
            this.deletingSessionSet
          );
        } else {
          await this.loadSessionCatalog();
        }

        const shouldLoadFallbackSession =
          deletingActiveEmptySession ||
          this.sessionId === targetSessionId ||
          !this.sessionList.some((session) => session.conversationId === this.sessionId);

        if (!shouldLoadFallbackSession) {
          debugLog("session:delete", {
            targetSessionId
          });
          return { ok: true };
        }

        // Wave-12 契约 C2（方案 A）：删除当前会话后一律进入全新空白会话。
        // 被删会话已由 removePersistedSessionState()/后端删除；此处不得
        // loadSessionState(历史会话)（会把历史会话切回来），也不得 persistHistory()。
        this.sessionList = this.sessionList.filter(
          (session) => session.conversationId !== targetSessionId
        );
        this.enterFreshSession();
      } catch (error) {
        if (persistedStateToRestore) {
          persistSessionState(targetSessionId, persistedStateToRestore);
        }
        restoreSessionRuntimeSnapshot(this, previousSnapshot);
        this.sessionError = `删除对话失败：${String(error)}`;
        debugLog("session:delete:error", {
          targetSessionId,
          error: String(error)
        });
      } finally {
        delete this.deletingSessionSet[targetSessionId];
        if (requiresGlobalSessionOperation) {
          this.sessionOperation = null;
        }
      }
      return { ok: true };
    },
    /** C-9：设置/清空中性反馈文案（成功、空态提示等），由状态栏以 info 模式展示。 */
    setSessionNotice(this: RuntimeStoreThis, text: string | null) {
      const trimmed = typeof text === "string" ? text.trim() : "";
      this.sessionNotice = trimmed ? trimmed : null;
    },
} as SessionCatalogActions;
