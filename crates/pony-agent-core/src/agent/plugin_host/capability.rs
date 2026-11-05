//! 插件宿主的隔离面：环境白名单（F8a）、工作目录限定、能力协商。
//!
//! 唯一真值约定：**子进程环境只有一个入口** [`apply_child_env`]（先 `env_clear()` 再按
//! [`ENV_ALLOWLIST`] 透传），任何新增的 spawn 路径都必须走它，否则隔离面会被悄悄重新打开。
//!
//! [`ENV_ALLOWLIST`] 直接复用 MCP 客户端（`agent/mcp_client.rs`）的 F8a 名单——不复制第二份
//! 名单，避免"插件比 MCP 多透传一个凭据变量"这类漂移。该名单已经刻意排除：
//! provider API key、`SSH_AUTH_SOCK`、`LD_*` / `DYLD_*` 加载器劫持项、TLS 信任覆盖项。

use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::agent::mcp_client::MCP_ENV_ALLOWLIST;

use super::manifest::{PluginManifest, CAPABILITY_HOOKS, CAPABILITY_TOOLS};

/// 插件/命令钩子子进程的环境白名单（F8a 单一真值，复用 MCP 侧名单）。
pub const ENV_ALLOWLIST: &[&str] = MCP_ENV_ALLOWLIST;

/// 代理变量：值里的 userinfo（`http://user:pass@host`）是宿主凭据，透传前必须剥掉。
const PROXY_ENV_NAMES: &[&str] = &[
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "NO_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
    "no_proxy",
];

/// 显式 inject 的环境键拒绝表（当前 v1 无 inject 面，保留下表供未来 spec.env 复用）。
///
/// 拒绝理由与 MCP 侧 F8a 一致：进程/加载器劫持向量（`PATH` 覆写、`LD_PRELOAD`）与凭据通道
/// （`SSH_AUTH_SOCK`、TLS 信任覆盖）。只在**显式声明**的键上生效；白名单透传不受影响。
const DENIED_ENV_KEYS: &[&str] = &[
    "PATH",
    "PATHEXT",
    "COMSPEC",
    "SHELL",
    "BASH_ENV",
    "ENV",
    "NODE_OPTIONS",
    "NODE_PATH",
    "PYTHONPATH",
    "PYTHONSTARTUP",
    "PYTHONHOME",
    "SSH_AUTH_SOCK",
    "SSH_AGENT_PID",
    "GIT_SSH",
    "GIT_SSH_COMMAND",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "NODE_EXTRA_CA_CERTS",
    "CURL_CA_BUNDLE",
    "REQUESTS_CA_BUNDLE",
];

const DENIED_ENV_PREFIXES: &[&str] = &["LD_", "DYLD_", "AWS_"];

/// 某个环境键是否属于"显式声明也必须拒绝"的劫持/凭据通道。
pub fn is_denied_env_key(key: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    DENIED_ENV_KEYS.iter().any(|denied| upper == *denied)
        || DENIED_ENV_PREFIXES
            .iter()
            .any(|prefix| upper.starts_with(prefix))
}

fn is_proxy_env_name(name: &str) -> bool {
    PROXY_ENV_NAMES
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(name))
}

