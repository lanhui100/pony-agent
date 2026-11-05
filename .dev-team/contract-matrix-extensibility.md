# 契约账本 —— 可扩展性五契约（ADR 0064–0068）

> 滚动账本，只记**已落盘事实**。ADR 0064–0068 状态仍为 `proposed`；**schema 是代码的投影，不是真值源**（与 Rust 类型不一致时以代码为准改 schema）。
> conformance kit 已落盘（进程内 28 例 + 外部 `--kit` 0/2/3）；`npm run verify` 接入由 Lead 统一完成，故不列为既有事实。

| 契约 | 版本 | 变更类型 | 消费方 | 兼容证据来源 |
|---|---|---|---|---|
| tool-descriptor | `pony-tool-descriptor-v1` | additive；**本轮修正**：`source` 枚举回到代码实况 4 值（`builtin/mcp/skill/dynamic`，`tools.rs::ToolDescriptorSource`），插件工具暂挂 `dynamic` + `descriptorId=dynamic:plugin:<name>:<tool>`；`plugin__`/`mcp__` 命名空间改由 modelName 前缀规则强制 | `ToolRegistrySnapshot` / `GovernedDispatcher` / `ToolProvider` | golden：真实内置 registry 全部 descriptor 逐条过 schema；插件 provider `list_tools()` descriptor 逐条过 schema |
| hooks | `agent-hooks-v1`（= `AGENT_HOOKS_CONTRACT_VERSION`） | additive；**本轮修正**：配置层未知键改为**容忍**（`hook_commands.rs` 显式不启用 `deny_unknown_fields`，拒绝会让存量共享配置整份失效） | `hook_commands.rs`（`.pony/hooks.json`） | golden：`command` 形态与 `argv` 形态各 1 例过 schema；负例「两形态同时出现」被拒 |
| mcp-config | `mcp-config-v1`（= `mcp_config.rs::MCP_CONFIG_SCHEMA_VERSION`） | additive；**本轮修正**：删除「显式拒绝 `url`」约束（代码无该判定，缺 `command` 即 `missing_command` 禁用该条） | `mcp_config.rs` 加载器 + `mcp_client.rs` | 负例「`env` 字面量 secret」被拒（只允许 `${env:VAR}`）；golden：`${env:}` 引用配置过 schema |
| skill-manifest | `pony-skill-manifest-v1` | additive（无变化） | `workspace_hotloader.rs` 的 SKILL.md frontmatter | 键名与 `SkillFrontmatter`（`name`/`description`/`user-invocable`/`allowed-tools`）一致；负例缺失 `name` 被拒 |
| plugin | `pony-plugin-v1` | additive；**本轮修正**：按 `manifest.rs::PluginManifest::validate` 对齐 —— 名称/工具名 `[A-Za-z0-9_-]{1,64}` 且不含 `__`、工具数 ≤256、description ≤4096、`hookPoints` 允许空数组、删除未落地的 `permissions`（fsRoots/exec/network 未实现）、移除 parse 期不存在的 tools/hookPoints 条件必填 | `plugin_host/`（supervisor + provider） | golden：T3 真实 fixture `plugin.json` 过 schema；负例「apiVersion 非枚举」「未知能力」被拒 |

## 校验收据（2026-11-05，本轮）

- `cargo test -p pony-agent-core --test conformance` → **28 passed / 0 failed / 0 ignored**（12 kit 例 + 5 退出码/反例 + 6 语义例 + 5 golden）。原 6 条 `#[ignore]` 已用内联 fixture 插件全部转正，无 `todo!()`。
- 语义例断言口径：只断言「必须失败 + 来源码家族可辨识」（`plugin_api_version_mismatch` / `plugin_capability_unsupported` / `plugin_protocol_error` / `plugin_timeout` / `plugin_unavailable`），不冻结错误串全文；超时/取消例用有界轮询断言子进程已回收。
- `node scripts/gen-extension-points.mjs --check` → **0**（归属表现已反映：`plugin_host/` 已存在、MCP 命中 `mcpServers` 配置面）；手工漂移探针 → **1**，恢复后 → 0。
- `cargo check -p pony-agent-core --bin conformance` → **0**。

## 未落盘（不得写成既成事实）

`npm run verify` 接线（Lead）、`--strict` 第三方外部用例集（当前只有 12 例内置契约形状）、扩展点归属表中 `ToolDescriptorSource` 的 `Plugin` 变体（需 T1 新增；届时 schema 与归属表由代码自动反映，不手改）。
