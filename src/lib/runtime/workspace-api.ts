// PA-081：Workspace 注册表前端 API（Tauri 命令封装）。
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
