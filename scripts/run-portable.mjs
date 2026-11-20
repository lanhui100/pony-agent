#!/usr/bin/env node
/**
 * scripts/run-portable.mjs —— 跨平台任务分派器（消除 npm script 对 PowerShell 的硬依赖）。
 *
 * 背景：仓库长期在 Windows + MSVC 上开发，`clean`/`dev:tauri`/`schemas:*` 一律写死
 * `powershell -File *.ps1`；在 Linux（本机 dev server / CI）上这些 script 直接失败，
 * 于是「换个平台跑一遍」变成手工拼命令 —— 这是「交叉编译/跨平台麻烦」的主要来源之一。
 *
 * 契约：任务名 → 各平台 argv 一一登记；未登记任务 / 无该平台实现 = 明确失败（exit 1），
 * 不做静默回退（静默回退会让人以为清理成功）。
 *
 * 用法：node scripts/run-portable.mjs <task> [--dry-run]
 */
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import process from 'node:process';
import { fileURLToPath, pathToFileURL } from 'node:url';

const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

const PS = (script, ...args) => [
  'powershell',
  '-NoProfile',
  '-ExecutionPolicy',
  'Bypass',
  '-File',
  script,
  ...args
];
const SH = (script, ...args) => ['bash', script, ...args];

/** 任务表：win32 / posix 各一条 argv。新增任务必须两平台都登记，否则用例红。 */
export const TASKS = Object.freeze({
  'clean:light': {
    win32: PS('scripts/clean-targets.ps1'),
    posix: SH('scripts/clean-targets.sh')
  },
  'clean:deep': {
    win32: PS('scripts/clean-targets.ps1', '-Deep'),
    posix: SH('scripts/clean-targets.sh', '--deep')
  },
  'clean:incremental': {
    win32: PS('scripts/clean-targets.ps1', '-Incremental'),
    posix: SH('scripts/clean-targets.sh', '--incremental')
  },
  'clean:frontend': {
    win32: PS('scripts/clean-targets.ps1', '-WithFrontend'),
    posix: SH('scripts/clean-targets.sh', '--with-frontend')
  },
  'dev:tauri': {
    win32: PS('scripts/start-tauri-dev.ps1'),
    posix: SH('scripts/start-tauri-dev.sh')
  },
  'schemas:normalize': {
    win32: PS('scripts/normalize-generated-schemas.ps1'),
    posix: SH('scripts/normalize-generated-schemas.sh')
  },
  'schemas:watch': {
    win32: PS('scripts/normalize-generated-schemas.ps1', '-Watch'),
    posix: SH('scripts/normalize-generated-schemas.sh', '--watch')
  }
});

export function usage() {
  return `usage: node scripts/run-portable.mjs <${Object.keys(TASKS).join('|')}> [--dry-run]`;
}

/** 纯函数：按平台解析出 { command, args }；未登记项抛错（不静默回退）。 */
export function resolveTask(task, platform = process.platform) {
  const entry = TASKS[task];
  if (!entry) throw new Error(`未知任务 "${task}"（可选：${Object.keys(TASKS).join(' / ')}）`);
  const argv = platform === 'win32' ? entry.win32 : entry.posix;
  if (!Array.isArray(argv) || argv.length === 0) {
    throw new Error(`任务 "${task}" 在平台 ${platform} 上没有实现`);
  }
  return { command: argv[0], args: argv.slice(1) };
}

/** 登记的命令若指向仓库内文件，必须真实存在（防登记了不存在的脚本还静默跑）。 */
export function missingScript(task, platform = process.platform, repoRoot = REPO_ROOT) {
  const entry = TASKS[task];
  if (!entry) return null;
  const argv = platform === 'win32' ? entry.win32 : entry.posix;
  const scriptArg = (argv ?? []).find(
    (token) => token.startsWith('scripts/') || token.startsWith('scripts\\')
  );
  if (!scriptArg) return null;
  const abs = path.join(repoRoot, scriptArg);
  return fs.existsSync(abs) ? null : abs;
}

export function main(argv) {
  const dryRun = argv.includes('--dry-run');
  const task = argv.find((token) => !token.startsWith('-'));
  if (!task) {
    console.error(usage());
    return 1;
  }

  let invocation;
  try {
    invocation = resolveTask(task);
  } catch (error) {
    console.error(`[portable] ${error.message}`);
    console.error(usage());
    return 1;
  }

  const missing = missingScript(task);
  if (missing) {
    console.error(`[portable] 登记的目标脚本不存在：${path.relative(REPO_ROOT, missing)}`);
    return 1;
  }

  if (dryRun) {
    console.log([invocation.command, ...invocation.args].join(' '));
    return 0;
  }

  const result = spawnSync(invocation.command, invocation.args, {
    cwd: REPO_ROOT,
    stdio: 'inherit'
  });
  if (result.error) {
    if (result.error.code === 'ENOENT') {
      console.error(`[portable] 找不到命令 ${invocation.command}（该平台未安装？）`);
      return 127;
    }
    console.error(`[portable] ${result.error.message}`);
    return 1;
  }
  if (result.signal) {
    console.error(`[portable] 被信号 ${result.signal} 终止`);
    return 1;
  }
  return result.status ?? 1;
}

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
