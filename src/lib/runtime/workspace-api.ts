// PA-081/PA-三级树：Workspace 注册表 + 会话操作的前端 API（Tauri 命令封装）。
// 浏览器预览模式下 safeInvoke 抛错，由调用方（store）捕获并保持空列表 +
// 仅默认组的降级呈现。
import { safeInvoke } from "@/lib/tauri";

export interface WorkspaceRecord {
  id: string;
  name: string;
  rootPath: string;
}

export async function fetchWorkspaces(): Promise<WorkspaceRecord[]> {
  return safeInvoke<WorkspaceRecord[]>("workspace_list");
}

export async function createWorkspace(name: string, rootPath: string): Promise<WorkspaceRecord> {
  return safeInvoke<WorkspaceRecord>("workspace_create", { name, rootPath });
}

/** 重命名工作区（仅显示名；id/root 不变）。失败抛出后端错误文案。 */
export async function renameWorkspace(workspaceId: string, name: string): Promise<WorkspaceRecord> {
  return safeInvoke<WorkspaceRecord>("workspace_rename", { workspaceId, name });
}

/** 删除工作区注册（目录与会话数据不动；名下会话归属重写为 default）。 */
export async function deleteWorkspace(workspaceId: string): Promise<void> {
  await safeInvoke<null>("workspace_delete", { workspaceId });
}

/** 会话重命名：写入持久化 override，后续每轮派生刷新不再覆盖。 */
export async function renameSession(sessionId: string, title: string): Promise<void> {
  await safeInvoke<null>("session_rename", { sessionId, title });
}

/** 归档会话（幂等）：分组面隐藏；日志与数据不动。 */
export async function archiveSession(sessionId: string): Promise<void> {
  await safeInvoke<null>("session_archive", { sessionId });
}

/**
 * 系统目录选择器（tauri-plugin-dialog）：仅允许选择已存在的目录；
 * 用户取消返回 null。浏览器模式由调用方先行降级，不进入本函数。
 */
export async function pickExistingDirectory(): Promise<string | null> {
  const { open } = await import("@tauri-apps/plugin-dialog");
  const picked = await open({
    directory: true,
    multiple: false,
    title: "选择工作区目录"
  });
  return typeof picked === "string" ? picked : null;
}
