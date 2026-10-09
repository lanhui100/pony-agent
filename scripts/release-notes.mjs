/**
 * Release Notes 生成器 —— 常规 ESM 正式实现（整体替换 polyglot 空桩）。
 * 契约基准：.dev-team/contract-matrix-release-notes.md 第 1/2/3/4/5 节。
 *
 * 导出 4 个命名函数：
 *  - parseCommit(subject)        解析单条 conventional commit subject
 *  - groupCommits(subjects)      分组（先排除 chore(release):，再按 type 落组）
 *  - buildReleaseNotes(input)    生成 markdown 全文（纯函数，字节级确定性）
 *  - main(argv)                  CLI 入口（唯一允许 I/O 的函数）
 *
 * 纯函数约束：parseCommit / groupCommits / buildReleaseNotes 无副作用、
 * 确定性、零网络调用；仅 main 执行 fs / git / stdout 侧 I/O。
 * 生成器只读本地 git 仓库，禁止任何网络调用。
 */

import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

/** 矩阵 §2 分组小节标题（冻结，含前缀空格与 emoji） */
const SECTION_HEADERS = Object.freeze({
  features: "## ✨ Features/新功能",
  bugFixes: "## 🐛 Bug Fixes/修复",
  performance: "## ⚡ Performance/性能",
  docs: "## 📝 Docs/文档",
  maintenance: "## 🧹 Maintenance/维护",
});

/** 输出顺序固定：features → bugFixes → performance → docs → maintenance */
const GROUP_ORDER = Object.freeze([
  "features",
  "bugFixes",
  "performance",
  "docs",
  "maintenance",
]);

/** 排除先行：chore(release): 前缀（bump/closure 提交，矩阵 §1.1） */
const RELEASE_BUMP_RE = /^chore\(release\):/;

/** git exec 统一选项（矩阵 §4：cwd、utf8、60s 超时） */
const GIT_EXEC_OPTS = Object.freeze({
  encoding: "utf8",
  timeout: 60_000,
  stdio: ["ignore", "pipe", "pipe"],
});

/**
 * 解析单条 conventional commit subject（矩阵 §1）。
 * 匹配 `type(scope): description`；type 大小写不敏感、识别后归一化为小写
 * （归一化后限定 [a-z]+）；scope 可空；description 两侧 trim。
 * 不匹配（无冒号 / 空串 / type 含非法字符 / breaking `!` 后缀等）→
 * { type: null, scope: null, description: <subject 原样 trim> }。
 */
export function parseCommit(subject) {
  const s = String(subject);
  const match = /^([A-Za-z]+)(?:\(([^)]*)\))?:\s*(.*)$/.exec(s);
  if (!match) {
    return { type: null, scope: null, description: s.trim() };
  }
  const [, type, scope, description] = match;
  const desc = description.trim();
  if (!desc) {
    return { type: null, scope: null, description: "" };
  }
  return {
    type: type.toLowerCase(),
    scope: scope === undefined ? null : scope,
    description: desc,
  };
}

/**
 * 分组（矩阵 §1/§2）。
 * 1) 排除先行：跳过 subject 匹配 /^chore\(release\):/ 的条目，不进入任何组；
 * 2) 其余按 parseCommit(subject).type 落组：feat→features、fix→bugFixes、
 *    perf→performance、docs→docs、其余全部→maintenance；
 * 3) 组内严格保留输入顺序，条目字段原样（subject/sha 不变）；
 * 4) 返回对象仅含五组键（键序固定），空组以空数组存在。
 */
export function groupCommits(subjects) {
  const groups = {
    features: [],
    bugFixes: [],
    performance: [],
    docs: [],
    maintenance: [],
  };
  for (const item of subjects) {
    if (RELEASE_BUMP_RE.test(item.subject)) continue;
    const { type } = parseCommit(item.subject);
    let key;
    if (type === "feat") key = "features";
    else if (type === "fix") key = "bugFixes";
    else if (type === "perf") key = "performance";
    else if (type === "docs") key = "docs";
    else key = "maintenance";
    groups[key].push(item);
  }
  return groups;
}

