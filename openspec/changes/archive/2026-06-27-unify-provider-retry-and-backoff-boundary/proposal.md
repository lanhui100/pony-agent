# Proposal: Unify Provider Retry And Backoff Boundary

## Why

Pony Agent 当前已经在 `pony-agent-core` 中实现了 provider timeout retry，但它仍然是一个较简化、分散且语义不完全稳定的实现：

- provider retry 主要按 timeout 字符串分类，缺少更明确的 transient / non-retryable / unsafe-to-retry 边界
- `decision`、`decision_stream`、`followup_sync`、`followup_stream` 已各自带有 retry/fallback 逻辑，但没有统一的 request-level contract
- 前端 `src/stores/runtime.ts` 仍持有 whole-turn 失败后的自动重试计时器，这与 provider request retry 的语义不同，却容易被误认为同一类“自动重试”
- stream 失败后的自动降级仍需要更精确的“可见输出边界”定义，否则容易引入重复文本、重复工具调用或错误归因
- 当前测试已覆盖 timeout retry 的基础路径，但尚未形成对 backoff、budget、`Retry-After`、stream safety boundary 的完整 spec 和 deterministic test strategy

如果不把这些边界正式写入 spec，后续实现很容易继续在 core、adapter、前端三层之间复制和漂移 retry 语义，也难以避免 request retry 与 whole-turn retry 的乘法放大风险。

## What Changes

- 建立 `provider retry and backoff boundary` 的正式 spec
- 将 retry 语义拆成三层：`request-level retry`、`phase-level fallback`、`turn-level retry`
- 明确三层之间的 `escalation contract`，避免 request retry、phase fallback 与 whole-turn retry 通过隐式异常传播重新耦合
- 明确 provider request retry 与 phase fallback 属于 `pony-agent-core`，而不是 `src-tauri` 或前端 store
- 明确 `src-tauri` 只负责宿主桥接，不持有 backoff / classifier / fallback policy
- 明确前端 existing whole-turn retry 将退场，不再保留静默自动 whole-turn 重提交流程；后续若需要 turn-level retry，只能作为显式 control-plane action 存在
- 明确 stream failure 的安全边界、tool-call commitment 定义与 fallback source 等级
- 明确结构化错误分类、budget/`Retry-After` 语义、最小可测抽象与 deterministic tests contract，避免真实时间 sleep 成为主验证手段

## Impact

- 后续 provider retry 重构可以围绕清晰的 core-side contract 实施，而不是继续复制局部 helper
- frontend / tauri / core 的职责边界会更稳定，减少“失败恢复逻辑到底应放哪一层”的争议
- stream/sync/local fallback 的来源等级、可信度与 telemetry 会有统一口径
- 测试可转向结构化分类 + 纯策略决策 + 最小执行器注入，减少 flaky timing tests

## Tracking

- Task card: `PA-070`
- OpenSpec Change: `unify-provider-retry-and-backoff-boundary`
