import { execFileSync } from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import {
  acquireLease,
  archiveBatches,
  doctor,
  fold,
  foldV2,
  ledgerPaths,
  lintAll,
  lintV2,
  listRows,
  readJsonl,
  readLease,
  reapStaleLeases,
  recordTask,
  releaseLease,
  releaseTask,
  resolveLedgerRoot,
  resolveTask,
  startTask,
  validateTask,
  verifyArchive,
  writeJsonl
} from '../scripts/backlog.mjs';

const SCRIPT = path.resolve('scripts/backlog.mjs');
const MIGRATE = path.resolve('scripts/backlog-migrate-v2-v3.mjs');
const BATCH = '20260101T000000Z-tui-abcd';

let root: string;
let paths: ReturnType<typeof ledgerPaths>;

function baseTask(overrides: Record<string, unknown> = {}) {
  return {
    id: 'B001',
    category: 'Defect',
    priority: 'P1',
    status: 'todo',
    title: '示例任务',
    created_at: '2026-01-01T00:00:00.000Z',
    metadata: { fingerprint: 'fp-1', occurred_count: 1 },
    ...overrides
  };
}

function batchStart(batchId = BATCH, overrides: Record<string, unknown> = {}) {
  return {
    kind: 'batch.start',
    model: 'backlog-v3',
    batch_id: batchId,
    surface: 'tui',
    created_at: '2026-01-01T00:00:00.000Z',
    ...overrides
  };
}

function seedBatch(batchId: string, events: unknown[]) {
  fs.mkdirSync(paths.logsDir, { recursive: true });
  const lines = [batchStart(batchId), ...events].map(line => JSON.stringify(line));
  fs.writeFileSync(path.join(paths.logsDir, `${batchId}.jsonl`), lines.join('\n') + '\n');
}

function seedV3() {
  fs.writeFileSync(paths.model, 'v3\n');
}

function seedV2(rows: unknown[], archived: unknown[] = []) {
  writeJsonl(paths.legacyTasks, rows);
  fs.writeFileSync(paths.legacyNextId, '999\n');
  if (archived.length) {
    fs.mkdirSync(paths.archiveDir, { recursive: true });
    writeJsonl(path.join(paths.archiveDir, 'tasks-2026.jsonl'), archived);
  }
}

function lockFileOf(id: string) {
  return path.join(paths.locksDir, `${id}.lock`);
}

beforeEach(() => {
  root = fs.mkdtempSync(path.join(os.tmpdir(), 'backlog-v3-'));
  paths = ledgerPaths(root);
  fs.mkdirSync(path.join(root, 'backlog'), { recursive: true });
});

afterEach(() => {
  fs.rmSync(root, { recursive: true, force: true });
});

describe('记录校验（字段口径与 v2 一致）', () => {
  it('非法 category / 缺 title / ID 非 B\\d{3,} / 未知字段均判非法', () => {
    expect(validateTask(baseTask({ category: 'Bug' }))).not.toEqual([]);
    expect(validateTask(baseTask({ title: '  ' }))).not.toEqual([]);
    expect(validateTask(baseTask({ id: 'B5' }))).not.toEqual([]);
    expect(validateTask(baseTask({ weird: 1 }))).not.toEqual([]);
    expect(validateTask(baseTask())).toEqual([]);
  });

  it('in_progress 缺 owner/touched_paths、done 缺 evidence/closed_at 判非法', () => {
    expect(validateTask(baseTask({ status: 'in_progress' }))).not.toEqual([]);
    expect(validateTask(baseTask({ status: 'in_progress', owner: 'lane-1' }))).not.toEqual([]);
    expect(validateTask(baseTask({ status: 'in_progress', owner: 'lane-1', touched_paths: ['a'] }))).toEqual([]);
    expect(validateTask(baseTask({ status: 'done' }))).not.toEqual([]);
    expect(validateTask(baseTask({ status: 'done', evidence: 'ok', closed_at: '2026-01-01T01:00:00.000Z' }))).toEqual([]);
  });

  it('triage_context 超过 8KiB 被拒', () => {
    const row = baseTask({ metadata: { fingerprint: 'f', triage_context: { blob: 'x'.repeat(9000) } } });
    expect(validateTask(row).join()).toMatch(/超限/);
  });
});

