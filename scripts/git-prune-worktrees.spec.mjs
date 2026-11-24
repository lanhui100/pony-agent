#!/usr/bin/env node
import assert from "node:assert/strict";
import { classifyPruneCandidates, parseDirtyEntries, parseWorktreeList } from "./git-prune-worktrees.mjs";

const porcelain = `worktree /repo
HEAD aaaaaaa
branch refs/heads/main

worktree /wt/merged-clean
HEAD bbbbbbb
branch refs/heads/feat/done

worktree /wt/dirty
HEAD ccccccc
branch refs/heads/feat/dirty

worktree /wt/open-pr
HEAD ddddddd
branch refs/heads/feat/wip

worktree /wt/occupied
HEAD eeeeeee
branch refs/heads/feat/active

worktree /wt/current
HEAD fffffff
branch refs/heads/chore/self
`;

const worktrees = parseWorktreeList(porcelain);
assert.equal(worktrees.length, 6, "应解析出 6 个 worktree");

assert.deepEqual(parseDirtyEntries("?? node_modules\n"), [], "node_modules 软链不应算脏文件");
assert.deepEqual(parseDirtyEntries(" M src/a.rs\n?? node_modules\n"), [" M src/a.rs"], "真实修改必须算脏文件");

const dirtyMap = new Map([
  ["/wt/merged-clean", []],
  ["/wt/dirty", [" M src/a.rs"]],
  ["/wt/open-pr", []],
  ["/wt/occupied", []],
  ["/wt/current", []]
]);

const { safe, skipped } = classifyPruneCandidates({
  worktrees,
  mainWorktreePath: "/repo",
  currentWorktreePath: "/wt/current",
  mergedBranches: new Set(["feat/done", "feat/dirty", "feat/wip", "feat/active", "chore/self"]),
  openBranches: new Set(["feat/wip"]),
  activeCwds: new Set(["/wt/occupied"]),
  dirtyMap
});

assert.deepEqual(
  safe.map((s) => s.branch),
  ["feat/done"],
  "仅干净、无人驻留、无 open PR 的已合并分支才可安全清理"
);
assert.equal(skipped.length, 5, "主工作区/当前工作区/脏目录/open PR/活跃进程驻留必须全部保留");

console.log("PASS git-prune-worktrees.spec: 安全分类与不变量全部通过");
