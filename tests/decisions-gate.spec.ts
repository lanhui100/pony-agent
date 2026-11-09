import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { describe, expect, it } from "vitest";

import { checkDecisions, fixIndex, insertRows, parseIndex } from "../scripts/check-decisions.mjs";

/**
 * 决策记录门禁（ADR 0109）。这组用例守的是治理事实本身：
 * 「目录即状态」「索引完整」「编号唯一」在 2026-11-09 之前只有 pwsh 版实现，
 * Linux 开发机上整段门禁不执行，于是 ADR 0107（proposed 留在主目录、且没进索引）
 * 与 22 篇索引缺行都能静静存在。
 */

const SCRIPT = path.resolve("scripts/check-decisions.mjs");
const README_BODY = `# 决策索引\n\n## 状态枚举\n\n- \`Status: proposed\`\n\n## 索引\n\n### 活跃\n\n### 提案中\n\n### 已接替\n\n### 已拒绝\n`;

function makeFixture(
  files: Array<{ rel: string; status?: string; title?: string }>,
  indexRows: Array<{ line: string; section?: string }> = [],
) {
  const dir = mkdtempSync(path.join(tmpdir(), "decisions-gate-"));
  for (const file of files) {
    const abs = path.join(dir, file.rel);
    mkdirSync(path.dirname(abs), { recursive: true });
    writeFileSync(
      abs,
      `# ${path.basename(file.rel).slice(0, 4)} ${file.title ?? "标题"}\n\nStatus: ${file.status ?? "implemented"}\n\n正文\n`,
      "utf-8",
    );
  }
  let readme = README_BODY;
  for (const row of indexRows) {
    const section = row.section ?? "活跃";
    const anchor = `### ${section}\n`;
    readme = readme.replace(anchor, `${anchor}\n${row.line}\n`);
  }
  writeFileSync(path.join(dir, "README.md"), readme, "utf-8");
  return dir;
}

function runCli(args: string[]) {
  try {
    const stdout = execFileSync(process.execPath, [SCRIPT, ...args], { encoding: "utf-8", stdio: "pipe" });
    return { code: 0, stdout, stderr: "" };
  } catch (err) {
    const e = err as { status?: number; stdout?: string; stderr?: string };
    return { code: e.status ?? -1, stdout: e.stdout ?? "", stderr: e.stderr ?? "" };
  }
}

