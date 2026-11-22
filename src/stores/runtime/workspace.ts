// D2 工作区树（5 键 / 0 getters / 7 actions）
//
// 实现自 `2f11dc2f:src/stores/runtime.ts` **逐字搬迁**（唯一改动：import 路径按新层级调整、
// action 签名补 `this: RuntimeStoreThis` 形参），语义零改动。键清单与签名真源见 `./types.ts`；
// 跨域读写仍直接落在同一 store 上（不引中间层）。workspace 的 IPC 一律经
// `src/lib/runtime/workspace-api.ts` 包装（不直调 `safeInvoke`）。
import { isTauriAvailable } from "@/lib/tauri";
import {
  createWorkspace as invokeCreateWorkspace,
  deleteWorkspace as invokeDeleteWorkspace,
  fetchWorkspaces,
  renameWorkspace as invokeRenameWorkspace,
  type WorkspaceRecord
} from "@/lib/runtime/workspace-api";
import {
  ACTIVE_WORKSPACE_STORAGE_KEY,
  DEFAULT_WORKSPACE_ID
} from "@/lib/runtime/workspace-constants";
import { debugLog } from "@/lib/runtime/utils";
import type { RuntimeStoreThis, WorkspaceActions, WorkspaceGetters } from "./types";

/** D2 工作区树 getters（本域 0 条，见事实图 §2）。 */
export const workspaceGetters = {} as WorkspaceGetters;