/// 代理 URL 的 userinfo 剥除（`http://user:pass@proxy:8080` → `http://proxy:8080`）。
///
/// 返回值语义：`Some(值)` = 可安全透传；`None` = **整体丢弃该变量**。
///
/// 处理顺序（安全 > 便利，三层）：
/// 1. **无 scheme 的值**（`user:pass@10.0.0.1:8080`，部分工具链里真实存在）必须在 `Url::parse`
///    之前处理：`url` 会把它解析成 scheme=`user`、path=`pass@10.0.0.1:8080` 的"合法 URL"，
///    username 为空 → 旧逻辑会把 userinfo 原样透传（正是要堵的凭据泄漏路径）。
///    这里按 `userinfo@host[:port]` 形状剥掉 `@` 之前的部分；
/// 2. 标准 URL：解析后清空 username/password（保留 scheme/host/port）；
/// 3. 含 `@` 但形状无法安全解析（畸形 userinfo）：**整条丢弃**，调用方把变量名打到 stderr
///    （只打名字，绝不打值——值里可能就是凭据）。丢一个畸形代理变量远好于泄漏凭据；
///    不含 `@` 且无 `://` 的值（如 `10.0.0.1:8080`、`::odd::`）没有 userinfo 可泄漏，
///    原样保留（不做无谓改写，避免让用户代理配置"看起来变了"）。
///
/// 该行为比 `mcp_client` 侧 F8a 更严格（那边解析失败时原样透传，属已知遗留，不在本批写域）。
fn redact_proxy_userinfo(value: &OsStr) -> Option<OsString> {
    let text = value.to_str()?;

    // 无 scheme 形态必须在 `Url::parse` **之前**处理：`url` 会把 `user:pass@10.0.0.1:8080`
    // 解析成 scheme=`user`、path=`pass@10.0.0.1:8080` 的"合法 URL"，username 为空，
    // 于是旧逻辑会原样透传 userinfo —— 这正是要堵的凭据泄漏路径。
    if !text.contains("://") {
        return match text.rsplit_once('@') {
            Some((userinfo, remainder)) if !userinfo.is_empty() && !remainder.is_empty() => {
                Some(OsString::from(remainder))
            }
            // 没有 `@`：没有 userinfo 可剥（如 `10.0.0.1:8080`），原样保留。
            None => Some(value.to_os_string()),
            // `user:pass@`（@ 后为空）等畸形：丢弃，不原样透传。
            Some(_) => None,
        };
    }

    let Ok(mut parsed) = url::Url::parse(text) else {
        // 带 scheme 但解析不了：可能是畸形 userinfo，丢弃（安全优先）。
        return None;
    };
    if parsed.username().is_empty() && parsed.password().is_none() {
        return Some(value.to_os_string());
    }
    if parsed.set_username("").is_err() || parsed.set_password(None).is_err() {
        // 解析成功但改不动（极端畸形）：宁可丢弃也不原样透传。
        return None;
    }
    let redacted = parsed.as_str().replace("://@", "://");
    // `url` 会把 `http://host:port` 规范成带尾斜杠；用户配置里没有的字符不要凭空添加
    // （只做 userinfo 剥离，其余保持原样，避免让代理配置"看起来变了"）。
    let redacted = if !text.ends_with('/') && redacted.ends_with('/') {
        redacted.trim_end_matches('/').to_string()
    } else {
        redacted
    };
    Some(OsString::from(redacted))
}

/// 构造子进程环境：白名单 ∩ 父进程 + 显式声明（拒绝键丢弃并告警）。
///
/// 返回顺序稳定（白名单顺序在前，显式声明在后，同名后写覆盖）。
pub fn child_env(extra: &[(String, String)]) -> Vec<(OsString, OsString)> {
    let mut vars: Vec<(OsString, OsString)> = Vec::new();
    for name in ENV_ALLOWLIST {
        if let Some(value) = std::env::var_os(name) {
            let value = if is_proxy_env_name(name) {
                match redact_proxy_userinfo(&value) {
                    Some(redacted) => redacted,
                    None => {
                        // 只打变量名，绝不打值（值里可能带代理账号口令）。
                        eprintln!(
                            "[pony-agent][plugin] 丢弃代理环境变量 `{name}`（无法安全剥离 userinfo）。"
                        );
                        continue;
                    }
                }
            } else {
                value
            };
            // 白名单变量名统一用大写形态落库，便于调用方/审计做集合比较。
            vars.push((OsString::from(name.to_ascii_uppercase()), value));
        }
    }
    for (key, value) in extra {
        if is_denied_env_key(key) {
            // 只打印键名，绝不打印值（值可能就是凭据）。
            eprintln!(
                "[pony-agent][plugin] 拒绝透传环境键 `{key}`（进程/加载器劫持或凭据通道向量）。"
            );
            continue;
        }
        let value = if is_proxy_env_name(key) {
            match redact_proxy_userinfo(OsStr::new(value)) {
                Some(redacted) => redacted,
                None => {
                    eprintln!(
                        "[pony-agent][plugin] 丢弃显式声明的代理环境变量 `{key}`（无法安全剥离 userinfo）。"
                    );
                    continue;
                }
            }
        } else {
            OsString::from(value)
        };
        vars.push((OsString::from(key), value));
    }
    vars
}

/// 子进程环境的唯一收口：`env_clear()` + 白名单透传。
pub fn apply_child_env(command: &mut Command, extra: &[(String, String)]) {
    command.env_clear();
    for (key, value) in child_env(extra) {
        command.env(key, value);
    }
}

