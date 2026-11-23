import { describe, test, expect } from 'vitest';
import { execSync } from 'child_process';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

/**
 * 长期压力与智能演进评估测试 (Stress & Evolution Eval)
 * 场景：模拟复杂长链路任务，验证 Agent 在多轮 ToolCall、上下文压缩与异常注入下的逻辑守恒性。
 *
 * 隔离约定（2026-11-23 加固）：
 * - 每次 run 用 `mkdtempSync` 拿**唯一**目录，不再复用 `<cwd>/temp-eval-env`。
 *   旧写法先 `rmSync` 再 `mkdirSync`：同一机器上并发的两个 run（或上一次的残留进程）
 *   会互相删掉对方的目录与 `loop-state.json`，表现为"偶发失败、重跑就好"。
 * - 结束时在 `finally` 里清掉自己的临时目录（只删本次 mkdtemp 出来的路径）。
 * - 显式给足 timeout：本用例串行执行 10 次 `python3` 子进程，CI 机器可能比本地慢一个量级。
 */
describe('Long-Running Agent Stress & Evolution Eval', () => {
    test('Stress Scenario: Gold Chain Long-Task Execution', async () => {
        const TEMP_DIR = fs.mkdtempSync(path.join(os.tmpdir(), 'pony-stress-eval-'));
        try {
            // 显式指定状态文件路径并确保符合 trace-audit-loop.py 期望
            const stateFile = path.join(TEMP_DIR, 'loop-state.json');

            // 驱动 10 轮次
            for (let i = 0; i < 10; i++) {
                if (fs.existsSync(stateFile)) {
                    const state = JSON.parse(fs.readFileSync(stateFile, 'utf-8'));
                    if (state.phase === 'await_approval') {
                        state.phase = 'verdict';
                        fs.writeFileSync(stateFile, JSON.stringify(state, null, 2));
                    }
                }
                execSync(`python3 scripts/trace-audit-loop.py --state-file ${stateFile} --headless --step`, { encoding: 'utf-8' });
            }

            // 验证状态最终落点
            const finalState = JSON.parse(fs.readFileSync(stateFile, 'utf-8'));
            expect(finalState.phase).toBe('patrol');
        } finally {
            fs.rmSync(TEMP_DIR, { recursive: true, force: true });
        }
    }, 120_000);
});
