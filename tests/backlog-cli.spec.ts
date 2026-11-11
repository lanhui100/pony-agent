import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import {
  acquireLease,
  archiveFileFor,
  doctor,
  heartbeatLease,
  reapStaleLeases,
  releaseLease,
  ledgerPaths,
  lintAll,
  list,
  normalizeLegacy,
  readHighWater,
  readJsonl,
  record,
  resolve,
  resolveLedgerRoot,
  update,
  validateRecord,
  writeJsonl
} from '../scripts/backlog.mjs';

const SCRIPT = path.resolve('scripts/backlog.mjs');

let root: string;
let paths: ReturnType<typeof ledgerPaths>;

function seed(rows: any[]) {
  fs.mkdirSync(path.join(root, 'backlog'), { recursive: true });
  writeJsonl(paths.tasks, rows);
}

function baseRow(overrides: Record<string, unknown> = {}) {
  return {
    id: 'B056',
    category: 'Defect',
    priority: 'P1',
    status: 'todo',
    title: '示例任务',
    created_at: '2026-11-11T00:00:00.000Z',
    metadata: { fingerprint: 'fp-1', occurred_count: 1 },
    ...overrides
  };
}

beforeEach(() => {
  root = fs.mkdtempSync(path.join(os.tmpdir(), 'backlog-cli-'));
  paths = ledgerPaths(root);
});

afterEach(() => {
  fs.rmSync(root, { recursive: true, force: true });
});

describe('schema 与状态机（AC1）', () => {
  it('非法 category / 缺 title / ID 非 B\\d{3,} 均被判非法', () => {
    expect(validateRecord(baseRow({ category: 'Bug' }))).not.toEqual([]);
    expect(validateRecord(baseRow({ title: '  ' }))).not.toEqual([]);
    expect(validateRecord(baseRow({ id: 'B5' }))).not.toEqual([]);
    expect(validateRecord(baseRow({ weird: 1 }))).not.toEqual([]);
    expect(validateRecord(baseRow())).toEqual([]);
  });

  it('todo→done 直跳被拒，in_progress 缺 owner/touched_paths 被拒', () => {
    seed([baseRow()]);
    expect(() => update(paths, 'B056', { status: 'done' } as any)).toThrow(/resolve/);
    expect(() => update(paths, 'B056', { status: 'in_progress' })).toThrow(/owner/);
    expect(() => update(paths, 'B056', { status: 'in_progress', owner: 'lane-1' })).toThrow(/touched_paths/);
    const row = update(paths, 'B056', { status: 'in_progress', owner: 'lane-1', touchedPaths: ['crates/x'] });
    expect(row.status).toBe('in_progress');
    expect(update(paths, 'B056', { status: 'todo' }).status).toBe('todo');
  });

  it('done 记录缺 evidence/closed_at 判非法（不要求 closing_commit）', () => {
    const done = baseRow({ status: 'done' });
    expect(validateRecord(done)).not.toEqual([]);
    expect(validateRecord(baseRow({ status: 'done', evidence: 'ok', closed_at: '2026-11-11T01:00:00.000Z' }))).toEqual([]);
  });
});

describe('ID 分配与高水位（AC2）', () => {
  it('首条从 B056 起分配，且高水位单调不减', () => {
    const first = record(paths, { category: 'Task', title: 'a', fingerprint: 'f1' });
    expect(first.row.id).toBe('B056');
    const second = record(paths, { category: 'Task', title: 'b', fingerprint: 'f2' });
    expect(second.row.id).toBe('B057');
    expect(readHighWater(paths)).toBe(58);
  });

  it('归档后再分配不复用退役编号（负例）', () => {
    const { row } = record(paths, { category: 'Task', title: 'a', fingerprint: 'f1' });
    resolve(paths, row.id, 'done');
    expect(readHighWater(paths)).toBe(57);
    // 只删归档文件（高水位必须独立于归档存在，这是不复用的唯一前提）
    fs.rmSync(archiveFileFor(paths), { force: true });
    seed([]);
    const again = record(paths, { category: 'Task', title: 'b', fingerprint: 'f2' });
    expect(again.row.id).not.toBe(row.id);
  });
});

