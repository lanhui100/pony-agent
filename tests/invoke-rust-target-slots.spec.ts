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


  it('五个 cargo 槽位都映射到各自的 target 目录', () => {
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

  it('package.json 的 cargo:*.shared 脚本走 mjs 且槽位名合法', () => {
    const pkg = JSON.parse(fs.readFileSync(path.join(repoRoot, 'package.json'), 'utf8'));
    const scripts = pkg.scripts as Record<string, string>;
    const expected: Record<string, string> = {
      'cargo:check:shared': 'check check',
      'cargo:test:shared': 'test test',
      'cargo:test:exact': 'exact-a test',
      'cargo:test:exact:b': 'exact-b test',
      'cargo:test:exact:c': 'exact-c test'
    };
    for (const [name, slotAndSubcommand] of Object.entries(expected)) {
      expect(scripts[name]).toBe(`node scripts/invoke-rust-target.mjs ${slotAndSubcommand}`);
      expect(scripts[name]).not.toMatch(/powershell|pwsh/i);
    }
  });

  it('包成 lane 调用时保留 cargo 参数原序，且不再直接 spawn cargo', () => {
    const cargoInvocation = buildCargoInvocation('check', ['check', '--all-targets'], '/repo');
    const lane = buildLaneInvocation(cargoInvocation, '/repo');
    expect(lane.command).toBe('bash');
    expect(lane.args[0]).toBe(path.join('/repo', 'scripts', 'cargo-lane.sh'));
    expect(lane.args.slice(1)).toEqual(cargoInvocation.args);
    expect(lane.args).not.toContain('--target-dir=undefined');
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