/// 校验并规范化工作目录（必须存在、必须是目录）。
///
/// 白名单环境 + 限定 cwd 的组合意义：插件进程即使被攻陷，也无法通过 `$HOME`/`$TMPDIR`
/// 之外的相对路径漂移到宿主的会话工作区之外。
pub fn resolve_workspace_dir(root: &Path) -> Result<PathBuf, String> {
    let metadata = std::fs::metadata(root).map_err(|err| {
        format!(
            "plugin_workspace_invalid: 插件工作目录 {} 不可用（{err}）。",
            root.display()
        )
    })?;
    if !metadata.is_dir() {
        return Err(format!(
            "plugin_workspace_invalid: 插件工作目录 {} 不是目录。",
            root.display()
        ));
    }
    // canonicalize 失败不致命（如权限受限的只读目录）：退化为原路径。
    Ok(std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf()))
}

/// 把配置里的 `cwd` 限定在会话工作区内（**canonicalize 后的**路径段边界判定）。
///
/// 语义：
/// - 相对路径 → 相对 `root`；绝对路径 → 必须落在 `root` 内；
/// - `..` 由文件系统解析（不是词法抵消），因此 `root/a/../b` 与符号链接都按真实落点判断；
/// - **符号链接逃逸一律拒绝**（fail-closed）：工作区内一个指向外部的 symlink 不得成为
///   插件/hook 进程的 cwd —— 纯词法 `starts_with` 会放过这种逃逸（L3 复审缺陷）；
/// - 候选不存在时，canonicalize 其**父目录**再拼接末段（仍做边界判定），因此"越界但不
///   存在"的路径同样被拒绝而不会误报成"不存在"；
/// - 目标必须是目录。
///
/// 残留风险（可接受）：`canonicalize` 与随后 `current_dir` 之间存在 TOCTOU 窗口。返回的是
/// **canonical 路径**（不含符号链接），替换它需要改写真实目录项，成本远高于改写 symlink；
/// 彻底的 no-follow 语义需要 `openat2`/`O_NOFOLLOW` 逐段打开，超出本批范围。
pub fn confine_dir(root: &Path, candidate: &Path) -> Result<PathBuf, String> {
    let root = resolve_workspace_dir(root)?;
    let joined = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        root.join(candidate)
    };

    // 1) 真实落点：优先整体 canonicalize；不存在则 canonicalize 父目录再拼末段。
    let resolved = match std::fs::canonicalize(&joined) {
        Ok(path) => path,
        Err(_) => {
            let (Some(parent), Some(name)) = (joined.parent(), joined.file_name()) else {
                return Err(format!(
                    "plugin_cwd_denied: 目录 {} 无法解析（不存在或不是可解析的路径）。",
                    candidate.display()
                ));
            };
            let parent = std::fs::canonicalize(parent).map_err(|_| {
                format!(
                    "plugin_cwd_denied: 目录 {} 的父目录不在可解析的文件系统中（不存在或不可访问）。",
                    candidate.display()
                )
            })?;
            parent.join(name)
        }
    };

    // 2) 边界判定用 canonical 后路径的**路径段**比较（`Path::starts_with` 按段匹配，
    //    所以 `/root/bc` 不会被 `/root/b` 误判为子路径）。
    if !resolved.starts_with(&root) {
        return Err(format!(
            "plugin_cwd_denied: 目录 {} 的真实落点 {} 不在会话工作区 {} 内（含符号链接逃逸）。",
            candidate.display(),
            resolved.display(),
            root.display()
        ));
    }

    // 3) 必须是目录（指向文件的符号链接同样拒绝）。
    if !resolved.is_dir() {
        return Err(format!(
            "plugin_cwd_denied: 目录 {} 不存在或不是目录。",
            resolved.display()
        ));
    }
    Ok(resolved)
}

/// 协商结果：宿主与插件共同支持的能力集合（默认空 = 无条件放行任何能力）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NegotiatedCapabilities {
    capabilities: BTreeSet<String>,
}

impl NegotiatedCapabilities {
    /// 空协商结果（initialize 之前的状态：无任何能力）。
    pub fn empty() -> Self {
        Self {
            capabilities: BTreeSet::new(),
        }
    }

