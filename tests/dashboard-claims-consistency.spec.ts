/**
 * wave-18c R7 契约测试：`management/task-system/00_DASHBOARD.md` 中关于
 * `MAX_SUBAGENT_DEPTH` 的表述必须与真实 `git grep` 命中集合**逐项一致**。
 *
 * 权威证据源：`.dev-team/review-wave18b-contract.json` finding#2（第二次假断言）。
 *
 * 红相基线：该断言行把「全仓零命中」改成**代码域**限定（该分句为真），但新增的存在性分句
 * 「仅见于任务卡 AC 文案 `03_TASKS/PA-110-...md:26`」在全仓口径下为假 ——
 * `git grep -n MAX_SUBAGENT_DEPTH` 实测 8 命中（6 处在 `.dev-team/` 审查档案、
 * 1 处为该断言行自身、1 处为任务卡），故该行仍不自洽。
 *
 * 判据（与 r3 §9.2 R7「改后重跑同一 grep，命中数须与文本陈述一致」同构）：
 * 把该行声明的命中范围解析为「已解释集合」，断言**全部真实命中都被解释**。
 * 解释规则（显式登记，避免启发式暗箱）：
 *   1. 行内反引号/裸写给出的具体 `path:line` 声明；
 *   2. 行内提及 `.dev-team` ⇒ 解释 `.dev-team/**` 下全部命中；
 *   3. 行内提及「自身/本行」⇒ 解释断言行所在行；
 *   4. 行内提及 `03_TASKS` 或 `PA-110` ⇒ 解释 `management/task-system/03_TASKS/**` 下命中。
 * 当前行只满足 1/4（未声明 2/3），故 6 处 `.dev-team/` 命中与该行自命中未被解释 ⇒ 红相成立。
 */
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const DASHBOARD_REL = "management/task-system/00_DASHBOARD.md";
const IDENTIFIER = "MAX_SUBAGENT_DEPTH";
const TASK_CARD_PREFIX = "management/task-system/03_TASKS/";

/** `git grep -n <IDENTIFIER> [-- pathspec...]`，归一化为 `path:lineno` 位置集合。 */
function gitGrepLocations(pathspecs: string[] = []): string[] {
  const args = ["grep", "-n", IDENTIFIER];
  if (pathspecs.length > 0) {
    args.push("--", ...pathspecs);
  }
  let stdout = "";
  try {
    stdout = execFileSync("git", args, { cwd: repoRoot, encoding: "utf8" });
  } catch {
    return []; // git grep 无命中 ⇒ exit 1
  }
  return stdout
    .split("\n")
    .filter((line) => line.length > 0)
    .map((line) => {
      const matched = line.match(/^(.*?):(\d+):/);
      return matched ? `${matched[1]}:${matched[2]}` : line;
    });
}

function dashboardClaimLine(): { lineNumber: number; text: string } {
  const content = fs.readFileSync(path.join(repoRoot, DASHBOARD_REL), "utf8");
  const lines = content.split(/\r?\n/);
  const index = lines.findIndex((line) => line.includes(IDENTIFIER));
  expect(
    index,
    `${DASHBOARD_REL} 必须存在包含 ${IDENTIFIER} 的事实断言行`
  ).toBeGreaterThanOrEqual(0);
  return { lineNumber: index + 1, text: lines[index] };
}

