/**
 * Release Notes 生成器 —— L2-T 功能契约测试（Test Agent 冻结，契约基准：
 * .dev-team/contract-matrix-release-notes.md 第 1/2/3/8 节）。
 *
 * 结构：
 * 1) L2-T 验收契约段（矩阵 §1/§2/§3）：parseCommit / groupCommits /
 *    buildReleaseNotes 的正式行为断言（绿相冻结；红相期临时 "RED PHASE 门禁"
 *    describe 段——矩阵 §8 空桩 globalThis 断言——已在绿相迁移时移除，红相证据
 *    见 .dev-team/red-phase-release-notes.log 与 git commit 4dafafa）。
 *
 * 契约解读说明（已提请协议方复核）：
 * - 矩阵 §1 `ReleaseNotesInput = { subjects, from, to }` 未含 repoName，但 §3
 *   标题格式要求 `{repoName}`（来自 package.json）。为保持 buildReleaseNotes
 *   纯函数（§1：无副作用、不写文件），本测试采用"repoName 由调用方显式传入输入
 *   对象"的解读（输入形如 { repoName, subjects, from, to }），与 §3 标题格式对齐。
 * - 矩阵 §1 "大小写不敏感匹配 type"：本测试按"识别后归一化为小写 type"解读
 *   （`FEAT(x): y` → type:"feat"），以保证 §2 分组映射（小写键）成立。
 */
import { beforeAll, describe, expect, it } from "vitest";
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, rmSync } from "node:fs";
import { dirname, join } from "node:path";
import { tmpdir } from "node:os";

/* ------------------------------------------------------------------ */
/* 冻结类型（与矩阵 §1 对齐）                                          */
/* ------------------------------------------------------------------ */

type CommitSubject = { subject: string; sha: string };
type ParseResult = { type: string | null; scope: string | null; description: string };
type GroupKey = "features" | "bugFixes" | "performance" | "docs" | "maintenance";
type GroupedCommits = Record<GroupKey, CommitSubject[]>;

interface ReleaseNotesInput {
  repoName: string;
  subjects: CommitSubject[];
  from: string | null;
  to: string;
}

interface ReleaseNotesModule {
  parseCommit: (subject: string) => ParseResult;
  groupCommits: (subjects: CommitSubject[]) => GroupedCommits;
  buildReleaseNotes: (input: ReleaseNotesInput) => string;
  main: (argv: string[]) => number;
}

/* ------------------------------------------------------------------ */
/* 共享夹具                                                           */
/* ------------------------------------------------------------------ */

/** 矩阵 §2 分组标题（冻结，含前缀空格与 emoji） */
const SECTION_HEADERS: Record<GroupKey, string> = {
  features: "## ✨ Features/新功能",
  bugFixes: "## 🐛 Bug Fixes/修复",
  performance: "## ⚡ Performance/性能",
  docs: "## 📝 Docs/文档",
  maintenance: "## 🧹 Maintenance/维护",
};

/** 含两条 chore(release) bump 排除项在内的完整提交序列（组内顺序保留基准） */
const FULL_SUBJECTS: CommitSubject[] = [
  { subject: "chore(release): bump version to 0.1.114", sha: "0000000" },
  { subject: "feat(ui): add dark mode", sha: "1111111" },
  { subject: "fix: handle null", sha: "2222222" },
  { subject: "perf(core): cache results", sha: "3333333" },
  { subject: "docs: update readme", sha: "4444444" },
  { subject: "refactor: rename module", sha: "5555555" },
  { subject: "Add untagged line", sha: "6666666" },
  { subject: "chore: misc", sha: "7777777" },
  { subject: "fix(ui): second fix keeps order", sha: "8888888" },
  { subject: "feat: also dark mode extra", sha: "9999999" },
  { subject: "chore(release): bump version to 0.1.113", sha: "aaaaaaa" },
];

/* ------------------------------------------------------------------ */
/* 模块装载（动态 import：红相期取 globalThis 桩；绿相期取 ESM 导出）  */
/* ------------------------------------------------------------------ */

let mod: ReleaseNotesModule;

beforeAll(async () => {
  mod = (await import("../scripts/release-notes.mjs")) as unknown as ReleaseNotesModule;
});

/* ================================================================== */
/* L2-T 验收契约段（矩阵 §1/§2/§3）                                    */
/* ================================================================== */

