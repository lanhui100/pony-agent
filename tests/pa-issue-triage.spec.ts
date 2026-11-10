import { execSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import {
  ARCHIVE_DIR,
  createPlan,
  ensureStorage,
  executeCli,
  ISSUES_FILE,
  listIssues,
  PLAN_FILE,
  readIssues,
  readPlan,
  recordIssue,
  resolveIssue,
  ROOT_DIR,
  updatePlanStep,
  writeIssues
} from '../scripts/pa-issue-triage.mjs';

const SCRIPT_PATH = path.resolve('scripts/pa-issue-triage.mjs');

function runCli(args: string): { stdout: string; status: number } {
  try {
    const stdout = execSync(`node ${SCRIPT_PATH} ${args}`, {
      encoding: 'utf8',
      stdio: ['pipe', 'pipe', 'pipe']
    });
    return { stdout, status: 0 };
  } catch (err: any) {
    return {
      stdout: err.stdout?.toString() || err.stderr?.toString() || '',
      status: err.status ?? 1
    };
  }
}

describe('pa-issue-triage (CLI, API & Skill Lifecycle Coverage)', () => {
  let backupIssues: string | null = null;
  let backupPlan: string | null = null;

  beforeEach(() => {
    if (fs.existsSync(ISSUES_FILE)) {
      backupIssues = fs.readFileSync(ISSUES_FILE, 'utf8');
    }
    if (fs.existsSync(PLAN_FILE)) {
      backupPlan = fs.readFileSync(PLAN_FILE, 'utf8');
    }
    if (fs.existsSync(ROOT_DIR)) {
      fs.rmSync(ROOT_DIR, { recursive: true, force: true });
    }
  });

  afterEach(() => {
    if (fs.existsSync(ROOT_DIR)) {
      fs.rmSync(ROOT_DIR, { recursive: true, force: true });
    }
    if (backupIssues !== null) {
      fs.mkdirSync(ROOT_DIR, { recursive: true });
      fs.writeFileSync(ISSUES_FILE, backupIssues, 'utf8');
    }
    if (backupPlan !== null) {
      fs.mkdirSync(ROOT_DIR, { recursive: true });
      fs.writeFileSync(PLAN_FILE, backupPlan, 'utf8');
    }
  });

  describe('1. 基础存储与去重指纹机制（Happy & Corner cases）', () => {
    it('ensureStorage 幂等建档', () => {
      ensureStorage();
      expect(fs.existsSync(ROOT_DIR)).toBe(true);
      expect(fs.existsSync(ARCHIVE_DIR)).toBe(true);
      expect(fs.existsSync(ISSUES_FILE)).toBe(true);
      expect(() => ensureStorage()).not.toThrow();
    });

    it('readIssues 容忍损坏行并不丢失正常记录', () => {
      ensureStorage();
      fs.writeFileSync(ISSUES_FILE, '{"id":"ok-1"}\nINVALID_TRASH\n{"id":"ok-2"}\n\n', 'utf8');
      const loaded = readIssues();
      expect(loaded.length).toBe(2);
      expect(loaded[0].id).toBe('ok-1');
      expect(loaded[1].id).toBe('ok-2');
    });

    it('recordIssue: 跨会话同指纹累加与上下文覆盖', () => {
      const res1 = recordIssue({
        sessionId: 'sess-alpha',
        priority: 'P0',
        title: '编译崩溃: rustc 101',
        fingerprint: 'core::panic_hop2',
        context: { error: 'signal 11' }
      });
      expect(res1.type).toBe('created');
      expect(res1.issue.occurred_count).toBe(1);

      const res2 = recordIssue({
        sessionId: 'sess-beta',
        priority: 'P0',
        title: '同类编译崩溃再次发生',
        fingerprint: 'core::panic_hop2',
        context: { error: 'signal 11 retry' }
      });
      expect(res2.type).toBe('dedup');
      expect(res2.issue.occurred_count).toBe(2);
      expect(res2.issue.last_session_id).toBe('sess-beta');
      expect(res2.issue.last_context).toEqual({ error: 'signal 11 retry' });
    });

    it('recordIssue 错误防御：非法 session_id 抛错', () => {
      expect(() => recordIssue({ sessionId: null as any })).toThrow('无效的 session_id');
      expect(() => recordIssue({ sessionId: '' as any })).toThrow('无效的 session_id');
    });
  });

  describe('2. 优先级排序与聚合调度场景', () => {
    it('listIssues 严格按 P0 > P1 > P2 > P3 升序排期，未识别排在末尾', () => {
      recordIssue({ sessionId: 's', priority: 'P2', title: 't2', fingerprint: 'fp-2' });
      recordIssue({ sessionId: 's', priority: 'CUSTOM', title: 'tc', fingerprint: 'fp-c' });
      recordIssue({ sessionId: 's', priority: 'P0', title: 't0', fingerprint: 'fp-0' });
      recordIssue({ sessionId: 's', priority: 'P3', title: 't3', fingerprint: 'fp-3' });
      recordIssue({ sessionId: 's', priority: 'P1', title: 't1', fingerprint: 'fp-1' });

      const sorted = listIssues();
      expect(sorted.map(i => i.priority)).toEqual(['P0', 'P1', 'P2', 'P3', 'CUSTOM']);
    });
  });

  describe('3. 计划编排与断点续做场景（Plan & Checkpointer）', () => {
    it('createPlan 错误场景：空目标列表拒绝创建', () => {
      expect(() => createPlan([], '无目标')).toThrow('创建计划必须包含至少一个 target_issue');
    });

    it('createPlan 成功并初始化可持久化步骤断点', () => {
      const { issue: item1 } = recordIssue({ sessionId: 's', priority: 'P0', title: 't1', fingerprint: 'fp-1' });
      const { issue: item2 } = recordIssue({ sessionId: 's', priority: 'P1', title: 't2', fingerprint: 'fp-2' });

      const plan = createPlan([item1.id, item2.id], '批次修复两个缺陷', ['步骤 1', '步骤 2']);
      expect(plan.target_issues).toEqual([item1.id, item2.id]);
      expect(plan.steps.length).toBe(2);
      expect(plan.steps[0].status).toBe('pending');

      const issues = readIssues();
      expect(issues.find(i => i.id === item1.id)?.status).toBe('in_progress');
      expect(issues.find(i => i.id === item2.id)?.status).toBe('in_progress');
    });

    it('updatePlanStep 成功流转断点并记录机器客观凭证', () => {
      const { issue } = recordIssue({ sessionId: 's', priority: 'P1', title: 't', fingerprint: 'fp' });
      createPlan([issue.id], '测试断点推进', ['定位问题', '跑单测通过']);

      // 推进第一步
      const p1 = updatePlanStep(1, { status: 'completed', evidence: '定位在第 30 行', sessionId: 'sess-step-1' });
      expect(p1.steps[0].status).toBe('completed');
      expect(p1.steps[0].evidence).toBe('定位在第 30 行');
      expect(p1.status).toBe('in_progress'); // 尚有未完成步骤

      // 推进第二步
      const p2 = updatePlanStep(2, { status: 'completed', evidence: 'vitest run exitCode=0', sessionId: 'sess-step-2' });
      expect(p2.steps[1].status).toBe('completed');
      expect(p2.status).toBe('completed'); // 全步骤已完成自动闭环计划
    });

    it('updatePlanStep 错误场景：无计划或非法 step_id 防御', () => {
      expect(() => updatePlanStep(1)).toThrow('当前没有活动计划');

      const { issue } = recordIssue({ sessionId: 's', priority: 'P1', title: 't', fingerprint: 'fp' });
      createPlan([issue.id], '目标');
      expect(() => updatePlanStep(999)).toThrow('未找到步骤 step_id: 999');
    });
  });

  describe('4. 证据闭环与归档清理场景（Resolution & Evidence Verification）', () => {
    it('resolveIssue 错误场景：缺 ID 或缺机器客观凭证必须拒绝', () => {
      expect(() => resolveIssue('', 'evidence')).toThrow('必须指定 issueId');
      expect(() => resolveIssue('ISSUE-999', '')).toThrow('必须提供解决机器客观凭证 (evidence)');
      expect(() => resolveIssue('ISSUE-999', '   ')).toThrow('必须提供解决机器客观凭证 (evidence)');
      expect(() => resolveIssue('NON_EXISTENT', 'exitCode=0')).toThrow('未找到 ID: NON_EXISTENT');
    });

    it('resolveIssue 成功闭环：从活动待办移除并落入 archive 归档', () => {
      const { issue } = recordIssue({ sessionId: 's', priority: 'P1', title: '已解问题', fingerprint: 'fp-res' });
      const { resolvedItem, archiveFile } = resolveIssue(issue.id, 'PASS vitest run - exitCode 0');

      expect(resolvedItem.status).toBe('resolved');
      expect(resolvedItem.evidence).toBe('PASS vitest run - exitCode 0');
      expect(fs.existsSync(archiveFile)).toBe(true);

      const archivedContent = fs.readFileSync(archiveFile, 'utf8');
      expect(archivedContent).toContain(issue.id);
      expect(archivedContent).toContain('PASS vitest run');

      // 活动待办被物理清理出 issues.jsonl
      const activeIssues = readIssues();
      expect(activeIssues.find(i => i.id === issue.id)).toBeUndefined();
    });
  });

  describe('5. 端到端命令行 CLI 场景全覆盖', () => {
    it('CLI 正常闭环：record -> list -> plan-create -> plan-step-update -> plan-status -> resolve', () => {
      // 1. 记录
      const rec = runCli('record test-sess-99 P0 "死锁问题" "fp-deadlock" \'{"stack":"thread lock"}\'');
      expect(rec.status).toBe(0);
      expect(rec.stdout).toContain('已记录新问题');

      const lines = fs.readFileSync(ISSUES_FILE, 'utf8').trim().split('\n');
      const issueId = JSON.parse(lines[0]).id;

      // 2. 查列表
      const list = runCli('list');
      expect(list.status).toBe(0);
      expect(list.stdout).toContain(issueId);
      expect(list.stdout).toContain('[P0]');

      // 3. 制定计划
      const planRes = runCli(`plan-create "${issueId}" "攻坚死锁"`);
      expect(planRes.status).toBe(0);
      expect(planRes.stdout).toContain('计划已建立');

      // 4. 推进断点步骤
      const stepRes = runCli('plan-step-update 1 completed "锁定 mutex 释放点" test-sess-99');
      expect(stepRes.status).toBe(0);
      expect(stepRes.stdout).toContain('步骤 1 更新成功');

      // 5. 查看断点状态
      const statusRes = runCli('plan-status');
      expect(statusRes.status).toBe(0);
      expect(statusRes.stdout).toContain('锁定 mutex 释放点');

      // 6. 凭证闭环归档
      const resolveRes = runCli(`resolve ${issueId} "mutex_test passed without timeout"`);
      expect(resolveRes.status).toBe(0);
      expect(resolveRes.stdout).toContain('已闭环并归档');

      // 7. 再次查看列表应为空
      const listAfter = runCli('list');
      expect(listAfter.stdout).toContain('当前无活动待办');
    });

    it('CLI 异常拦截与防御：非零退出码与中文报错提示', () => {
      // 未知命令
      const badCmd = runCli('unknown-action');
      expect(badCmd.status).toBe(1);
      expect(badCmd.stdout).toContain('用法:');

      // 缺失 resolve 凭证
      const badResolve = runCli('resolve ISSUE-NON-EXISTENT');
      expect(badResolve.status).toBe(1);
      expect(badResolve.stdout).toContain('必须提供解决机器客观凭证');

      // 非法 plan-create 目标
      const badPlan = runCli('plan-create');
      expect(badPlan.status).toBe(1);
      expect(badPlan.stdout).toContain('必须包含至少一个 target_issue');
    });
  });
});
