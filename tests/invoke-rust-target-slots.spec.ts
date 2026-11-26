import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { SLOT_TARGET_DIRS, buildCargoInvocation, buildLaneInvocation, shouldUseLane, usage } from '../scripts/invoke-rust-target.mjs';

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

/**
 * 读取 PowerShell 版调用器源码。
 *
 * 契约已从「两份槽位表逐槽比对」升级为「ps1 退化成薄壳」：旧实现里 ps1 自带 `$targetMap`
 * 并裸调 cargo（不吃并发预算、不加全局 flock），与 .mjs 的漂移实测已让 Windows 侧绕过构建治理。
 * 现在槽位表只存在于 .mjs，因此这里断言「ps1 不再有第二份槽位表」——漂移这一类缺陷从根上消失。
 */
function readPs1Source(): string {
  return fs.readFileSync(path.join(repoRoot, 'scripts/invoke-rust-target.ps1'), 'utf8');
}

describe('invoke-rust-target 槽位调用器', () => {
  it('ps1 已收敛为薄壳：不含独立槽位表（漂移类别消失）', () => {
    const source = readPs1Source();
    expect(source).not.toContain('$targetMap');
    expect(source).not.toMatch(/target-(check|test|perf)/);
  });

  it('ps1 强制转派 mjs 单一真源（所有平台，含 Windows）', () => {
    const source = readPs1Source();
    expect(source).toContain('lib/posix-dispatch.ps1');
    expect(source).toContain("Invoke-PonyPosixTwin -Name 'invoke-rust-target'");
    expect(source).toContain('-AlwaysDelegate');
    expect(source).toContain('invoke-rust-target.mjs');
  });


  it('六个 cargo 槽位都映射到各自的 target 目录', () => {
    for (const [slot, targetDir] of Object.entries(SLOT_TARGET_DIRS)) {
      const invocation = buildCargoInvocation(slot, ['check'], '/repo');
      expect(invocation.args.slice(0, 5)).toEqual([
        'check',
        '--manifest-path',
        path.join('/repo', 'src-tauri', 'Cargo.toml'),
        '--target-dir',
        path.join('/repo', targetDir)
      ]);
      expect(invocation.cwd).toBe('/repo');
      expect(invocation.command).toBe('cargo');
    }
    expect(SLOT_TARGET_DIRS.release).toBe('target');
  });

  it('nextest 子命令将 manifest-path 与 target-dir 置于 nextest run 之后', () => {
    const invocation = buildCargoInvocation('test', ['nextest', 'run', '--profile', 'quick', '-p', 'pony-agent-core'], '/repo');
    expect(invocation.args).toEqual([
      'nextest',
      'run',
      '--manifest-path',
      path.join('/repo', 'src-tauri', 'Cargo.toml'),
      '--target-dir',
      path.join('/repo', 'target-test'),
      '--profile',
      'quick',
      '-p',
      'pony-agent-core'
    ]);
  });

  it('额外 cargo 参数原序透传（不重排、不加引号）', () => {
    const invocation = buildCargoInvocation('exact-b', [
      'test',
      '--package=pony-agent-core',
      '--no-fail-fast'
    ]);
    expect(invocation.args.slice(-2)).toEqual(['--package=pony-agent-core', '--no-fail-fast']);
    expect(invocation.args[0]).toBe('test');
  });

  it('未知槽位与缺子命令一律抛错，不静默回退默认 target 目录', () => {
    expect(() => buildCargoInvocation('bogus', ['check'])).toThrow(/未知槽位/);
    expect(() => buildCargoInvocation('check', [])).toThrow(/缺少 cargo 子命令/);
  });

  it('usage 列出全部槽位', () => {
    for (const slot of Object.keys(SLOT_TARGET_DIRS)) {
      expect(usage()).toContain(slot);
    }
  });

  it('package.json 的 cargo:* 与 cli:build 脚本统一走 mjs 且无裸 cargo 绕过', () => {
    const pkg = JSON.parse(fs.readFileSync(path.join(repoRoot, 'package.json'), 'utf8'));
    const scripts = pkg.scripts as Record<string, string>;
    const expected: Record<string, string> = {
      'cargo:check:shared': 'check check',
      'cargo:test:shared': 'test test',
      'cargo:test:exact': 'exact-a test',
      'cargo:test:exact:b': 'exact-b test',
      'cargo:test:exact:c': 'exact-c test',
      'cargo:check': 'check check',
      'cargo:test': 'test test',
      'cargo:test:lib': 'test test --lib',
      'cargo:test:regression':
        'test test -p pony-agent-core --test session_storage_suite --test runtime_suite --test tools_stop_suite',
      'conformance:test': 'test test -p pony-agent-core --test runtime_suite -- conformance'
    };
    for (const [name, slotAndSubcommand] of Object.entries(expected)) {
      expect(scripts[name]).toBe(`node scripts/invoke-rust-target.mjs ${slotAndSubcommand}`);
      expect(scripts[name]).not.toMatch(/powershell|pwsh/i);
    }
    expect(scripts['cli:build']).toBe(
      'node scripts/invoke-rust-target.mjs release build --release -p pony-tui -p pony-cli --bin pony --bin ponycli && node scripts/install-cli.mjs'
    );
  });

  it('包成 lane 调用时保留 cargo 参数原序，heavy 模式置于子命令之前', () => {
    const cargoInvocation = buildCargoInvocation('check', ['check', '--all-targets'], '/repo');
    const lane = buildLaneInvocation(cargoInvocation, '/repo');
    expect(lane.command).toBe('bash');
    expect(lane.args[0]).toBe(path.join('/repo', 'scripts', 'cargo-lane.sh'));
    expect(lane.args.slice(1)).toEqual(cargoInvocation.args);
    expect(lane.args).not.toContain('--target-dir=undefined');

    const releaseInvocation = buildCargoInvocation('release', ['build', '--release'], '/repo');
    const heavyLane = buildLaneInvocation(releaseInvocation, '/repo', { heavy: true });
    expect(heavyLane.args[0]).toBe(path.join('/repo', 'scripts', 'cargo-lane.sh'));
    expect(heavyLane.args[1]).toBe('--heavy');
    expect(heavyLane.args.slice(2)).toEqual(releaseInvocation.args);
  });

  it('lane 启用判定：预算缺失/显式关闭/win32 默认都不上 lane', () => {
    const exists = () => true;
    expect(shouldUseLane({ env: {}, platform: 'linux', fileExists: exists })).toBe(true);
    expect(shouldUseLane({ env: { PONY_BUILD_LANE: '0' }, platform: 'linux', fileExists: exists })).toBe(false);
    expect(shouldUseLane({ env: {}, platform: 'win32', fileExists: exists })).toBe(false);
    expect(shouldUseLane({ env: { PONY_BUILD_LANE: '1' }, platform: 'win32', fileExists: exists })).toBe(true);
    expect(shouldUseLane({ env: {}, platform: 'linux', fileExists: () => false })).toBe(false);
  });
});
