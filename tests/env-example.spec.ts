/**
 * wave-18b C6 契约测试：部署配置模板 `.env.example`
 *
 * 契约基线：`.dev-team/contract-matrix-wave18.md` wave18-r3 §6（§6.1 冻结要求 / §6.2 冻结断言）。
 * 权威证据源：`.dev-team/review-l3p-wave18.json` finding#2（`ls .env.example` EXIT=2）。
 *
 * 红相基线：仓库根不存在 `.env.example`，`PONYSENTRY_ENABLED` 默认翻转 false 后
 * 部署方在仓库内无任何线索得知须显式置 true 才能恢复上报。
 */
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { runChecks } from "../scripts/check-secrets.mjs";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const ENV_EXAMPLE_PATH = path.join(repoRoot, ".env.example");

/** §6.1 要求逐项记录的 5 个变量。 */
const REQUIRED_VARS = [
  "PONYSENTRY_INGEST_URL",
  "PONYSENTRY_CLIENT_TOKEN",
  "PONYSENTRY_ENABLED",
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

  it("C6-2 必须逐项记录 5 个变量名（各 ≥1 次）", () => {
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

  it("C6-5 必须写明 fail-closed 默认值（false）与恢复上报的最小配置（PONYSENTRY_ENABLED=true）", () => {
    // Arrange
    const content = readEnvExample();

    // Act / Assert
    expect(/\bfalse\b/.test(content)).toBe(true);
    expect(/(^|\n)\s*#?\s*PONYSENTRY_ENABLED\s*=\s*true/.test(content)).toBe(true);
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