describe("parseCommit（矩阵 §1 解析规则）", () => {
  it("feat: x → {type:'feat', scope:null, description:'x'}", () => {
    expect(mod.parseCommit("feat: x")).toEqual({ type: "feat", scope: null, description: "x" });
  });

  it("fix(ui): handle null → {type:'fix', scope:'ui', description:'handle null'}", () => {
    expect(mod.parseCommit("fix(ui): handle null")).toEqual({
      type: "fix",
      scope: "ui",
      description: "handle null",
    });
  });

  it("description 两侧空白 trim", () => {
    expect(mod.parseCommit("feat:   spaced   ")).toEqual({
      type: "feat",
      scope: null,
      description: "spaced",
    });
  });

  it("无冒号（非 conventional）→ type:null，description 为原文 trim", () => {
    expect(mod.parseCommit("Add cool feature")).toEqual({
      type: null,
      scope: null,
      description: "Add cool feature",
    });
  });

  it("空串 → 视为不匹配 {type:null, scope:null, description:''}", () => {
    expect(mod.parseCommit("")).toEqual({ type: null, scope: null, description: "" });
  });

  it("type 含非法字符（`unknown-type`）→ 不匹配，description 原样 trim", () => {
    expect(mod.parseCommit("unknown-type: whatever")).toEqual({
      type: null,
      scope: null,
      description: "unknown-type: whatever",
    });
  });

  it("无 description（`feat:`）→ 不匹配 {type:null, scope:null, description:''}（L3-B 契约角落漂移闭环，矩阵 §1 无 description → 不匹配）", () => {
    expect(mod.parseCommit("feat:")).toEqual({ type: null, scope: null, description: "" });
  });

  it("breaking 后缀（feat!:) → 不匹配（Non-Goal §0，落入 maintenance 由 group 承接）", () => {
    expect(mod.parseCommit("feat!: break the api")).toEqual({
      type: null,
      scope: null,
      description: "feat!: break the api",
    });
  });

  it("type 大小写不敏感（冻结解读：归一化为小写）", () => {
    expect(mod.parseCommit("FEAT(core): uppercase type")).toEqual({
      type: "feat",
      scope: "core",
      description: "uppercase type",
    });
  });
});

describe("groupCommits（矩阵 §1 规则 / §2 分组映射）", () => {
  it("返回对象恰含五组键，键序固定 features→bugFixes→performance→docs→maintenance", () => {
    const groups = mod.groupCommits(FULL_SUBJECTS);
    expect(Object.keys(groups)).toEqual([
      "features",
      "bugFixes",
      "performance",
      "docs",
      "maintenance",
    ]);
  });

  it("chore(release): bump 提交被排除，不进入任何组", () => {
    const groups = mod.groupCommits(FULL_SUBJECTS);
    const all = Object.values(groups).flat().map((c) => c.subject);
    expect(all).not.toContain("chore(release): bump version to 0.1.114");
    expect(all).not.toContain("chore(release): bump version to 0.1.113");
    const total =
      Object.values(groups).reduce((n, list) => n + list.length, 0);
    expect(total).toBe(FULL_SUBJECTS.length - 2);
  });

  it("feat/fix/perf/docs 归入对应组，其余（refactor/无前缀/chore 非 release/test 等）入 maintenance", () => {
    const groups = mod.groupCommits(FULL_SUBJECTS);
    expect(groups.features.map((c) => c.subject)).toEqual([
      "feat(ui): add dark mode",
      "feat: also dark mode extra",
    ]);
    expect(groups.bugFixes.map((c) => c.subject)).toEqual([
      "fix: handle null",
      "fix(ui): second fix keeps order",
    ]);
    expect(groups.performance.map((c) => c.subject)).toEqual(["perf(core): cache results"]);
    expect(groups.docs.map((c) => c.subject)).toEqual(["docs: update readme"]);
    expect(groups.maintenance.map((c) => c.subject)).toEqual([
      "refactor: rename module",
      "Add untagged line",
      "chore: misc",
    ]);
  });

  it("组内严格保留输入顺序，且不改变条目内部字段（subject/sha 原样）", () => {
    const groups = mod.groupCommits(FULL_SUBJECTS);
    const expectedFixOrder = FULL_SUBJECTS.filter((c) => c.subject.startsWith("fix"));
    expect(groups.bugFixes).toEqual(expectedFixOrder);
    expect(groups.features[0]).toEqual({ subject: "feat(ui): add dark mode", sha: "1111111" });
  });

  it("空组以空数组存在（仅排除项输入 → 五组全空）", () => {
    const groups = mod.groupCommits([
      { subject: "chore(release): bump version to 0.1.114", sha: "abc1234" },
    ]);
    expect(Object.keys(groups)).toHaveLength(5);
    expect(Object.values(groups).every((list) => Array.isArray(list) && list.length === 0)).toBe(
      true
    );
  });

  it("确定性：同输入两次分组深度相等", () => {
    expect(mod.groupCommits(FULL_SUBJECTS)).toEqual(mod.groupCommits(FULL_SUBJECTS));
  });
});