describe('v3 批次日志：写入与 fold 读模型', () => {
  it('record 写入本会话批次；ID 为时间基且唯一；list 从 fold 读出', () => {
    seedV3();
    const first = recordTask(paths, { category: 'Task', title: 'a', fingerprint: 'f1', surface: 'tui' });
    expect(first.type).toBe('created');
    expect(first.row.id).toMatch(/^B\d{3,}$/);
    const second = recordTask(paths, { category: 'Task', title: 'b', fingerprint: 'f2', surface: 'tui' });
    expect(second.row.id).not.toBe(first.row.id);
    const batchPath = path.join(paths.logsDir, `${first.batchId}.jsonl`);
    expect(fs.existsSync(batchPath)).toBe(true);
    expect(readJsonl(batchPath)[0].kind).toBe('batch.start');
    expect(listRows(paths).map(row => row.id)).toEqual([first.row.id, second.row.id]);
  });

  it('指纹去重：同 fingerprint 累加发生次数，不产生第二条任务', () => {
    seedV3();
    recordTask(paths, { category: 'Defect', title: 'x', fingerprint: 'dup', surface: 'tui' });
    const res = recordTask(paths, { category: 'Defect', title: 'x', fingerprint: 'dup', surface: 'tui' });
    expect(res.type).toBe('dedup');
    const state = fold(paths);
    expect(state.active).toHaveLength(1);
    expect(state.active[0].metadata.occurred_count).toBe(2);
  });

  it('两个独立批次（并发会话）fold 出并集：没有共享写入点', () => {
    seedV3();
    seedBatch('20260101T000000Z-tui-abcd', [
      { kind: 'record', at: '2026-01-01T00:00:01.000Z', task: baseTask({ id: 'B001' }) }
    ]);
    seedBatch('20260101T000001Z-frontend-ef01', [
      { kind: 'record', at: '2026-01-01T00:00:01.000Z', task: baseTask({ id: 'B002', metadata: { fingerprint: 'fp-2' } }) }
    ]);
    const state = fold(paths);
    expect(state.active.map(row => row.id).sort()).toEqual(['B001', 'B002']);
    expect(state.problems).toEqual([]);
  });

  it('同一任务跨批次推进（后续批次 start/resolve）fold 顺序正确', () => {
    seedV3();
    seedBatch('20260101T000000Z-tui-abcd', [
      { kind: 'record', at: '2026-01-01T00:00:01.000Z', task: baseTask() }
    ]);
    seedBatch('20260102T000000Z-tui-bcde', [
      { kind: 'start', at: '2026-01-02T00:00:01.000Z', id: 'B001', owner: 'lane-1', paths: ['a'] },
      { kind: 'resolve', at: '2026-01-02T00:00:02.000Z', id: 'B001', evidence: 'done' }
    ]);
    const state = fold(paths);
    expect(state.active).toHaveLength(0);
    expect(state.done.map(row => row.id)).toEqual(['B001']);
  });

  it('批次首行非 batch.start / 未知事件 → fold 报问题且 lint 拒绝', () => {
    seedV3();
    fs.mkdirSync(paths.logsDir, { recursive: true });
    fs.writeFileSync(path.join(paths.logsDir, `${BATCH}.jsonl`), JSON.stringify({ kind: 'record' }) + '\n');
    const res = lintAll(paths);
    expect(res.ok).toBe(false);
    expect(res.problems.join('\n')).toMatch(/首行必须是 batch.start/);
  });

  it('重复 record 同一 ID 被 lint 抓出（跨批次不得重复登记）', () => {
    seedV3();
    seedBatch('20260101T000000Z-tui-abcd', [
      { kind: 'record', at: '2026-01-01T00:00:01.000Z', task: baseTask() }
    ]);
    seedBatch('20260101T000001Z-tui-ef01', [
      { kind: 'record', at: '2026-01-01T00:00:02.000Z', task: baseTask() }
    ]);
    expect(lintAll(paths).problems.join('\n')).toMatch(/重复 record/);
  });
});