/** D2 工作区树 actions（键清单见 `types.ts` 的 `WorkspaceActions` 与事实图 §3）。 */
export const workspaceActions = {
    // PA-081：Workspace 注册表加载（Tauri 可用时；失败 contained 保持空列表）。
    // 幂等：已加载过则跳过（侧边栏挂载可能多次触发）。
    async loadWorkspaces(this: RuntimeStoreThis, force = false) {
      if (this.workspaceListLoaded && !force) {
        return;
      }
      if (!isTauriAvailable()) {
        this.workspaceList = [];
        this.workspaceListLoaded = true;
        this.normalizeActiveWorkspace();
        return;
      }
      try {
        const workspaces = await fetchWorkspaces();
        // 防御：mock/异常通道可能返回非数组（contained 保持空列表）。
        this.workspaceList = Array.isArray(workspaces) ? workspaces : [];
        this.workspaceListLoaded = true;
        this.normalizeActiveWorkspace();
      } catch (error) {
        // 失败不锁死（loaded 保持 false 允许重试）；照常归一激活项（空注册表 →
        // 残留的失效激活 id 回退 default 并清 key），防止孤儿 id 经 TurnInput
        // 首轮盖章进会话（实施后审核 P2）。
        this.workspaceList = [];
        debugLog("workspace:list:failed", { error: String(error) });
        this.normalizeActiveWorkspace();
      }
    },
    // 激活项不在注册表内 → 回退 default 并清理残留 localStorage key。
    normalizeActiveWorkspace(this: RuntimeStoreThis) {
      const exists = this.workspaceList.some((workspace) => workspace.id === this.activeWorkspaceId);
      if (exists) {
        return;
      }
      if (this.activeWorkspaceId !== DEFAULT_WORKSPACE_ID && typeof window !== "undefined") {
        try {
          window.localStorage.removeItem(ACTIVE_WORKSPACE_STORAGE_KEY);
        } catch {
          // 存储不可写：忽略（内存态已归一）。
        }
      }
      this.activeWorkspaceId = DEFAULT_WORKSPACE_ID;
    },
    async createNewWorkspace(this: RuntimeStoreThis, name: string, rootPath: string): Promise<{ ok: boolean; record?: WorkspaceRecord; error?: string }> {
      const trimmedName = name.trim();
      const trimmedPath = rootPath.trim();
      if (!trimmedName || !trimmedPath) {
        return { ok: false, error: "名称与根路径不能为空" };
      }
      try {
        const record = await invokeCreateWorkspace(trimmedName, trimmedPath);
        this.workspaceList = [
          ...this.workspaceList.filter((workspace) => workspace.id !== record.id),
          record
        ];
        // 三级树裁决①：创建不再自动激活——激活态仅在注册表变更时归一清洗。
        return { ok: true, record };
      } catch (error) {
        debugLog("workspace:create:failed", { error: String(error) });
        // 若报错包含重复 root，说明后端已登记过此目录：重新拉取注册表，尝试命中既有记录
        const errStr = String(error);
        if (errStr.includes("重复 root") || errStr.includes("已存在")) {
          await this.loadWorkspaces(true);
          const existing = this.workspaceList.find(
            (w) => w.rootPath === trimmedPath || w.name === trimmedName
          );
          if (existing) {
            return { ok: true, record: existing };
          }
        }
        return { ok: false, error: errStr };
      }
    },
    // ── 侧边栏三级树操作（design「调用方契约」模板的 store 侧落地）────────
    isSidebarOpInflight(this: RuntimeStoreThis, key: string) {
      return Boolean(this.sidebarOpInflightSet[key]);
    },
    async renameWorkspace(this: RuntimeStoreThis, workspaceId: string, name: string): Promise<{ ok: boolean; error?: string }> {
      const key = `ws-rename:${workspaceId}`;
      if (this.sidebarOpInflightSet[key]) {
        return { ok: false };
      }
      const trimmed = name.trim();
      if (!trimmed) {
        return { ok: false, error: "名称不能为空" };
      }
      if ([...trimmed].length > 64) {
        return { ok: false, error: "名称不能超过 64 个字符" };
      }
      this.sidebarOpInflightSet[key] = true;
      try {
        const record = await invokeRenameWorkspace(workspaceId, trimmed);
        this.workspaceList = [
          ...this.workspaceList.filter((workspace) => workspace.id !== record.id),
          record
        ];
        return { ok: true };
      } catch (error) {
        debugLog("workspace:rename:failed", { error: String(error) });
        return { ok: false, error: String(error) };
      } finally {
        delete this.sidebarOpInflightSet[key];
      }
    },
    async deleteWorkspace(this: RuntimeStoreThis, workspaceId: string): Promise<{ ok: boolean; error?: string }> {
      const key = `ws-delete:${workspaceId}`;
      if (this.sidebarOpInflightSet[key]) {
        return { ok: false };
      }
      if (workspaceId === DEFAULT_WORKSPACE_ID) {
        return { ok: false, error: "默认工作区不可删除" };
      }
      this.sidebarOpInflightSet[key] = true;
      try {
        await invokeDeleteWorkspace(workspaceId);
        this.workspaceList = this.workspaceList.filter((workspace) => workspace.id !== workspaceId);
        // 归属/创建目标归一：后端已重写持久化侧；本地瞬态与激活态同步
        // （spec 场景 Creation target outlives deleted workspace）。
        if (this.sessionWorkspaceId === workspaceId) {
          this.sessionWorkspaceId = DEFAULT_WORKSPACE_ID;
        }
        this.normalizeActiveWorkspace();
        // 目录刷新拉回归属重写后的会话投影。
        await this.loadSessionCatalog();
        return { ok: true };
      } catch (error) {
        debugLog("workspace:delete:failed", { error: String(error) });
        return { ok: false, error: String(error) };
      } finally {
        delete this.sidebarOpInflightSet[key];
      }
    },
    activateWorkspace(this: RuntimeStoreThis, workspaceId: string) {
      // 仅切换"新会话创建目标"，不隐藏其他组（AC2）；localStorage 单一真相源。
      this.activeWorkspaceId = workspaceId;
      if (typeof window !== "undefined") {
        try {
          window.localStorage.setItem(ACTIVE_WORKSPACE_STORAGE_KEY, workspaceId);
        } catch {
          // 存储不可写：内存态生效即可。
        }
      }
    },
} as WorkspaceActions;