describe("wave-18c R7: 看板 MAX_SUBAGENT_DEPTH 表述与真实 grep 逐项一致", () => {
  it("R7-1 代码域分句必须为真：`git grep -n MAX_SUBAGENT_DEPTH -- '*.rs' '*.ts'` 零命中", () => {
    // Arrange
    const { text } = dashboardClaimLine();

    // Assert：该行确实作出代码域零命中断言，且该断言与真实 grep 一致
    expect(text.includes("--include=*.rs")).toBe(true);
    expect(gitGrepLocations(["*.rs", "*.ts"])).toEqual([]);
  });

  it("R7-2 排他性分句必须为真：全部真实命中都必须被该行的命中范围声明所解释", () => {
    // Arrange
    const { lineNumber, text } = dashboardClaimLine();
    const allHits = gitGrepLocations();
    const selfLocation = `${DASHBOARD_REL}:${lineNumber}`;

    // 1) 解析行内声明的具体位置（反引号 `path:line` 与裸写 path.ext:line）
    const declaredRaw = new Set<string>();
    for (const matched of text.matchAll(/`([^`\s]+?:\d+)`/g)) {
      declaredRaw.add(matched[1]);
    }
    for (const matched of text.matchAll(/([\w./-]+\.(?:md|rs|ts|json|mjs)):(\d+)/g)) {
      declaredRaw.add(`${matched[1]}:${matched[2]}`);
    }
    // 看板内引用的路径可能相对 `management/task-system/` 书写 ⇒ 两种解析都纳入
    const dashboardDir = path.posix.dirname(DASHBOARD_REL);
    const declared = new Set<string>();
    for (const raw of declaredRaw) {
      declared.add(raw);
      declared.add(path.posix.normalize(path.posix.join(dashboardDir, raw)));
    }

    // 2) 解析行内声明的排除类别
    const declaresDevTeam = text.includes(".dev-team");
    const declaresSelf = /自身|本行/.test(text);
    const declaresTaskCards = text.includes("03_TASKS") || text.includes("PA-110");

    // 3) 组装「已解释集合」
    const explained = new Set<string>(declared);
    if (declaresDevTeam) {
      for (const hit of allHits) {
        if (hit.startsWith(".dev-team/")) explained.add(hit);
      }
    }
    if (declaresSelf) explained.add(selfLocation);
    if (declaresTaskCards) {
      for (const hit of allHits) {
        if (hit.startsWith(TASK_CARD_PREFIX)) explained.add(hit);
      }
    }

    // 4) 断言必须落在真实命中上（禁止空口声明导致空转）
    const grounded = [...declared].filter((location) => allHits.includes(location));
    expect(grounded.length).toBeGreaterThanOrEqual(1);

    // 5) 未被解释的真实命中必须为空 ⇒「仅见于/命中范围」与真实 grep 逐项一致
    const unexplained = allHits.filter((hit) => !explained.has(hit));
    expect(unexplained).toEqual([]);
  });

  it("R7-3 防回退：行内每一处「零命中」声明都必须带代码域限定（不得退回全仓口径假断言）", () => {
    // Arrange
    const { text } = dashboardClaimLine();
    const codeScopeQualifier = /(--include=\*\.rs|--include=\*\.ts|\*\*代码\*\*|代码域)/;

    // Act：逐处检查「零命中」的限定语境（其前 80 字符内必须出现代码域限定词）
    const unqualified: number[] = [];
    let cursor = text.indexOf("零命中");
    while (cursor >= 0) {
      const context = text.slice(Math.max(0, cursor - 80), cursor);
      if (!codeScopeQualifier.test(context)) {
        unqualified.push(cursor);
      }
      cursor = text.indexOf("零命中", cursor + 1);
    }

    // Assert：不得存在未限定作用域的「零命中」声明
    expect(text.includes("零命中")).toBe(true);
    expect(unqualified).toEqual([]);
  });

  it("R7-4（r4 §9.2 判据 ②）：排除 `.dev-team/` 后的命中必须恰好 2 处 —— 断言行自身 + PA-110 任务卡 AC 文案", () => {
    // Arrange
    const { lineNumber } = dashboardClaimLine();
    const allHits = gitGrepLocations();

    // Act
    const nonDevTeam = allHits.filter((hit) => !hit.startsWith(".dev-team/"));

    // Assert：集合口径（不依赖固定总数，命中数随引用该标识符的新文档变化）
    expect(nonDevTeam.length).toBe(2);
    expect(nonDevTeam).toContain(`${DASHBOARD_REL}:${lineNumber}`);
    expect(
      nonDevTeam.some((hit) => hit.startsWith(TASK_CARD_PREFIX) && hit.includes("PA-110"))
    ).toBe(true);
  });
});
