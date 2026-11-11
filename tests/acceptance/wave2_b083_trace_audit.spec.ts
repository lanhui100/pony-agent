import { test, expect } from '@playwright/test';
import { execSync } from 'child_process';
import * as fs from 'fs';

test.describe('B083 Trace Audit Loop Automation Acceptance', () => {
    test('L2-T: headless trace-audit-loop should advance 3 rounds without unapproved executions', async () => {
        // 执行 3 次一次性推进
        for (let i = 0; i < 3; i++) {
            const output = execSync('python3 scripts/trace-audit-loop.py --headless --step', { encoding: 'utf-8' });
            expect(output).toContain('ADVANCED_TO_NEXT_STOP');
            expect(output).not.toContain('UNAPPROVED_EXECUTION');
        }
    });

    test('L2-AT: trace-audit-loop handles corrupt state gracefully', async () => {
        const dummyPath = 'docs/audit/loop-state.test.json';
        fs.writeFileSync(dummyPath, '{ malformed: json }');
        
        try {
            const run = () => execSync(`python3 scripts/trace-audit-loop.py --state-file ${dummyPath}`, { encoding: 'utf-8' });
            expect(run).toThrow();
        } finally {
            if (fs.existsSync(dummyPath)) fs.unlinkSync(dummyPath);
        }
    });
});