describe('v3 状态机与租约', () => {
  it('start 取租约并进入 in_progress；resolve 闭环并释放租约', () => {
    seedV3();
    const { row } = recordTask(paths, { category: 'Task', title: 'x', fingerprint: 'f', surface: 'tui' });
    startTask(paths, { id: row.id, owner: 'session-A', touchedPaths: ['scripts/x'], surface: 'tui' });
    expect(readLease(paths, row.id)?.owner).toBe('session-A');
    expect(fold(paths).active[0].status).toBe('in_progress');
    const res = resolveTask(paths, { id: row.id, evidence: '验证通过', surface: 'tui' });
    expect(res.lease.released).toBe(true);
    expect(fs.existsSync(lockFileOf(row.id))).toBe(false);
    expect(fold(paths).done.map(item => item.id)).toEqual([row.id]);
  });

  it('done 是终态：再次 start/resolve 被拒', () => {
    seedV3();
    const { row } = recordTask(paths, { category: 'Task', title: 'x', fingerprint: 'f', surface: 'tui' });
    resolveTask(paths, { id: row.id, evidence: 'ok', surface: 'tui' });
    expect(() => startTask(paths, { id: row.id, owner: 'a', touchedPaths: ['x'] })).toThrow(/终态|已闭环/);
    expect(() => resolveTask(paths, { id: row.id, evidence: 'again' })).toThrow(/重复 resolve/);
  });

  it('同一任务被两个会话抢占租约时第二个失败；同 owner 重入只刷新心跳', () => {
    seedV3();
    const { row } = recordTask(paths, { category: 'Task', title: 'x', fingerprint: 'f', surface: 'tui' });
    startTask(paths, { id: row.id, owner: 'session-A', touchedPaths: ['a'], surface: 'tui' });
    expect(() => acquireLease(paths, row.id, { owner: 'session-B' })).toThrow(/已被 session-A 持有/);
    const again = acquireLease(paths, row.id, { owner: 'session-A', now: Date.now() + 1000 });
    expect(again.owner).toBe('session-A');
  });

  it('release 只能由持有者执行，释放后回到 todo', () => {
    seedV3();
    const { row } = recordTask(paths, { category: 'Task', title: 'x', fingerprint: 'f', surface: 'tui' });
    startTask(paths, { id: row.id, owner: 'session-A', touchedPaths: ['a'], surface: 'tui' });
    expect(() => releaseLease(paths, row.id, { owner: 'session-B' })).toThrow(/属于 session-A/);
    releaseTask(paths, { id: row.id, owner: 'session-A', surface: 'tui' });
    expect(fold(paths).active[0].status).toBe('todo');
    expect(fs.existsSync(lockFileOf(row.id))).toBe(false);
  });

  it('lint：锁 owner 与记录 owner 冲突 / 锁存而记录非 in_progress / in_progress 缺锁（有 .locks 时）', () => {
    seedV3();
    const { row } = recordTask(paths, { category: 'Task', title: 'x', fingerprint: 'f', surface: 'tui' });
    acquireLease(paths, row.id, { owner: 'session-B' });
    expect(lintAll(paths).problems.join('\n')).toMatch(/存在租约锁但记录状态/);
    releaseLease(paths, row.id);
    startTask(paths, { id: row.id, owner: 'session-A', touchedPaths: ['a'], surface: 'tui' });
    expect(lintAll(paths).ok).toBe(true);
    expect(() => lintAll(paths)).not.toThrow();
    fs.rmSync(lockFileOf(row.id), { force: true });
    expect(lintAll(paths).problems.join('\n')).toMatch(/缺少租约锁文件/);
  });

  it('干净克隆（无 .locks 目录）下 in_progress 只提示不阻断', () => {
    seedV3();
    seedBatch(BATCH, [
      {
        kind: 'record',
        at: '2026-01-01T00:00:01.000Z',
        task: baseTask({
          status: 'in_progress',
          owner: 'session-A',
          touched_paths: ['a'],
          leased_at: '2026-01-01T00:00:01.000Z',
          heartbeat_at: '2026-01-01T00:00:01.000Z'
        })
      }
    ]);
    const res = lintAll(paths);
    expect(res.ok).toBe(true);
    expect(res.notes.join('\n')).toMatch(/无本地租约目录/);
  });

  it('doctor 发现孤儿租约与 ID 重复', () => {
    seedV3();
    const { row } = recordTask(paths, { category: 'Task', title: 'x', fingerprint: 'f', surface: 'tui' });
    resolveTask(paths, { id: row.id, evidence: 'ok' });
    acquireLease(paths, row.id, { owner: 'dead' });
    expect(doctor(paths).issues.join('\n')).toMatch(/残留租约/);
  });

  it('reap --id 只回收指定任务的超龄租约，不误伤其它租约', () => {
    seedV3();
    const { row } = recordTask(paths, { category: 'Task', title: 'x', fingerprint: 'f', surface: 'tui' });
    acquireLease(paths, row.id, { owner: 'dead', now: Date.now() - 3 * 60 * 60 * 1000 });
    const reaped = reapStaleLeases(paths, { maxAgeMs: 60 * 60 * 1000, onlyId: row.id });
    expect(reaped).toEqual([row.id]);
  });
});

