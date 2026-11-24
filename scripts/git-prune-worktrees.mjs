#!/usr/bin/env node
/**
 * git-prune-worktrees.mjs —— 安全清理已 Squash/Merge 并入主干的孤儿 worktree 与本地分支。
 *
 * 为什么存在：本仓远端采用 Squash and merge，导致 `git branch --merged origin/main`
 * 对已合并特性分支完全失明（2026-11-24 实测滞留 30 个已合并 worktree）。
 *
 * 默认只读（dry-run）：只汇报候选与跳过原因；传 `--apply` 才真正删除。
 *
 * 安全不变量（任一不满足即跳过，宁漏删不误删）：
 *   1. 永不删除主工作区、当前工作区（cwd）、`main` 分支；
 *   2. 目标工作树必须无脏改动（仅忽略根目录下的 `node_modules` 软链）；
 *   3. 没有任何活跃进程（Linux `/proc/<pid>/cwd`）当前驻留在该工作树；
 *   4. 该分支在 GitHub 有 `[MERGED]` PR 且无 `[OPEN]` PR，或提交已为 `origin/main` 祖先且内容无差异。
 */
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

function git(args, cwd) {
  return execFileSync("git", args, {
    cwd,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"]
  });
}

export function parseWorktreeList(porcelainText) {
  const entries = [];
  let current = {};
  for (const rawLine of porcelainText.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (!line) {
      if (current.path) entries.push(current);
      current = {};
      continue;
    }
    if (line.startsWith("worktree ")) current.path = line.slice("worktree ".length);
    else if (line.startsWith("HEAD ")) current.head = line.slice("HEAD ".length);
    else if (line.startsWith("branch ")) current.branch = line.slice("branch ".length).replace(/^refs\/heads\//, "");
    else if (line === "detached") current.detached = true;
  }
  if (current.path) entries.push(current);
  return entries;
}

export function parseDirtyEntries(statusShortText) {
  return statusShortText
    .split(/\r?\n/)
    .map((line) => line.trimEnd())
    .filter(Boolean)
    .filter((line) => !/^\?\?\s+node_modules\/?$/.test(line));
}

/**
 * 纯分类函数（供单测离线验证）：
 * 返回 { safe: [...], skipped: [...] }。
 */
export function classifyPruneCandidates({
  worktrees,
  mainWorktreePath,
  currentWorktreePath,
  mergedBranches,
  openBranches,
  activeCwds,
  dirtyMap,
  ancestorBranches = new Set()
}) {
  const safe = [];
  const skipped = [];
  const mainNorm = path.resolve(mainWorktreePath);
  const currentNorm = path.resolve(currentWorktreePath);

  for (const wt of worktrees) {
    const wtNorm = path.resolve(wt.path);
    const branch = wt.branch || null;
    if (wtNorm === mainNorm) {
      skipped.push({ path: wt.path, branch, reason: "主工作区" });
      continue;
    }
    if (wtNorm === currentNorm) {
      skipped.push({ path: wt.path, branch, reason: "当前工作区" });
      continue;
    }
    if (!branch || branch === "main") {
      skipped.push({ path: wt.path, branch, reason: branch === "main" ? "受保护分支 main" : "游离 HEAD" });
      continue;
    }
    if (openBranches.has(branch)) {
      skipped.push({ path: wt.path, branch, reason: "存在未关闭 PR" });
      continue;
    }
    const isMerged = mergedBranches.has(branch) || ancestorBranches.has(branch);
    if (!isMerged) {
      skipped.push({ path: wt.path, branch, reason: "尚未合入 origin/main（无已合并 PR）" });
      continue;
    }
    const dirty = dirtyMap.get(wtNorm) || [];
    if (dirty.length > 0) {
      skipped.push({ path: wt.path, branch, reason: `存在未提交改动 (${dirty.length} 项)` });
      continue;
    }
    const occupied = [...activeCwds].some((cwd) => cwd === wtNorm || cwd.startsWith(wtNorm + path.sep));
    if (occupied) {
      skipped.push({ path: wt.path, branch, reason: "有活跃进程驻留该目录" });
      continue;
    }
    safe.push({ path: wt.path, branch, reason: mergedBranches.has(branch) ? "PR 已合并" : "已并入 origin/main" });
  }
  return { safe, skipped };
}

function collectActiveCwds() {
  const cwds = new Set();
  if (process.platform !== "linux") return cwds;
  let entries = [];
  try {
    entries = fs.readdirSync("/proc");
  } catch {
    return cwds;
  }
  for (const entry of entries) {
    if (!/^\d+$/.test(entry)) continue;
    try {
      const target = fs.readlinkSync(`/proc/${entry}/cwd`);
      cwds.add(path.resolve(target));
    } catch {
      // 权限不足或进程已退出
    }
  }
  return cwds;
}

function fetchGhPrBranches(repoRoot, offline) {
  if (offline) return { mergedBranches: new Set(), openBranches: new Set(), source: "offline" };
  try {
    const mergedRaw = execFileSync(
      "gh",
      ["pr", "list", "--state", "merged", "--limit", "200", "--json", "headRefName"],
      { cwd: repoRoot, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }
    );
    const openRaw = execFileSync(
      "gh",
      ["pr", "list", "--state", "open", "--limit", "100", "--json", "headRefName"],
      { cwd: repoRoot, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }
    );
    const mergedBranches = new Set(JSON.parse(mergedRaw).map((row) => row.headRefName).filter(Boolean));
    const openBranches = new Set(JSON.parse(openRaw).map((row) => row.headRefName).filter(Boolean));
    return { mergedBranches, openBranches, source: "gh" };
  } catch (err) {
    throw new Error(`无法从 GitHub 查询 PR 状态（可传 --offline 仅按 git 祖先判定）：${err.message}`);
  }
}

function isBranchMergedInMain(repoRoot, branch) {
  try {
    git(["merge-base", "--is-ancestor", branch, "origin/main"], repoRoot);
    return true;
  } catch {
    return false;
  }
}

function runCli(argv) {
  const apply = argv.includes("--apply");
  const offline = argv.includes("--offline");
  const asJson = argv.includes("--json");
  if (argv.includes("--help") || argv.includes("-h")) {
    console.log("用法: node scripts/git-prune-worktrees.mjs [--apply] [--offline] [--json]");
    return 0;
  }

  const currentWorktreePath = git(["rev-parse", "--show-toplevel"], process.cwd()).trim();
  const worktrees = parseWorktreeList(git(["worktree", "list", "--porcelain"], currentWorktreePath));
  const mainWorktreePath = worktrees[0]?.path || currentWorktreePath;

  const { mergedBranches, openBranches, source } = fetchGhPrBranches(mainWorktreePath, offline);
  const activeCwds = collectActiveCwds();
  const dirtyMap = new Map();
  const ancestorBranches = new Set();

  for (const wt of worktrees) {
    const wtNorm = path.resolve(wt.path);
    try {
      const st = git(["status", "--short"], wt.path);
      dirtyMap.set(wtNorm, parseDirtyEntries(st));
    } catch {
      dirtyMap.set(wtNorm, ["(git status 失败)"]);
    }
    if (wt.branch && isBranchMergedInMain(mainWorktreePath, wt.branch)) {
      ancestorBranches.add(wt.branch);
    }
  }

  const { safe, skipped } = classifyPruneCandidates({
    worktrees,
    mainWorktreePath,
    currentWorktreePath,
    mergedBranches,
    openBranches,
    activeCwds,
    dirtyMap,
    ancestorBranches
  });

  const removed = [];
  if (apply) {
    for (const item of safe) {
      const nmPath = path.join(item.path, "node_modules");
      try {
        if (fs.lstatSync(nmPath).isSymbolicLink()) fs.unlinkSync(nmPath);
      } catch {
        // 不存在或非软链则交由 git worktree remove 处理
      }
      git(["worktree", "remove", item.path], mainWorktreePath);
      git(["branch", "-D", item.branch], mainWorktreePath);
      removed.push(item);
    }
    if (removed.length > 0) git(["worktree", "prune"], mainWorktreePath);
  }

  if (asJson) {
    console.log(JSON.stringify({ mode: apply ? "apply" : "dry-run", source, safe, skipped, removed }, null, 2));
    return 0;
  }

  console.log(`[git-prune] 模式=${apply ? "apply" : "dry-run"} 数据源=${source} 可安全清理=${safe.length} 保留/跳过=${skipped.length}`);
  for (const item of safe) {
    console.log(`  ${apply ? "REMOVED" : "CAN_PRUNE"} ${item.branch} (${item.path}) —— ${item.reason}`);
  }
  for (const item of skipped) {
    console.log(`  KEEP      ${item.branch || "(detached)"} (${item.path}) —— ${item.reason}`);
  }
  if (!apply && safe.length > 0) {
    console.log("[git-prune] dry-run 未修改任何目录；确认清理请加 --apply（或 npm run git:prune:apply）。");
  }
  return 0;
}

let isDirect = false;
if (process.argv[1]) {
  try {
    isDirect = fs.realpathSync(process.argv[1]) === fs.realpathSync(fileURLToPath(import.meta.url));
  } catch {
    isDirect = false;
  }
}

if (isDirect) {
  try {
    process.exit(runCli(process.argv.slice(2)));
  } catch (err) {
    console.error(`[git-prune] 失败：${err.message}`);
    process.exit(2);
  }
}
