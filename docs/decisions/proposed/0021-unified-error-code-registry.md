# 0021 Agent 运行时统一错误码注册表：上游与协议不匹配可扩展治理

Status: proposed

## 背景

Agent 运行时错误处理将来需要从三层局部系统收敛为顶层统一治理。Test Agent 在 `.dev-team/error-contract.md` 中已验证：请求级 `retry.rs`（`FailureKind` + `StreamState` + `RetryBudget`）、工具级 `DispatchError{code,message}` / `ToolError{kind,message}` / `CapabilityFailureKind` 六类 / `limits.rs` 熔断 / `tool_recovery.rs` 本地收口、Turn 级 `build_failed_turn_result` + `fallback_reason` + `trace_timeline` 三形状兼容 + `HookFailurePolicy` 各自成系统，但顶层存在三处断裂：全链 `Result<T,String>` 靠子串匹配分类、 Sauv `kind` vs `code` 实为三形状并存、无错误码注册表与 ADR。LLM provider 上游错误（500/501/505、quota-only 独立码、类型化网络）与协议不匹配错误（SSE 解析、tool_call schema、model catalog 未知模型、reasoning effort 冲突、事件版本）将来需要系统性类目，否则每新增一种上游或协议形态都要在调用点加 `lower.contains("…")` 分支。

## 候选方案

**方案 A：全链强类型错误枚举替换 `String`（如 `thiserror` 统一 `AgentError`）**

- 优点：编译期穷尽匹配最彻底，分类逻辑可随类型走。
- 落选原因：`provider/mod.rs`、`dispatcher.rs`、`hooks.rs` 等 70+ 处 `Result<_,String>` 签名与 `TurnStreamEvent.error: Option<String>`、`TurnResult phase=failed` 的 wire 形状需要同步断裂式变更；`handler_failure` 的 `code: message` 透传、`trace_timeline` 三形状读侧、`docs/architecture/runtime.md` 的 timeout 线码 `timeout` 别名冻结都需要在一次提交内迁移，爆炸半径超过单次可验收边界；且上游 `reqwest` flags 到 wire 的兼容需要在中间保留字符串桥，反而增加双轨期维护成本。

**方案 B：中央错误码注册表 + `String` wire 兼容 + 双写收敛（本篇采纳方向）**

- 做法：新增 `error_code` 模块定义 `ErrorCode` 枚举（`as_str` / `from_str`，wire 仍为 `String`），分类前缀 `upstream_*` / `protocol_*` / `tool_*` / `turn_*` / `hook_*` / `capability_*` / `skill_*`，未知码以 `unknown:*` 包装 Fail-Closed；`retry.rs classify` 关键词表收编为注册表单一来源（签名 `classify(&str)` 先保留做遗留桥，新增分类输入改为类型化上游事实）；`ToolError` / `DispatchError` 双写 `kind` + `code`，读侧永久保留三形状兼容；`CapabilityFailureKind` / `SkillFailureLayer` 经 `From` 映射入注册表；provider 新增 `Upstream` / `ProtocolMismatch` 可扩展分支，不破坏 RL 长退避切换与 sync/stream fallback 链。
- 采纳理由：调用点只传码构造器，加码等于注册表加一行加单测；`trace_timeline` 现有三形状单测可直接作为回归基线；`timeout` 线码别名可冻结不改名；`handler_failure` 未注册前缀回落 `handler_error` 的 Fail-Closed 语义可机械验证。

**方案 C：维持现状，仅保留 `trace_timeline` 三形状兼容层**

- 优点：零改造成本，短期无回归风险。
- 落选原因：上游 8 类与协议 5 类将来无处可挂，新错误只能继续以无码中文句或裸 `String` 进入 `TurnResult` / timeline；`quota-only`、`SSE 解析`、`tool_call 坏 JSON 静默 {}`、`catalog`、`effort 冲突`、`事件版本直通`六处已验证缺口无法被黑盒观测，违背"同意维持错误码整体管理机制以便未来拓展"的本次目标。

## 决策

将来会新建 `0021` 注册表模块并按 `.dev-team/error-contract.md` C1–C7 黑盒矩阵实施：C1 码形 `^[a-z][a-z0-9_]{1,63}$` 与分层前缀；C2 全链路写 `error.code`、读永久兼容三形状；C3 上游八类（`upstream_timeout` 线码冻结为 `timeout`、`upstream_rate_limited`、`upstream_server_error` 覆盖 5xx 全段、`upstream_auth`、`upstream_quota` 独立于 429、`upstream_context_overflow`、`upstream_network`、`upstream_bad_request`）分类输入改为类型化事实；C4 协议五类（`protocol_sse_parse`、`protocol_tool_call_schema` 消灭静默 default `{}`、`protocol_model_catalog` + `protocol_unknown_model`、`protocol_reasoning_effort` 本地冲突报错、`protocol_event_version`）；C5 turn/hook/limits/recovery 码（失败 turn 不得 `fallback_reason: None`，熔断输出 `turn_*`，收口以 `turn_recovery_*` 开头）；C6 `turn-event-v1` 不 bump；C7 加码只加注册表行加单测、禁止新增 `contains` 关键词分支、同提交附 ADR、未知码 `unknown:*` 包装。

## 权衡

采纳 B 意味着双写期 `kind` + `code` 并存，`is_rate_limit_error` 与 `classify` 的 quota 混合语义（quota-only Abort、quota+429 切 RL 长退避约 60s 白烧残余）需要原样保留并以单测锁定；`provider/mod.rs:6006` 锁定测试与 30+ retry 单测在重构期间不得改断言，只允许新增码的单测。收益是未来新增上游或协议错误时调用点零分支、审查时以 `grep contains` 零新增为门禁。

## 影响

- 约束：后续任何新增或变更错误码的提交必须同提交更新本篇或新增接替 ADR，并延续"文件:行号举证"格式；`docs/architecture/runtime.md` 的 timeout 收口段将来需要补注册表映射说明。
- 后续动作：本篇 proposed 通过评审后转入实施任务（注册表模块 → 双写兼容 → provider 上游/协议分支 → 全量回归），实施期间 `crates/pony-agent-core/src/agent/` 写锁由实施任务持有。
