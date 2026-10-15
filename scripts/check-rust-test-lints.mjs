#!/usr/bin/env node
/**
 * Wave-13 C1 前置断言门禁：`pony-agent-core` 测试目标在 `RUSTFLAGS=-D warnings` 下必须可编译。
 *
 * 契约：`.dev-team/contracts/wave-13-ci-tui-lint.md`（C1）
 * 背景：CI `verify` 作业（windows-latest）注入 `RUSTFLAGS=-D warnings`
 *   （`actions-rust-lang/setup-rust-toolchain@v1` 默认行为），执行
 *   `cargo test --manifest-path src-tauri/Cargo.toml --target-dir target-test --package=pony-agent-core`，
 *   于是 `tui_acceptance` 测试目标的 warning 全部升级为 error：
 *   - 15 条 `dead_code`：`crates/pony-agent-core/tests/tui_acceptance.rs` 以
 *     `#[path = "../src/bin/tui.rs"] mod tui;` 把 **bin 目标当模块引入**，仅被
 *     `main()` / `run_tui()` / `ui()` 触达的条目在测试 crate 中判为 never used；
 *   - 1 条 Windows 专属 `unused_variables`（`src/bin/tui.rs:1402` 的 `log_file`
 *     只在 `#[cfg(unix)]` 分支内被使用，非 unix 目标下未使用）。
 *
 * 本脚本逐字执行 C1 命令（不绕道、不改 CI 命令、不删测试目标），
 * 按**子进程原生 EXIT** 判定自身 EXIT，并透传 cargo 的 stdout/stderr。
 *
 * 退出码：
 *   0   = 通过（子进程 EXIT=0）
 *   N   = 子进程原生 EXIT（编译失败时 cargo 返回 101），原样透传
 *   2   = 脚本自身错误（cargo 不可执行 / 超时 / 用法错误）
 *
 * 副作用：仅向 `--target-dir`（默认 `target-test`）写入 cargo 构建产物，可重复运行。
 * 跨平台：纯 Node（`child_process.spawn`），不依赖 PowerShell / shell 展开；
 *         Windows 上 `cargo` 由 Node 直接解析 `cargo.exe`。
 *
 * 用法：
 *   node scripts/check-rust-test-lints.mjs
 *   node scripts/check-rust-test-lints.mjs --timeout-ms 1800000
 *   node scripts/check-rust-test-lints.mjs -- cargo test -- --list   # 追加参数（罕见）
 */

import { spawn } from "node:child_process";
import process from "node:process";

const DEFAULT_TARGET_DIR = "target-test";
const DEFAULT_MANIFEST = "src-tauri/Cargo.toml";
const DEFAULT_PACKAGE = "pony-agent-core";
const DEFAULT_TIMEOUT_MS = 30 * 60 * 1000; // 首次编译可能数分钟；给足 30 分钟
const DENY_WARNINGS = "-D warnings";

function parseArgs(argv) {
  const options = {
    targetDir: DEFAULT_TARGET_DIR,
    manifest: DEFAULT_MANIFEST,
    packageName: DEFAULT_PACKAGE,
    timeoutMs: DEFAULT_TIMEOUT_MS,
    extraArgs: []
  };

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--help" || arg === "-h") {
      options.help = true;
      continue;
    }
    if (arg === "--") {
      options.extraArgs = argv.slice(i + 1);
      break;
    }
    if (arg === "--target-dir") {
      options.targetDir = argv[++i];
      continue;
    }
    if (arg === "--manifest-path") {
      options.manifest = argv[++i];
      continue;
    }
    if (arg === "--package") {
      options.packageName = argv[++i];
      continue;
    }
    if (arg === "--timeout-ms") {
      options.timeoutMs = Number(argv[++i]);
      continue;
    }
    process.stderr.write(`[check-rust-test-lints] 未知参数：${arg}\n`);
    options.usageError = true;
    return options;
  }

  if (!Number.isFinite(options.timeoutMs) || options.timeoutMs <= 0) {
    process.stderr.write("[check-rust-test-lints] --timeout-ms 必须是正数\n");
    options.usageError = true;
  }
  return options;
}

function printHelp() {
  process.stdout.write(
    [
      "用法：node scripts/check-rust-test-lints.mjs [选项] [-- <追加 cargo 参数>]",
      "",
      "  --target-dir <dir>      cargo 构建目录（默认 target-test）",
      "  --manifest-path <path>  manifest 路径（默认 src-tauri/Cargo.toml）",
      "  --package <name>        包名（默认 pony-agent-core）",
      "  --timeout-ms <ms>       子进程超时（默认 1800000 = 30 分钟）",
      "  -h, --help              显示本帮助",
      "",
      "退出码：0=通过；其余=子进程原生 EXIT（编译失败通常 101）；2=脚本自身错误。"
    ].join("\n") + "\n"
  );
}

