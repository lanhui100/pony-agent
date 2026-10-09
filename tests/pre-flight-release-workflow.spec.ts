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

  it("gh release create 使用 --notes-file 参数传 notes 文件（矩阵 §6.2）", () => {
    expect(workflow).toContain("--notes-file");
  });

  it("latest.json 组装段的 notes 字段从生成文件全文读取，而非空串字面量（矩阵 §6.3）", () => {
    expect(workflow).toMatch(/\$manifest\.notes\s*=\s*\(Get-Content[^)]*\)-Raw\)/);
    expect(workflow).not.toMatch(/notes\s*=\s*""/);
  });
});