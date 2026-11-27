#!/usr/bin/env node
// 「非执行者测试证据」机械门（`backlog:B1795571368443`）。
//
// ## 为什么需要它
//
// dev-team SKILL 的强制派生条款要求 B 级任务派生 Protocol Agent / Test Agent + Executor，且
// test-agent 首步加载 test-expert 跑 L2-T/L2-AT 红相。但它只是**流程约束**：模型可以只派生
// executor，再用「执行者自跑 pytest/ruff exit 0」放行——单点自证。实证（`~/job_copilot`
// 2026-11-24 批次）就是这样绕过就绪门禁的，仓内没有任何东西能判红。
//
// ## 载体（唯一真源）
//
// 提交信息里的 trailer 行：
//
//     Test-Evidence: <producer> | <command> | exit=<code>
//
// - `<producer>` 是产出这条证据的角色/人名（`test-agent`、`reviewer`、`ci`…）。
// - 本门**不重新跑命令**（那是 CI 的活），只判「存在一条产出者 ≠ 代码作者的证据声明」。
// - 用 trailer 而不是另造 JSON 回执文件：提交信息本身随代码入库、无法在 review 后被悄悄改而
//   不留痕，且 `git log` 直接可读——不制造第二份真值。
//
// ## 判定（可机械查）
//
// 1. **范围**：`--base <sha>..--head <sha>` 内的提交（默认 `HEAD~1..HEAD`）。
//    `--base` 为**全零 sha** 时是 GitHub 的「无前序提交」哨兵（新分支首次 push 的
//    `event.before` 即为此值）⇒ 明确跳过并 exit 0，由同一提交的 pull_request 事件
//    以有效 `base.sha` 复核；只有「给了 ref 却解析不出来」才 fail-closed exit 2。
// 2. **触发**：范围内存在 dev-team 标记（trailer `Team-Run:` 或提交信息含 `dev-team`）时才判；
//    无标记 ⇒ 跳过并 exit 0（无可判定项，不打扰普通 PR——与 no-residue 门同一惯例）。
// 3. **判据**：范围内若改动了代码面（非纯文档/账本路径），必须存在至少一条 `Test-Evidence`，
//    且其 `producer` **不在**该范围的作者集合里。
// 4. **退出码**：0 通过/跳过；1 违例；2 无法判定（git 解析失败 / 参数非法）。
//
// ## 诚实条款
//
// producer 是**自述**，本门不保证它真由该角色跑过（伪造属 review 面）。本门能保证的是：
// 「要么有一条显式的非作者证据，要么红」——把「忘了找独立验证」从静默变成必须显式面对。
//
// 用法：
//   node scripts/check-independent-test-evidence.mjs [--base <sha>] [--head <sha>] [--json]
import { execFileSync } from "node:child_process";

/** 纯文档/账本路径：改了它们不需要测试证据（与 ci-scope 的 SAFE_PREFIXES 同口径）。 */
const DOC_ONLY_PREFIXES = ["docs/", "backlog/", "openspec/", ".agents/notes/", ".agents/skills/"];

/** dev-team 标记：命中即进入判定（否则跳过）。 */
const DEV_TEAM_MARKER = /(^|\n)\s*Team-Run:\s*\S|dev-team/i;

/** 证据 trailer：`Test-Evidence: <producer> | <command> | exit=<code>`。 */
const TEST_EVIDENCE_RE = /(^|\n)\s*Test-Evidence:\s*([^|\n]+?)\s*\|/g;

function parseArgs(argv) {
  const out = { base: "", head: "HEAD", json: false, help: false };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--base") out.base = argv[i + 1] ?? "";
    else if (arg === "--head") out.head = argv[i + 1] ?? "HEAD";
    else if (arg === "--json") out.json = true;
    else if (arg === "--help" || arg === "-h") out.help = true;
  }
  return out;
}

function git(args) {
  return execFileSync("git", args, { encoding: "utf8", stdio: ["pipe", "pipe", "pipe"] });
}

/** base 是全零 sha（GitHub 在「无前序提交」时的哨兵值，如新分支首次 push）。 */
const ZERO_SHA_RE = /^0+$/;

/** 解析 base：空/不可解析 ⇒ null（调用方据此判「无法判定」）。 */
function resolveCommit(ref) {
  if (!ref || ZERO_SHA_RE.test(ref)) return null;
  try {
    return git(["rev-parse", "--verify", "--quiet", `${ref}^{commit}`]).trim();
  } catch {
    return null;
  }
}

function isDocOnly(file) {
  if (DOC_ONLY_PREFIXES.some((prefix) => file.startsWith(prefix))) return true;
  // 根级 markdown 是纯文案；子目录未列入白名单的 markdown 保守判为代码面。
  return !file.includes("/") && file.endsWith(".md");
}

/** 收集 `base..head` 的提交：作者名、提交信息、改动文件。 */
function collectCommits(base, head) {
  const raw = git([
    "log",
    "--format=%H%x00%an%x00%B%x00==END==",
    `${base}..${head}`,
  ]);
  const commits = [];
  for (const chunk of raw.split("==END==")) {
    const text = chunk.replace(/^\n/, "");
    if (!text.trim()) continue;
    const [sha, author, ...rest] = text.split("\u0000");
    const message = rest.join("\u0000");
    let files = [];
    try {
      files = git(["show", "--pretty=format:", "--name-only", sha])
        .split("\n")
        .map((line) => line.trim())
        .filter(Boolean);
    } catch {
      files = [];
    }
    commits.push({ sha, author, message, files });
  }
  return commits;
}

