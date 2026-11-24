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
  release: 'target',
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

/** 并发预算包装器（真源 `.dev-team/build-budget.json`）。 */
export const LANE_SCRIPT_RELATIVE = path.join('scripts', 'cargo-lane.sh');

/**
 * 把一次 cargo 调用包成 lane 调用：`bash scripts/cargo-lane.sh <cargo 子命令> <参数...>`。
 * 纯函数（不读环境、不执行进程），便于 parity 用例断言。
 */
export function buildLaneInvocation(invocation, repoRoot = REPO_ROOT, options = {}) {
  const { heavy = false } = options;
  const lanePrefix = heavy ? ['--heavy'] : [];
  return {
    command: 'bash',
    cwd: invocation.cwd ?? repoRoot,
    args: [path.join(repoRoot, LANE_SCRIPT_RELATIVE), ...lanePrefix, ...invocation.args]
  };
}

/**
 * 是否经 lane 执行。返回 true 的条件：预算配置存在 且 未被 `PONY_BUILD_LANE=0` 关闭 且
 * 该平台可获得 bash（win32 默认不做假设，需显式 `PONY_BUILD_LANE=1`）。
 */
export function shouldUseLane(options = {}) {
  const {
    env = process.env,
    platform = process.platform,
    repoRoot = REPO_ROOT,
    fileExists = fs.existsSync
  } = options;
  const flag = env.PONY_BUILD_LANE;
  if (flag === '0' || flag === 'false') return false;
  if (!fileExists(path.join(repoRoot, '.dev-team', 'build-budget.json'))) return false;
  if (platform === 'win32') return flag === '1' || flag === 'true';
  return flag !== '0';
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

  if (shouldUseLane()) {
    const heavy = slot === 'release' || cargoArgs.includes('--release');
    invocation = buildLaneInvocation(invocation, REPO_ROOT, { heavy });
    if (process.env.PONY_BUILD_VERBOSE === '1') {
      console.error(`[rust-target] 经 scripts/cargo-lane.sh${heavy ? ' --heavy' : ''} 执行（并发预算 + 负载熔断）`);
    }
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
