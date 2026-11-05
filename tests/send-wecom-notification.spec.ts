import { describe, expect, it } from "vitest";
import http from "node:http";
import { execFile } from "node:child_process";
import { join } from "node:path";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);
const scriptPath = join(process.cwd(), "scripts", "send-wecom-notification.mjs");

describe("send-wecom-notification.mjs", () => {
  it("未配置 WX_WORK 环境变量时优雅跳过并退出 0", async () => {
    const { stdout, stderr } = await execFileAsync("node", [scriptPath, "v1.0.0"], {
      env: { ...process.env, WX_WORK_EXTERNAL_API_URL: "", WX_WORK_EXTERNAL_API_KEY: "" },
    });
    expect(stdout).toContain("未配置 WX_WORK_EXTERNAL_API_URL 或 WX_WORK_EXTERNAL_API_KEY");
  });

  it("正确格式化 Payload 并成功发送到 relay 接口", async () => {
    let receivedPayload: any = null;
    let receivedHeaders: any = null;

    const server = http.createServer((req, res) => {
      receivedHeaders = req.headers;
      let body = "";
      req.on("data", (chunk) => {
        body += chunk;
      });
      req.on("end", () => {
        receivedPayload = JSON.parse(body);
        res.writeHead(200, { "Content-Type": "application/json" });
        res.end(JSON.stringify({ status: "success", data: { errcode: 0 } }));
      });
    });

    await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
    const port = (server.address() as any).port;
    const testUrl = `http://127.0.0.1:${port}/api/v1/send`;

    try {
      const { stdout } = await execFileAsync("node", [scriptPath, "v0.9.9"], {
        env: {
          ...process.env,
          WX_WORK_EXTERNAL_API_URL: testUrl,
          WX_WORK_EXTERNAL_API_KEY: "test-api-key-12345",
          WX_WORK_TO_USER: "TestUser",
          GITHUB_REPOSITORY: "test-org/test-repo",
        },
      });

      expect(stdout).toContain("企业微信通知发送成功");
      expect(receivedHeaders["x-api-key"]).toBe("test-api-key-12345");
      expect(receivedPayload).toBeDefined();
      expect(receivedPayload.to_user).toBe("TestUser");
      expect(receivedPayload.content).toContain("v0.9.9");
      expect(receivedPayload.content).toContain("test-org/test-repo");
    } finally {
      server.close();
    }
  });

  it("当接口返回业务错误时非 0 退出", async () => {
    const server = http.createServer((req, res) => {
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ success: false, errcode: 40001 }));
    });

    await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
    const port = (server.address() as any).port;
    const testUrl = `http://127.0.0.1:${port}/api/v1/send`;

    try {
      await expect(
        execFileAsync("node", [scriptPath, "v0.9.9"], {
          env: {
            ...process.env,
            WX_WORK_EXTERNAL_API_URL: testUrl,
            WX_WORK_EXTERNAL_API_KEY: "test-key",
          },
        })
      ).rejects.toThrow();
    } finally {
      server.close();
    }
  });
});
