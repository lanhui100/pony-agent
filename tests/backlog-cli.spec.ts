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

function lockFileOf(id: string) {
  return path.join(paths.locksDir, `${id}.lock`);
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
  it('新账本首条从 B001 起分配（无硬编码下限），且高水位单调不减', () => {
    const first = record(paths, { category: 'Task', title: 'a', fingerprint: 'f1' });
    expect(first.row.id).toBe('B001');
    const second = record(paths, { category: 'Task', title: 'b', fingerprint: 'f2' });
    expect(second.row.id).toBe('B002');
    expect(readHighWater(paths)).toBe(3);
  });

  it('归档后再分配不复用退役编号（负例）', () => {
    const { row } = record(paths, { category: 'Task', title: 'a', fingerprint: 'f1' });
    resolve(paths, row.id, 'done');
    expect(readHighWater(paths)).toBe(2);
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

  it('reap --id 缺值/空值/非法值 fail-closed，不回落为全局清锁', () => {
    seed([baseRow({ status: 'in_progress', owner: 'lane-A', touched_paths: ['crates/x'] })]);
    acquireLease(paths, 'B056', { owner: 'lane-A' });
    for (const args of [
      ['reap', '--id', '', '--minutes', '0'],
      ['reap', '--id', '--minutes', '0'],
      ['reap', '--id', 'B9', '--minutes', '0'],
      ['reap', '--id=B056', '--minutes', '0'],
      ['reap', '--ide', 'B056', '--minutes', '0'],
      ['reap', '-id', 'B056', '--minutes', '0'],
      ['reap', 'B056', '--minutes', '0'],
      ['reap', '--ID', 'B056', '--minutes', '0'],
      ['reap', '--minutes', 'abc']
    ]) {
      let code = 0;
      try {
        execFileSync(process.execPath, [SCRIPT, ...args], {
          cwd: path.resolve('.'),
          env: { ...process.env, PONY_BACKLOG_ROOT: root },
          encoding: 'utf8',
          stdio: 'pipe'
        });
      } catch (error: any) {
        code = error.status;
      }
      expect(code).toBe(2);
    }
    expect(fs.existsSync(lockFileOf('B056'))).toBe(true);
  });

  it('reap --dry-run 只演练不删锁，输出自证 dry-run', () => {
    seed([baseRow()]);
    acquireLease(paths, 'B056', { owner: 'lane-A' });
    const out = execFileSync(process.execPath, [SCRIPT, 'reap', '--dry-run', '--minutes', '0'], {
      cwd: path.resolve('.'),
      env: { ...process.env, PONY_BACKLOG_ROOT: root },
      encoding: 'utf8'
    });
    expect(out).toMatch(/\[dry-run\]/);
    expect(out).toMatch(/未执行|无超龄租约/);
    expect(fs.existsSync(lockFileOf('B056'))).toBe(true);

    const real = execFileSync(process.execPath, [SCRIPT, 'reap', '--minutes', '0'], {
      cwd: path.resolve('.'),
      env: { ...process.env, PONY_BACKLOG_ROOT: root },
      encoding: 'utf8'
    });
    expect(real).not.toMatch(/\[dry-run\]/);
    expect(fs.existsSync(lockFileOf('B056'))).toBe(false);
  });

  it('record 命令写入后可被 list 读出', () => {
    execFileSync(process.execPath, [SCRIPT, 'record', '--category', 'Task', '--title', '冒烟任务', '--fingerprint', 'smoke'], {
      cwd: path.resolve('.'),
      env: { ...process.env, PONY_BACKLOG_ROOT: root },
      encoding: 'utf8'
    });
    expect(readJsonl(paths.tasks)).toHaveLength(1);
  });

  // 回归：governance CI 曾恒红——闭环「最高编号任务」后未回写 `.next-id`，高水位落后于
  // 「活跃∪归档」最大编号，lint 立刻判「ID 越过高水位（疑似手工改号）」（实测 B066 案例）。
  it('resolve 闭环最高编号任务后推进高水位，lint 不得判越界', () => {
    seed([baseRow({ id: 'B066', status: 'in_progress', owner: 'session-A', touched_paths: ['scripts/backlog.mjs'] })]);
    fs.writeFileSync(paths.nextId, '66\n');
    expect(lintAll(paths).ok).toBe(false); // 前置：B066 与高水位 66 撞线，先红
    resolve(paths, 'B066', 'fix(backlog): 演示闭环证据');
    expect(readHighWater(paths)).toBe(67);
    const result = lintAll(paths);
    expect(result.ok).toBe(true);
    expect(result.problems.join()).not.toMatch(/越过高水位/);
  });

  // 回归（B081）：resolve 只归档不清锁 ⇒ `.locks/<ID>.lock` 残留到 TTL 被 reap，
  // 期间该 ID 对其它会话是「被占用」的假信号。库与 CLI 同一路径，此处断言可观测输出。
  it('CLI 全链路：start 建锁 → resolve 清锁并打印回收信息', () => {
    seed([baseRow()]);
    fs.writeFileSync(paths.nextId, '57\n');
    const env = { ...process.env, PONY_BACKLOG_ROOT: root };
    const started = execFileSync(process.execPath, [SCRIPT, 'start', 'B056', '--owner', 'cli-session', '--paths', 'scripts/backlog.mjs'], {
      cwd: path.resolve('.'),
      env,
      encoding: 'utf8'
    });
    expect(started).toMatch(/in_progress/);
    expect(fs.existsSync(lockFileOf('B056'))).toBe(true);

    const closed = execFileSync(process.execPath, [SCRIPT, 'resolve', 'B056', '--evidence', 'fix(backlog): 闭环释放租约'], {
      cwd: path.resolve('.'),
      env,
      encoding: 'utf8'
    });
    expect(closed).toMatch(/租约已释放（原持有者 @cli-session）/);
    expect(fs.existsSync(lockFileOf('B056'))).toBe(false);
    expect(doctor(paths).ok).toBe(true);
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

  it('reap --id 只回收指定任务，不误伤其它并发租约', () => {
    seed([baseRow(), baseRow({ id: 'B057', title: '另一任务' })]);
    acquireLease(paths, 'B056', { owner: 'lane-A', now: Date.now() - 3 * 60 * 60 * 1000 });
    acquireLease(paths, 'B057', { owner: 'lane-B' });
    const reaped = reapStaleLeases(paths, { maxAgeMs: 60 * 60 * 1000, onlyId: 'B056' });
    expect(reaped).toEqual(['B056']);
    expect(fs.existsSync(lockFileOf('B057'))).toBe(true);
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

  it('存在租约目录时，in_progress 缺锁文件（疑似手改状态）判违规', () => {
    seed([baseRow({
      status: 'in_progress',
      owner: 'session-A',
      touched_paths: ['crates/x'],
      leased_at: new Date().toISOString(),
      heartbeat_at: new Date().toISOString()
    })]);
    fs.mkdirSync(paths.locksDir, { recursive: true });
    expect(lintAll(paths).problems.join('\n')).toMatch(/缺少租约锁文件/);
  });

  it('锁存在但记录不是 in_progress 判违规（手改降级 / start 中断）', () => {
    seed([baseRow()]);
    acquireLease(paths, 'B056', { owner: 'session-A' });
    const res = lintAll(paths);
    expect(res.ok).toBe(false);
    expect(res.problems.join('\n')).toMatch(/存在租约锁但记录状态/);
    expect(doctor(paths).ok).toBe(false);
  });

  it('干净克隆（无 .locks 目录）下同一记录只提示不阻断（B069）', () => {
    seed([baseRow({
      status: 'in_progress',
      owner: 'session-A',
      touched_paths: ['crates/x'],
      leased_at: new Date().toISOString(),
      heartbeat_at: new Date().toISOString()
    })]);
    const res = lintAll(paths);
    expect(res.problems).toEqual([]);
    expect(res.ok).toBe(true);
    expect(res.notes.join('\n')).toMatch(/无本地租约目录/);
  });

  it('doctor 在存在 in_progress 任务时不崩溃，并复用 lint 的租约判据（B069）', () => {
    seed([baseRow({
      status: 'in_progress',
      owner: 'session-A',
      touched_paths: ['crates/x'],
      leased_at: new Date().toISOString(),
      heartbeat_at: new Date().toISOString()
    })]);
    fs.mkdirSync(paths.locksDir, { recursive: true });
    expect(() => doctor(paths)).not.toThrow();
    expect(doctor(paths).issues.join('\n')).toMatch(/缺少租约锁文件/);
  });
});

// B081：闭环是终态，租约必须随之失效——否则 `.locks/<ID>.lock` 残留到 TTL，
// 期间挡住 ID 复用，且 `.locks/` 常驻让 lint 永久走严格支（B069 的分流语义腐化）。
describe('闭环释放租约（B081）', () => {
  it('resolve 归档成功后释放租约，锁文件不残留', () => {
    seed([baseRow()]);
    acquireLease(paths, 'B056', { owner: 'session-A' });
    expect(fs.existsSync(lockFileOf('B056'))).toBe(true);

    const { lease } = resolve(paths, 'B056', '验证通过');
    expect(lease.released).toBe(true);
    expect(lease.previous.owner).toBe('session-A');
    expect(fs.existsSync(lockFileOf('B056'))).toBe(false);
  });

  it('无租约时 resolve 幂等：不抛错，reported released=false', () => {
    seed([baseRow()]);
    const { lease } = resolve(paths, 'B056', '验证通过');
    expect(lease.released).toBe(false);
    expect(readJsonl(paths.tasks)).toHaveLength(0);
  });

  it('他人持有的租约随闭环一并回收（闭环是终态，不因 owner 不同而卡住）', () => {
    seed([baseRow({ owner: 'session-A' })]);
    acquireLease(paths, 'B056', { owner: 'session-B' });
    expect(() => resolve(paths, 'B056', '验证通过')).not.toThrow();
    expect(fs.existsSync(lockFileOf('B056'))).toBe(false);
  });

  it('doctor 把「无活跃记录」的残留锁判为 issue（机器可发现）', () => {
    seed([baseRow({ id: 'B057', metadata: { fingerprint: 'fp-2' } })]);
    acquireLease(paths, 'B056', { owner: 'dead-session' }); // 活跃池无 B056 ⇒ 残留
    expect(doctor(paths).issues.join('\n')).toMatch(/残留租约 B056\.lock/);

    acquireLease(paths, 'B057', { owner: 'session-A' }); // 有活跃记录 ⇒ 不算残留
    expect(doctor(paths).issues.join('\n')).not.toMatch(/残留租约 B057\.lock/);
  });
});
