/**
 * [RED DEGRADED: STATIC-ONLY]
 *
 * release.yml 发布流程静态前置断言（OPS/Infra 红相，契约矩阵 §6）。
 * 读取 .github/workflows/release.yml 原文（fs.readFileSync，不做任何执行），
 * 红相期（workflow 仍为旧版硬编码）以下断言全部必然 FAIL：
 *   1) 旧版仍含硬编码 `--notes "Automated signed release ...`（现第 111 行）→ 断言 1 FAIL；
 *   2) 旧版不含 scripts/release-notes.mjs 调用与 --notes-file → 断言 2/3 FAIL；
 *   3) 旧版 latest.json 组装段 notes 硬编码空串 ""（现第 145 行）→ 断言 4 FAIL。
 * 绿相（Executor 按矩阵 §6 接线后）本文件断言转为全绿，无需修改。
 * L3 视角 C 审查修订（review-release-notes-cli.json）：① §6.3 断言改为匹配
 * 契约规范式 `(Get-Content <path> -Raw)`（-Raw 前有空格、后有 `)`）；② 增加
 * Generate 步骤参数完整性断言（--repo-root / --to / --out）；③ 增加 §6.4
 * 同一真源断言（--out / --notes-file / Get-Content 三处路径一致）；④ 增加
 * checkout fetch-depth: 0 全量历史前置条件断言。
 */
import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { join } from "node:path";

// vitest 在 jsdom 环境下会改写 import.meta.url（非 file: scheme），故用 process.cwd()
// （vitest run 从仓库根启动，cwd === 仓库根）解析 workflow 路径。
const workflowPath = join(process.cwd(), ".github", "workflows", "release.yml");
const workflow = readFileSync(workflowPath, "utf8");

describe("[RED DEGRADED: STATIC-ONLY] release.yml 发布流程接线（契约矩阵 §6，红相期必 FAIL）", () => {
  it("不再包含硬编码 `--notes \"Automated signed release` 字符串（矩阵 §6.2）", () => {
    expect(workflow).not.toContain('--notes "Automated signed release');
  });

  it("包含对 scripts/release-notes.mjs 的调用（矩阵 §6.1）", () => {
    expect(workflow).toMatch(/scripts\/release-notes\.mjs/);
  });

  it("Generate 步骤参数完整：--repo-root / --to / --out 齐备（矩阵 §6.1，L3-C 补强）", () => {
    // 匹配 generate 步骤 run 行：node scripts/release-notes.mjs --repo-root <dir> --to <tag> --out <file>
    expect(workflow).toMatch(
      /node scripts\/release-notes\.mjs[^\n]*--repo-root[^\n]*--to[^\n]*--out[^\n]*/
    );
  });

  it("gh release create 使用 --notes-file 参数传 notes 文件（矩阵 §6.2）", () => {
    expect(workflow).toContain("--notes-file");
  });

  it("latest.json 组装段的 notes 字段从生成文件全文读取（规范式 (Get-Content <path> -Raw)），而非空串字面量（矩阵 §6.3，L3-C 修订）", () => {
    // 契约规范式：$manifest.notes = (Get-Content <notesFile> -Raw)
    // （-Raw 前有空格、后有 `)`）；L3-C 审查：旧正则耦合贴附写法 `)-Raw)`。
    expect(workflow).toMatch(/\$manifest\.notes\s*=\s*\(Get-Content[^\n]* -Raw\)/);
    expect(workflow).not.toMatch(/notes\s*=\s*""/);
  });

  it("§6.4 同一真源：--out 路径、--notes-file 路径、latest.json Get-Content 路径三处一致（L3-C 补强）", () => {
    const outPath =
      workflow.match(/release-notes\.mjs[^\n]*--out\s+"([^"]+)"/)?.[1] ?? "";
    const notesFilePath = workflow.match(/--notes-file\s+"([^"]+)"/)?.[1] ?? "";
    const contentPath =
      workflow.match(/\$manifest\.notes\s*=\s*\(Get-Content\s+"([^"]+)"\s*-Raw\)/)?.[1] ?? "";
    expect(outPath).not.toBe("");
    expect(notesFilePath).not.toBe("");
    expect(contentPath).not.toBe("");
    expect(outPath).toBe(notesFilePath);
    expect(notesFilePath).toBe(contentPath);
  });

  it("checkout 步骤配置 fetch-depth: 0（全量历史前置条件，L3-C 补强）", () => {
    const lines = workflow.split("\n");
    const checkoutIdx = lines.findIndex((l) => /uses:\s*actions\/checkout@v4/.test(l));
    expect(checkoutIdx).toBeGreaterThanOrEqual(0);
    // checkout 步骤的 with 块（其后 ~15 行内）须含 fetch-depth: 0
    const withBlock = lines.slice(checkoutIdx, checkoutIdx + 15);
    expect(withBlock.some((l) => /^\s*fetch-depth:\s*0\s*$/.test(l))).toBe(true);
  });
});