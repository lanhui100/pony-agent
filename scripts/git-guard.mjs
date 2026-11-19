#!/usr/bin/env node
/**
 * git-guard.mjs —— 跨平台本地 git 门禁（替代原 scripts/git-guard.ps1）
 *
 * 为什么存在：原 .ps1 实现只在装有 pwsh 的机器上生效，Linux 工作区里
 * `if command -v pwsh` 会静默跳过整段暂存卫生检查（见
 * docs/audit/engineering-quality-audit-2026-11-01.md:122）。本文件用 Node
 * 单实现覆盖三平台，消除「门禁在刀背上的」不对称。
 *
 * 用法（由 .githooks/* 调用，不手工执行）：
 *   node scripts/git-guard.mjs pre-commit
 *   node scripts/git-guard.mjs pre-push
 *   node scripts/git-guard.mjs commit-msg <msgFile>
 *
 * 退出码：0 = 放行；1 = 拦截（fail-closed）。
 * 绕过：git commit/push --no-verify（必须在提交信息里说明理由）。
 */
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

const MODE = process.argv[2] || "pre-commit";
const PROTECTED_BRANCH = "main";

/** 受保护分支的本地推送逃生门：CI/灾备场景显式设置才放行 main 直推。 */
const ALLOW_PROTECTED_PUSH_ENV = "PONY_GUARD_ALLOW_MAIN_PUSH";

const BLOCKED_PATH_PATTERNS = [
  /^src-tauri\/target(?:\/|$)/,
  /^src-tauri\/target-codex-[^/]+(?:\/|$)/,
  /^src-tauri\/target-test[^/]*(?:\/|$)/,
  /^src-tauri\/target-check-tests[^/]*(?:\/|$)/,
  /^target(?:\/|$)/,
  /^target-codex-[^/]+(?:\/|$)/,
  /^target-check[^/]*(?:\/|$)/,
  /^target-test[^/]*(?:\/|$)/,
  /^coverage(?:\/|$)/,
  /^dist(?:\/|$)/,
  /^outputs(?:\/|$)/,
  /^playwright-report(?:\/|$)/,
  /^test-results(?:\/|$)/,
  /^tmp(?:\/|$)/,
  /^attachments(?:\/|$)/,
  /^sessions(?:\/|$)/
];

const BLOCKED_EXTENSIONS = new Set([".rlib", ".lib", ".a", ".pdb"]);
const SIZE_LIMIT_BYTES = 20 * 1024 * 1024;
const TEXT_EXTENSIONS = new Set([
  ".ps1", ".mjs", ".js", ".ts", ".tsx", ".vue", ".md", ".json",
  ".yml", ".yaml", ".toml", ".rs", ".css", ".scss", ".html", ".sh"
]);

/** 分支名前缀白名单：并发会话靠前缀一眼分清归属与生命周期。 */
const BRANCH_PREFIXES = [
  "feat", "fix", "chore", "docs", "refactor", "test", "perf",
  "build", "ci", "revert", "release", "integrate", "backup", "exp"
];

/** Conventional Commits 允许的类型（含 fixup/squash 等 git 自动生成前缀）。 */
const COMMIT_TYPES = [
  "feat", "fix", "chore", "docs", "refactor", "test", "perf",
  "build", "ci", "revert", "style"
];

function git(args, opts = {}) {
  return execFileSync("git", args, {
    encoding: "utf8",
    stdio: ["pipe", "pipe", "pipe"],
    ...opts
  });
}

function repoRoot() {
  try {
    return git(["rev-parse", "--show-toplevel"]).trim();
  } catch {
    return process.cwd();
  }
}

const ROOT = repoRoot();

function toPosix(p) {
  return p.replace(/\\/g, "/");
}

function stagedPaths() {
  try {
    return git(["diff", "--cached", "--name-only", "--diff-filter=AM"])
      .split("\n")
      .map((s) => s.trim())
      .filter(Boolean);
  } catch {
    return [];
  }
}

function hasExactSingleTrailingNewline(absPath) {
  const ext = path.extname(absPath).toLowerCase();
  if (!TEXT_EXTENSIONS.has(ext)) return true;
  let buf;
  try {
    buf = fs.readFileSync(absPath);
  } catch {
    return true;
  }
  if (buf.length === 0) return true;
  if (buf[buf.length - 1] !== 0x0a) return false;
  if (buf.length >= 2 && buf[buf.length - 2] === 0x0a) return false;
  return true;
}

