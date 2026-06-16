# PA-051 实现 Phase A 工具缺口：Edit / Write / Run

## 状态
- Status: `Done`
- Priority: `P0`
- Owner: `Codex`

## OpenSpec Change
- 依赖：
  [add-second-wave-tool-surface](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-second-wave-tool-surface>)

## Canonical Spec
- 依赖：
  [second-wave-tool-surface/spec.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-second-wave-tool-surface/specs/second-wave-tool-surface/spec.md>)

## 背景
`PA-050` 已经完成第二批工具面的 spec 收口，并明确 `Phase A` 必须优先补齐：

- `Edit`
- `Write`
- `Run`

当前代码真实状态仍是：

- `Run -> time_now`
- `Ask -> echo_input`
- `Read -> workspace_gather_context`
- `List -> workspace_list_files`
- `Search -> workspace_search_text`
- `Plan -> workspace_batch`

也就是说，`Edit / Write` 尚未存在真实 builtin primitive，`Run` 也还没有变成真正的受控执行工具。

## 目标
把 `Phase A` 的三个核心工具缺口实现为真实 builtin 工具能力，并补齐对应验证。

## 输出
- `Edit` builtin primitive
- `Write` builtin primitive
- `Run` 的受控 shell/command 执行 primitive
- 对应工具注册、权限事实、前端工具展示数据和测试

## 范围边界
- 本卡只做 `Phase A`
- 不提前实现 `Glob / Grep / WebFetch / WebSearch / MCP Resource / ToolSearch`
- 如果 `Search` 需要为后续 `Grep` 预留 seam，可以顺手补 internal seam，但不扩成完整 `Phase B`

## 验收标准
- `Edit / Write / Run` 出现在真实 builtin tool surface 中
- `Edit / Write / Run` 可以通过 `ToolRouter` 执行
- `Run` 具备基本的 `cwd / timeout / exit_code / stdout / stderr` 结果结构
- `Edit` 具备零匹配、多匹配、非法输入的结构化失败语义
- `Write` 与 `Edit` 的边界清晰，权限事实正确
- Rust 测试、前端相关测试与端到端测试通过

## 当前进展
- 已完成 `PA-050` 规划与审核
- 已完成 `Phase A` 第一轮落地实现：
  - `workspace_write_file`
  - `workspace_edit_file`
  - `workspace_run_command`
- 已完成 builtin tool 注册、模型可见映射、permission facts 和前端默认工具目录同步
- 已用 `opencode / deepseek-v4-flash` 完成一轮实现审阅，结果见：
  [.tmp/pa051-opencode-review-phase-a.jsonl](/C:/Users/HUAWEI/Documents/pony-agent/.tmp/pa051-opencode-review-phase-a.jsonl)
- 已采纳部分审阅意见并完成修复：
  - 修复 `Write` 写入不存在嵌套目录时的路径构造错误
  - 补强 `Run` 的高风险命令识别，覆盖 `rm -r -f` / `git reset --hard` / `git clean -fd` 等模式
  - 为 `Run` 的非零退出码补充结构化错误信息
- 已通过的精确验证：
  - `cargo check --manifest-path crates/pony-agent-core/Cargo.toml`
  - 多条 `tools.rs` 精确单测：
    - `write_file_creates_new_file_in_workspace`
    - `write_file_respects_overwrite_false_for_existing_file`
    - `edit_file_requires_replace_all_for_multiple_matches`
    - `edit_file_returns_no_match_when_old_text_missing`
    - `run_command_returns_stdout_and_exit_code`
    - `run_command_denies_high_risk_commands`
    - `run_command_denies_rm_with_split_flags`
    - `run_command_returns_error_for_non_zero_exit_code`
    - `builtin_tool_contract_views_deduplicate_to_model_surface`
  - 前端精确回归：
    - `tests/runtime-store.spec.ts` 中 builtin capability fallback 已对齐新 8 工具面并单测通过
  - 宿主层精确回归：
    - `cargo test --manifest-path src-tauri/Cargo.toml --test tool_router_regression write_tool_rejects_existing_file_when_overwrite_false --target-dir target-test -- --exact --nocapture`
    - `cargo test --manifest-path src-tauri/Cargo.toml --test tool_router_regression write_and_edit_tools_work_for_workspace_files --target-dir target-test -- --exact --nocapture`
    - `cargo test --manifest-path src-tauri/Cargo.toml --test tool_router_regression run_command_returns_structured_result --target-dir target-test -- --exact --nocapture`
- 已完成全量验证：
  - `npm run test:unit`
  - `cargo test --manifest-path crates/pony-agent-core/Cargo.toml --target-dir target-test`
  - `npm run cargo:test:regression`
  - `npm run test:tauri:smoke`
  - `npm run test:e2e`

## 下一步动作
1. 当前卡已完成；后续仅在第三波工具扩展时复用 `Run / Write / Edit` 的边界与验证口径

## 当前卡点
- 暂无。当前卡已完成态收口。

## 断点续跑提示
继续前先看：

- [PA-050](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-050-build-second-wave-tool-surface.md>)
- [tools.rs](/C:/Users/HUAWEI/Documents/pony-agent/crates/pony-agent-core/src/agent/tools.rs)
- [2026-06-15-pa050-spec-review.md](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-15-pa050-spec-review.md>)