    /// 协商（失败闭合）：
    ///
    /// - `hooks` 在 v1 未迁移（ADR 0067 Non-Goal）：请求或声明了它一律拒绝，
    ///   避免出现"声明了 hooks 却永不触发"的假承诺；
    /// - 插件侧声明的能力**必须**在 manifest 中声明过（"声明外一律拒绝"）；
    /// - 结果取交集。
    pub fn negotiate(
        manifest: &PluginManifest,
        plugin_declared: &[String],
    ) -> Result<Self, String> {
        let mut granted = BTreeSet::new();
        for capability in plugin_declared {
            if capability == CAPABILITY_HOOKS {
                return Err(
                    "plugin_capability_denied: hook 能力在插件协议 v1 未迁移（ADR 0067 Non-Goal），拒绝协商。"
                        .to_string(),
                );
            }
            if !manifest.declares(capability) {
                return Err(format!(
                    "plugin_capability_denied: 插件在 initialize 中声明了 manifest 未声明的能力 `{capability}`。"
                ));
            }
            granted.insert(capability.clone());
        }

        if manifest.declares(CAPABILITY_HOOKS) {
            return Err(
                "plugin_capability_denied: manifest 声明了 hook 能力，但插件协议 v1 不迁移 hooks（ADR 0067 Non-Goal）。"
                    .to_string(),
            );
        }
        // tools 能力必须双向声明：manifest 声明后插件在 initialize 中也必须接受。
        if manifest.declares_tools() && !granted.contains(CAPABILITY_TOOLS) {
            // 插件未在 initialize 显式回显能力时，按 manifest 授予（宽容读法：能力协商只做
            // 收窄，不作扩权；扩权仍然不可能，因为工具注册还要经过 manifest 白名单校验）。
            granted.insert(CAPABILITY_TOOLS.to_string());
        }

        Ok(Self { capabilities: granted })
    }

    /// 空集合 = 无任何能力。
    pub fn is_empty(&self) -> bool {
        self.capabilities.is_empty()
    }

    /// 是否授予某能力。
    pub fn allows(&self, capability: &str) -> bool {
        self.capabilities.contains(capability)
    }