/** 从 cargo 输出中提取 error 行与 lint 种类，供收据引用（仅统计，不改变判定）。 */
function summarize(output) {
  const lines = output.split(/\r?\n/);
  const errorLines = lines.filter((line) => /^error(\[|:)/.test(line.trim()));
  const lintCounts = new Map();
  for (const line of lines) {
    const match = line.match(/^error(\[[^\]]+\])?:\s*(.+)$/);
    if (!match) continue;
    const kind = match[1] ? match[1].slice(1, -1) : "error";
    const message = match[2].trim();
    const lintName = message.startsWith("unused") || message.includes("is never used") || message.includes("is never read")
      ? message
      : kind;
    lintCounts.set(lintName, (lintCounts.get(lintName) ?? 0) + 1);
  }
  const deadCodeLocations = [];
  for (let i = 0; i < lines.length; i += 1) {
    const line = lines[i];
    const match = line.match(/^\s*-->\s+([^:]+):(\d+):(\d+)\s*$/);
    if (!match) continue;
    // 仅登记紧随其后的 lint 段落（向前回溯 3 行内出现 error[dead_code] / never used 等）
    const context = lines.slice(Math.max(0, i - 3), i).join(" ");
    if (/dead_code|never used|never read/.test(context)) {
      deadCodeLocations.push({ file: match[1], line: Number(match[2]), column: Number(match[3]) });
    }
  }
  return { errorCount: errorLines.length, lintCounts, deadCodeLocations };
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  if (options.help) {
    printHelp();
    return 0;
  }
  if (options.usageError) {
    printHelp();
    return 2;
  }

  const cargoArgs = [
    "test",
    "--manifest-path",
    options.manifest,
    "--target-dir",
    options.targetDir,
    `--package=${options.packageName}`,
    ...options.extraArgs
  ];

  const env = { ...process.env, RUSTFLAGS: DENY_WARNINGS };
  if (process.env.RUSTFLAGS && process.env.RUSTFLAGS !== DENY_WARNINGS) {
    process.stderr.write(
      `[check-rust-test-lints] 覆盖既有 RUSTFLAGS（原值：${process.env.RUSTFLAGS}）→ ${DENY_WARNINGS}\n`
    );
  }

  process.stderr.write(
    `[check-rust-test-lints] RUSTFLAGS="${DENY_WARNINGS}" cargo ${cargoArgs.join(" ")}\n`
  );

  const exitCode = await new Promise((resolve) => {
    let captured = "";
    let settled = false;
    const child = spawn("cargo", cargoArgs, { env, stdio: ["inherit", "pipe", "pipe"] });

    const timer = setTimeout(() => {
      if (settled) return;
      settled = true;
      process.stderr.write(
        `[check-rust-test-lints] 超时（${options.timeoutMs}ms），终止 cargo 子进程\n`
      );
      child.kill("SIGKILL");
      resolve(2);
    }, options.timeoutMs);

    child.stdout.on("data", (chunk) => {
      captured += chunk.toString();
      process.stdout.write(chunk);
    });
    child.stderr.on("data", (chunk) => {
      captured += chunk.toString();
      process.stderr.write(chunk);
    });

    child.on("error", (error) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      process.stderr.write(
        `[check-rust-test-lints] 无法执行 cargo：${error.message}（请确认 Rust 工具链已安装并在 PATH 中）\n`
      );
      resolve(2);
    });

    child.on("close", (code, signal) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      const summary = summarize(captured);
      process.stderr.write(
        `[check-rust-test-lints] 子进程退出：code=${code ?? "null"} signal=${signal ?? "null"} ` +
          `error 行数=${summary.errorCount} dead_code 定位=${summary.deadCodeLocations.length}\n`
      );
      if (summary.lintCounts.size > 0) {
        const detail = [...summary.lintCounts.entries()]
          .map(([kind, count]) => `${kind}×${count}`)
          .join(", ");
        process.stderr.write(`[check-rust-test-lints] lint 分布：${detail}\n`);
      }
      if (code === 0) {
        process.stderr.write("[check-rust-test-lints] PASS（RUSTFLAGS=-D warnings 下测试目标可编译）\n");
        resolve(0);
        return;
      }
      process.stderr.write(
        [
          "[check-rust-test-lints] FAIL：`RUSTFLAGS=-D warnings` 下 pony-agent-core 测试目标编译失败。",
          "  请检查：",
          "   1) crates/pony-agent-core/tests/tui_acceptance.rs —— bin 当模块引入（`#[path = \"../src/bin/tui.rs\"] mod tui;`）",
          "      产生的 dead_code 是否为最小 lint 集合 + 中文注释说明（契约 C3：禁止 #![allow(warnings)] 全局放水）；",
          "   2) crates/pony-agent-core/src/bin/tui.rs —— 仅在 #[cfg(unix)] 内使用的绑定（如 log_file）",
          "      在非 unix 目标下是否产生 unused_variables（契约 C2：须保持 unix 行为不变）。"
        ].join("\n") + "\n"
      );
      resolve(typeof code === "number" ? code : 2);
    });
  });

  return exitCode;
}

main()
  .then((code) => {
    process.exitCode = code;
  })
  .catch((error) => {
    process.stderr.write(`[check-rust-test-lints] 未捕获异常：${error?.stack ?? String(error)}\n`);
    process.exitCode = 2;
  });
