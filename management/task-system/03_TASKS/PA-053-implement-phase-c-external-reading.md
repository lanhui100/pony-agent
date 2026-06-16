# PA-053 实现 Phase C 外部读取与知识获取：WebFetch / WebSearch

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
`PA-050` 已定义第二批工具按 `Phase A -> Phase B -> Phase C -> Phase D` 顺序推进。当前 `Phase A` 已进入稳定验证，`Phase B` 已完成第一轮实现与验证，下一步应补齐外部读取与知识获取层：

- `WebFetch`
- `WebSearch`

当前真实代码状态仍缺少统一的外部网页读取与外部搜索 builtin primitive。

## 目标
把 `Phase C` 的外部读取能力实现为真实 builtin 工具能力，并保持与本地工具边界分层。

## 输出
- `WebFetch` builtin primitive
- `WebSearch` builtin primitive
- 对应工具注册、权限事实、前端默认工具目录和测试

## 范围边界
- 本卡只做 `Phase C`
- 不提前实现 `MCP Resource / ToolSearch`
- `WebFetch` 只负责读取指定 URL 内容
- `WebSearch` 只负责外部搜索，不把搜索和抓取混成一个工具

## 验收标准
- `WebFetch / WebSearch` 出现在真实 builtin tool surface 中
- `WebFetch` 返回结构化的 URL / status / title / content 预览或错误
- `WebSearch` 返回结构化的 query / result list / source URL / snippet
- Rust 测试、宿主层相关回归和前端相关测试通过

## 当前进展
- 已确认 `Phase C` 的 spec / design 边界
- 已完成 `WebFetch / WebSearch` 第一轮落地实现：
  - `web_fetch_url`
  - `web_search_query`
- 已完成 builtin surface、前端默认工具目录与 capability fallback 对齐
- 已用 `opencode / deepseek-v4-flash` 完成一轮 `Phase C` 实现审阅，结果见：
  [.tmp/pa053-opencode-review-phase-c.jsonl](/C:/Users/HUAWEI/Documents/pony-agent/.tmp/pa053-opencode-review-phase-c.jsonl)
- 已采纳部分审阅意见：
  - 补充 `extract_duckduckgo_results` 的纯函数成功路径测试
- 已通过的精确验证：
  - `cargo check --manifest-path crates/pony-agent-core/Cargo.toml --message-format short`
  - Rust 单测：
    - `web_fetch_rejects_non_http_urls`
    - `web_search_rejects_empty_query`
    - `extract_duckduckgo_results_parses_anchor_rows`
  - 前端精确回归：
    - `tests/runtime-store.spec.ts` 中 builtin capability fallback 已包含 `builtin:web_fetch_url` / `builtin:web_search_query` 并单测通过
- 已补成功路径与 HTTP 错误路径验证：
  - `web_fetch_returns_success_payload_for_http_response`
  - `web_search_returns_results_for_http_success_page`
  - `web_fetch_returns_structured_http_error_for_non_2xx_response`
  - `web_search_returns_structured_http_error_for_non_2xx_response`
- 已完成全量验证：
  - `npm run test:unit`
  - `cargo test --manifest-path crates/pony-agent-core/Cargo.toml --target-dir target-test`
  - `npm run cargo:test:regression`
  - `npm run test:tauri:smoke`
  - `npm run test:e2e`

## 下一步动作
1. 当前卡已完成；后续如需把外部读取权限从 `workspace.read` 细化到独立 scope，再另开权限策略卡

## 当前卡点
- 暂无。当前卡已完成态收口。

## 断点续跑提示
继续前先看：

- [PA-050](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-050-build-second-wave-tool-surface.md>)
- [PA-051](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-051-implement-phase-a-tool-gaps.md>)
- [PA-052](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-052-implement-phase-b-repo-discovery.md>)
- [tools.rs](/C:/Users/HUAWEI/Documents/pony-agent/crates/pony-agent-core/src/agent/tools.rs)
- [second-wave-tool-surface spec](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-second-wave-tool-surface/specs/second-wave-tool-surface/spec.md>)
