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

## Addendum（同日续：重读技能后的第二处收口）

重读 `SKILL.md` 与 `references/prompting-and-patterns.md`、`references/search.md`、`references/common-mistakes.md`
后，仍有四处未完全对齐，本次一并收口（依旧不新增端点）：

1. **结果映射**（新增纯函数 `exa_search_result_items`）：技能 design tip 要求“返回 `highlights` 而非整页 `text`
   以压低工具结果 token”，且 `/search` 每条结果天然携带 `title` / `url` / `publishedDate`。原实现只把
   `highlights[0]` 投影成 `snippet`，其余摘录与发布日期全部丢弃。现改为
   `title` / `url` / `publishedDate` / `highlights[]`（全量）/ `score`，移除 `snippet`。
   仓库内无 `snippet` 消费方（`src/` 零命中；`telemetry.rs` 只做通用 JSON 摘要，不按字段取用），属无损替换。
2. **工具描述**：技能建议描述出现 "search the live web ... fresh information beyond the model's training data"，
   以便模型在需要新鲜信息时选该工具；`schema.rs` 与前端镜像 `src/lib/runtime/browser-preview.ts` 同步改写。
3. **错误体**：原实现只读 `error.message`（对象形态）。Exa 也会返回 `{"error":"...","tag":"..."}` 字符串形态，
   会被降级成「未知错误」。新增 `exa_error_message`，两种形态都读。
4. **冻结面事实（只记录、不改）**：`tool_surface_singularity` 的签名仅覆盖
   `(descriptor_id, model_name, kind, exposure, FNV(input_schema))`，**不含描述文本**。因此本轮改了描述但
   golden 不变（实测 23 passed）。这也意味着描述漂移不会被该门禁拦住——已知缺口，扩展签名需一次性重算
   45 条 golden，属独立卡，本轮不静默扩大改动面。

## Alternatives considered（第二轮）

1. **保留 `snippet` 并额外返回 `highlights`** — 否决：两者内容重复（`snippet` 恒等于 `highlights[0]`），
   只会让工具结果 token 翻倍，与技能“压低 token”取向相反。
2. **不返回 `publishedDate`** — 否决：技能明确把 `publishedDate` 列为每条结果天然携带的字段，
   缺失会让模型无法判断“最新/近期”类需求是否被满足。
3. **对 `objective` 按 4096 上限做本地校验** — 暂缓：技能只说明该字段上限，未要求客户端校验；
   新增一条模型可见错误码属额外契约，等出现真实超长 `objective` 反馈再定。
4. **顺手扩展冻结签名覆盖描述文本** — 否决（本轮）：需重算全部 45 条 golden，属独立卡；
   本轮在 note 与 `docs/architecture/runtime.md` 明示该缺口即可。
5. **为请求体/结果形态做真实 HTTP 抓包** — 沿用上轮结论：会新增 `EXA_API_BASE_URL` 之类环境契约与出网面，
   继续用纯函数（`exa_search_request_body` / `exa_search_result_items` / `exa_error_message`）离线断言形态。

## Evidence（第二轮）

- `cargo test -p pony-agent-core --lib -- agent::tools::tests::web`：**40 passed**（含新增
  `exa_search_result_items_keep_all_highlights_and_published_date`、`exa_error_message_accepts_string_and_object_shapes`）。
- `cargo test -p pony-agent-core --test tool_surface_singularity`：**23 passed**（描述变更不影响 golden，见上文第 4 点）。
- `node scripts/check-ts-contract.mjs`：通过；`node scripts/check-decisions.mjs`：117 篇一致。
- `npx vitest run tests/runtime-store.spec.ts`：**119 passed**（前端镜像描述改动无回归）。