/**
 * subject markdown 转义（矩阵 §3.4，冻结执行顺序，逐字符）：
 * ① `\` → `\\`；② `` ` `` → `` \` ``；③ `#` → `\#`；
 * ④ 换行符 \n / \r / CRLF → 单个空格。
 */
function escapeSubject(subject) {
  let out = "";
  for (let i = 0; i < subject.length; i++) {
    const ch = subject[i];
    if (ch === "\r" && subject[i + 1] === "\n") {
      out += " ";
      i += 1;
      continue;
    }
    switch (ch) {
      case "\\":
        out += "\\\\";
        break;
      case "`":
        out += "\\`";
        break;
      case "#":
        out += "\\#";
        break;
      case "\n":
      case "\r":
        out += " ";
        break;
      default:
        out += ch;
    }
  }
  return out;
}

/**
 * 生成 release notes markdown 全文（矩阵 §3，纯函数）。
 * 确定性：同输入 → 字节级同输出。
 * 1) 标题：from 非空 → `# {repoName} release notes ({from} → {to})`；
 *    from=null → `# {repoName} release notes ({to} / 全量历史)`。
 * 2) 非空分组按固定顺序输出小节头 + `- {escapedSubject} ([{sha}])` 条目。
 * 3) 全部为空（含排除后无条目）→ 标题下直接输出占位行，不再输出小节头。
 * 4) 输出以恰好一个 \n 结尾。
 */
export function buildReleaseNotes({ repoName, subjects, from, to }) {
  const groups = groupCommits(subjects);
  const title =
    from !== null && from !== undefined
      ? `# ${repoName} release notes (${from} → ${to})`
      : `# ${repoName} release notes (${to} / 全量历史)`;

  const hasAny = GROUP_ORDER.some((key) => groups[key].length > 0);
  if (!hasAny) {
    return `${title}\n本版本无用户可见变更\n`;
  }

  const lines = [title];
  for (const key of GROUP_ORDER) {
    if (groups[key].length === 0) continue;
    lines.push(SECTION_HEADERS[key]);
    for (const item of groups[key]) {
      lines.push(`- ${escapeSubject(item.subject)} ([${item.sha}])`);
    }
  }
  return `${lines.join("\n")}\n`;
}

/** 解析 `vX.Y.Z` 为版本三元组；不匹配 → null */
function parseVersion(tag) {
  const m = /^v(\d+)\.(\d+)\.(\d+)$/.exec(tag);
  if (!m) return null;
  return [Number(m[1]), Number(m[2]), Number(m[3])];
}

/** 版本三元组比较：a < b → -1，相等 → 0，a > b → 1 */
function compareVersions(a, b) {
  for (let i = 0; i < 3; i++) {
    if (a[i] < b[i]) return -1;
    if (a[i] > b[i]) return 1;
  }
  return 0;
}

/** git exec 统一封装：失败抛带可诊断上下文的 Error（命令、repo-root、stderr 尾部） */
function execGit(repoRoot, gitArgs, what) {
  try {
    return execFileSync("git", gitArgs, { ...GIT_EXEC_OPTS, cwd: repoRoot });
  } catch (err) {
    const stderrTail = (err && err.stderr ? String(err.stderr) : "")
      .trim()
      .split("\n")
      .slice(-5)
      .join("\n");
    const message =
      `release-notes: git ${gitArgs.join(" ")} failed while trying to ${what}` +
      ` (repo-root: ${repoRoot})`;
    throw new Error(stderrTail ? `${message}\ngit stderr (tail):\n${stderrTail}` : message);
  }
}

/** 解析 `git log --format=%h%x00%s` 输出（NUL 分隔 hash 与 subject）→ {subject, sha}[] */
function parseLogOutput(output) {
  const subjects = [];
  if (!output) return subjects;
  for (const record of output.split("\n")) {
    if (!record) continue;
    const nul = record.indexOf("\0");
    if (nul === -1) continue;
    subjects.push({ sha: record.slice(0, nul), subject: record.slice(nul + 1) });
  }
  return subjects;
}