describe("buildReleaseNotes（矩阵 §3 输出格式）", () => {
  const baseInput: ReleaseNotesInput = {
    repoName: "pony-agent",
    subjects: FULL_SUBJECTS,
    from: "v0.1.113",
    to: "v0.1.114",
  };

  it("完整输出：标题 + 非空小节（固定顺序）+ `- subject ([sha])` 条目 + 末尾 \\n", () => {
    const expected = [
      "# pony-agent release notes (v0.1.113 → v0.1.114)",
      SECTION_HEADERS.features,
      "- feat(ui): add dark mode ([1111111])",
      "- feat: also dark mode extra ([9999999])",
      SECTION_HEADERS.bugFixes,
      "- fix: handle null ([2222222])",
      "- fix(ui): second fix keeps order ([8888888])",
      SECTION_HEADERS.performance,
      "- perf(core): cache results ([3333333])",
      SECTION_HEADERS.docs,
      "- docs: update readme ([4444444])",
      SECTION_HEADERS.maintenance,
      "- refactor: rename module ([5555555])",
      "- Add untagged line ([6666666])",
      "- chore: misc ([7777777])",
      "",
    ].join("\n");
    expect(mod.buildReleaseNotes(baseInput)).toBe(expected);
  });

  it("区间标题格式：# {repoName} release notes ({from} → {to})", () => {
    const out = mod.buildReleaseNotes(baseInput);
    expect(out.startsWith("# pony-agent release notes (v0.1.113 → v0.1.114)\n")).toBe(true);
  });

  it("全量回退标题格式（from=null）：# {repoName} release notes ({to} / 全量历史)", () => {
    const out = mod.buildReleaseNotes({ ...baseInput, from: null });
    expect(out.startsWith("# pony-agent release notes (v0.1.114 / 全量历史)\n")).toBe(true);
  });

  it("空组不输出小节头", () => {
    const out = mod.buildReleaseNotes({
      repoName: "pony-agent",
      from: "v0.1.113",
      to: "v0.1.114",
      subjects: [
        { subject: "feat: only feature", sha: "aaaaaa1" },
        { subject: "fix: only fix", sha: "aaaaaa2" },
      ],
    });
    expect(out).toContain(SECTION_HEADERS.features);
    expect(out).toContain(SECTION_HEADERS.bugFixes);
    expect(out).not.toContain(SECTION_HEADERS.performance);
    expect(out).not.toContain(SECTION_HEADERS.docs);
    expect(out).not.toContain(SECTION_HEADERS.maintenance);
  });

  it("subjects 为空 → 标题下直接输出占位行（精确文案、无小节头）", () => {
    const out = mod.buildReleaseNotes({ ...baseInput, subjects: [], from: null });
    expect(out).toBe("# pony-agent release notes (v0.1.114 / 全量历史)\n本版本无用户可见变更\n");
  });

  it("排除后无任何条目（全为 chore(release) bump）→ 占位行", () => {
    const out = mod.buildReleaseNotes({
      ...baseInput,
      subjects: [{ subject: "chore(release): bump version to 0.1.114", sha: "abc1234" }],
    });
    expect(out).toBe("# pony-agent release notes (v0.1.113 → v0.1.114)\n本版本无用户可见变更\n");
  });

  it("输出以恰好一个换行符结尾（不出现 \\n\\n 结尾）", () => {
    const out = mod.buildReleaseNotes(baseInput);
    expect(out.endsWith("\n")).toBe(true);
    expect(out.endsWith("\n\n")).toBe(false);
  });

  it("subject 转义（冻结顺序 ①\\ → \\\\ ②` → \\` ③# → \\#；$ 不转义透传）", () => {
    // raw 实际字符: feat(x): use `code` #12 \\ $5 （两个反斜杠）
    const raw = "feat(x): use `code` #12 \\\\ $5";
    // expected 实际字符: feat(x): use \`code\` \#12 \\\\ $5 （四个反斜杠）
    const expected = "- feat(x): use \\`code\\` \\#12 \\\\\\\\ $5 ([abc1234])";
    const out = mod.buildReleaseNotes({
      repoName: "pony-agent",
      from: "v0.1.113",
      to: "v0.1.114",
      subjects: [{ subject: raw, sha: "abc1234" }],
    });
    expect(out).toBe(
      ["# pony-agent release notes (v0.1.113 → v0.1.114)", SECTION_HEADERS.features, expected, ""].join(
        "\n"
      )
    );
  });

  it("subject 内换行（LF / CRLF / CR）折叠为单个空格", () => {
    const out = mod.buildReleaseNotes({
      repoName: "pony-agent",
      from: "v0.1.113",
      to: "v0.1.114",
      subjects: [
        { subject: "fix: first line\nsecond line", sha: "aaaaaa1" },
        { subject: "fix: crlf line\r\nnext", sha: "aaaaaa2" },
        { subject: "fix: cr line\rnext", sha: "aaaaaa3" },
      ],
    });
    expect(out).toContain("- fix: first line second line ([aaaaaa1])");
    expect(out).toContain("- fix: crlf line next ([aaaaaa2])");
    expect(out).toContain("- fix: cr line next ([aaaaaa3])");
    // 输出中除末尾外不出现未转义换行（矩阵 §3.5）
    expect(out.slice(0, -1)).not.toContain("\n\n");
  });

  it("确定性：同输入两次输出字节级一致（无时间戳/随机）", () => {
    expect(mod.buildReleaseNotes(baseInput)).toBe(
      mod.buildReleaseNotes({
        repoName: "pony-agent",
        subjects: FULL_SUBJECTS.map((c) => ({ ...c })),
        from: "v0.1.113",
        to: "v0.1.114",
      })
    );
  });
});

