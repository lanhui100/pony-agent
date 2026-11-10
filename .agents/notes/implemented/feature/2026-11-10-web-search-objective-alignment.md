# Agent Note: `web_search_query` 对齐 build-with-exa 技能（objective 必填 + 请求体收敛）

Status: implemented

- 日期：2026-11-10
- 触发：安装 `build-with-exa` 技能（`exa-labs/agent-skills`，`.agents/skills/build-with-exa/`）后，对现有 Exa 集成做合规复核
- 交付面：`crates/pony-agent-core`（`web_search_query` 工具）+ 前端 browser-preview 兜底目录镜像

## Context

项目内既有的 Exa 搜索工具 `web_search_query`（`agent/tools/web.rs` + `agent/tools/schema.rs`）只接收
`query`，并固定发送 `{query, type:"auto", numResults:limit, contents:{highlights:true}}`。这与
`build-with-exa` 技能的第一条硬红线冲突：

- **LLM 作为工具调用的搜索必须带必填 `objective`**：技能要求在 `query` 旁提供 `objective`，其描述逐字为
  “Goal for this search turn; say which documents should rank first, which should be excluded, and what
  specific facts or figures to pull from them.”，并把模型给的值原样透传给 `search`。
- **推荐请求 = query + token 省内容抽取，别的都不要**：`numResults` 默认 10、`type` 默认 auto，
  技能要求除 `objective` 外的字段一律按需门控，不得为“看起来有用”而堆砌。
  原实现把 `limit` 映射成 `numResults`、并显式 `type:"auto"`，属被点名的“过度装饰”反模式。

## Decision

按用户批准的 A 方案，只改造既有 `web_search_query`，不新增端点：

1. **schema**（`schema.rs`）：`input_schema.properties` 用 `objective`（必填、逐字英文描述）替换原 `limit`；
   `required` 变为 `["query","objective"]`（`with_description` 仍会追加 UI `description`）。改动行数中性，
   不抬高 `builtin_tools` 的函数行数棘轮。
2. **请求体**（`web.rs`）：新增纯函数 `exa_search_request_body(query, objective)`，请求体收敛为
   `{query, objective, contents:{highlights:true}}`；删除 `type`/`numResults`/`limit` 解析。`timeoutMs`
   仍是本地传输超时（非 Exa 字段），保留。
3. **校验**：缺失 `objective` → `missing_argument`；空串 → `empty_objective`（与既有 `empty_query` 同风格）。
   输出回显 `objective` 便于取证。
4. **前端镜像**（`src/lib/runtime/browser-preview.ts`）：WebSearch 目录项同步 `objective`、移除 `limit`。
5. **测试**：新增 `web_search_requires_objective` / `web_search_rejects_empty_objective` /
   `exa_search_request_body_matches_recommended_shape`；`web_search_returns_timeout_error_after_retries`
   补齐 `objective`。
6. **冻结 golden**（`tests/tool_surface_singularity.rs`）：`builtin:web_search_query` 哈希由
   `0xe28717c7e81f74c2` 更新为实测 `0x8e6bb1b6acd76fd1`，附注来源。属模型面契约变更，须走 decl-change 评审，
   commit 信息带 `[decl-change-approved]`。

### 顺带修复（非本卡引入）

同一冻结测试还暴露 `builtin:ask_user` 的 golden 与 HEAD 不一致：HEAD `73857b6e`
（ADR 0117「Ask 面板端到端多选」）给 `ask_user` schema 增加了 `multiSelect`，但同提交未同步 golden，
导致干净 HEAD 该用例本就红。本次把 `builtin:ask_user` 哈希由 `0x3151a4babe493151` 补齐为实测
`0x2450d4c82f154d1c` 并注释来源——这是该已批准变更的收尾，不是本卡 web_search 改造的一部分。

## Alternatives considered

1. **只在 schema 加 `objective`、保留 `type/numResults`** — 否决：仍违反技能“除 objective 外字段按需门控、推荐请求别的都不要”，
   且 `limit` 与固定 `numResults` 是技能明列的反模式。
2. **新增独立 `/contents`、`/answer` 或 `/agent` 端点模块** — 否决：当前只有一个“LLM 调用的搜索工具”场景，
   优先把既有端点改对；新端点属另一波需求（用户选 A 时已排除）。
3. **给 `web_search_query` 增加 `EXA_API_BASE_URL` 环境覆盖以便请求体走真实 HTTP 抓包测试** — 否决：
   会新增一个环境变量契约与出网面，需单独的 observability/声明变更；改用纯函数 `exa_search_request_body`
   做形态断言，既锁定契约又不扩大生产面。
4. **把冻结断言弱化为“只看字段名集合、不校验哈希”** — 否决：哈希冻结的目标正是 KV-cache 前缀与模型契约
   的逐字节稳定，弱化后字段顺序/描述漂移会静默通过（与 plan_control 收尾 note 同口径）。
5. **对 `ask_user` 的既有漂移视而不见** — 否决：那会让本卡改动的同一条冻结门禁长期真红；但也不“照抄实测值”，
   而是先定因（ADR 0117 multiSelect 漏同步）再补齐。

## Evidence

- `cargo test -p pony-agent-core --test tool_surface_singularity t1_builtin_surface_identity_is_frozen`：1 passed（45/45 冻结面一致）。
- `cargo test -p pony-agent-core --lib web_search`：5 passed（empty_query / requires_objective / empty_objective / timeout_retries / telemetry）。
- `cargo test -p pony-agent-core --lib exa_search_request_body`：1 passed（请求体只含 query+objective+contents.highlights）。
- `cargo test -p pony-agent-core --lib builtin_tool_schemas_require_ui_descriptions`：1 passed。
- `cargo test --manifest-path src-tauri/Cargo.toml --test tool_router_regression web_search`：1 passed。
- `node scripts/check-file-size.mjs --baseline scripts/file-size-baseline.json`：`builtin_tools` 行数 1074 与 HEAD 持平
  （既有基线 1070 已陈旧，属存量债，本卡未新增）；`web_search` 仍在 200 行内。
