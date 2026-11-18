//! ACP 代理自动注入 + 模型可见 agent id（B183 / task-26）。
//!
//! 真源：`.dev-team/contract-matrix-acp-proxy-autowire.md`（G1–G7）。
//!
//! 覆盖：
//! - G1 `resolve_proxy_env()` 解析边界（临时 HOME 隔离，**不碰**真实 `~/.pony/proxy.env`）；
//! - G2 只保留 `*_proxy`（PATH/FOO/PONY_* 丢弃）；
//! - G3 deny 键拒绝（LD_* / SSH_AUTH_SOCK 等）；
//! - G4 合并顺序：白色名单 < proxy.env < `spec.env` < `extra_env`（fake agent 回显实收 env）；
//! - G5 `inheritProxyEnv=false` ⇒ 子进程完全看不到 proxy.env 的 `*_proxy`；
//! - G6 工具描述与 `ponycli acp --list` 中的 agent id。
//!
//! 纪律：无 `#[ignore]`、无空断言；只碰 ACP 测试与夹具。

#[path = "support/paths.rs"]
mod support;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::Instant;

use pony_agent_core::acp::config::{
    resolve_proxy_env, AcpAgentSpec, AcpCapabilitiesHint, AcpEnvVariable,
};
use pony_agent_core::acp::protocol::method;
use pony_agent_core::acp::transport::{AcpTransport, ACP_REQUEST_TIMEOUT};
use pony_agent_core::agent::tools::builtin_tools;
use serde_json::Value;

// ───────────────────────────── 设施 ─────────────────────────────

static TEMP_SEQ: AtomicU32 = AtomicU32::new(0);
/// HOME 是进程级全局状态 ⇒ 环境类用例串行执行。
static HOME_LOCK: Mutex<()> = Mutex::new(());

