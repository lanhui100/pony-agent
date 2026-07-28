# 2026-07-26 PA-076 阶段 2 收口与首次真实 Rust 执行

## 本轮目标

- 完成 `PA-076 / harden-and-expand-agent-tool-runtime` 的 task 2.8
- 解除自 2026-07-18 起记录的 MSVC `link.exe` 环境阻塞
- 在真实执行 Rust 测试的前提下判定阶段 2 是否可以收口

## 环境阻塞的真实原因

前两轮把 `link.exe not found` 记为「需要安装 Build Tools」，实际结论不同：

- MSVC Build Tools 一直存在于 `C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools`，
  仅是 shell 未加载 `vcvars64.bat` 环境变量。
- 新增 `scripts/run-rust-msvc.bat`：预加载 `vcvars64.bat` 后转发任意命令。
- 另一个此前未被发现的问题：`scripts/invoke-rust-target.ps1` 固定
  `--manifest-path src-tauri/Cargo.toml`，因此
  `npm run cargo:test:exact -- --lib agent::tools::contract_view_tests::`
  作用于 tauri crate，即使编译成功也只会输出 `running 0 tests`。
  core crate 的测试必须显式 `-p pony-agent-core`。

统一入口：

```bash
cmd //c "scripts\run-rust-msvc.bat cargo test -p pony-agent-core --lib --target-dir target-test-exact-a <filter>"
```

## 首次真实执行暴露的 3 个 phase-2 缺陷

### 1. provider 工具数组顺序回归（影响缓存命中）

旧 `builtin_tool_contract_views()` 按 product name **首次出现**的位置排序：
`time_now` 先出现，把 `Run` 钉在第 1 位，而最终胜出的 primitive 是
`workspace_run_command`。新 registry 投影改按胜出 descriptor 自身位置排序，
导致 provider 工具数组顺序变化。

该顺序属于 `PA-025 / PA-029` 收窄的 cache-friendly stable prefix，
顺序变化会掉缓存命中。

修法：在 builtin 构建阶段就按「模型可见槽位」重排 descriptor
（`product_first_index` 记录首次出现槽位，模型可见者优先），
使 registry 位置成为所有投影点的唯一顺序真相源，
而不是让每个投影点各自排序。

### 2. 未知工具名被静默改写为 builtin 产品名

`model_visible_tool_name` 的 `_ => "Run"` 兜底会把任意未知或外部工具名
改写成 builtin 产品名。这是 tool-count 推断之外的**第二条名称推断路径**，
而 task 2.7 的意图正是消除这类推断。

修法：新增 `model_visible_tool_name_opt`（未知名返回 `None`）与
`product_visible_tool_name`（未知名原样透传）。
`ToolDefinition / ToolCall / ToolResult` 三处 contract view 改用透传版本。
`model_visible_tool_name` 保留给已持有已知 primitive 且需要 `&'static str` 的调用点。

### 3. 新测试断言错误（非实现缺陷）

新测试断言 `workspace_path_info` 的产品名为 `Read`，
与既有映射 `List` 冲突。task 2.1/2.6 明确要求阶段 2 不改变产品可见行为，
因此修正测试断言而非映射。`workspace_path_info` 仍是 `List` 背后的
deferred descriptor，由 ToolSearch 提升按 turn 暴露。

## 验证

全部经 `scripts/run-rust-msvc.bat` 加载 MSVC 环境后执行：

- `cargo test -p pony-agent-core --lib ... agent::tools::contract_view_tests::`
  首次 `9 passed; 3 failed`，修复后 `12 passed; 0 failed`
- `... agent::tools::`：`65 passed; 0 failed`
- `... agent::tool_runtime::`：`4 passed; 0 failed`
- `... agent::provider::tests::`：`provider_does_not_infer_builtin_surface_from_tool_count`
  首次失败，修复后通过；同批 `33 passed`，另 1 项属既有环境失败（见下）
- `cargo test --manifest-path src-tauri/Cargo.toml --test tool_router_regression`：`13 passed; 0 failed`
- `rustfmt --edition 2021 tools.rs tool_runtime.rs`：通过
- `git diff --check`：clean
- `npm run openspec -- validate harden-and-expand-agent-tool-runtime --strict`：valid

## 与 PA-076 无关的既有失败（未计入本卡门禁）

core 全量 `--lib` 另有 7 项失败，断言均不经过工具投影路径：

- 6 项 history / branch cursor：`session`、`sqlite_session`、`control_plane`、
  `runtime` 中的 node id 与 branch head 解析。疑与工作区未提交的
  `runtime/mod.rs`、`turn_flow.rs` markdown/model-hop trace 改动相关。
- 1 项 `openai_reasoning_tool_followup_stream_attempts_live_stream_before_fallback`：
  不可达端点下降级到 `local_fallback` 而非期望的 `stream_sync_fallback`，属网络环境相关。

建议单独定位，不阻塞 PA-076。

## 当前状态

- 阶段 1（1.1~1.3）与阶段 2（2.1~2.8）全部完成，任务清单 11/48。
- 未改动用户的 `runtime/mod.rs`、`turn_flow.rs`。
- Ask、Run、任意 URL WebFetch 仍保持 fail-closed，未启用新行为。

## 下一步

进入 task 3.1 governed dispatcher lifecycle：origin 鉴权、raw validation、
mutable hooks、final validation/normalization、final-argument policy 与 sandbox 决策、
budgets、执行、control outcome、telemetry、exactly-once lifecycle hooks。
先补 fake dispatcher/policy/budget harness，再只在 runtime 最小接线点
消费 `ToolOutcome.control_outcome`。

## Resume Hint

接线 dispatcher 时必须保持两条本轮修复的不变量（回归代价高）：

1. registry descriptor 顺序即 provider 工具数组顺序（stable prefix 依赖）。
2. 外部 / 未知工具名必须原样透传，不得回落到 builtin 产品名。

复跑门禁：`agent::tools::`、`agent::tool_runtime::`、`--test tool_router_regression`。
架构说明见 `docs/architecture/tool-runtime-descriptor-registry.md`。