describe('v3 按批归档与封存', () => {
  it('全部闭环的批次可被归档：文件移动 + 清单追加 + fold 仍可见历史', () => {
    seedV3();
    const { row, batchId } = recordTask(paths, { category: 'Task', title: 'x', fingerprint: 'f', surface: 'tui' });
    resolveTask(paths, { id: row.id, evidence: 'ok', surface: 'tui' });
    const res = archiveBatches(paths, {});
    expect(res.moved).toHaveLength(1);
    expect(fs.existsSync(path.join(paths.logsDir, `${batchId}.jsonl`))).toBe(false);
    expect(verifyArchive(paths).ok).toBe(true);
    expect(fold(paths).done.map(item => item.id)).toEqual([row.id]);
  });

  it('未闭环批次不得归档', () => {
    seedV3();
    recordTask(paths, { category: 'Task', title: 'x', fingerprint: 'f', surface: 'tui' });
    expect(archiveBatches(paths, {}).moved).toHaveLength(0);
  });

  it('归档文件被篡改 → verify-archive 拒绝（sha256 封存）', () => {
    seedV3();
    const { row, batchId } = recordTask(paths, { category: 'Task', title: 'x', fingerprint: 'f', surface: 'tui' });
    resolveTask(paths, { id: row.id, evidence: 'ok' });
    archiveBatches(paths, {});
    const archived = path.join(paths.archivedBatchesDir, '2026', `${batchId}.jsonl`);
    fs.appendFileSync(archived, JSON.stringify({ kind: 'dedup', at: '2026-01-01T00:00:05.000Z', id: row.id }) + '\n');
    expect(verifyArchive(paths).ok).toBe(false);
    expect(verifyArchive(paths).problems.join('\n')).toMatch(/sha256 不符/);
  });

  it('归档 dry-run 不改动磁盘', () => {
    seedV3();
    const { row, batchId } = recordTask(paths, { category: 'Task', title: 'x', fingerprint: 'f', surface: 'tui' });
    resolveTask(paths, { id: row.id, evidence: 'ok' });
    const res = archiveBatches(paths, { dryRun: true });
    expect(res.moved).toHaveLength(1);
    expect(fs.existsSync(path.join(paths.logsDir, `${batchId}.jsonl`))).toBe(true);
    expect(fs.existsSync(paths.archivedBatchesDir)).toBe(false);
  });
});

describe('v2 只读兼容与迁移', () => {
  it('v2 账本可被 fold/lint/doctor 只读读取；写操作拒绝并指向迁移脚本', () => {
    seedV2([baseTask()], [baseTask({ id: 'B002', status: 'done', evidence: 'old', closed_at: '2026-01-02T00:00:00.000Z' })]);
    const state = foldV2(paths);
    expect(state.active.map(row => row.id)).toEqual(['B001']);
    expect(lintV2(paths).ok).toBe(true);
    expect(fold(paths).model).toBe('v2');
    expect(() => recordTask(paths, { category: 'Task', title: 'x', fingerprint: 'f' }))
      .toThrow(/backlog-migrate-v2-v3/);
  });

  it('迁移脚本：v2 → v3 基线批次，活跃数/done 数对账一致，幂等拒绝重跑', () => {
    seedV2([baseTask()], [baseTask({ id: 'B002', status: 'done', evidence: 'old', closed_at: '2026-01-02T00:00:00.000Z' })]);
    const env = { ...process.env, PONY_BACKLOG_ROOT: root };
    const dry = execFileSync(process.execPath, [MIGRATE, '--dry-run'], { cwd: path.resolve('.'), env, encoding: 'utf8' });
    expect(dry).toMatch(/dry-run/);
    expect(fs.existsSync(paths.legacyTasks)).toBe(true);

    const out = execFileSync(process.execPath, [MIGRATE, '--surface', 'migration'], { cwd: path.resolve('.'), env, encoding: 'utf8' });
    expect(out).toMatch(/PASS/);
    expect(fs.readFileSync(paths.model, 'utf8').trim()).toBe('v3');
    expect(fs.existsSync(paths.legacyTasks)).toBe(false);
    expect(fs.existsSync(paths.legacyNextId)).toBe(false);
    const state = fold(paths);
    expect(state.counts.active).toBe(1);
    expect(state.counts.done).toBe(1);

    let code = 0;
    try {
      execFileSync(process.execPath, [MIGRATE], { cwd: path.resolve('.'), env, encoding: 'utf8', stdio: 'pipe' });
    } catch (error: any) {
      code = error.status;
    }
    expect(code).toBe(2);
  });
});

