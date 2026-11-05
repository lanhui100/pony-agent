#!/usr/bin/env node
/**
 * scripts/send-wecom-notification.mjs
 * 发送企业微信发版通知脚本（参考 job_copilot 契约）。
 *
 * 依赖环境变量:
 *   - WX_WORK_EXTERNAL_API_URL
 *   - WX_WORK_EXTERNAL_API_KEY
 *   - WX_WORK_TO_USER (可选，默认 MaHui)
 */

import https from "node:https";
import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import { URL } from "node:url";

const apiUrl = process.env.WX_WORK_EXTERNAL_API_URL?.trim();
const apiKey = process.env.WX_WORK_EXTERNAL_API_KEY?.trim();
const toUser = process.env.WX_WORK_TO_USER?.trim() || "MaHui";

const tag = process.env.RELEASE_TAG || process.argv[2] || "unknown";
const repo = process.env.GITHUB_REPOSITORY || "lanhui100/pony-agent";
const releaseUrl = `https://github.com/${repo}/releases/tag/${tag}`;

if (!apiUrl || !apiKey) {
  console.log("⚠️ 未配置 WX_WORK_EXTERNAL_API_URL 或 WX_WORK_EXTERNAL_API_KEY，跳过企业微信通知。");
  process.exit(0);
}

// 尝试读取 release notes 摘要
let notesSummary = "";
const possibleNotesPaths = [
  path.resolve("target/release/bundle/nsis/release-notes.md"),
  path.resolve("release-notes.md"),
];

for (const p of possibleNotesPaths) {
  if (fs.existsSync(p)) {
    try {
      const raw = fs.readFileSync(p, "utf8").trim();
      if (raw) {
        // 限制摘要长度不超过 800 字符
        notesSummary = raw.length > 800 ? raw.slice(0, 800) + "\n..." : raw;
        break;
      }
    } catch {}
  }
}

const lines = [
  `✅ Pony Agent 桌面端发布成功`,
  `────────────────`,
  `🏷️ 版本: ${tag}`,
  `📦 仓库: ${repo}`,
  `🔗 发布地址: ${releaseUrl}`,
];

if (notesSummary) {
  lines.push(`\n📝 更新日志摘要:\n${notesSummary}`);
}

const content = lines.join("\n");
const payload = JSON.stringify({
  content,
  to_user: toUser,
});

const parsedUrl = new URL(apiUrl);
const isHttps = parsedUrl.protocol === "https:";
const transport = isHttps ? https : http;

const options = {
  hostname: parsedUrl.hostname,
  port: parsedUrl.port || (isHttps ? 443 : 80),
  path: parsedUrl.pathname + parsedUrl.search,
  method: "POST",
  headers: {
    "Content-Type": "application/json",
    "Content-Length": Buffer.byteLength(payload),
    "X-API-KEY": apiKey,
  },
  timeout: 15000,
};

console.log(`正在向 ${parsedUrl.origin}${parsedUrl.pathname} 发送企业微信通知 (${tag})...`);

const req = transport.request(options, (res) => {
  let resData = "";
  res.on("data", (chunk) => {
    resData += chunk;
  });

  res.on("end", () => {
    console.log(`通知响应状态码: ${res.statusCode}`);
    if (resData) console.log(`通知响应体: ${resData}`);

    if (res.statusCode && res.statusCode >= 200 && res.statusCode < 300) {
      try {
        const body = JSON.parse(resData);
        if (
          (body.success === false) ||
          (body.ok === false) ||
          (body.errcode && body.errcode !== 0) ||
          (body.status && body.status !== "success" && body.status !== "ok")
        ) {
          console.error("❌ 企业微信通知接口返回业务失败");
          process.exit(1);
        }
      } catch {
        // 非 JSON 格式但 HTTP 2xx 视为成功
      }
      console.log("✅ 企业微信通知发送成功。");
      process.exit(0);
    } else {
      console.error(`❌ 企业微信通知发送失败，HTTP 状态码: ${res.statusCode}`);
      process.exit(1);
    }
  });
});

req.on("error", (err) => {
  console.error(`❌ 请求企业微信通知接口异常: ${err.message}`);
  process.exit(1);
});

req.on("timeout", () => {
  req.destroy();
  console.error("❌ 请求企业微信通知接口超时 (15s)");
  process.exit(1);
});

req.write(payload);
req.end();
