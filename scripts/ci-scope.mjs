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
//   needs_frontend=true|false
//   needs_core=true|false
//   needs_tui=true|false
//   needs_cli=true|false
//   needs_tauri=true|false
//   scope_reason=code|docs-only|empty-diff|no-base|error
import { execFileSync } from "node:child_process";

// 即使文件以 .md 结尾也要跑全量的树：.meta 的 lint 表是真值源、.github 里是 CI 自身、
// scripts/Cargo 根定义影响全工作区。路径一旦落在这里，全量判定短路为 true。
const GLOBAL_UNSAFE_PREFIXES = [
  ".github/",
  ".meta/",
  ".githooks/",
  "scripts/",
  "Cargo.toml",
  "Cargo.lock",
  "rust-toolchain.toml",
];

// 纯前端面：只影响 Vite/TS 构建与 vitest，**不进入任何 Rust 编译单元**，
// 故不需要 Rust 全量门（省下 Windows 近 30min + 三平台矩阵）。
// 例外口径（必须仍判 needs_rust=true）：
//   - `src/types/`：Rust serde 线缆键的 TS 镜像，由 `docs/contracts/ts-contract.snapshot.json`
//     与 `npm run contract:check` 双向核对；
//   - `docs/contracts/`：Rust conformance / ts_contract_snapshot 测试的事实源（见下方分支）。
// 新增 src 子目录默认落在这里（fail-open 到「前端面」）——前端树里出现 Rust 依赖前的
// 结构性前提是「它得先被 Rust 编译单元引用」，那种改动必然同时落在 crates/ 或 src-tauri/。
const FRONTEND_ONLY_PREFIX = "src/";
const FRONTEND_ONLY_EXCLUDE_PREFIXES = ["src/types/"];

// 与 Rust 侧核心编译面（含其传递下游）对齐；用于保证旧契约 needs_rust 的保守边界。
const FRONTEND_CODE_PREFIXES = [
  "src/",
  "tests/",
  "public/",
  "index.html",
  "package.json",
  "package-lock.json",
  "vite.config",
  "tsconfig",
  "vitest.config",
];

const CORE_CRATE_PREFIXES = [
  "crates/pony-protocol/",
  "crates/pony-observability/",
  "crates/pony-storage/",
  "crates/pony-provider/",
  "crates/pony-tools/",
  "crates/pony-acp/",
  "crates/pony-agent-core/",
];

const TUI_PREFIXES = ["crates/pony-tui/", "tools/tui-audit/"];
const CLI_PREFIXES = ["crates/pony-cli/"];
const TAURI_PREFIXES = ["src-tauri/"];
const TEST_SUPPORT_PREFIXES = ["crates/pony-test-support/"];

const UNSAFE_PREFIXES = [
  ...GLOBAL_UNSAFE_PREFIXES,
  ...FRONTEND_CODE_PREFIXES,
  "crates/",
  "src-tauri/",
  "tools/",
];

// 纯文档/账本树：只改这里不影响 Rust 编译与前端测试面。
const SAFE_PREFIXES = ["docs/", "backlog/", "openspec/", ".agents/notes/", ".agents/skills/"];

function isDocsOnlyFile(file) {
  if (UNSAFE_PREFIXES.some((p) => file.startsWith(p))) return false;
  // docs/contracts 虽在 docs/ 下，却是 Rust 侧 conformance / ts_contract_snapshot 测试的
  // 事实源（schema 与 ts-contract.snapshot.json 都被 include/读盘断言）⇒ 不算纯文档。
  if (file.startsWith("docs/contracts/")) return false;
  if (SAFE_PREFIXES.some((p) => file.startsWith(p))) return true;
  // 根级 markdown（README.md / AGENTS.md…）是纯文案；子目录里未列入 SAFE 树的
  // markdown 一律保守判代码面——「看起来像文档」不是证据，路径白名单才是。
  if (!file.includes("/") && file.endsWith(".md")) return true;
  return false;
}

