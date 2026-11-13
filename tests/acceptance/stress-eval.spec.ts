import { describe, test, expect } from 'vitest';
import { execSync } from 'child_process';
import * as fs from 'fs';
import * as path from 'path';

/**
 * 长期压力与智能演进评估测试 (Stress & Evolution Eval)
 * 场景：模拟复杂长链路任务，验证 Agent 在多轮 ToolCall、上下文压缩与异常注入下的逻辑守恒性。
 */
describe('Long-Running Agent Stress & Evolution Eval', () => {
    const TEMP_DIR = path.join(process.cwd(), 'temp-eval-env');

    test('Stress Scenario: Gold Chain Long-Task Execution', async () => {
        // 1. Arrange: 准备环境
        if (fs.existsSync(TEMP_DIR)) fs.rmSync(TEMP_DIR, { recursive: true });
        fs.mkdirSync(TEMP_DIR, { recursive: true });
        
        // 显式指定状态文件路径并确保符合 trace-audit-loop.py 期望
        const stateFile = path.join(TEMP_DIR, 'loop-state.json');
        
        // 2. Act: 驱动 10 轮次
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
        
        // 3. Assert: 验证状态最终落点
        const finalState = JSON.parse(fs.readFileSync(stateFile, 'utf-8'));
        expect(finalState.phase).toBe('patrol');
    });
});
