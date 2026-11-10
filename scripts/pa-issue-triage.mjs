#!/usr/bin/env node
/**
 * pa-issue-triage helper CLI
 * 提供会话异常捕获、指纹去重、聚合编排、断点续做与闭环归档
 */

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export const ROOT_DIR = path.resolve('.agents/issue-triage');
export const ISSUES_FILE = path.join(ROOT_DIR, 'issues.jsonl');
export const PLAN_FILE = path.join(ROOT_DIR, 'current-plan.json');
export const ARCHIVE_DIR = path.join(ROOT_DIR, 'archive');

export function ensureStorage() {
  if (!fs.existsSync(ROOT_DIR)) fs.mkdirSync(ROOT_DIR, { recursive: true });
  if (!fs.existsSync(ARCHIVE_DIR)) fs.mkdirSync(ARCHIVE_DIR, { recursive: true });
  if (!fs.existsSync(ISSUES_FILE)) fs.writeFileSync(ISSUES_FILE, '', 'utf8');
}

export function readIssues() {
  ensureStorage();
  const content = fs.readFileSync(ISSUES_FILE, 'utf8');
  return content
    .split('\n')
    .map(line => line.trim())
    .filter(Boolean)
    .map(line => {
      try {
        return JSON.parse(line);
      } catch {
        return null;
      }
    })
    .filter(Boolean);
}

export function writeIssues(issues) {
  ensureStorage();
  const text = issues.map(it => JSON.stringify(it)).join('\n') + (issues.length ? '\n' : '');
  fs.writeFileSync(ISSUES_FILE, text, 'utf8');
}

export function recordIssue({ sessionId = 'unknown', priority = 'P1', title = '未命名问题', fingerprint = 'default-fp', context = {} }) {
  if (!sessionId || typeof sessionId !== 'string') {
    throw new Error('无效的 session_id');
  }
  const issues = readIssues();
  const now = new Date().toISOString();
  const existing = issues.find(
    it => it.fingerprint === fingerprint && (it.status === 'pending' || it.status === 'in_progress')
  );

  if (existing) {
    existing.occurred_count = (existing.occurred_count || 1) + 1;
    existing.updated_at = now;
    existing.last_session_id = sessionId;
    if (Object.keys(context).length > 0) {
      existing.last_context = context;
    }
    writeIssues(issues);
    return { type: 'dedup', issue: existing };
  }

  const datePart = new Date().toISOString().slice(0, 10).replace(/-/g, '');
  const id = `ISSUE-${datePart}-${String(issues.length + 1).padStart(3, '0')}`;
  const newIssue = {
    id,
    session_id: sessionId,
    fingerprint,
    priority,
    status: 'pending',
    title,
    context,
    occurred_count: 1,
    created_at: now,
    updated_at: now
  };
  issues.push(newIssue);
  writeIssues(issues);
  return { type: 'created', issue: newIssue };
}

export function listIssues() {
  const issues = readIssues();
  const prioWeight = { P0: 0, P1: 1, P2: 2, P3: 3 };
  issues.sort((a, b) => (prioWeight[a.priority] ?? 9) - (prioWeight[b.priority] ?? 9));
  return issues;
}

export function readPlan() {
  ensureStorage();
  if (!fs.existsSync(PLAN_FILE)) return null;
  try {
    return JSON.parse(fs.readFileSync(PLAN_FILE, 'utf8'));
  } catch {
    return null;
  }
}

export function createPlan(targetIssueIds = [], objective = '解决待办问题批次', steps = []) {
  if (!targetIssueIds || targetIssueIds.length === 0) {
    throw new Error('创建计划必须包含至少一个 target_issue');
  }
  const rawSteps = steps.length > 0 ? steps : ['排查定位根因', '修复代码并执行单测', '完成回归并收集凭证'];
  const plan = {
    plan_id: `PLAN-${new Date().toISOString().slice(0, 10).replace(/-/g, '')}-01`,
    target_issues: targetIssueIds,
    objective,
    status: 'in_progress',
    steps: rawSteps.map((s, idx) => ({
      step_id: idx + 1,
      description: typeof s === 'string' ? s : s.description || '未命名步骤',
      status: typeof s === 'object' && s.status ? s.status : 'pending',
      evidence: typeof s === 'object' && s.evidence ? s.evidence : null
    })),
    last_session_id: 'unknown',
    updated_at: new Date().toISOString()
  };
  ensureStorage();
  fs.writeFileSync(PLAN_FILE, JSON.stringify(plan, null, 2), 'utf8');

  const issues = readIssues();
  issues.forEach(it => {
    if (targetIssueIds.includes(it.id)) {
      it.status = 'in_progress';
      it.updated_at = new Date().toISOString();
    }
  });
  writeIssues(issues);
  return plan;
}

