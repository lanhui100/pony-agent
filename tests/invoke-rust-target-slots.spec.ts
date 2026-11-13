import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { SLOT_TARGET_DIRS, buildCargoInvocation, usage } from '../scripts/invoke-rust-target.mjs';

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

/** 从 PowerShell 版调用器里抽取槽位表，用于拦截两份实现漂移。 */
function readPs1SlotMap(): Record<string, string> {
  const source = fs.readFileSync(path.join(repoRoot, 'scripts/invoke-rust-target.ps1'), 'utf8');
  const body = source.slice(source.indexOf('$targetMap'), source.indexOf('$targetDir'));
  const map: Record<string, string> = {};
  for (const line of body.split('\n')) {
    const match = line.match(/^\s*"?([\w-]+)"?\s*=\s*"([^"]+)"\s*$/);
    if (match) map[match[1]] = match[2];
  }
  return map;
}

describe('invoke-rust-target 槽位调用器', () => {
  it('mjs 与 ps1 的槽位表逐槽一致（漂移即红）', () => {
    expect(readPs1SlotMap()).toEqual({ ...SLOT_TARGET_DIRS });
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
});
