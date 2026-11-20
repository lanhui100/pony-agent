#!/usr/bin/env node
// CI 变更面判定：决定本次改动是否需要 Rust 全量检查（cargo check / clippy / cargo test）。
//
// 契约（两条，改动前先读）：
//   1. **fail-safe 方向是「跑全量」**：任何拿不准的路径、解析不了 base、git 命令失败，
//      一律判 needs_rust=true。判错方向的代价不对称——多跑一次是几分钟机器时间，
//      少跑一次是「门禁静默消失」。
//   2. **本脚本永远 exit 0**：它的输出是下游 job 的 if 输入。脚本非零退出会让
//      changes job 失败、Rust job 被 skip，required check `verify` 随之消失，PR 直接卡死；
//      这比「多跑一次」严重得多。所以异常只降级为 true，不向上抛。
//
// 用法：
//   node scripts/ci-scope.mjs --base <sha> [--head HEAD] [--json]
// 输出（stdout，可直接 >> $GITHUB_OUTPUT）：
//   needs_rust=true|false
//   scope_reason=code|docs-only|empty-diff|no-base|error
import { execFileSync } from "node:child_process";

// 即使文件以 .md 结尾也要跑 Rust 的树：.meta 的 lint 表是真值源、.github 里是 CI 自身、
// scripts/crates/src-tauri 是编译面本体。路径一旦落在这里，判定直接短路为 true。
const UNSAFE_PREFIXES = [
  ".github/",
  ".meta/",
  ".githooks/",
  "scripts/",
  "crates/",
  "src-tauri/",
  "src/",
  "tools/",
  "tests/",
  "public/",
  "package.json",
  "package-lock.json",
  "Cargo.toml",
  "Cargo.lock",
  "rust-toolchain.toml",
  "vite.config",
  "tsconfig",
];

// 纯文档/账本树：只改这里不影响 Rust 编译与测试面。
const SAFE_PREFIXES = ["docs/", "backlog/", "openspec/", ".agents/notes/", ".agents/skills/"];

function needsRust(file) {
  if (UNSAFE_PREFIXES.some((p) => file.startsWith(p))) return true;
  if (SAFE_PREFIXES.some((p) => file.startsWith(p))) return false;
  // 根级 markdown（README.md / AGENTS.md…）是纯文案；子目录里未列入 SAFE 树的
  // markdown 一律保守判 true——「看起来像文档」不是证据，路径白名单才是。
  if (!file.includes("/") && file.endsWith(".md")) return false;
  return true;
}

function parseArgs(argv) {
  const out = { base: "", head: "HEAD", json: false };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--base") out.base = argv[i + 1] ?? "";
    else if (arg === "--head") out.head = argv[i + 1] ?? "HEAD";
    else if (arg === "--json") out.json = true;
    else if (arg === "--help" || arg === "-h") out.help = true;
  }
  return out;
}

function resolveBase(base) {
  if (!base || /^0+$/.test(base)) return null;
  try {
    execFileSync("git", ["rev-parse", "--verify", "--quiet", `${base}^{commit}`], {
      stdio: "pipe",
    });
    return base;
  } catch {
    return null;
  }
}

function listChanged(base, head) {
  const stdout = execFileSync("git", ["diff", "--name-only", base, head], {
    encoding: "utf8",
    stdio: ["pipe", "pipe", "pipe"],
  });
  return stdout
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean);
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.help) {
    console.log("用法: node scripts/ci-scope.mjs --base <sha> [--head HEAD] [--json]");
    return;
  }

  let needs_rust = true;
  let scope_reason = "no-base";
  let changed = [];

  try {
    const base = resolveBase(args.base);
    if (base) {
      changed = listChanged(base, args.head);
      if (changed.length === 0) {
        needs_rust = false;
        scope_reason = "empty-diff";
      } else if (changed.some(needsRust)) {
        needs_rust = true;
        scope_reason = "code";
      } else {
        needs_rust = false;
        scope_reason = "docs-only";
      }
    }
  } catch (error) {
    // 上面契约第 2 条：任何意外都只降级为「跑全量」，不向上抛。
    needs_rust = true;
    scope_reason = `error:${error instanceof Error ? error.message : String(error)}`;
  }

  if (args.json) {
    console.log(
      JSON.stringify({ needs_rust, scope_reason, base: args.base, head: args.head, changed }, null, 2),
    );
  } else {
    console.log(`needs_rust=${needs_rust}`);
    console.log(`scope_reason=${scope_reason.replace(/\n/g, " ")}`);
  }
}

main();
process.exit(0);
