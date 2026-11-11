# PA-120: PonySentry Trace a0fa4c67 级联超时与推诿行为闭环治理 (Wave 9)

## 状态
- **状态**: Completed
- **负责人**: Dev Team (Lead / Protocol / Test / Executors)
- **创建日期**: 2026-10-14
- **闭环日期**: 2026-10-14

## 需求背景
PonySentry 新上报 trace `a0fa4c67-f9b8-4388-ae48-2ad13871f4ff` 反映了 Agent 在执行发版推送任务时遭遇网络超时、平台命令语法失误（cmd 环境下 POSIX 内联 env 导致 9009）后引发级联超时挂起，且由于指纹比对机制未做参数归一化导致连续失败断路器形同虚设，最终导致 Agent 防御性逃逸并向用户甩包手动操作清单。

## 核心交付契约
1. **P0-1 指纹归一化**: `tool_failure_signal` 在 Run 类工具发生 `timeout` 错误时自动剔除 `timeoutMs` 并归一化行首赋值前缀，修复断路器失效问题。
2. **P0-2 超时物理证据保留**: `run_command` 超时错误载荷根级保留 `stdout`、`stderr`、`cleanupError`、`exitCode` 等核心观测依据。
3. **P1-1 清理幂等与存活探针**: 消除清理短路，执行无条件二次幂等清理，并在句柄无法释放时上报 `environment_unavailable`。
4. **P1-2 网络命令超时 Hint 分类**: 智能识别网络推送命令并引导用户自检网络/代理/SSH 配置。
5. **P1-3 Windows 内联赋值拦截**: `posix_inline_env_assignment_reason` 在 Windows 拦截 POSIX `VAR=val cmd` 并指引正确语法，拒绝自动改写。
6. **P2-1 进程 Job 逃逸窗口收敛**: 采用 `CREATE_SUSPENDED` 启动子进程，确保在 Job 纳管成功后再执行恢复。
7. **P2-2 Prompt 反推诿回执条款**: `BASE_SYSTEM_PROMPT` 增加 handoff receipt contract，禁止无凭证推诿。

## 验收收据
- `cargo check --manifest-path crates/pony-agent-core/Cargo.toml --tests`: EXIT 0
- `harness_optimizations_test`: 12 passed / 0 failed, EXIT 0
- `wave9_adversarial_test`: 4 passed / 0 failed, EXIT 0
- `wave9_green_acceptance_test`: 2 passed / 0 failed, EXIT 0
- 报告索引: [`docs/analysis/trace-analysis-pa-0af4c67.md`](../../docs/analysis/trace-analysis-pa-0af4c67.md)
