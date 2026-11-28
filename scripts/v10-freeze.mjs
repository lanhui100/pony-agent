#!/usr/bin/env node
/**
 * test:v10-freeze —— V10 冻结基线校验（含「全量里再跑一遍全量」的 V10-3）。
 *
 * 为什么独立成脚本（B1795831357586，2026-11-28）：
 * `tests/acceptance/pds-streaming/v10-baseline-freeze.spec.ts` 的 V10-3 会在 vitest 内部
 * 再 spawn 一次全量 vitest（`--no-file-parallelism`）。它验证的东西与外层那次 run 完全重合
 * ——同代码、同用例、同断言，第二遍零增量信号，却让 `npm run test:unit` 从 ~54s 涨到 ~350s
 * （实测 V10-3 单例 231.6s，占该文件 244.2s 的 94.8%）。
 *
 * 于是拆成两条路径：
 *   · `npm run test:unit`（默认）：V10-3 显式 skip，只留 V10-1/2/4 的轻量判据（~13s）；
 *   · `npm run test:v10-freeze`（本脚本）：置 `PDS_V10_FULL_SUITE=1` 后单跑本文件，
 *     V10-3 真实执行。CI 在 needs_frontend 时跑一次，发版前亦可本地手动跑。
 *
 * 判据强度不变：仍是「真实子进程 exit 0 + 用例计数自检」，只是把成本从每次本地全量
 * 挪到一次显式调用。环境变量注入走本脚本而非 shell 前缀，避免 Windows 下 `VAR=1 cmd`
 * 不被 cmd.exe 识别（本仓 CI 有 windows-latest runner）。
 */
import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(__dirname, "..");
const vitestEntry = path.join(repoRoot, "node_modules", "vitest", "vitest.mjs");
const specPath = "tests/acceptance/pds-streaming/v10-baseline-freeze.spec.ts";

if (!existsSync(vitestEntry)) {
  console.error(`[v10-freeze] 找不到 vitest 入口：${vitestEntry}（先跑 npm ci）`);
  process.exit(2);
}
if (!existsSync(path.join(repoRoot, specPath))) {
  console.error(`[v10-freeze] 找不到验收文件：${specPath}`);
  process.exit(2);
}

const result = spawnSync(
  process.execPath,
  [vitestEntry, "run", "--reporter=default", specPath],
  {
    cwd: repoRoot,
    stdio: "inherit",
    env: {
      ...process.env,
      // V10-3 的唯一开关：本脚本就是它的显式启用入口。
      PDS_V10_FULL_SUITE: "1",
      CI: "1",
      FORCE_COLOR: "0",
      NO_COLOR: "1",
    },
  }
);

if (result.error) {
  console.error(`[v10-freeze] 子进程启动失败：${result.error.message}`);
  process.exit(2);
}
if (result.status !== 0) {
  console.error(`[v10-freeze] V10 冻结校验失败（exit=${result.status}）`);
  process.exit(result.status ?? 1);
}
console.log("[v10-freeze] V10-1/2/3/4 全绿（含 V10-3 全量重跑）");
