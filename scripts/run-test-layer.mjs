#!/usr/bin/env node
/**
 * scripts/run-test-layer.mjs —— L0–L3 编译与测试分层调度器（支持 nextest 自动探测与降级）。
 *
 * 用法：
 *   node scripts/run-test-layer.mjs <l0|l1|l2|l3|nextest> [--dry-run] [-- <透传参数...>]
 *
 * 层级契约：
 *   l0      秒级快回路：core --lib 类型/借用检查（check:fast）
 *   l1      模块单测：core --lib 单测（优先 nextest --profile quick，缺 nextest 降级 cargo test）+ 前端 typecheck
 *   l2      域收口：前端单测（test:unit）+ clippy 棘轮双基线 + core --lib（nextest --profile default）
 *   l3      全量回归：core 全量单元与集成（nextest --profile ci / cargo test --no-fail-fast）+ test:unit + build
 *   nextest 透传到 target-test 槽位执行 nextest run（无 cargo-nextest 时剥离 --profile 降级为 cargo test）
 */
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import process from 'node:process';
import { fileURLToPath, pathToFileURL } from 'node:url';

const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const NPM_BIN = process.platform === 'win32' ? 'npm.cmd' : 'npm';

export function resolveNextestBin(env = process.env, homeDir = os.homedir()) {
  const exe = process.platform === 'win32' ? 'cargo-nextest.exe' : 'cargo-nextest';
  const pathDirs = (env.PATH || '').split(path.delimiter).filter(Boolean);
  const candidates = [
    ...pathDirs.map((d) => path.join(d, exe)),
    path.join(homeDir, '.local', 'bin', exe),
    path.join(homeDir, '.cargo', 'bin', exe),
    '/usr/local/bin/' + exe
  ];
  for (const p of candidates) {
    try {
      if (fs.existsSync(p) && fs.statSync(p).isFile()) {
        return p;
      }
    } catch {
      // 忽略无权限或损坏的路径
    }
  }
  return null;
}

function stripNextestOnlyArgs(args) {
  const out = [];
  for (let i = 0; i < args.length; i += 1) {
    const a = args[i];
    if (a === '--profile' || a === '-P' || a === '-E' || a === '--filter-expr') {
      i += 1;
      continue;
    }
    if (a.startsWith('--profile=') || a.startsWith('--filter-expr=')) continue;
    out.push(a);
  }
  return out;
}

export function buildRustTestStep({ slot = 'test', profile = 'default', extraArgs = [], hasNextest }) {
  if (hasNextest) {
    return {
      command: 'node',
      args: [
        path.join('scripts', 'invoke-rust-target.mjs'),
        slot,
        'nextest',
        'run',
        '--profile',
        profile,
        ...extraArgs
      ]
    };
  }
  return {
    command: 'node',
    args: [
      path.join('scripts', 'invoke-rust-target.mjs'),
      slot,
      'test',
      ...stripNextestOnlyArgs(extraArgs)
    ]
  };
}

export function buildLayerSteps(layer, extraArgs = [], hasNextest = Boolean(resolveNextestBin())) {
  switch (layer) {
    case 'l0':
      return [{ command: NPM_BIN, args: ['run', 'check:fast'] }];
    case 'l1':
      return [
        buildRustTestStep({
          profile: 'quick',
          extraArgs: ['--workspace', '--exclude', 'pony-agent', '--lib', ...extraArgs],
          hasNextest
        }),
        { command: NPM_BIN, args: ['run', 'typecheck'] }
      ];
    case 'l2':
      return [
        { command: NPM_BIN, args: ['run', 'test:unit'] },
        { command: NPM_BIN, args: ['run', 'rust:lint'] },
        { command: NPM_BIN, args: ['run', 'rust:lint:tauri'] },
        buildRustTestStep({
          profile: 'default',
          extraArgs: ['--workspace', '--exclude', 'pony-agent', '--lib', ...extraArgs],
          hasNextest
        })
      ];
    case 'l3': {
      const rustArgs = hasNextest
        ? ['--workspace', '--exclude', 'pony-agent', ...extraArgs]
        : ['--workspace', '--exclude', 'pony-agent', '--no-fail-fast', ...extraArgs];
      return [
        buildRustTestStep({
          profile: 'ci',
          extraArgs: rustArgs,
          hasNextest
        }),
        { command: NPM_BIN, args: ['run', 'test:unit'] },
        { command: NPM_BIN, args: ['run', 'build'] }
      ];
    }
    case 'nextest':
      return [
        buildRustTestStep({
          profile: 'default',
          extraArgs,
          hasNextest
        })
      ];
    default:
      throw new Error(`未知分层 "${layer}"（可选：l0 | l1 | l2 | l3 | nextest）`);
  }
}

export function main(argv) {
  const dryRun = argv.includes('--dry-run');
  const sepIdx = argv.indexOf('--');
  const headArgs = sepIdx === -1 ? argv : argv.slice(0, sepIdx);
  const passArgs = sepIdx === -1 ? [] : argv.slice(sepIdx + 1);
  const layer = headArgs.find((a) => !a.startsWith('-'));
  const trailing = headArgs.filter((a) => a !== layer && a !== '--dry-run');

  if (!layer || headArgs.includes('--help') || headArgs.includes('-h')) {
    console.log('用法: node scripts/run-test-layer.mjs <l0|l1|l2|l3|nextest> [--dry-run] [-- <参数...>]');
    return layer ? 0 : 1;
  }

  const nextestBin = resolveNextestBin();
  const hasNextest = Boolean(nextestBin);

  let steps;
  try {
    steps = buildLayerSteps(layer, [...trailing, ...passArgs], hasNextest);
  } catch (err) {
    console.error(`[test-layer] ${err.message}`);
    return 1;
  }

  if (nextestBin) {
    console.error(`[test-layer] layer=${layer} nextest=${nextestBin}`);
  } else {
    console.error(`[test-layer] layer=${layer} nextest=未安装（自动降级为 cargo test）`);
  }

  const env = { ...process.env };
  const localBin = path.join(os.homedir(), '.local', 'bin');
  if (fs.existsSync(localBin) && !(env.PATH || '').split(path.delimiter).includes(localBin)) {
    env.PATH = `${localBin}${path.delimiter}${env.PATH || ''}`;
  }
  if (!process.stderr.isTTY && !env.NEXTEST_SHOW_PROGRESS) {
    env.NEXTEST_SHOW_PROGRESS = 'none';
  }

  for (const step of steps) {
    const rendered = [step.command, ...step.args].join(' ');
    if (dryRun) {
      console.log(rendered);
      continue;
    }
    console.error(`[test-layer] 执行: ${rendered}`);
    const res = spawnSync(step.command, step.args, {
      cwd: REPO_ROOT,
      env,
      stdio: 'inherit'
    });
    if (res.error) {
      console.error(`[test-layer] 执行失败: ${res.error.message}`);
      return 1;
    }
    if (res.status !== 0) {
      return res.status ?? 1;
    }
  }
  return 0;
}

function isDirectInvocation() {
  const entry = process.argv[1];
  if (!entry) return false;
  try {
    return fs.realpathSync(entry) === fs.realpathSync(fileURLToPath(import.meta.url));
  } catch {
    return false;
  }
}

if (isDirectInvocation()) {
  process.exit(main(process.argv.slice(2)));
}
