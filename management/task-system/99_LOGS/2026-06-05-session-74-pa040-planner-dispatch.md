# 2026-06-05 Session 74 PA-040 Planner Dispatch

## 本轮目标

沿着 `PA-040` 剩余主阻断，补 planner `preflight / tool selection` 的真实 hook dispatch，并把对应的 control-plane drilldown 证据补齐。

## 本轮完成

- 已扩展 planner hook dispatch，使 `plan_turn()` 与 stream planner 路径会先执行 planner hooks，再决定最终 tool call
- 已支持 planner 白名单路径：
  - `provider_tool_call`
  - `selected_tool_call`
- 已新增 Rust 定向测试：
  - `planner_preflight_hooks_can_rewrite_tool_call_before_execution`
  - `planner_tool_selection_hooks_can_rewrite_selected_tool_before_execution`
  - `monitor_and_drilldown_read_runtime_generated_planner_hook_evidence`

## 新证据

1. preflight hook 的 patch 会真实改写最终工具执行输入，而不是只写一条 observe trace。
2. tool-selection hook 的 patch 会真实改写最终 selected tool call。
3. `planner.preflight.observe / planner.tool_selection.observe` 现在已能被 control-plane runtime view 与 session drilldown 读回。

## 对 PA-040 阻断项的影响

- `planner preflight / tool selection` 的真实 dispatch 缺口已关闭
- `planner preflight / tool selection` 的 control-plane drilldown 回归缺口已关闭
- `PA-040` 当前主要剩余阻断已收窄为：
  - `planner.graph_decision` 仍未纳入同一套真实 mediation dispatch

## 验证

```powershell
cargo check --manifest-path src-tauri/Cargo.toml --lib
cargo test --manifest-path src-tauri/Cargo.toml --lib planner_preflight_hooks_can_rewrite_tool_call_before_execution -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib planner_tool_selection_hooks_can_rewrite_selected_tool_before_execution -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib monitor_and_drilldown_read_runtime_generated_planner_hook_evidence -- --nocapture
```

结果：全部通过。
