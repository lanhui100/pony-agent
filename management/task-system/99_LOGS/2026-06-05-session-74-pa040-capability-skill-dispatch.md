# 2026-06-05 Session 74 PA-040 Capability / Skill Dispatch

## 本轮目标

沿着 `PA-040` 第一次 acceptance 审计的阻断项，补第一段真实的 planner / capability mediation hook dispatch，而不是继续只加 trace。

## 本轮完成

- 已扩展 `AgentHookExecutor` 契约，使其能接收 capability mediation envelope，而不再只能收到裸 `hook_point`
- 已把 capability / skill mediation hooks 的真实 dispatch 接入 `execute_registered_tool_call(...)`
- 已支持 capability mediation 白名单路径 `request.arguments` 的 patch 合并与实参改写
- 已在 capability / skill 路径上补“hook 阻断后不继续执行工具”的 blocked result 分支
- 已新增 Rust 定向测试：
  - `capability_mediation_hooks_can_rewrite_arguments_before_tool_execution`
  - `skill_mediation_hooks_can_rewrite_arguments_before_skill_execution`
  - `monitor_and_drilldown_read_runtime_generated_skill_hook_evidence`

## 新证据

1. capability hook 的 patch 会真实改写最终工具调用 arguments，而不是只生成一条 observe trace。
2. skill mediation hook 的 patch 会真实改写 composed capability 执行 arguments。
3. `skill.tool_actions.observe` 现在已能被 control-plane runtime view 与 session drilldown 读回，不再只停留在 runtime 层最小 evidence。

## 对审计阻断项的影响

- `PA-040 acceptance audit` 中“capability / skill mediation 没有真实 hook dispatch”的判断，现已被部分修复：
  - capability / skill mediation dispatch 已落地
  - planner dispatch 仍未落地
- `skill.tool_actions.observe` 缺少 control-plane drilldown 的阻断项，现已关闭

## 仍待继续

- planner `preflight / tool selection / graph decision` 的真实 hook dispatch
- planner `preflight / tool selection` 的 control-plane drilldown 回归

## 验证

```powershell
cargo check --manifest-path src-tauri/Cargo.toml --lib
cargo test --manifest-path src-tauri/Cargo.toml --lib capability_mediation_hooks_can_rewrite_arguments_before_tool_execution -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib skill_mediation_hooks_can_rewrite_arguments_before_skill_execution -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib monitor_and_drilldown_read_runtime_generated_skill_hook_evidence -- --nocapture
```

结果：全部通过。
