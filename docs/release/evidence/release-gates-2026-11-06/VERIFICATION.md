# 发版门禁清零验证收据（2026-11-06）

目的：把「能否发版/发测试版」从主观判断变成机器事实。三项既有红逐一归因并清除。

## 0. 结论

**发版门禁已全绿**（本机 Linux 实测）：`secrets:check:all` 0、`test:unit` 149/149、`cargo test --lib` 1300/1300、`typecheck` 0、`build` 0、`contract:check` 0、`ext:check` PASS、`version:check` PASS。
唯一未在本机验证的门禁是 CI 的 `./scripts/verify-decisions.ps1`（PowerShell，Linux 无 pwsh），需在 CI/Windows 侧确认。

## 1. 三项红与归因

| 项 | 现象 | 根因（证据） | 处置 |
|---|---|---|---|
| **A** | `npm run secrets:check:all` exit 1，5 命中 | ① `mcp_channel_closure.rs:482`、`plugin_host_contract.rs:310` 两处测试占位符不符合 `sk-mock-*` 豁免前缀（规则见 `scripts/check-secrets.mjs:73-76`）；② `.dev-team/l2at-done-*.json:260-262` 是 `final_test_file_sha256` 收据哈希，其值行不含良性关键词 | ① 占位符改 `sk-mock-*`（断言均引用变量，零语义变更）；② 新增**最小路径豁免** `isExemptGeneratedReceipt`（仅 `.dev-team/` 下 basename 匹配 `l2at-done-*.json`），未放宽 64-hex 规则本体、未豁免整个 `.dev-team/`。边界自测：`.dev-team/l2at-done-x/y.json`、`.dev-team/other.json`、`x/.dev-team/...` 均仍扫描 |
| **B** | `cargo test --lib` 1 failed | `runtime/tests.rs` 断言 `history.last().content == CANCELLED_TURN_MESSAGE`，而产品自 wave-tui-session-resilience C5 起对「工具执行中被中断」的轮次持久化检查点投影（`session/store.rs:62-92`、`bin/tui.rs:8611` 依赖该标记）⇒ **测试断言过期，非产品缺陷** | 断言改为 `starts_with("[中断轮次检查点]")` + `contains("已执行的工具步骤：")` + `contains(<工具步骤>)`；原 cancelled phase / `stopped_by_user` / tool_activities 长度 / 无 `tool_call_completed` / `request_bodies==1` **全部保留** |
| **C** | 全量 vitest「3 文件红」 | **误读**：`tests/update-signed-store.spec.ts:118/132/186`、`src/stores/__tests__/plan-goal-refresh.spec.ts:114` 是 `console.warn(error)` 的 **stderr 创建栈**，被 `v10-baseline-freeze.spec.ts`（V10-3 把子进程输出尾部拼进失败块）带入失败块。真实红只有 `check-secrets.spec`（= A）与其级联 V10-3；两个目标文件**从未失败** | **零改动**。发版冻结期不引入全量 mock 语义变更；隔离加固与告警静音登记 **B037**，T7 脏树敏感登记 **B038** |

## 2. 机器收据（真实 exit code）

| 门禁 | 结果 |
| --- | --- |
| `npm run secrets:check:all` | **0**（0 leaks detected；5 → 0） |
| `cargo test -p pony-agent-core --lib` | **0**：`1300 passed; 0 failed`（Lead 独立复跑） |
| `npx vitest run`（全量，干净树） | **0**：`149 passed (149)`，`1382 passed / 12 skipped`，含 V10-3 嵌套全量子跑 exit 0 |
| `npx vitest run`（另一轮，干净树） | **0**：同上 149/149 |
| `cargo check -p pony-agent-core --all-targets` | 0，warning 2 条（基线） |
| `npm run typecheck` | 0 |
| `npm run build` | 0（vite build 5.97s，仅既有 chunk 警告） |
| `npm run contract:check` | 0（`TurnMetricsSnapshot 22/22`、`TurnStreamEvent 38/38`） |
| `npm run ext:check` | PASS |
| `npm run version:check` | PASS（Tauri 4 处同步 0.1.181） |
| `audit-diff.sh` / `check-declaration.sh` | 0 / 0（四维 PASS） |

## 3. 发版前仍需注意（非阻塞）

1. **CI 侧唯一未验证**：`./scripts/verify-decisions.ps1`（pwsh）与 `npx tauri build --bundles nsis`（Windows）本机无法执行。
2. **B038**：`wave47-ob2-enforcement` T7 读实时工作树，本地脏树时可能翻红（本波 run2 即此例）；CI 在干净树 + 提交后运行不受影响。
3. **既有未修但已登记**：B033（cache 写入未计价）、B034（治理汇总前端零调用）、B035（熔断阈值语义）、B036（成本聚合粒度三面）、B030①（首跳窗口）。
4. 本轮未改产品行为：所有改动限于测试夹具/断言与 secrets 规则的最小豁免。