fn temp_dir(tag: &str) -> PathBuf {
    let seq = TEMP_SEQ.fetch_add(1, Ordering::SeqCst);
    let path = std::env::temp_dir().join(format!(
        "pony-acp-proxy-{}-{}-{tag}-{seq}",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    std::fs::create_dir_all(&path).expect("创建临时工作目录");
    support::deverbatim(&path.canonicalize().expect("canonicalize 临时工作目录"))
}

fn fixture_script() -> PathBuf {
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("acp_fake_agent.sh");
    assert!(script.is_file(), "fake agent fixture 缺失: {}", script.display());
    script
}

/// 临时 HOME（`<root>/home/.pony/proxy.env`），Drop 时恢复 HOME/USERPROFILE。
struct TempHome {
    _guard: MutexGuard<'static, ()>,
    root: PathBuf,
    restore: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

impl TempHome {
    fn new(tag: &str, proxy_env_body: Option<&str>) -> Self {
        let guard = HOME_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let root = temp_dir(tag);
        let home = root.join("home");
        std::fs::create_dir_all(home.join(".pony")).expect("创建 .pony 目录");
        if let Some(body) = proxy_env_body {
            std::fs::write(home.join(".pony").join("proxy.env"), body).expect("写 proxy.env");
        }
        let mut restore = Vec::new();
        for key in ["HOME", "USERPROFILE", "PONY_AGENT_HOME"] {
            restore.push((key, std::env::var_os(key)));
            std::env::set_var(key, &home);
        }
        Self {
            _guard: guard,
            root,
            restore,
        }
    }

    fn proxy_env_path(&self) -> PathBuf {
        self.root.join("home").join(".pony").join("proxy.env")
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        for (key, old) in self.restore.drain(..).rev() {
            match old {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn proxy_map(pairs: Vec<(String, String)>) -> std::collections::BTreeMap<String, String> {
    pairs.into_iter().collect()
}

fn spec_for_env_dump(out_file: &Path, inherit_proxy_env: bool) -> AcpAgentSpec {
    AcpAgentSpec {
        id: "fake".to_string(),
        label: "Fake ACP agent".to_string(),
        command: "sh".to_string(),
        args: vec![
            fixture_script().to_string_lossy().into_owned(),
            "env_dump".to_string(),
            out_file.to_string_lossy().into_owned(),
        ],
        env: Vec::new(),
        default_cwd: None,
        capabilities_hint: AcpCapabilitiesHint::default(),
        inherit_proxy_env,
        mcp_servers: Some(Vec::new()),
    }
}

/// 起一次 transport 让 fake agent 落盘 env 快照，返回 `KEY=VALUE` map。
fn dumped_child_env(spec: &AcpAgentSpec, cwd: &Path, out_file: &Path, extra: &[(String, String)]) -> std::collections::BTreeMap<String, String> {
    let mut transport = match AcpTransport::spawn(spec, cwd, extra) {
        Ok(transport) => transport,
        Err(error) => panic!("env_dump：transport 必须可启动，实际 {error:?}"),
    };
    // fake agent 在 initialize 处理时落 env 快照：必须先发一次 initialize 往返。
    transport
        .request(
            method::INITIALIZE,
            serde_json::json!({"protocolVersion": 1, "clientCapabilities": {}}),
            ACP_REQUEST_TIMEOUT,
        )
        .unwrap_or_else(|error| panic!("env_dump：initialize 往返必须成功，实际 {error:?}"));
    // 等子进程写出 env 快照（有界条件轮询）。
    let started = Instant::now();
    while !out_file.exists() || std::fs::read_to_string(out_file).unwrap_or_default().trim().is_empty() {
        assert!(
            started.elapsed().as_secs() < 5,
            "env_dump：fake agent 必须在 5s 内写出 env 快照（{}）",
            out_file.display()
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    transport.close();
    let body = std::fs::read_to_string(out_file).expect("读取 env 快照");
    let mut map = std::collections::BTreeMap::new();
    for line in body.lines() {
        if let Some((key, value)) = line.split_once('=') {
            map.insert(key.to_string(), value.to_string());
        }
    }
    map
}

// ───────────────────────── G1 解析边界 ─────────────────────────

/// G1：`KEY=VALUE` / `export KEY=VALUE` / 引号 / `#` 注释 / 空行 / 行尾空白 / 非法行；
/// 文件不存在 ⇒ 空 `Vec`（不报错）。
#[test]
fn g1_proxy_env_parsing_boundaries() {
    let body = "\
# 代理配置（注释行）
export http_proxy=\"http://proxy.example:8080\"
https_proxy='http://secure.example:8443'
all_proxy=http://all.example:1080   
no_proxy=localhost,127.0.0.1
   
not_a_valid_line
FTP_PROXY=http://ftp.example:2121
";
    let home = TempHome::new("g1", Some(body));
    let parsed = proxy_map(resolve_proxy_env());

    assert_eq!(
        parsed.get("http_proxy").map(String::as_str),
        Some("http://proxy.example:8080"),
        "G1 违约：`export KEY=\"V\"` 必须去 export 前缀与双引号，实际 {parsed:?}"
    );
    assert_eq!(
        parsed.get("https_proxy").map(String::as_str),
        Some("http://secure.example:8443"),
        "G1 违约：单引号必须去掉，实际 {parsed:?}"
    );
    assert_eq!(
        parsed.get("all_proxy").map(String::as_str),
        Some("http://all.example:1080"),
        "G1 违约：行尾空白必须裁剪，实际 {parsed:?}"
    );
    assert_eq!(
        parsed.get("no_proxy").map(String::as_str),
        Some("localhost,127.0.0.1"),
        "G1 违约：no_proxy 必须保留，实际 {parsed:?}"
    );
    assert_eq!(
        parsed.get("FTP_PROXY").map(String::as_str),
        Some("http://ftp.example:2121"),
        "G1 违约：大写代理键必须保留（大小写不敏感匹配），实际 {parsed:?}"
    );
    assert!(
        !parsed.contains_key("not_a_valid_line"),
        "G1 违约：无 `=` 的非法行必须跳过，实际 {parsed:?}"
    );
    assert!(
        !parsed.is_empty(),
        "G1：合法行必须被解析（不是恒空）"
    );
    assert!(
        home.proxy_env_path().exists(),
        "G1 前置：临时 HOME 的 proxy.env 必须存在"
    );

    // 文件不存在 ⇒ 空 Vec，不报错。
    // 注意：HOME_LOCK 不可重入，必须先释放上一个 TempHome 的守卫。
    drop(home);
    let empty_home = TempHome::new("g1-missing", None);
    assert!(
        !empty_home.proxy_env_path().exists(),
        "G1 前置：缺文件场景不得先创建 proxy.env"
    );
    let missing = resolve_proxy_env();
    assert!(
        missing.is_empty(),
        "G1 违约：proxy.env 不存在时必须返回空 Vec（合法常态，不报错），实际 {missing:?}"
    );
}

// ───────────────────────── G2 只保留 *_proxy ─────────────────────────

/// G2：文件混入 `PATH` / `LD_PRELOAD` / `FOO` / `PONY_*` ⇒ 结果只含 `*_proxy`。
#[test]
fn g2_only_proxy_keys_are_kept() {
    let body = "\
PATH=/evil/bin
LD_PRELOAD=/evil/preload.so
FOO=bar
PONY_MCP_TOKEN=secret-token
http_proxy=http://proxy.example:8080
https_proxy=http://secure.example:8443
all_proxy=http://all.example:1080
no_proxy=localhost
";
    let _home = TempHome::new("g2", Some(body));
    let parsed = proxy_map(resolve_proxy_env());

    let proxy_keys: Vec<&String> = parsed
        .keys()
        .filter(|key| key.to_ascii_lowercase().ends_with("_proxy"))
        .collect();
    assert_eq!(
        parsed.len(),
        proxy_keys.len(),
        "G2 违约：结果只允许 `*_proxy` 键，实际 {parsed:?}"
    );
    for key in ["PATH", "LD_PRELOAD", "FOO", "PONY_MCP_TOKEN"] {
        assert!(
            !parsed.contains_key(key),
            "G2 违约：非代理键 `{key}` 不得注入，实际 {parsed:?}"
        );
    }
    assert_eq!(parsed.get("http_proxy").map(String::as_str), Some("http://proxy.example:8080"));
    assert_eq!(parsed.get("all_proxy").map(String::as_str), Some("http://all.example:1080"));
}

// ───────────────────────── G3 deny 键拒绝 ─────────────────────────

/// G3：命中 deny 表的键即使写入 proxy.env 也不得返回（`LD_*` / `SSH_AUTH_SOCK` /
/// `SSL_CERT_FILE` 等进程/凭据通道向量）；代理键本身不受其影响。
#[test]
fn g3_denied_env_keys_are_rejected() {
    let body = "\
LD_PRELOAD=/evil/preload.so
LD_LIBRARY_PATH=/evil/lib
SSH_AUTH_SOCK=/tmp/evil-agent.sock
SSL_CERT_FILE=/evil/cert.pem
AWS_SECRET_ACCESS_KEY=aws-secret
http_proxy=http://proxy.example:8080
";
    let _home = TempHome::new("g3", Some(body));
    let parsed = proxy_map(resolve_proxy_env());

    for denied in [
        "LD_PRELOAD",
        "LD_LIBRARY_PATH",
        "SSH_AUTH_SOCK",
        "SSL_CERT_FILE",
        "AWS_SECRET_ACCESS_KEY",
    ] {
        assert!(
            !parsed.contains_key(denied),
            "G3 违约：deny 键 `{denied}` 不得注入，实际 {parsed:?}"
        );
    }
    assert_eq!(
        parsed.get("http_proxy").map(String::as_str),
        Some("http://proxy.example:8080"),
        "G3：合法代理键不得因 deny 表误伤，实际 {parsed:?}"
    );
    // 值绝不回显——resolve 只返回键值对，测试侧确保没有把 deny 值带出来。
    assert!(
        !format!("{parsed:?}").contains("evil"),
        "G3 违约：结果不得包含被拒键的值，实际 {parsed:?}"
    );
}

// ───────────────────────── G4 合并顺序 ─────────────────────────

/// G4：同名键覆盖顺序 = 白名单 < proxy.env < `spec.env` < `extra_env`；
/// 全部来源仍经 child env 白名单/deny 二次收口。
#[test]
fn g4_proxy_env_merge_order() {
    let body = "https_proxy=http://proxy-from-file:1111\nPATH=/evil/bin\n";
    let _home = TempHome::new("g4", Some(body));
    let cwd = temp_dir("g4-cwd");

    // (a) 只有 proxy.env ⇒ 子进程看到文件值；PATH 注入被丢。
    let out_a = cwd.join("env-a.txt");
    let spec_a = spec_for_env_dump(&out_a, true);
    let env_a = dumped_child_env(&spec_a, &cwd, &out_a, &[]);
    assert_eq!(
        env_a.get("https_proxy").map(String::as_str),
        Some("http://proxy-from-file:1111"),
        "G4 违约：proxy.env 的代理键必须注入子进程，实际 {env_a:?}"
    );
    assert_ne!(
        env_a.get("PATH").map(String::as_str),
        Some("/evil/bin"),
        "G4 违约：proxy.env 的 PATH 不得绕过白名单注入，实际 {env_a:?}"
    );

    // (b) spec.env 同名键覆盖 proxy.env。
    let out_b = cwd.join("env-b.txt");
    let mut spec_b = spec_for_env_dump(&out_b, true);
    spec_b.env = vec![AcpEnvVariable {
        name: "https_proxy".to_string(),
        value: "http://proxy-from-spec:2222".to_string(),
    }];
    let env_b = dumped_child_env(&spec_b, &cwd, &out_b, &[]);
    assert_eq!(
        env_b.get("https_proxy").map(String::as_str),
        Some("http://proxy-from-spec:2222"),
        "G4 违约：spec.env 必须覆盖 proxy.env 同名键，实际 {env_b:?}"
    );

    // (c) extra_env 最高优先（覆盖 spec.env 与 proxy.env）。
    let out_c = cwd.join("env-c.txt");
    let mut spec_c = spec_for_env_dump(&out_c, true);
    spec_c.env = vec![AcpEnvVariable {
        name: "https_proxy".to_string(),
        value: "http://proxy-from-spec:3333".to_string(),
    }];
    let extra = vec![(
        "https_proxy".to_string(),
        "http://proxy-from-extra:4444".to_string(),
    )];
    let env_c = dumped_child_env(&spec_c, &cwd, &out_c, &extra);
    assert_eq!(
        env_c.get("https_proxy").map(String::as_str),
        Some("http://proxy-from-extra:4444"),
        "G4 违约：extra_env 必须最高优先，实际 {env_c:?}"
    );

    let _ = std::fs::remove_dir_all(&cwd);
}

// ───────────────────────── G5 inheritProxyEnv=false ─────────────────────────

/// G5：`inheritProxyEnv=false` ⇒ 子进程完全看不到 proxy.env 的 `*_proxy`
///（其他来源的显式 env 仍生效）。
#[test]
fn g5_inherit_proxy_env_false_disables_injection() {
    let body = "http_proxy=http://proxy-from-file:1111\nhttps_proxy=http://proxy-from-file:2222\n";
    let _home = TempHome::new("g5", Some(body));
    let cwd = temp_dir("g5-cwd");

    let out = cwd.join("env.txt");
    let mut spec = spec_for_env_dump(&out, false);
    spec.env = vec![AcpEnvVariable {
        name: "all_proxy".to_string(),
        value: "http://explicit-all:3333".to_string(),
    }];
    let env = dumped_child_env(&spec, &cwd, &out, &[]);

    for key in ["http_proxy", "https_proxy"] {
        assert!(
            !env.contains_key(key),
            "G5 违约：inheritProxyEnv=false 时不得注入 proxy.env 的 `{key}`，实际 {env:?}"
        );
    }
    assert_eq!(
        env.get("all_proxy").map(String::as_str),
        Some("http://explicit-all:3333"),
        "G5：显式 spec.env 不受 inheritProxyEnv=false 影响，实际 {env:?}"
    );

    let _ = std::fs::remove_dir_all(&cwd);
}

// ───────────────────────── G6 模型可见 agent id ─────────────────────────

/// G6：`acp_delegate` 的工具 description 与 `agent` 参数 description 必须列出
/// `opencode` / `antigravity` 与 `ponycli acp --list`；CLI `--list` 也真的能列出两者。
#[test]
fn g6_acp_delegate_exposes_builtin_agent_ids() {
    let tools = builtin_tools();
    let definition = tools
        .iter()
        .find(|tool| tool.name == "acp_delegate")
        .unwrap_or_else(|| panic!("G6 违约：builtin_tools 必须含 acp_delegate"));

    for needle in ["opencode", "antigravity", "ponycli acp --list"] {
        assert!(
            definition.description.contains(needle),
            "G6 违约：acp_delegate description 必须含 `{needle}`，实际 {}",
            definition.description
        );
    }
    let agent_description = definition.input_schema["properties"]["agent"]["description"]
        .as_str()
        .unwrap_or_else(|| panic!("G6 违约：agent 参数必须有 description，实际 {}", definition.input_schema));
    for needle in ["opencode", "antigravity"] {
        assert!(
            agent_description.contains(needle),
            "G6 违约：agent 参数 description 必须含 `{needle}`，实际 {agent_description}"
        );
    }
    assert!(
        agent_description.contains("ponycli acp --list"),
        "G6 违约：agent 参数 description 必须指引 `ponycli acp --list`，实际 {agent_description}"
    );
    // schema 形状不因描述改动而变化（required / additionalProperties 冻结）。
    assert_eq!(
        definition.input_schema["additionalProperties"],
        Value::Bool(false),
        "G6：additionalProperties 必须仍为 false"
    );
    let required: Vec<&str> = definition.input_schema["required"]
        .as_array()
        .expect("required 必须是数组")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(
        required.contains(&"agent") && required.contains(&"prompt"),
        "G6：required 必须仍含 agent/prompt，实际 {required:?}"
    );

    // 真实 CLI：`ponycli acp --list` 必须列出两个内置 id（隔离 XDG，不碰用户配置）。
    let root = temp_dir("g6-cli");
    let config = root.join("config");
    std::fs::create_dir_all(&config).expect("创建隔离配置目录");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_ponycli"))
        .args(["acp", "--list"])
        .env("HOME", root.join("home"))
        .env("XDG_CONFIG_HOME", &config)
        .env("XDG_DATA_HOME", root.join("data"))
        .env("XDG_STATE_HOME", root.join("state"))
        .env("XDG_CACHE_HOME", root.join("cache"))
        .output()
        .expect("ponycli 必须可执行");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        output.status.code(),
        Some(0),
        "G6 违约：ponycli acp --list 必须 EXIT 0\nstdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for id in ["opencode", "antigravity"] {
        assert!(
            stdout.contains(id),
            "G6 违约：ponycli acp --list 必须列出 `{id}`，实际 {stdout}"
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}
