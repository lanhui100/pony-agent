# `pony-plugin-v1` fixture：echo 插件

进程外插件宿主（`crates/pony-agent-core/src/agent/plugin_host/`，ADR 0067）的端到端夹具。
不引入任何第三方依赖（仅 Python 3 标准库），用于验证：
`spawn → initialize（apiVersion + 能力协商）→ tools/list（注册）→ tools/call → shutdown`
以及隔离面与失败语义。

## 文件

| 文件 | 作用 |
|---|---|
| `echo-plugin.py` | 插件实现（stdio 按行分隔 JSON-RPC 2.0，`apiVersion = pony-plugin-v1`） |
| `plugin.json` | manifest 契约（`apiVersion/name/version/capabilities/tools/hookPoints`） |

## 工具

| 工具 | 行为 | 用途 |
|---|---|---|
| `echo` | 回显入参 `{"echo": <arguments>}` | 全链路握手/调用的正向证据 |
| `env` | 回显子进程完整环境 `{"env": {...}}` | 断言"env 集合精确等于白名单"，多一个键即泄漏 |
| `sleep` | 睡 `arguments.ms` 毫秒后返回 `{"sleptMs": N}` | 触发调用超时，断言宿主 kill 进程组后无残留子进程 |
| `crash` | 立即 `os._exit(3)` | 断言崩溃 fail-closed（`plugin_unavailable`）且不重试 |

## 用法（Unix）

```bash
chmod +x crates/pony-agent-core/tests/fixtures/plugin/echo-plugin.py   # 仓库已带执行位
```

```rust
use std::path::PathBuf;
use pony_agent_core::agent::plugin_host::{PluginHostConfig, PluginManifest, PluginSupervisor};

let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/plugin");
let manifest = PluginManifest::parse(&std::fs::read_to_string(root.join("plugin.json"))?)?;
let mut supervisor = PluginSupervisor::spawn(
    manifest,
    root.join("echo-plugin.py"),
    // cwd 必须存在且是目录：插件进程的 cwd 被限定在会话工作区内。
    PluginHostConfig::default().with_workspace_root(root.clone()),
)?;
supervisor.initialize()?;
let payload = supervisor.call_tool("echo", serde_json::json!({"hi": "there"}))?;
assert_eq!(payload["echo"]["hi"], "there");
supervisor.shutdown()?;
```

## 平台

- **macOS / Linux**：可用（`plugin.json` + `echo-plugin.py`，shebang + 执行位）。
- **Windows**：**跳过**。理由有二：① Windows 无 shebang 执行语义（需 `.cmd`/`py` 包装，
  属另一套 fixture）；② Windows 宿主**无进程组**，`PluginTransport::terminate` 只能 kill
  直接子进程，插件派生的孙进程为"尽力回收"（ADR 0067 已知局限）。验收测试应以
  `#[cfg(windows)]` 明确跳过，不做伪装覆盖。