describe("决策记录门禁 check-decisions", () => {
  it("真实仓：除编号唯一性外无其它违规（状态/目录/索引三项闭合）", () => {
    const verdict = checkDecisions("docs/decisions");
    expect(verdict.checked).toBeGreaterThan(100);
    expect(verdict.violations.filter((v) => !v.startsWith("R6"))).toEqual([]);
  });

  it("R1：缺 Status 行", () => {
    const dir = mkdtempSync(path.join(tmpdir(), "decisions-r1-"));
    writeFileSync(path.join(dir, "0001-a.md"), "# 0001 A\n\n正文\n", "utf-8");
    writeFileSync(path.join(dir, "README.md"), README_BODY, "utf-8");
    expect(checkDecisions(dir).violations.some((v) => v.startsWith("R1"))).toBe(true);
    rmSync(dir, { recursive: true, force: true });
  });

  it("R2：proposed 落在主目录 = 目录与状态不一致", () => {
    const dir = makeFixture([{ rel: "0001-a.md", status: "proposed" }], [{ line: "- [0001 A](0001-a.md)" }]);
    const violations = checkDecisions(dir).violations;
    expect(violations.some((v) => v.startsWith("R2") && v.includes("0001-a.md"))).toBe(true);
    rmSync(dir, { recursive: true, force: true });
  });

  it("R2：子目录状态必须与目录名一致", () => {
    const dir = makeFixture([{ rel: "proposed/0001-a.md", status: "superseded" }]);
    expect(checkDecisions(dir).violations.some((v) => v.startsWith("R2"))).toBe(true);
    rmSync(dir, { recursive: true, force: true });
  });

  it("R3：superseded by 指向不存在的编号", () => {
    const dir = makeFixture([
      { rel: "superseded/0002-b.md", status: "superseded by 9999" },
      { rel: "0001-a.md" },
    ]);
    expect(checkDecisions(dir).violations.some((v) => v.startsWith("R3") && v.includes("9999"))).toBe(true);
    rmSync(dir, { recursive: true, force: true });
  });

  it("R4：索引缺行 / 分节错误", () => {
    const missing = makeFixture([{ rel: "0001-a.md" }]);
    expect(checkDecisions(missing).violations.some((v) => v.startsWith("R4") && v.includes("索引缺行"))).toBe(true);
    rmSync(missing, { recursive: true, force: true });

    const wrong = makeFixture([{ rel: "proposed/0001-a.md", status: "proposed" }], [
      { line: "- [0001 A](proposed/0001-a.md)", section: "活跃" },
    ]);
    expect(checkDecisions(wrong).violations.some((v) => v.includes("索引分节错误"))).toBe(true);
    rmSync(wrong, { recursive: true, force: true });
  });

  it("R5：索引悬挂链接", () => {
    const dir = makeFixture([{ rel: "0001-a.md" }], [
      { line: "- [0001 A](0001-a.md) —— 见 [0002](0002-ghost.md)" },
    ]);
    expect(checkDecisions(dir).violations.some((v) => v.startsWith("R5") && v.includes("0002-ghost.md"))).toBe(true);
    rmSync(dir, { recursive: true, force: true });
  });

  it("R6：同一编号不得对应两个文件", () => {
    const dir = makeFixture([
      { rel: "0001-a.md", title: "甲" },
      { rel: "0001-b.md", title: "乙" },
    ]);
    expect(checkDecisions(dir).violations.some((v) => v.startsWith("R6") && v.includes("0001"))).toBe(true);
    rmSync(dir, { recursive: true, force: true });
  });

  it("R7：同一篇 ADR 不允许两行主体行", () => {
    const dir = makeFixture([{ rel: "0001-a.md" }], [
      { line: "- [0001 A](0001-a.md)" },
      { line: "- [0001 A 副本](0001-a.md)" },
    ]);
    expect(checkDecisions(dir).violations.some((v) => v.startsWith("R7"))).toBe(true);
    rmSync(dir, { recursive: true, force: true });
  });

  it("修订互链不算登记：只有别人行尾引用它 = 仍然索引缺行", () => {
    const dir = makeFixture([{ rel: "0001-a.md" }, { rel: "0002-b.md" }], [
      { line: "- [0001 A](0001-a.md) —— 已被 [0002](0002-b.md) 修订" },
    ]);
    const violations = checkDecisions(dir).violations;
    expect(violations.some((v) => v.startsWith("R4") && v.includes("0002-b.md"))).toBe(true);
    rmSync(dir, { recursive: true, force: true });
  });

  it("fixIndex：补齐缺行后归零，且不代写摘要", () => {
    const dir = makeFixture([{ rel: "0001-a.md", title: "只做标题搬运" }, { rel: "proposed/0002-b.md", status: "proposed", title: "提案" }]);
    expect(fixIndex(dir)).toEqual(["0001", "0002"]);
    expect(checkDecisions(dir).violations).toEqual([]);
    const readme = readFileSync(path.join(dir, "README.md"), "utf-8");
    expect(readme).toContain("- [0001 只做标题搬运](0001-a.md)");
    expect(readme).toContain("- [0002 提案](proposed/0002-b.md)");
    rmSync(dir, { recursive: true, force: true });
  });

  it("fixIndex 幂等：再跑一次不补任何行", () => {
    const dir = makeFixture([{ rel: "0001-a.md" }]);
    fixIndex(dir);
    expect(fixIndex(dir)).toEqual([]);
    rmSync(dir, { recursive: true, force: true });
  });

  it("排序键取行主体编号：带修订互链的行不会把新行插错位", () => {
    const text = `## 索引\n\n### 活跃\n\n- [0098 母版](0098-a.md) —— 已被 [0105](0105-b.md) 修订\n- [0105 缩放](0105-b.md)\n`;
    const filled = insertRows(text, [{ id: "0099", rel: "0099-c.md", status: "implemented", title: "事件源" }]);
    const order = parseIndex(filled).map((r) => r.links[0]);
    expect(order).toEqual(["0098-a.md", "0099-c.md", "0105-b.md"]);
  });

  it("CLI 退出码：0 通过 / 1 违规 / 2 用法错误", () => {
    const bad = makeFixture([{ rel: "0001-a.md", status: "proposed" }]);
    expect(runCli(["--decisions", bad]).code).toBe(1);
    const good = makeFixture([{ rel: "0001-a.md" }], [{ line: "- [0001 A](0001-a.md)" }]);
    expect(runCli(["--decisions", good]).code).toBe(0);
    expect(runCli(["--nope"]).code).toBe(2);
    expect(runCli(["--decisions", path.join(good, "missing")]).code).toBe(2);
    rmSync(bad, { recursive: true, force: true });
    rmSync(good, { recursive: true, force: true });
  });
});
