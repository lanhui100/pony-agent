#!/usr/bin/env node
/**
 * scripts/invoke-rust-target.mjs —— Rust target 槽位调用器（跨平台，B140）
 *
 * 契约（与 scripts/invoke-rust-target.ps1 逐槽对齐；漂移由
 * tests/invoke-rust-target-slots.spec.ts 机械拦截）：
 *
 *   node scripts/invoke-rust-target.mjs <slot> <cargo 子命令> [cargo 额外参数...]
 *
 * 等价于：
 *   cargo <子命令> --manifest-path <repo>/src-tauri/Cargo.toml \
 *         --target-dir <repo>/<槽位目录> [cargo 额外参数...]
 *
 * 退出码：cargo 的退出码；cargo 不在 PATH = 127；用法错误（未知槽位/缺子命令）= 1。
 * 环境变量（RUSTFLAGS 等）走 stdio/stderr 继承，不做改写。
 */
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import process from 'node:process';
import { fileURLToPath, pathToFileURL } from 'node:url';

/** 槽位 → target 目录。新增槽位两条路（.mjs / .ps1）必须同步，否则 parity 用例红。 */
export const SLOT_TARGET_DIRS = Object.freeze({
  check: 'target-check',
  test: 'target-test',
  'exact-a': 'target-test-exact-a',
  'exact-b': 'target-test-exact-b',
  'exact-c': 'target-test-exact-c'
});

const MANIFEST_RELATIVE = path.join('src-tauri', 'Cargo.toml');
const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

export function usage() {
  return `usage: node scripts/invoke-rust-target.mjs <${Object.keys(SLOT_TARGET_DIRS).join('|')}> <cargo 子命令> [cargo 参数...]`;
}

/**
 * 纯函数：把 (槽位, cargo 参数) 翻译成一次 cargo 调用。不触碰文件系统、不执行进程，
 * 故 parity 用例可以直接断言 argv 形状。
 */
export function buildCargoInvocation(slot, cargoArgs, repoRoot = REPO_ROOT) {
  if (!Object.prototype.hasOwnProperty.call(SLOT_TARGET_DIRS, slot)) {
    throw new Error(`未知槽位 "${slot}"（可选：${Object.keys(SLOT_TARGET_DIRS).join(' / ')}）`);
  }
  if (!cargoArgs || cargoArgs.length === 0) {
    throw new Error('缺少 cargo 子命令（示例：check check / exact-a test）');
  }
  const [subcommand, ...extraArgs] = cargoArgs;
  return {
    command: 'cargo',
    cwd: repoRoot,
    args: [
      subcommand,
      '--manifest-path',
      path.join(repoRoot, MANIFEST_RELATIVE),
      '--target-dir',
      path.join(repoRoot, SLOT_TARGET_DIRS[slot]),
      ...extraArgs
    ]
  };
}

export function main(argv) {
  const [slot, ...cargoArgs] = argv;
  if (!slot) {
    console.error(usage());
    return 1;
  }

  let invocation;
  try {
    invocation = buildCargoInvocation(slot, cargoArgs);
  } catch (error) {
    console.error(`[rust-target] ${error.message}`);
    console.error(usage());
    return 1;
  }

  const result = spawnSync(invocation.command, invocation.args, {
    cwd: invocation.cwd,
    stdio: 'inherit'
  });

  if (result.error) {
    if (result.error.code === 'ENOENT') {
      console.error('[rust-target] cargo 不在 PATH：请先安装 Rust 工具链（https://rustup.rs）');
      return 127;
    }
    console.error(`[rust-target] ${result.error.message}`);
    return 1;
  }
  if (result.signal) {
    console.error(`[rust-target] cargo 被信号 ${result.signal} 终止`);
    return 1;
  }
  return result.status ?? 1;
}

/** 与 scripts/backlog.mjs 同款守卫：既可被 import（测试），也可直接执行（npm 脚本）。 */
function isDirectInvocation() {
  const entry = process.argv[1];
  if (!entry) return false;
  try {
    const entryUrl = pathToFileURL(fs.realpathSync(entry)).href;
    const selfUrl = pathToFileURL(fs.realpathSync(fileURLToPath(import.meta.url))).href;
    return process.platform === 'win32'
      ? entryUrl.toLowerCase() === selfUrl.toLowerCase()
      : entryUrl === selfUrl;
  } catch {
    return false;
  }
}

if (isDirectInvocation()) {
  process.exit(main(process.argv.slice(2)));
}