export function updatePlanStep(stepId, { status = 'completed', evidence = null, sessionId = 'unknown' } = {}) {
  const plan = readPlan();
  if (!plan) {
    throw new Error('当前没有活动计划，无法更新步骤');
  }
  const step = (plan.steps || []).find(s => Number(s.step_id) === Number(stepId));
  if (!step) {
    throw new Error(`未找到步骤 step_id: ${stepId}`);
  }
  step.status = status;
  if (evidence !== undefined) {
    step.evidence = evidence;
  }
  plan.last_session_id = sessionId;
  plan.updated_at = new Date().toISOString();

  const allCompleted = plan.steps.length > 0 && plan.steps.every(s => s.status === 'completed');
  if (allCompleted) {
    plan.status = 'completed';
  }

  fs.writeFileSync(PLAN_FILE, JSON.stringify(plan, null, 2), 'utf8');
  return plan;
}

export function resolveIssue(issueId, evidence) {
  if (!issueId || typeof issueId !== 'string' || !issueId.trim()) {
    throw new Error('必须指定 issueId');
  }
  if (!evidence || typeof evidence !== 'string' || !evidence.trim()) {
    throw new Error('必须提供解决机器客观凭证 (evidence)');
  }
  const issues = readIssues();
  const idx = issues.findIndex(it => it.id === issueId);
  if (idx === -1) {
    throw new Error(`未找到 ID: ${issueId}`);
  }

  const [resolvedItem] = issues.splice(idx, 1);
  resolvedItem.status = 'resolved';
  resolvedItem.evidence = evidence;
  resolvedItem.resolved_at = new Date().toISOString();

  const ym = new Date().toISOString().slice(0, 7).replace(/-/g, '');
  const archiveFile = path.join(ARCHIVE_DIR, `resolved-${ym}.jsonl`);
  fs.appendFileSync(archiveFile, JSON.stringify(resolvedItem) + '\n', 'utf8');
  writeIssues(issues);

  return { resolvedItem, archiveFile };
}

export function executeCli(args = process.argv.slice(2)) {
  const command = args[0] || 'list';

  switch (command) {
    case 'record': {
      const sessionId = args[1] || 'unknown';
      const priority = args[2] || 'P1';
      const title = args[3] || '未命名问题';
      const fingerprint = args[4] || 'default-fp';
      let context = {};
      try {
        context = args[5] ? JSON.parse(args[5]) : {};
      } catch {
        context = { raw: args[5] };
      }
      try {
        const res = recordIssue({ sessionId, priority, title, fingerprint, context });
        if (res.type === 'dedup') {
          console.log(`[pa-issue-triage] 发现重复指纹 [${fingerprint}]，已更新待办 ${res.issue.id} (频次: ${res.issue.occurred_count})`);
        } else {
          console.log(`[pa-issue-triage] 已记录新问题: ${res.issue.id} - ${title} [${priority}]`);
        }
      } catch (err) {
        console.error(`[pa-issue-triage] ${err.message}`);
        process.exit(1);
      }
      break;
    }

    case 'list': {
      const issues = listIssues();
      console.log(`=== 待办问题看板 (${issues.length} 项) ===`);
      if (issues.length === 0) {
        console.log('（当前无活动待办）');
        break;
      }
      issues.forEach(it => {
        console.log(
          `[${it.priority}] [${it.status}] ${it.id} - ${it.title} (频次: ${it.occurred_count || 1}, 指纹: ${it.fingerprint}, 会话: ${it.session_id})`
        );
      });
      break;
    }

    case 'plan-create': {
      const targetIssues = (args[1] || '').split(',').map(s => s.trim()).filter(Boolean);
      const objective = args[2] || '解决待办问题批次';
      try {
        const plan = createPlan(targetIssues, objective);
        console.log(`[pa-issue-triage] 计划已建立: ${plan.plan_id} -> ${objective} (共 ${plan.steps.length} 步)`);
      } catch (err) {
        console.error(`[pa-issue-triage] ${err.message}`);
        process.exit(1);
      }
      break;
    }

    case 'plan-step-update': {
      const stepId = args[1];
      const status = args[2] || 'completed';
      const evidence = args[3] || null;
      const sessionId = args[4] || 'unknown';
      try {
        const plan = updatePlanStep(stepId, { status, evidence, sessionId });
        console.log(`[pa-issue-triage] 步骤 ${stepId} 更新成功 (整体状态: ${plan.status})`);
      } catch (err) {
        console.error(`[pa-issue-triage] ${err.message}`);
        process.exit(1);
      }
      break;
    }

    case 'plan-status': {
      ensureStorage();
      if (!fs.existsSync(PLAN_FILE)) {
        console.log('[pa-issue-triage] 当前没有活动计划');
        break;
      }
      console.log(fs.readFileSync(PLAN_FILE, 'utf8'));
      break;
    }

    case 'resolve': {
      const issueId = args[1];
      const evidence = args[2];
      try {
        const { archiveFile } = resolveIssue(issueId, evidence);
        console.log(`[pa-issue-triage] 问题 ${issueId} 已闭环并归档至 ${archiveFile}`);
      } catch (err) {
        console.error(`[pa-issue-triage] ${err.message}`);
        process.exit(1);
      }
      break;
    }

    default:
      console.log('用法: node scripts/pa-issue-triage.mjs {record|list|plan-create|plan-step-update|plan-status|resolve}');
      process.exit(1);
  }
}

// 直调 CLI 执行
const isDirectEntry = process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1]);
if (isDirectEntry) {
  executeCli();
}
