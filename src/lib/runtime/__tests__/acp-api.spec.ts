/**
 * ACP P2（B163）· `listAcpAgents` 等价判据：命令名与返回值原样透传。
 *
 * 判据口径：这里冻结「前端 → 宿主」的边界（命令名 + 参数形状 + 返回透传），
 * 清单解析语义由 Rust 侧 `acp/config.rs` 的单测覆盖，不在前端重复断言。
 */
import { describe, expect, it, vi } from "vitest";

const tauriMocks = vi.hoisted(() => ({
  mockSafeInvoke: vi.fn()
}));

vi.mock("@/lib/tauri", () => ({
  safeInvoke: tauriMocks.mockSafeInvoke
}));

import { listAcpAgents } from "../acp-api";

describe("listAcpAgents", () => {
  it("透传 acp_list_agents 的返回值，且不额外传参", async () => {
    const agents = [
      {
        id: "opencode",
        label: "opencode",
        command: "opencode",
        args: ["acp"],
        defaultCwd: null,
        builtin: true,
        envKeys: [],
        capabilities: { fs: false, terminal: false }
      }
    ];
    tauriMocks.mockSafeInvoke.mockResolvedValueOnce(agents);

    await expect(listAcpAgents()).resolves.toEqual(agents);
    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith("acp_list_agents");
  });

  it("宿主错误原样抛出（不吞成空清单）", async () => {
    tauriMocks.mockSafeInvoke.mockRejectedValueOnce(new Error("acp_config_parse: 清单损坏"));

    await expect(listAcpAgents()).rejects.toThrow("acp_config_parse");
  });
});
