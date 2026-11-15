# ACP 代理自动注入 + agent id 暴露契约矩阵（backlog B183 / task-25）

> 状态：契约冻结（常量 + 字段 + 签名 + 文档；实现在 task-26）。Owner：Protocol Agent。
> 目标用户诉求：在 Pony（TUI/模型对话）里直接让 agy 干活——①代理自动注入（不再手动 `source ~/.pony/proxy.env`）；②模型知道可用 agent id。
> 写域：`crates/pony-agent-core/src/acp/`、`agent/tools/schema.rs`、`.dev-team/`。

## 1. 签名清单（path:line）

| 位置 | 内容 |
|---|---|
| `acp/config.rs:29 / 37` | `ACP_PROXY_ENV_DIR = ".pony"` / `ACP_PROXY_ENV_FILE_NAME = "proxy.env"` ⇒ 完整路径 `<HOME>/.pony/proxy.env`（用户只需维护这一份 `KEY=VALUE`，可带 `export `） |
| `acp/config.rs:87` | `AcpAgentSpec.inherit_proxy_env: bool`，`#[serde(default = "default_true", rename = "inheritProxyEnv")]`（缺省 **true**；false ⇒ 该 agent 不读代理文件） |
| `acp/config.rs:40` | `fn default_true() -> bool`（serde 缺省值 = true） |
| `acp/config.rs:570` | `pub fn resolve_proxy_env() -> Vec<(String, String)>`（**签名空桩**，task-26 实现）：行格式 `KEY=VALUE`、忽略空行/`#` 注释、可选 `export `、去引号；**只保留 `*_proxy`（大小写不敏感）**；逐键过 `crate::agent::plugin_host::capability::is_denied_env_key`；**文件不存在 ⇒ 空 Vec 不报错**；读失败只记诊断并返回空 |
| `acp/transport.rs:255` | `spawn` 文档冻结 **env 合并顺序**：宿主白名单（`apply_child_env`）→ `resolve_proxy_env()`（若 `spec.inherit_proxy_env`）→ `spec.env`（覆盖）→ `extra_env`（调用方最高优先） |
| `acp/transport.rs:297` | 实现处注释标出 task-26 的插入点（proxy.env 插在 `spec.env` 之前，保证清单 env 可覆盖代理键）；当前实现只做白名单/spec.env/extra_env |
| `agent/tools/schema.rs:1137` | `acp_delegate` 工具 description 显式列出 builtin id：**`opencode`、`antigravity`** + 「自定义 agent 见 `ponycli acp --list`」 |
| `agent/tools/schema.rs:1143` | `agent` 参数 description 同步（模型据此直接填 id，不必猜） |

## 2. 断言面

| ID | 断言 |
|---|---|
| G1 | **proxy.env 解析边界**：`KEY=VALUE` / `export KEY=VALUE` / 值单双引号 / `#` 注释 / 空行 / 行尾空白逐一正确；非法行（无 `=`）跳过不 panic；**文件不存在 ⇒ 空 `Vec`**（不报错、不阻断 spawn）。 |
| G2 | **只注入 `*_proxy`**：`http_proxy` / `https_proxy` / `all_proxy` / `no_proxy`（含大写下划线与小写形态）保留；`PATH` / `FOO` / `PONY_*` 一律丢弃。 |
| G3 | **deny 键拒绝**：命中 `is_denied_env_key` 的键（`LD_PRELOAD` / `LD_LIBRARY_PATH` / `SSH_AUTH_SOCK` / provider key 类）即使形如 `*_proxy` 也不得注入；诊断只记键名、绝不回显值。 |
| G4 | **覆盖顺序**：同名键以 `spec.env` 覆盖 proxy.env；`extra_env` 再覆盖 `spec.env`；所有来源最终仍经 `apply_child_env` 白名单/拒绝键二次收口（三层不得绕过 F8a 隔离）。 |
| G5 | **inheritProxyEnv=false**：`spawn` 完全不读 `<HOME>/.pony/proxy.env`（子进程 env 与未实现 B183 前一致）；缺省字段（清单未写）= true。 |
| G6 | **模型可见 agent id**：`acp_delegate` 的 `description` 同时包含 `opencode` 与 `antigravity` 两个 id + `ponycli acp --list`；`agent` 参数 description 同源；工具面清单/JSON schema 断言（数量、required、additionalProperties）不变。 |
| G7 | **无回归**：既有 env 隔离（白名单 + `env_clear()`）、`AcpAgentSpec` 其余字段语义、`spawn` 签名不变；`inherit_proxy_env` 序列化名 `inheritProxyEnv` 且缺省不破坏旧清单（缺字段 = true）。 |

## 3. 边界与已知限制

- `resolve_proxy_env()` 本批为签名空桩（无调用点），由 task-26 实现并在 `transport::spawn` 接线；**在 task-26 落地前，代理注入不生效**（文档已显式标注，避免误判）。
- 代理文件是**用户自维护的明文**：只做键过滤与去引号，不做值级脱敏；日志只记键名/条数。
- `<HOME>` 解析走既有 `dirs`/`HOME` 口径（与 `providers.json` 同源）；Windows 下 `HOME` 缺失时按用户目录回退（executor 实现时对齐 `pony_agent_home` 既有逻辑）。
- `AcpAgentSpec` 新增字段属 additive；测试若显式构造字面量需补 `inherit_proxy_env: true`（Test Agent 写域）。

## 4. 门禁收据

- `cargo check -p pony-agent-core --lib` → **Exit 0**（0 warning）；`npm run cargo:check:shared` → **Exit 0**（`Finished dev profile ... in 16.53s`）。
- 未改 `acp/client.rs`、`acp/session.rs`、`acp/protocol.rs`、`acp/handlers.rs` 与 `agent/*`（除 `tools/schema.rs` 描述）。
