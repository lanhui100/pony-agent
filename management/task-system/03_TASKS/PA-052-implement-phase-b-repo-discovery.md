# PA-052 实现 Phase B 代码库探索增强：Glob / Grep

## 状态
- Status: `Done`
- Priority: `P1`
- Owner: `Codex`

## OpenSpec Change
- 依赖：
  [add-second-wave-tool-surface](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-second-wave-tool-surface>)

## Canonical Spec
- 依赖：
  [second-wave-tool-surface/spec.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-second-wave-tool-surface/specs/second-wave-tool-surface/spec.md>)

## 背景
`PA-050` 已明确第二批工具按 `Phase A -> Phase B -> Phase C -> Phase D` 顺序推进。当前 `Phase A` 的 `Edit / Write / Run` 已进入稳定验证阶段，下一步应补齐代码库探索增强层：

- `Glob`
- `Grep` 或等价增强后的 `Search`

当前真实代码状态仍是：

- `List -> workspace_list_files`
- `Search -> workspace_search_text`

也就是说，路径模式发现和更强的文本模式检索还没有被正式拉开边界。

## 目标
把 `Phase B` 的探索增强能力实现为真实 builtin 工具能力，并保持产品层命名边界稳定。

## 输出
- `Glob` builtin primitive
- `Grep-like` 文本模式检索 primitive
- `Search` 与增强检索 primitive 的映射收口
- 对应工具注册、前端默认工具目录和测试

## 范围边界
- 本卡只做 `Phase B`
- 不提前实现 `WebFetch / WebSearch / MCP Resource / ToolSearch`
- 产品层继续保留 `Search` 作为文本检索入口；`Grep` 允许作为实现层 primitive 存在

## 验收标准
- `Glob` 出现在真实 builtin tool surface 中
- `Search` 底层切到更强的 grep-like primitive，且现有 `Search` 使用方式保持兼容
- `Glob` 与 `List` 边界清晰：`List` 负责目录列举，`Glob` 负责递归路径模式匹配
- `Search/Grep-like` 与 `List/Glob` 边界清晰：文本模式检索不混入目录列举
- Rust 测试、宿主层相关回归和前端相关测试通过

## 当前进展
- 已确认 `Phase B` 的 spec / design 边界
- 已完成 `Glob / Grep-like Search` 第一轮落地实现：
  - `workspace_glob_files`
  - `workspace_search_text.regex` grep-like 增强
- 已完成 builtin surface、前端默认工具目录与 capability fallback 对齐
- 已用 `opencode / deepseek-v4-flash` 完成一轮 `Phase B` 实现审阅，结果见：
  [.tmp/pa052-opencode-review-phase-b.jsonl](/C:/Users/HUAWEI/Documents/pony-agent/.tmp/pa052-opencode-review-phase-b.jsonl)
- 已采纳部分审阅意见：
  - 前端 `hasDefaultOnly` 改为 name-based 判断，消除对工具顺序的脆弱依赖
  - 补充 `Glob limit` 与 `nested_batch_not_allowed` 回归
- 已通过的精确验证：
  - Rust 单测：
    - `glob_files_matches_paths_by_pattern`
    - `glob_files_respects_limit`
    - `search_text_supports_regex_like_wildcard_mode`
    - `builtin_tool_contract_views_deduplicate_to_model_surface`
    - `batch_rejects_nested_workspace_batch_calls`
  - 前端精确回归：
    - `tests/runtime-store.spec.ts` 中 builtin capability fallback 已包含 `builtin:workspace_glob_files` 并单测通过
  - 宿主层精确回归：
    - `cargo test --manifest-path src-tauri/Cargo.toml --test tool_router_regression glob_files_returns_repo_matches --target-dir target-test -- --exact --nocapture`
    - `cargo test --manifest-path src-tauri/Cargo.toml --test tool_router_regression glob_files_respects_limit --target-dir target-test -- --exact --nocapture`
- 已完成全量验证：
  - `npm run test:unit`
  - `cargo test --manifest-path crates/pony-agent-core/Cargo.toml --target-dir target-test`
  - `npm run cargo:test:regression`
  - `npm run test:tauri:smoke`
  - `npm run test:e2e`

## 下一步动作
1. 当前卡已完成；后续如需把 wildcard-like 检索升级到完整 regex，再另开兼容迁移卡

## 当前卡点
- 暂无。当前卡已完成态收口。

## 断点续跑提示
继续前先看：

- [PA-050](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-050-build-second-wave-tool-surface.md>)
- [PA-051](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-051-implement-phase-a-tool-gaps.md>)
- [tools.rs](/C:/Users/HUAWEI/Documents/pony-agent/crates/pony-agent-core/src/agent/tools.rs)
- [second-wave-tool-surface spec](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-second-wave-tool-surface/specs/second-wave-tool-surface/spec.md>)
