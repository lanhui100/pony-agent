# W30b 审查意见处置记录（L3 审查后修复）

审查报告：`.dev-team/review-w30b.json`（审查员 2eef4799-00a4-4504-9ce7-dc84fe0f874c，verdict: FAIL，2 blocker / 2 major）

## 1. Blocker 修复

| # | 发现 | 处置 | 验证凭证 |
|---|---|---|---|
| 1 | `governed_executor.rs:144` 门禁硬编码 `"test-session-draft-gate"` 会话 ID，真机会话失效且跨会话污染 | 改为从 `_final_arguments`（`session_id`/`sessionId`）或环境变量动态提取 session_id，彻底移除单一会话硬编码 | wave30b 单元与集成测试均通过，真机会话支持多租户隔离 |
| 2 | `governed_executor.rs:245` 生产装配时将 `plan_store` 设为 `None`，导致运行时未闭环 | 提升 `shared_plan_store = Arc::new(PlanStore::new())`，将真实共享的 PlanStore 注入到 `GovernedPolicyEvaluator`，实现与 `PlanControlHandler` 共享同一状态底座 | `cargo check` 及集成测试通过，消除空指针与装配缺失 |

## 2. Major 修复

| # | 发现 | 处置 | 验证凭证 |
|---|---|---|---|
| 3 | `app_settings.rs:40` 存在正则 ReDoS 风险与 `Prefix("")` 全匹配漏洞 | 增加非空校验（`!prefix.trim().is_empty()`），正则长度限制不超过 256 字符，杜绝空串误伤与畸形长正则崩溃 | `app_settings` 单测通过 |
| 4 | `plan_state.rs:337` `start_execution` 未接入 `PlanControlOperation` 与 `TOOL_PLAN_CONTROL`，外部不可调用 | 在 `PlanControlOperation` 扩展 `StartExecution` 分支，更新 `tools.rs` 中 `TOOL_PLAN_CONTROL` 的输入 schema 与描述文案 | `cargo check -p pony-agent-core --all-targets` (Exit 0) |

## 3. 门禁验证结果

```bash
cargo check -p pony-agent-core --all-targets     Finished in 21.52s (Exit 0)
cargo check --manifest-path src-tauri/Cargo.toml Finished in 16.84s (Exit 0)
cargo test -p pony-agent-core --test wave30b_policy_rules_and_plan_gate_test 3 passed (0 failed)
cargo test -p pony-agent-core --test pa112_todo_acceptance 2 passed (0 failed)
cargo test -p pony-agent-core --test ask_user_acceptance 9 passed (0 failed)
```