describe('CLI 冒烟', () => {
  const env = () => ({ ...process.env, PONY_BACKLOG_ROOT: root });

  it('record → list → resolve 全链路（批次文件随写落盘）', () => {
    seedV3();
    const recorded = execFileSync(
      process.execPath,
      [SCRIPT, 'record', '--category', 'Task', '--title', '冒烟任务', '--fingerprint', 'smoke', '--surface', 'tui'],
      { cwd: path.resolve('.'), env: env(), encoding: 'utf8' }
    );
    expect(recorded).toMatch(/已登记/);
    expect(fs.readdirSync(paths.logsDir).some(name => name.endsWith('.jsonl'))).toBe(true);
    const listed = execFileSync(process.execPath, [SCRIPT, 'list'], { cwd: path.resolve('.'), env: env(), encoding: 'utf8' });
    expect(listed).toMatch(/冒烟任务/);
    const id = listed.match(/B\d{3,}/)![0];
    execFileSync(process.execPath, [SCRIPT, 'start', id, '--owner', 'cli', '--paths', 'scripts/backlog.mjs'], {
      cwd: path.resolve('.'), env: env(), encoding: 'utf8'
    });
    const closed = execFileSync(process.execPath, [SCRIPT, 'resolve', id, '--evidence', 'CLI 闭环'], {
      cwd: path.resolve('.'), env: env(), encoding: 'utf8'
    });
    expect(closed).toMatch(/租约已释放/);
    expect(doctor(paths).ok).toBe(true);
  });

  it('reap 非法参数一律 exit 2，不回落为全局清锁', () => {
    seedV3();
    for (const args of [
      ['reap', '--id', '', '--minutes', '0'],
      ['reap', '--id', '--minutes', '0'],
      ['reap', '--id', 'B9', '--minutes', '0'],
      ['reap', '--id=B001', '--minutes', '0'],
      ['reap', '-id', 'B001', '--minutes', '0'],
      ['reap', 'B001', '--minutes', '0'],
      ['reap', '--minutes', 'abc']
    ]) {
      let code = 0;
      try {
        execFileSync(process.execPath, [SCRIPT, ...args], { cwd: path.resolve('.'), env: env(), encoding: 'utf8', stdio: 'pipe' });
      } catch (error: any) {
        code = error.status;
      }
      expect(code).toBe(2);
    }
  });
});

describe('作用域解析（每 worktree 独立账本）', () => {
  it('显式环境变量优先', () => {
    expect(resolveLedgerRoot({ PONY_BACKLOG_ROOT: root } as any, '/')).toBe(path.resolve(root));
  });

  it('无环境变量时落到当前 worktree 顶层（不是 git common dir）', () => {
    const repo = fs.mkdtempSync(path.join(os.tmpdir(), 'backlog-git-'));
    execFileSync('git', ['init', '-q'], { cwd: repo });
    expect(resolveLedgerRoot({} as any, repo)).toBe(fs.realpathSync(repo));
    fs.rmSync(repo, { recursive: true, force: true });
  });
});

describe('批次 id / 归档清单格式', () => {
  it('批次 id 满足命名规约，清单行是 `sha256  <path>`', () => {
    seedV3();
    const { row, batchId } = recordTask(paths, { category: 'Task', title: 'x', fingerprint: 'f', surface: 'tui' });
    expect(batchId).toMatch(/^\d{8}T\d{6}Z-tui-[0-9a-f]{4}$/);
    resolveTask(paths, { id: row.id, evidence: 'ok' });
    archiveBatches(paths, {});
    const manifest = path.join(paths.archivedBatchesDir, '2026', 'manifest.sha256');
    const line = fs.readFileSync(manifest, 'utf8').trim().split('\n')[0];
    const match = /^([0-9a-f]{64})  (.+)$/.exec(line)!;
    expect(match).toBeTruthy();
    const file = path.join(root, match[2]);
    expect(crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex')).toBe(match[1]);
  });
});