function runSecretsScanner(flag) {
  const script = path.join(ROOT, "scripts", "check-secrets.mjs");
  if (!fs.existsSync(script)) return true;
  try {
    execFileSync(process.execPath, [script, flag], { stdio: "inherit", cwd: ROOT });
    return true;
  } catch {
    console.error("[pony-agent] Git guard 拦截到未脱敏敏感信息，已阻止操作。");
    return false;
  }
}

function maybeAutoBump(paths) {
  const bumpScript = path.join(ROOT, "scripts", "bump-version.ps1");
  if (!fs.existsSync(bumpScript)) return;
  const touchedCore = paths.some((p) => p.startsWith("crates/pony-agent-core/"));
  const touchedTauri = paths.some((p) => p.startsWith("src-tauri/") || p.startsWith("src/"));
  if (!touchedCore && !touchedTauri) return;

  // 版本 bump 依赖 bump-version.ps1（PowerShell）。Linux/macOS 无 pwsh 时明确提示跳过，
  // 而不是静默——静默跳过正是本轮要消灭的失败模式。
  const pwsh = which("pwsh");
  if (!pwsh) {
    console.log(
      "[pony-agent] NOTE: 未检测到 pwsh，跳过版本 auto-bump（提交层版本 bump 仅 Windows 侧生效）。"
    );
    return;
  }
  console.log("[pony-agent] Version auto-bump: detecting changed components...");
  try {
    execFileSync(pwsh, ["-NoProfile", "-File", bumpScript, "-AutoDetect", "-Stage"], {
      stdio: "inherit",
      cwd: ROOT
    });
  } catch (err) {
    console.warn(`[pony-agent] Warning: version bump exited with code ${err.status ?? "?"}`);
  }
}

function which(cmd) {
  const dirs = (process.env.PATH || "").split(path.delimiter);
  const exts = process.platform === "win32" ? [".exe", ".cmd", ".bat", ""] : [""];
  for (const dir of dirs) {
    if (!dir) continue;
    for (const ext of exts) {
      const candidate = path.join(dir, cmd + ext);
      try {
        fs.accessSync(candidate, fs.constants.X_OK);
        return candidate;
      } catch {
        /* keep looking */
      }
    }
  }
  return null;
}

function guardPreCommit() {
  if (!runSecretsScanner("--staged")) return 1;

  const paths = stagedPaths();
  if (paths.length === 0) return 0;

  const violations = [];
  for (const rel of paths) {
    const normalized = toPosix(rel);

    if (BLOCKED_PATH_PATTERNS.some((re) => re.test(normalized))) {
      violations.push(`禁止提交构建/运行产物目录: ${normalized}`);
      continue;
    }

    const ext = path.extname(normalized).toLowerCase();
    if (BLOCKED_EXTENSIONS.has(ext)) {
      violations.push(`禁止提交二进制构建产物: ${normalized}`);
      continue;
    }

    const abs = path.join(ROOT, normalized);
    let size = 0;
    try {
      const st = fs.statSync(abs);
      if (!st.isFile()) continue;
      size = st.size;
    } catch {
      continue;
    }

    if (size > SIZE_LIMIT_BYTES) {
      const mb = (size / 1024 / 1024).toFixed(2);
      violations.push(`禁止提交超过 20MB 的文件: ${normalized} (${mb}MB)`);
      continue;
    }

    if (!hasExactSingleTrailingNewline(abs)) {
      violations.push(`文件必须以恰好一个换行符结尾: ${normalized}`);
    }
  }

  if (violations.length > 0) {
    console.error("");
    console.error("[pony-agent] Git guard 阻止了本次提交。");
    console.error("原因：");
    for (const v of violations) console.error(`  - ${v}`);
    console.error("");
    console.error("建议：");
    console.error("  1. 把构建产物移出暂存区（git restore --staged <path>）。");
    console.error("  2. 确认目标目录已被 .gitignore 覆盖。");
    console.error("  3. 如需保留真正的大资源，请单独评审其版本管理策略。");
    return 1;
  }

  maybeAutoBump(paths);
  return 0;
}

/** pre-push 的远端信息从 stdin 读取：<local ref> <local sha> <remote ref> <remote sha> */
function readPushRefs() {
  let raw = "";
  try {
    raw = fs.readFileSync(0, "utf8");
  } catch {
    return [];
  }
  const rows = [];
  for (const line of raw.split("\n")) {
    const parts = line.trim().split(/\s+/);
    if (parts.length !== 4) continue;
    const [localRef, localSha, remoteRef, remoteSha] = parts;
    if (remoteRef === "(delete)" || /^0+$/.test(localSha)) continue;
    rows.push({ localRef, localSha, remoteRef, remoteSha });
  }
  return rows;
}

