import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { TASKS, resolveTask, missingScript } from '../scripts/run-portable.mjs';

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

describe('run-portable 跨平台任务分派', () => {
  it('每个任务在 win32 / posix 两侧都有实现（缺一侧即红）', () => {
    for (const [task, entry] of Object.entries(TASKS)) {
      expect(Array.isArray(entry.win32) && entry.win32.length > 0, `${task}.win32`).toBe(true);
      expect(Array.isArray(entry.posix) && entry.posix.length > 0, `${task}.posix`).toBe(true);
    }
  });

  it('登记的目标脚本真实存在（防登记了不存在的脚本静默跑）', () => {
    for (const task of Object.keys(TASKS)) {
      expect(missingScript(task, 'linux', repoRoot), `${task} posix`).toBeNull();
      expect(missingScript(task, 'win32', repoRoot), `${task} win32`).toBeNull();
    }
  });

  it('按平台解析 argv，且 posix 侧走 bash 而非 powershell', () => {
    const posix = resolveTask('clean:incremental', 'linux');
    expect(posix.command).toBe('bash');
    expect(posix.args).toEqual(['scripts/clean-targets.sh', '--incremental']);

    const win = resolveTask('clean:incremental', 'win32');
    expect(win.command).toBe('powershell');
    expect(win.args.join(' ')).toContain('scripts/clean-targets.ps1');
    expect(win.args.join(' ')).toContain('-Incremental');
  });

  it('package.json 里 clean / schemas / dev 入口不再写死 powershell', () => {
    const pkg = JSON.parse(fs.readFileSync(path.join(repoRoot, 'package.json'), 'utf8'));
    const scripts = pkg.scripts as Record<string, string>;
    for (const name of [
      'dev:tauri',
      'clean:tauri:light',
      'clean:tauri:deep',
      'clean:incremental',
      'schemas:normalize',
      'schemas:watch'
    ]) {
      expect(scripts[name], name).toContain('run-portable.mjs');
      expect(scripts[name], name).not.toMatch(/powershell|pwsh/i);
    }
  });

  it('未知任务明确失败，不静默回退', () => {
    expect(() => resolveTask('nope', 'linux')).toThrow(/未知任务/);
  });
});