    /// 授予的能力（稳定排序，供审计）。
    pub fn as_slice(&self) -> Vec<String> {
        self.capabilities.iter().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::plugin_host::manifest::PluginManifest;

    fn manifest_with(capabilities: &str) -> PluginManifest {
        PluginManifest::parse(&format!(
            r#"{{"apiVersion":"pony-plugin-v1","name":"probe","version":"0.1.0","capabilities":{capabilities},"tools":[]}}"#
        ))
        .expect("manifest")
    }

    #[test]
    fn env_allowlist_is_the_single_truth_of_mcp_f8a() {
        assert_eq!(ENV_ALLOWLIST, MCP_ENV_ALLOWLIST);
        assert!(ENV_ALLOWLIST.contains(&"PATH"));
        // 凭据/加载器向量不得出现在白名单里。
        for forbidden in ["LD_PRELOAD", "DYLD_INSERT_LIBRARIES", "SSH_AUTH_SOCK", "OPENAI_API_KEY"] {
            assert!(!ENV_ALLOWLIST.contains(&forbidden), "{forbidden} 不得透传");
        }
    }

    #[test]
    fn child_env_never_leaks_non_allowlisted_keys() {
        std::env::set_var("PONY_PLUGIN_SECRET_PROBE", "leak-me");
        let env = child_env(&[]);
        assert!(
            env.iter().all(|(key, _)| {
                let key = key.to_string_lossy().to_string();
                ENV_ALLOWLIST.contains(&key.as_str())
            }),
            "子进程环境必须严格等于白名单子集"
        );
        std::env::remove_var("PONY_PLUGIN_SECRET_PROBE");
    }

    #[test]
    fn denied_env_keys_are_dropped_even_when_explicit() {
        let env = child_env(&[
            ("LD_PRELOAD".to_string(), "/tmp/evil.so".to_string()),
            ("PONY_OK".to_string(), "1".to_string()),
        ]);
        let keys: Vec<String> = env
            .iter()
            .map(|(key, _)| key.to_string_lossy().to_string())
            .collect();
        assert!(!keys.contains(&"LD_PRELOAD".to_string()));
        assert!(keys.contains(&"PONY_OK".to_string()));
    }

    #[test]
    fn proxy_userinfo_is_stripped_or_the_variable_is_dropped() {
        // 标准 URL：剥掉 userinfo，保留 scheme/host/port。
        assert_eq!(
            redact_proxy_userinfo(OsStr::new("http://user:pass@proxy.local:8080")),
            Some(OsString::from("http://proxy.local:8080"))
        );
        // 无 scheme 形态（三层处理的第 2 层）：不得原样透传凭据。
        assert_eq!(
            redact_proxy_userinfo(OsStr::new("user:pass@10.0.0.1:8080")),
            Some(OsString::from("10.0.0.1:8080"))
        );
        assert_eq!(
            redact_proxy_userinfo(OsStr::new("socks5://u:p@proxy.local:1080")),
            Some(OsString::from("socks5://proxy.local:1080"))
        );
        // 无 userinfo：原样保留（不改写用户配置）。
        assert_eq!(
            redact_proxy_userinfo(OsStr::new("http://proxy.local:8080")),
            Some(OsString::from("http://proxy.local:8080"))
        );
        // 无 userinfo 形状（无 `@`、无 `://`）：没有可泄漏内容，原样保留。
        assert_eq!(
            redact_proxy_userinfo(OsStr::new("::not a proxy::")),
            Some(OsString::from("::not a proxy::"))
        );
        assert_eq!(
            redact_proxy_userinfo(OsStr::new("10.0.0.1:8080")),
            Some(OsString::from("10.0.0.1:8080"))
        );
        // 含 `@` 但无法安全剥离（`@` 后为空 / 带 scheme 却无 host）：整条丢弃。
        assert_eq!(redact_proxy_userinfo(OsStr::new("user:pass@")), None);
        assert_eq!(redact_proxy_userinfo(OsStr::new("http://user:pass@")), None);
    }

    #[test]
    fn capability_negotiation_rejects_undeclared_and_hooks() {
        let manifest = manifest_with(r#"["tools"]"#);
        let granted = NegotiatedCapabilities::negotiate(&manifest, &["tools".to_string()])
            .expect("tools must negotiate");
        assert!(granted.allows(CAPABILITY_TOOLS));

        let err = NegotiatedCapabilities::negotiate(&manifest, &["network".to_string()])
            .expect_err("undeclared capability must fail");
        assert!(err.starts_with("plugin_capability_denied:"), "{err}");

        let hooks_manifest = manifest_with(r#"["tools","hooks"]"#);
        let err = NegotiatedCapabilities::negotiate(&hooks_manifest, &["tools".to_string()])
            .expect_err("hooks capability must be denied in v1");
        assert!(err.starts_with("plugin_capability_denied:"), "{err}");
    }

    #[test]
    fn confine_dir_blocks_escape_and_accepts_inside_paths() {
        let root = std::env::temp_dir().join(format!("pony-plugin-confine-{}", std::process::id()));
        let inside = root.join("nested");
        std::fs::create_dir_all(&inside).expect("create dirs");

        let resolved = confine_dir(&root, Path::new("nested")).expect("inside path must pass");
        assert!(resolved.ends_with("nested"), "{}", resolved.display());

        let err = confine_dir(&root, Path::new("../outside")).expect_err("escape must fail");
        assert!(err.starts_with("plugin_cwd_denied:"), "{err}");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// 工作区内指向外部的符号链接不得成为插件/hook 的 cwd（词法 `starts_with` 会放过它）。
    #[cfg(unix)]
    #[test]
    fn confine_dir_rejects_symlink_escape_but_allows_inner_symlink() {
        use std::os::unix::fs::symlink;

        let base = std::env::temp_dir().join(format!(
            "pony-plugin-confine-link-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let root = base.join("workspace");
        let outside = base.join("outside");
        std::fs::create_dir_all(&root).expect("create root");
        std::fs::create_dir_all(&outside).expect("create outside");
        std::fs::create_dir_all(root.join("real")).expect("create real");

        // 指向工作区外的符号链接：必须 fail-closed。
        let escape = root.join("escape");
        symlink(&outside, &escape).expect("create escape symlink");
        let err = confine_dir(&root, Path::new("escape"))
            .expect_err("symlink escaping the workspace must be rejected");
        assert!(err.starts_with("plugin_cwd_denied:"), "{err}");

        // 指向工作区内的符号链接：合法（不是逃逸），解析到 canonical 目标。
        let alias = root.join("alias");
        symlink(root.join("real"), &alias).expect("create inner symlink");
        let resolved = confine_dir(&root, Path::new("alias")).expect("inner symlink must pass");
        assert_eq!(
            resolved,
            std::fs::canonicalize(root.join("real")).expect("canonical real")
        );

        let _ = std::fs::remove_dir_all(&base);
    }
}