function classifyFiles(files) {
  if (files.length === 0) {
    return {
      needs_rust: false,
      needs_frontend: false,
      needs_core: false,
      needs_tui: false,
      needs_cli: false,
      needs_tauri: false,
      scope_reason: "empty-diff",
    };
  }

  let needs_rust = false;
  let needs_frontend = false;
  let needs_core = false;
  let needs_tui = false;
  let needs_cli = false;
  let needs_tauri = false;

  for (const file of files) {
    if (isDocsOnlyFile(file)) {
      continue;
    }

    // 纯前端面短路：不置 needs_rust，只置 needs_frontend。
    if (
      file.startsWith(FRONTEND_ONLY_PREFIX) &&
      !FRONTEND_ONLY_EXCLUDE_PREFIXES.some((p) => file.startsWith(p))
    ) {
      needs_frontend = true;
      continue;
    }

    needs_rust = true;

    if (GLOBAL_UNSAFE_PREFIXES.some((p) => file.startsWith(p))) {
      needs_frontend = true;
      needs_core = true;
      needs_tui = true;
      needs_cli = true;
      needs_tauri = true;
      continue;
    }

    if (FRONTEND_CODE_PREFIXES.some((p) => file.startsWith(p))) {
      needs_frontend = true;
      if (file.startsWith("tests/")) {
        needs_core = true;
        needs_tui = true;
        needs_cli = true;
        needs_tauri = true;
      }
      continue;
    }

    if (CORE_CRATE_PREFIXES.some((p) => file.startsWith(p))) {
      needs_core = true;
      needs_tui = true;
      needs_cli = true;
      needs_tauri = true;
      continue;
    }

    // Rust 侧测试直接读 docs/contracts 下的 schema 与 ts-contract 快照
    // （crates/pony-agent-core/tests/runtime_suite/{conformance,ts_contract_snapshot}.rs），
    // 故它不属于「只改 docs/ 就不用跑 Rust」的安全面。
    if (file.startsWith("docs/contracts/")) {
      needs_core = true;
      continue;
    }

    if (TEST_SUPPORT_PREFIXES.some((p) => file.startsWith(p))) {
      needs_core = true;
      needs_tui = true;
      needs_cli = true;
      continue;
    }

    if (TUI_PREFIXES.some((p) => file.startsWith(p))) {
      needs_tui = true;
      continue;
    }

    if (CLI_PREFIXES.some((p) => file.startsWith(p))) {
      needs_cli = true;
      continue;
    }

    if (TAURI_PREFIXES.some((p) => file.startsWith(p))) {
      needs_tauri = true;
      continue;
    }

    // 未在白/灰表列明的路径：fail-safe 跑全量
    needs_frontend = true;
    needs_core = true;
    needs_tui = true;
    needs_cli = true;
    needs_tauri = true;
  }

  return {
    needs_rust,
    needs_frontend,
    needs_core,
    needs_tui,
    needs_cli,
    needs_tauri,
    scope_reason: needs_rust ? "code" : needs_frontend ? "frontend-only" : "docs-only",
  };
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

  let flags = {
    needs_rust: true,
    needs_frontend: true,
    needs_core: true,
    needs_tui: true,
    needs_cli: true,
    needs_tauri: true,
    scope_reason: "no-base",
  };
  let changed = [];

  try {
    const base = resolveBase(args.base);
    if (base) {
      changed = listChanged(base, args.head);
      flags = classifyFiles(changed);
    }
  } catch (error) {
    // 上面契约第 2 条：任何意外都只降级为「跑全量」，不向上抛。
    flags = {
      needs_rust: true,
      needs_frontend: true,
      needs_core: true,
      needs_tui: true,
      needs_cli: true,
      needs_tauri: true,
      scope_reason: `error:${error instanceof Error ? error.message : String(error)}`,
    };
  }

  if (args.json) {
    console.log(
      JSON.stringify({ ...flags, base: args.base, head: args.head, changed }, null, 2),
    );
  } else {
    console.log(`needs_rust=${flags.needs_rust}`);
    console.log(`needs_frontend=${flags.needs_frontend}`);
    console.log(`needs_core=${flags.needs_core}`);
    console.log(`needs_tui=${flags.needs_tui}`);
    console.log(`needs_cli=${flags.needs_cli}`);
    console.log(`needs_tauri=${flags.needs_tauri}`);
    console.log(`scope_reason=${flags.scope_reason.replace(/\n/g, " ")}`);
  }
}

main();
process.exit(0);