/** 从提交信息里抽出全部证据产出者（去重、保序、trim）。 */
function evidenceProducers(message) {
  const producers = [];
  for (const match of message.matchAll(TEST_EVIDENCE_RE)) {
    const producer = match[2].trim();
    if (producer && !producers.includes(producer)) producers.push(producer);
  }
  return producers;
}

function evaluate(commits) {
  const authors = new Set(commits.map((commit) => commit.author));
  const touched = commits.flatMap((commit) => commit.files);
  const codeTouched = touched.filter((file) => !isDocOnly(file));
  const devTeamMarked = commits.some((commit) => DEV_TEAM_MARKER.test(commit.message));
  const producers = commits.flatMap((commit) => evidenceProducers(commit.message));
  const independent = producers.filter((producer) => !authors.has(producer));

  if (!devTeamMarked) {
    return {
      verdict: "skipped",
      reason: "no-dev-team-marker",
      authors: [...authors],
      producers,
      independent,
      code_files: codeTouched.length,
    };
  }
  if (codeTouched.length === 0) {
    return {
      verdict: "skipped",
      reason: "docs-only",
      authors: [...authors],
      producers,
      independent,
      code_files: 0,
    };
  }
  if (producers.length === 0) {
    return {
      verdict: "fail",
      reason: "missing-test-evidence",
      detail: "范围改动了代码面，但没有任何 `Test-Evidence:` trailer",
      authors: [...authors],
      producers,
      independent,
      code_files: codeTouched.length,
    };
  }
  if (independent.length === 0) {
    return {
      verdict: "fail",
      reason: "self-attested-only",
      detail: `证据产出者全部落在代码作者集合内（authors=${[...authors].join(",")}；producers=${producers.join(",")}）`,
      authors: [...authors],
      producers,
      independent,
      code_files: codeTouched.length,
    };
  }
  return {
    verdict: "pass",
    reason: "independent-evidence",
    authors: [...authors],
    producers,
    independent,
    code_files: codeTouched.length,
  };
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.help) {
    console.log(
      "用法: node scripts/check-independent-test-evidence.mjs [--base <sha>] [--head <sha>] [--json]",
    );
    console.log("载体: 提交信息 trailer `Test-Evidence: <producer> | <command> | exit=<code>`");
    console.log("退出码: 0 通过/跳过；1 违例；2 无法判定（git 解析失败或参数非法）");
    return 0;
  }

  let head;
  try {
    head = resolveCommit(args.head);
  } catch {
    head = null;
  }
  if (!head) {
    console.error(`[test-evidence] 无法解析 head: ${args.head}`);
    return 2;
  }

  const baseRef = args.base || `${args.head}~1`;
  // 全零 sha 是 GitHub 的「无前序提交」哨兵（新分支首次 push 时 `event.before` 即为此值），
  // 它表达的是**确定的事实**（这条分支上没有任何更早的提交可比），不是「解析失败」。
  // 旧实现把它与浅克隆/坏 ref 一并 fail-closed exit 2，于是**每个新分支的首次 push 都红一次**，
  // 而同一提交的 pull_request 事件用有效 base.sha 照常判定——判定责任没有缺口，只有噪声。
  // 故此处跳过并 exit 0，真正的「给了 ref 但解析不出来」仍走下方 exit 2。
  if (baseRef && ZERO_SHA_RE.test(baseRef)) {
    const report = { verdict: "skipped", reason: "no-base-sentinel", base: baseRef, head };
    console.log(
      args.json
        ? JSON.stringify(report, null, 2)
        : `[test-evidence] skipped（no-base-sentinel）：base 为全零哨兵（${baseRef}），本分支无前序提交可比；由 pull_request 事件以 base.sha 复核`,
    );
    return 0;
  }
  const base = resolveCommit(baseRef);
  if (!base) {
    // 给了 base 却解析不出来（浅克隆 / 坏 ref）⇒ 无法判定，不假装通过。
    const report = { verdict: "unknown", reason: "no-base", base: baseRef, head };
    console.log(args.json ? JSON.stringify(report, null, 2) : `[test-evidence] 跳过判定：无法解析 base ${baseRef}`);
    return 2;
  }

  let commits;
  try {
    commits = collectCommits(base, head);
  } catch (error) {
    console.error(`[test-evidence] git log 失败: ${error instanceof Error ? error.message : error}`);
    return 2;
  }

  const result = evaluate(commits);
  if (args.json) {
    console.log(JSON.stringify({ ...result, base, head }, null, 2));
  } else {
    const line = `[test-evidence] ${result.verdict}（${result.reason}）authors=${result.authors.join(",") || "-"} producers=${result.producers.join(",") || "-"} code_files=${result.code_files}`;
    console.log(line);
    if (result.detail) console.error(`[test-evidence] ${result.detail}`);
  }
  return result.verdict === "fail" ? 1 : 0;
}

process.exit(main());
