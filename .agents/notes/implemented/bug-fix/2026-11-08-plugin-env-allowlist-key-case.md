# Agent Note: 插件子进程 env 白名单键名原样透传（B040）

Status: implemented

## Problem

backlog `B040`（2026-11-06 gate 实测登记）：`plugin_host/capability.rs` 的 `child_env()`
对白名单变量名统一 `to_ascii_uppercase()` 落库，而 F8a 名单（复用 `MCP_ENV_ALLOWLIST`）
同时含大小写两组代理键。Linux 环境键大小写敏感：父进程只有小写 `http_proxy` /
`https_proxy` / `no_proxy` 时，`var_os("http_proxy")` 命中后却被改写成 `HTTP_PROXY`
交给子进程 ⇒ `t3_plugin_child_env_is_exactly_the_allowlist` 红
（left={HOME,HTTPS_PROXY,HTTP_PROXY,LANG,NO_PROXY,PATH} vs right=…+{http_proxy,https_proxy,no_proxy}），
且 curl 等工具链只认小写 `http_proxy`，插件实际丢失代理配置。
MCP 侧 `mcp_child_env` 一直是原样透传（`OsString::from(name)`），插件侧属漂移。

## Decision

- `child_env()` 白名单段落改为 `vars.push((OsString::from(*name), value))`——键名按平台
  语义原样透传，与 MCP 侧 F8a 对齐：POSIX 大小写敏感，`var_os(name)` 命中的就是名单原始
  形态；Windows `var_os` 走 `GetEnvironmentVariableW` 大小写不敏感，键名形态不影响匹配。
- 安全面不 loosening：`is_proxy_env_name` 本就 `eq_ignore_ascii_case`，小写键仍走
  `redact_proxy_userinfo`（userinfo 剥除/畸形丢弃）；拒绝键判定 `is_denied_env_key`
  亦大小写不敏感，均不受键名形态影响。
- 新增单测 `proxy_name_detection_covers_lowercase_keys` 固化「小写代理键必须被判为
  代理变量并走脱敏」；键名精确契约由既有 hermetic 集成用例 t3 覆盖（不新增 env 改写型
  单测，避免并行测试间进程环境串扰）。

## Alternatives considered

- **名单去重只留大写 + 查找时大小写不敏感扫描父进程**：要自实现 env 扫描（`var_os`
  在 POSIX 无法大小写不敏感命中），且改变「白名单 ∩ 父进程」的精确集合语义，t3 契约
  反而更难过；原样透传零新增机制。
- **仅 Windows 保留大写、POSIX 条件编译**：原样透传在 Windows 上行为等价
  （`var_os` 不敏感 + `Command::env` 落库形态不改变量语义），无需 `#[cfg]` 分叉。

## Verification

- `cargo test -p pony-agent-core --lib plugin_host::capability`：8 passed / 0 failed
  （含新增 `proxy_name_detection_covers_lowercase_keys`）。
- `cargo test -p pony-agent-core --test plugin_host_contract`：11 passed / 0 failed
  （t3 转绿，本机父进程带小写代理键的真实环境下验证）。
- 登记：backlog `B040` 勾选完成。