/** 探测严格小于 --to 的最大 vX.Y.Z tag（矩阵 §4 冻结算法）；不存在 → null */
function detectPrevTag(repoRoot, to) {
  const out = execGit(repoRoot, ["tag", "-l"], "list tags");
  const candidates = [];
  for (const line of out.split("\n")) {
    const tag = line.trim();
    if (!tag) continue;
    const v = parseVersion(tag);
    if (v) candidates.push({ tag, v });
  }
  candidates.sort((a, b) => compareVersions(a.v, b.v));
  const targetV = parseVersion(to);
  if (!targetV) return null;
  let prev = null;
  for (const c of candidates) {
    if (compareVersions(c.v, targetV) < 0) prev = c.tag;
  }
  return prev;
}

/** 解析 `--key value` 形式参数；未知参数忽略（可诊断优先）。 */
function parseArgs(argv) {
  const map = new Map();
  for (let i = 0; i < argv.length; i++) {
    const token = argv[i];
    if (!token.startsWith("--")) continue;
    const key = token.slice(2);
    if (i + 1 < argv.length && !argv[i + 1].startsWith("--")) {
      map.set(key, argv[i + 1]);
      i += 1;
    }
  }
  return map;
}

/**
 * CLI 入口（矩阵 §4，唯一允许 I/O 的函数）。
 * 用法：node scripts/release-notes.mjs --repo-root <dir> --to <tag>
 *       [--from <tag>] [--out <file>]
 * 成功返回 0；一切失败抛 Error（消息含可诊断上下文），未捕获 →
 * Node 默认 stderr 输出 + 非 0 退出码。
 */
export function main(argv) {
  const args = parseArgs(argv);

  const repoRoot = args.get("repo-root");
  const to = args.get("to");
  if (!repoRoot) throw new Error("release-notes: missing required --repo-root <dir>");
  if (!to) throw new Error("release-notes: missing required --to <tag>");
  const from = args.get("from") ?? null;
  const out = args.get("out") ?? null;

  // 1) repoName 来自 --repo-root/package.json 的 name（读取/解析失败 → 致命）
  const pkgPath = resolve(repoRoot, "package.json");
  let repoName;
  try {
    repoName = JSON.parse(readFileSync(pkgPath, "utf8")).name;
  } catch (err) {
    throw new Error(`release-notes: failed to read/parse ${pkgPath}: ${err.message}`);
  }
  if (typeof repoName !== "string" || repoName.length === 0) {
    throw new Error(`release-notes: package.json at ${pkgPath} has no usable "name" field`);
  }

  // 2) --to 必须可解析（git rev-parse --verify）
  execGit(repoRoot, ["rev-parse", "--verify", to], `verify --to tag ${to}`);

  // 3) --from 未提供 → 自动探测前一 tag；不存在 → from=null（全量回退）
  const resolvedFrom = from === null ? detectPrevTag(repoRoot, to) : from;

  // 4) 收集提交：区间 `from..to`；全量 `to`（自 --to 起全量历史）
  const logArgs = resolvedFrom
    ? ["log", "--format=%h%x00%s", `${resolvedFrom}..${to}`]
    : ["log", "--format=%h%x00%s", to];
  const logOut = execGit(repoRoot, logArgs, "collect commits");
  const subjects = parseLogOutput(logOut);

  // 5) 生成 + 输出（--out 写文件 UTF-8，否则 stdout）
  const notes = buildReleaseNotes({ repoName, subjects, from: resolvedFrom, to });
  if (out) {
    try {
      writeFileSync(out, notes, "utf8");
    } catch (err) {
      throw new Error(`release-notes: failed to write output file ${out}: ${err.message}`);
    }
  } else {
    process.stdout.write(notes);
  }
  return 0;
}

/* 直接以 node 运行本文件时执行 CLI；被测试 import 时不触发 */
if (process.argv[1]) {
  let invokedPath;
  try {
    invokedPath = fileURLToPath(process.argv[1]);
  } catch {
    invokedPath = resolve(process.argv[1]);
  }
  if (invokedPath === fileURLToPath(import.meta.url)) {
    process.exitCode = main(process.argv.slice(2));
  }
}