describe('闭环与归档（AC5 语义）', () => {
  it('resolve 归档后活跃池清空，重放不产生重复', () => {
    const { row } = record(paths, { category: 'Defect', title: 'x', fingerprint: 'fp' });
    const first = resolve(paths, row.id, '验证通过');
    expect(readJsonl(paths.tasks)).toHaveLength(0);
    expect(readJsonl(first.archiveFile)).toHaveLength(1);
    // 重放：把同一行塞回活跃池再 resolve 一次
    writeJsonl(paths.tasks, [row]);
    resolve(paths, row.id, '验证通过');
    expect(readJsonl(first.archiveFile)).toHaveLength(1);
    expect(doctor(paths).ok).toBe(true);
  });

  it('归档记录必须是 done（防脏写）', () => {
    const file = archiveFileFor(paths);
    writeJsonl(file, [baseRow({ status: 'todo' })]);
    expect(lintAll(paths).ok).toBe(false);
  });
});

describe('lint / doctor（AC4 语义）', () => {
  it('活跃池残留 done 被判违规', () => {
    seed([baseRow({ status: 'done', evidence: 'x', closed_at: '2026-11-11T01:00:00.000Z' })]);
    expect(lintAll(paths).ok).toBe(false);
    expect(doctor(paths).ok).toBe(false);
  });

  it('ID 跨活跃池与归档重复可被发现', () => {
    seed([baseRow()]);
    writeJsonl(archiveFileFor(paths), [baseRow({ status: 'done', evidence: 'y', closed_at: '2026-11-11T01:00:00.000Z' })]);
    const report = doctor(paths);
    expect(report.issues.some(issue => issue.includes('ID 重复'))).toBe(true);
  });

  it('高水位未越过最大编号会被告警', () => {
    seed([baseRow({ id: 'B090' })]);
    const report = doctor(paths);
    expect(report.issues.some(issue => issue.includes('高水位'))).toBe(true);
  });
});

describe('指纹去重与字段上限', () => {
  it('同 fingerprint 的活跃任务只累加发生次数', () => {
    record(paths, { category: 'Defect', title: 'x', fingerprint: 'dup' });
    const res = record(paths, { category: 'Defect', title: 'x', fingerprint: 'dup' });
    expect(res.type).toBe('dedup');
    expect(res.row.metadata.occurred_count).toBe(2);
    expect(list(paths)).toHaveLength(1);
  });

  it('triage_context 超过 8KiB 被拒', () => {
    const row = baseRow({ metadata: { fingerprint: 'f', triage_context: { blob: 'x'.repeat(9000) } } });
    expect(validateRecord(row).join()).toMatch(/超限/);
  });
});

describe('作用域解析（跨 worktree 单例）', () => {
  it('显式环境变量优先', () => {
    expect(resolveLedgerRoot({ PONY_BACKLOG_ROOT: root } as any, '/')).toBe(path.resolve(root));
  });

  it('无环境变量时回落到 git common dir 的父目录（主工作树）', () => {
    const repo = fs.mkdtempSync(path.join(os.tmpdir(), 'backlog-git-'));
    execFileSync('git', ['init', '-q'], { cwd: repo });
    expect(resolveLedgerRoot({} as any, repo)).toBe(fs.realpathSync(repo));
    fs.rmSync(repo, { recursive: true, force: true });
  });
});

describe('迁移规范化', () => {
  it('legacy 行重编号为 B056 起，done 行补 evidence/closed_at', () => {
    const legacy = [
      { id: 'ISSUE-1', status: 'resolved', title: 'a', priority: 'P1', fingerprint: 'f1', context: { command: 'x' } },
      { id: 'ISSUE-2', status: 'pending', title: 'b', priority: 'P2', fingerprint: 'f2' }
    ];
    const rows = normalizeLegacy(legacy, { startId: 56 });
    expect(rows.map(row => row.id)).toEqual(['B056', 'B057']);
    expect(rows[0].category).toBe('Defect');
    expect(rows[0].evidence).toBeTruthy();
    expect(rows[0].closed_at).toBeTruthy();
    expect(rows[1].status).toBe('todo');
    for (const row of rows) expect(validateRecord(row)).toEqual([]);
  });
});

