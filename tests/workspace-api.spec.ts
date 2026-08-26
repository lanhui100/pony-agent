// workspace-api 封装参数形状测试：command 名 + camelCase args 逐一钉板；
// pickExistingDirectory 的取消映射与浏览器降级守卫。
import { beforeEach, describe, expect, it, vi } from "vitest";

const mockSafeInvoke = vi.fn();
const mockIsTauriAvailable = vi.fn();

vi.mock("@/lib/tauri", () => ({
  safeInvoke: (...args: unknown[]) => mockSafeInvoke(...args),
  isTauriAvailable: () => mockIsTauriAvailable()
}));

const mockDialogOpen = vi.fn();
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: (...args: unknown[]) => mockDialogOpen(...args)
}));

import {
  archiveSession,
  createWorkspace,
  deleteWorkspace,
  fetchWorkspaces,
  pickExistingDirectory,
  renameSession,
  renameWorkspace
} from "@/lib/runtime/workspace-api";

beforeEach(() => {
  mockSafeInvoke.mockReset();
  mockIsTauriAvailable.mockReset();
  mockDialogOpen.mockReset();
});

describe("workspace-api 命令封装", () => {
  it("五个封装的 command 名与 camelCase 参数逐一对应", async () => {
    mockSafeInvoke.mockResolvedValue(null);

    await fetchWorkspaces();
    expect(mockSafeInvoke).toHaveBeenLastCalledWith("workspace_list");

    await createWorkspace("A", "D:\\ws");
    expect(mockSafeInvoke).toHaveBeenLastCalledWith("workspace_create", { name: "A", rootPath: "D:\\ws" });

    await renameWorkspace("ws-1", "新名");
    expect(mockSafeInvoke).toHaveBeenLastCalledWith("workspace_rename", { workspaceId: "ws-1", name: "新名" });

    await deleteWorkspace("ws-1");
    expect(mockSafeInvoke).toHaveBeenLastCalledWith("workspace_delete", { workspaceId: "ws-1" });

    await renameSession("s-1", "标题");
    expect(mockSafeInvoke).toHaveBeenLastCalledWith("session_rename", { sessionId: "s-1", title: "标题" });

    await archiveSession("s-1");
    expect(mockSafeInvoke).toHaveBeenLastCalledWith("session_archive", { sessionId: "s-1" });
  });

  describe("pickExistingDirectory", () => {
    it("用户取消（null）→ 返回 null；选中目录 → 返回字符串；options 固定 directory/multiple", async () => {
      mockIsTauriAvailable.mockReturnValue(true);

      mockDialogOpen.mockResolvedValue(null);
      await expect(pickExistingDirectory()).resolves.toBeNull();
      expect(mockDialogOpen).toHaveBeenCalledWith(
        expect.objectContaining({ directory: true, multiple: false })
      );

      mockDialogOpen.mockResolvedValue("D:\\picked\\dir");
      await expect(pickExistingDirectory()).resolves.toBe("D:\\picked\\dir");
    });

    it("浏览器模式抛统一降级文案且不触达 dialog 插件", async () => {
      mockIsTauriAvailable.mockReturnValue(false);
      await expect(pickExistingDirectory()).rejects.toThrow("浏览器预览模式");
      expect(mockDialogOpen).not.toHaveBeenCalled();
    });
  });
});