/* ================================================================== */
/* main / CLI 黑盒（矩阵 §4；L3 视角 C 审查补测）                      */
/* 纯本地 git 调用（git log / git tag / git rev-parse，零网络），      */
/* 经 spawnSync("node", ["scripts/release-notes.mjs", ...]) 执行正式    */
/* 实现的可执行入口。依赖仓库真实 tag：v0.1.113 / v0.1.114 存在。      */
/* ================================================================== */

describe("main / CLI 黑盒（矩阵 §4，纯本地 git 零网络）", () => {
  const REPO_ROOT = process.cwd(); // vitest run 自仓库根启动（cwd === 仓库根）

  function runCli(args: string[]): { status: number | null; stdout: string; stderr: string } {
    const res = spawnSync("node", ["scripts/release-notes.mjs", ...args], {
      cwd: REPO_ROOT,
      encoding: "utf8",
      timeout: 60_000,
    });
    return { status: res.status, stdout: res.stdout ?? "", stderr: res.stderr ?? "" };
  }

  it("① 区间模式：--repo-root . --to v0.1.114 --from v0.1.113 → exit 0 且 stdout 含区间标题", () => {
    const res = runCli(["--repo-root", ".", "--to", "v0.1.114", "--from", "v0.1.113"]);
    expect(res.status).toBe(0);
    expect(res.stdout).toContain("# pony-agent release notes (v0.1.113 → v0.1.114)");
  });

  it("② 缺 --to（参数缺失）→ exit 非 0 且 stderr 可诊断（矩阵 §4 失败契约）", () => {
    const res = runCli(["--repo-root", "."]);
    expect(res.status).not.toBe(0);
    expect(res.stderr).toMatch(/missing required --to/);
  });

  it("③ 空区间（--from 与 --to 相同 tag，git log 无提交）→ exit 0 且输出占位行", () => {
    const res = runCli(["--repo-root", ".", "--to", "v0.1.114", "--from", "v0.1.114"]);
    expect(res.status).toBe(0);
    expect(res.stdout).toContain("本版本无用户可见变更");
  });

  it("④ --out 写文件：文件存在且非空（矩阵 §4 --out 契约）", () => {
    const outFile = join(tmpdir(), `rn-cli-out-${process.pid}-${Date.now()}.md`);
    try {
      const res = runCli([
        "--repo-root",
        ".",
        "--to",
        "v0.1.114",
        "--from",
        "v0.1.113",
        "--out",
        outFile,
      ]);
      expect(res.status).toBe(0);
      expect(existsSync(outFile)).toBe(true);
      expect(readFileSync(outFile, "utf8").trim().length).toBeGreaterThan(0);
    } finally {
      // 谁创建谁清理（test-expert：测试数据生命周期）
      rmSync(outFile, { force: true });
    }
  });
});