describe('CLI 冒烟', () => {
  it('lint 在合法账本上 exit 0，在非法账本上 exit 1', () => {
    seed([baseRow()]);
    const ok = execFileSync(process.execPath, [SCRIPT, 'lint'], {
      cwd: path.resolve('.'),
      env: { ...process.env, PONY_BACKLOG_ROOT: root },
      encoding: 'utf8'
    });
    expect(ok).toMatch(/PASS/);

    seed([baseRow({ status: 'done', evidence: 'x', closed_at: '2026-11-11T01:00:00.000Z' })]);
    let code = 0;
    try {
      execFileSync(process.execPath, [SCRIPT, 'lint'], {
        cwd: path.resolve('.'),
        env: { ...process.env, PONY_BACKLOG_ROOT: root },
        encoding: 'utf8',
        stdio: 'pipe'
      });
    } catch (error: any) {
      code = error.status;
    }
    expect(code).toBe(1);
  });

  it('record 命令写入后可被 list 读出', () => {
    execFileSync(process.execPath, [SCRIPT, 'record', '--category', 'Task', '--title', '冒烟任务', '--fingerprint', 'smoke'], {
      cwd: path.resolve('.'),
      env: { ...process.env, PONY_BACKLOG_ROOT: root },
      encoding: 'utf8'
    });
    expect(readJsonl(paths.tasks)).toHaveLength(1);
  });
});

describe('任务级租约（单任务单会话，不同任务可并行）', () => {
  it('同一任务被两个会话抢占时第二个失败，同 owner 重入刷新心跳', () => {
    seed([baseRow()]);
    const lease = acquireLease(paths, 'B056', { owner: 'session-A' });
    expect(lease.owner).toBe('session-A');
    expect(() => acquireLease(paths, 'B056', { owner: 'session-B' })).toThrow(/已被 session-A 持有/);
    const again = acquireLease(paths, 'B056', { owner: 'session-A', now: Date.now() + 1000 });
    expect(again.heartbeat_at).not.toBe(lease.heartbeat_at);
  });

  it('不同任务可被不同会话并行持有', () => {
    seed([baseRow(), baseRow({ id: 'B057', metadata: { fingerprint: 'fp-2' } })]);
    expect(acquireLease(paths, 'B056', { owner: 'session-A' }).owner).toBe('session-A');
    expect(acquireLease(paths, 'B057', { owner: 'session-B' }).owner).toBe('session-B');
  });

  it('release 只能由持有者执行，释放后锁文件消失', () => {
    seed([baseRow()]);
    acquireLease(paths, 'B056', { owner: 'session-A' });
    expect(() => releaseLease(paths, 'B056', { owner: 'session-B' })).toThrow(/属于 session-A/);
    expect(releaseLease(paths, 'B056', { owner: 'session-A' }).released).toBe(true);
    expect(releaseLease(paths, 'B056', { owner: 'session-A' }).released).toBe(false);
  });

  it('会话停止后租约过期可被 reap 回收（不阻塞其它会话）', () => {
    seed([baseRow()]);
    acquireLease(paths, 'B056', { owner: 'dead-session', now: Date.now() - 3 * 60 * 60 * 1000 });
    const reaped = reapStaleLeases(paths, { maxAgeMs: 60 * 60 * 1000 });
    expect(reaped).toEqual(['B056']);
    expect(() => acquireLease(paths, 'B056', { owner: 'new-session' })).not.toThrow();
  });

  it('lint 发现记录 owner 与租约 owner 冲突时判违规', () => {
    seed([baseRow({
      status: 'in_progress',
      owner: 'session-A',
      touched_paths: ['crates/x'],
      leased_at: '2026-11-11T00:00:00.000Z',
      heartbeat_at: new Date().toISOString()
    })]);
    acquireLease(paths, 'B056', { owner: 'session-B' });
    const res = lintAll(paths);
    expect(res.ok).toBe(false);
    expect(res.problems.join('\n')).toMatch(/租约与记录 owner 冲突/);
  });

  it('in_progress 但缺少租约锁文件（疑似手改状态）判违规', () => {
    seed([baseRow({
      status: 'in_progress',
      owner: 'session-A',
      touched_paths: ['crates/x'],
      leased_at: new Date().toISOString(),
      heartbeat_at: new Date().toISOString()
    })]);
    expect(lintAll(paths).problems.join('\n')).toMatch(/缺少租约锁文件/);
  });
});
