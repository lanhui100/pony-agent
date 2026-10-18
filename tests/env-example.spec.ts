/**
 * wave-18b C6 契约测试：部署配置模板 `.env.example`
 *
 * 契约基线：`.dev-team/contract-matrix-wave18.md` wave18-r3 §6（§6.1 冻结要求 / §6.2 冻结断言）。
 * 权威证据源：`.dev-team/review-l3p-wave18.json` finding#2（`ls .env.example` EXIT=2）。
 *
 * 红相基线：仓库根不存在 `.env.example`，`PONYSENTRY_ENABLED` 默认翻转 false 后
 * 部署方在仓库内无任何线索得知须显式置 true 才能恢复上报。
 *
 * wave-18c 契约演进（ADR 0037）：`PONYSENTRY_ENABLED` 开关变量**已废除** —— 上报门禁改由
 * “是否配置 client_token”承担（有 token 才有 egress），桌面发布产物的凭证经 CI 以
 * `PONYSENTRY_BUILD_CLIENT_TOKEN` 在**构建期**烧录（用户机器无需任何环境变量）。
 * 因此 C6-2 的变量清单与 C6-5 的断言同步迁移到 wave-18c 契约，模板本身不得再出现可被
 * 部署方照抄的 `PONYSENTRY_ENABLED=` 赋值。
 */
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { runChecks } from "../scripts/check-secrets.mjs";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const ENV_EXAMPLE_PATH = path.join(repoRoot, ".env.example");

/** §6.1 要求逐项记录的变量（wave-18c：`PONYSENTRY_ENABLED` 已废除，替换为构建期烧录通道）。 */
const REQUIRED_VARS = [
  "PONYSENTRY_INGEST_URL",
  "PONYSENTRY_CLIENT_TOKEN",
  "PONYSENTRY_BUILD_CLIENT_TOKEN",
  "VITE_PONYSENTRY_INGEST_URL",
  "VITE_PONYSENTRY_CLIENT_TOKEN",
];

/** 返回源码中“裸 64-hex 连续段”所在行号（只回显行号，不回显内容）。 */
function bare64HexLines(source: string): number[] {
  const hits: number[] = [];
  source.split(/\r?\n/).forEach((line, index) => {
    if (/(?<![0-9a-fA-F])[0-9a-fA-F]{64}(?![0-9a-fA-F])/.test(line)) {
      hits.push(index + 1);
    }
  });
  return hits;
}

function readEnvExample(): string {
  expect(fs.existsSync(ENV_EXAMPLE_PATH), "仓库根必须存在 .env.example").toBe(true);
  return fs.readFileSync(ENV_EXAMPLE_PATH, "utf8");
}

describe("wave-18b C6: .env.example 部署配置模板", () => {
  it("C6-1 必须存在且被 git 跟踪", () => {
    // Arrange / Act
    const exists = fs.existsSync(ENV_EXAMPLE_PATH);
    let tracked = false;
    try {
      execFileSync("git", ["ls-files", "--error-unmatch", ".env.example"], {
        cwd: repoRoot,
        stdio: "pipe",
      });
      tracked = true;
    } catch {
      tracked = false;
    }

    // Assert
    expect(exists).toBe(true);
    expect(tracked).toBe(true);
  });

  it("C6-2（wave-18c）必须逐项记录 5 个变量名（各 ≥1 次）", () => {
    // Arrange
    const content = readEnvExample();

    // Act
    const missing = REQUIRED_VARS.filter((name) => !content.includes(name));

    // Assert
    expect(missing).toEqual([]);
  });

  it("C6-3 模板内容必须零 64-hex 字面量（与 C4 规则交互硬约束）", () => {
    // Arrange
    const content = readEnvExample();

    // Act
    const offenders = bare64HexLines(content);

    // Assert
    expect(offenders).toEqual([]);
  });

  it("C6-4 必须写明自检方式 `node scripts/check-secrets.mjs --all`", () => {
    // Arrange
    const content = readEnvExample();

    // Act / Assert
    expect(content.includes("node scripts/check-secrets.mjs --all")).toBe(true);
  });

  it("C6-5（wave-18c）必须写明：无独立开关变量、fail-closed 由 token 缺失承担、恢复上报最小配置与桌面构建期烧录通道", () => {
    // Arrange
    const content = readEnvExample();

    // Assert 1：开关变量已废除 —— 模板不得再出现任何可被部署方照抄的赋值（含注释示例）
    expect(/^[^\n]*\bPONYSENTRY_ENABLED\s*=/m.test(content)).toBe(false);

    // Assert 2：未配置即零 egress 的设计原则仍在模板中显式声明
    expect(content.includes("fail-closed")).toBe(true);

    // Assert 3：恢复上报的最小配置 = 仅需 token（有 token 即启用，无第二开关）
    expect(/PONYSENTRY_CLIENT_TOKEN=<real-token>/.test(content)).toBe(true);

    // Assert 4：桌面发布产物的凭证通道（CI 构建期烧录，用户机器无需环境变量）
    expect(content.includes("PONYSENTRY_BUILD_CLIENT_TOKEN")).toBe(true);
  });

  it("C6-6 `.env.example` 本身不得触发 secret 门禁（全仓扫描中零违规）", () => {
    // Arrange / Act
    const violations = runChecks({ mode: "all" });
    const offenders = violations
      .filter((item: { filename: string }) => item.filename.endsWith(".env.example"))
      .map(
        (item: { filename: string; line?: number; rule: string }) =>
          `${item.filename}:${item.line ?? "-"} [${item.rule}]`
      );

    // Assert
    expect(offenders).toEqual([]);
  });
});