function shortRef(ref) {
  return ref.replace(/^refs\/heads\//, "");
}

function isAncestor(ancestor, descendant) {
  try {
    git(["merge-base", "--is-ancestor", ancestor, descendant]);
    return true;
  } catch {
    return false;
  }
}

function objectExists(sha) {
  if (!sha || /^0+$/.test(sha)) return false;
  try {
    git(["cat-file", "-e", `${sha}^{commit}`]);
    return true;
  } catch {
    return false;
  }
}

function guardPrePush() {
  if (!runSecretsScanner("--pre-push")) return 1;

  const refs = readPushRefs();
  const violations = [];
  const allowMainPush = process.env[ALLOW_PROTECTED_PUSH_ENV] === "1";

  for (const { localRef, localSha, remoteRef, remoteSha } of refs) {
    const branch = shortRef(remoteRef);

    // 1) 受保护分支：禁止任何形式的直接推送（含 force push）。
    if (branch === PROTECTED_BRANCH) {
      if (!allowMainPush) {
        violations.push(
          `禁止直接推送 ${PROTECTED_BRANCH}：请推送特性分支并通过 PR 合并（人工点 Merge）。` +
            `灾备场景确需直推时显式设置 ${ALLOW_PROTECTED_PUSH_ENV}=1 并在提交信息说明理由。`
        );
      }
      continue;
    }

    // 2) 非受保护分支：禁止改写远端既有历史（force push 会踩掉他人提交）。
    if (objectExists(remoteSha) && !isAncestor(remoteSha, localSha)) {
      violations.push(
        `拒绝改写远端已有历史: ${branch}（远端 ${remoteSha.slice(0, 8)} 不是本地 ${localSha.slice(0, 8)} 的祖先）。` +
          `若确需 rebase/force，请先与协作者确认分支归属。`
      );
      continue;
    }

    // 3) 新分支命名纪律：并发会话靠前缀区分归属，避免 integration/main 之类的裸名。
    const prefix = branch.split("/")[0];
    if (!branch.includes("/") || !BRANCH_PREFIXES.includes(prefix)) {
      violations.push(
        `分支名不符合约定: ${branch}。应为 <类型>/<任务号或主题>，类型 ∈ {${BRANCH_PREFIXES.join(", ")}}。`
      );
    }
  }

  if (violations.length > 0) {
    console.error("");
    console.error("[pony-agent] Git guard 阻止了本次推送。");
    for (const v of violations) console.error(`  - ${v}`);
    console.error("");
    return 1;
  }

  return 0;
}

function guardCommitMsg(msgFile) {
  if (!msgFile || !fs.existsSync(msgFile)) return 0;
  const raw = fs.readFileSync(msgFile, "utf8");
  const firstLine = raw.split("\n").find((l) => l.trim() && !l.startsWith("#"));
  if (!firstLine) return 0;

  const msg = firstLine.trim();

  // git 自身的机械前缀：merge/revert/fixup/squash 一律放行，不与之对抗。
  if (/^(Merge |Revert |fixup!|squash!|amend!)/.test(msg)) return 0;

  const matched = COMMIT_TYPES.some(
    (t) => msg.startsWith(`${t}: `) || msg.startsWith(`${t}(`)
  );
  if (!matched) {
    console.error("");
    console.error("[pony-agent] Git guard 阻止了本次提交：提交信息不符合 Conventional Commits。");
    console.error(`  实得: ${msg}`);
    console.error(`  期望: <type>(<scope>): <subject>，type ∈ {${COMMIT_TYPES.join(", ")}}`);
    console.error("  例: fix(tui): 修复 Overlay 取消后焦点丢失");
    console.error("");
    return 1;
  }

  // backlog 编号只提示不阻断：小改动没有卡片是合法的，阻断会逼迫伪造编号。
  if (!/\bB\d{3,}\b/.test(raw)) {
    console.log("[pony-agent] NOTE: 提交信息未关联 backlog 编号（B###）；非平凡变更请补上。");
  }
  return 0;
}

let code = 0;
switch (MODE) {
  case "pre-commit":
    code = guardPreCommit();
    break;
  case "pre-push":
    code = guardPrePush();
    break;
  case "commit-msg":
    code = guardCommitMsg(process.argv[3]);
    break;
  default:
    console.error(`git-guard: 未知模式 ${MODE}（期望 pre-commit | pre-push | commit-msg）`);
    code = 2;
}

process.exit(code);
