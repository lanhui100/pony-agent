//! T3 验收契约（ADR 0067）：进程外插件宿主 v1 的机器可判定边界。
//!
//! ## 红相声明（README for reviewers）
//!
//! 本文件由**独立 Test Agent** 在实现落地前写成，约束的是 ADR 0067 的对外承诺：
//!
//! | 用例 | 契约出处 | 期望 |
//! |---|---|---|
//! | `t3_fixture_plugin_runs_spawn_initialize_call_shutdown_lifecycle` | 决策 2（生命周期） | 绿：fixture 全链路跑通 |
//! | `t3_plugin_child_env_is_exactly_the_allowlist` | 决策 3（`env_clear()` + 白名单） | 绿：子进程 env 精确等于白名单 |
//! | `t3_manifest_rejects_unknown_api_version` | 决策 1（契约版本） | 绿：负例逐例独立断言 |
//! | `t3_handshake_rejects_plugin_api_version_mismatch` | 决策 2（initialize 协商） | 绿：负例逐例独立断言 |
//! | `t3_manifest_rejects_unknown_capability` | 决策 3（能力默认拒绝） | 绿 |
//! | `t3_manifest_rejects_malformed_json` | 决策 1（manifest schema） | 绿 |
//! | `t3_initialize_rejects_undeclared_tool_registration` | 决策 2（"声明外一律拒绝"） | 绿 |
//! | `t3_call_tool_rejects_undeclared_tool_name` | 决策 2/4（注册面白名单） | 绿 |
//! | `t3_call_timeout_kills_process_group_without_residue` | 决策 3（超时杀进程组） | 绿 |
//! | `t3_plugin_crash_fails_closed_without_restart` | 决策 2（崩溃 fail-closed、不重试风暴） | 绿 |
//!
//! ## 冻结的 API 表面（Lead 裁决：test-agent 冻结，executor 只读）
//!
//! ```text
//! plugin_host::{PluginManifest, PluginSupervisor, PluginHostConfig, ENV_ALLOWLIST}
//! PluginManifest::parse(&str) -> Result<Self, String>
//! PluginSupervisor::spawn(manifest, command: PathBuf, config: PluginHostConfig) -> Result<Self, String>
//! PluginSupervisor::{initialize, call_tool(&mut self, &str, Value) -> Result<Value, String>,
//!                    shutdown(self) -> Result<(), String>, child_pid() -> Option<u32>,
//!                    try_wait(&mut self) -> Result<Option<ExitStatus>, String>, tools()}
//! ```
//!
//! `[CONTRACT-PENDING]` 标注只出现在 **ADR 未逐字规定、由实现自由裁量** 的点上，
//! 且每处都写明了「可接受的最小语义」，不锁定具体错误码字面量：
//! - 负例只断言「必须失败 + 失败原因可辨识（包含来源前缀）」，不冻结具体错误串；
//! - 正常路径只断言 fixture 自己的标记与 payload 必须原样回来，不冻结外壳包装形状。
//!
//! ## 测试纪律
//!
//! - fixture 插件在**运行期内联生成**到临时目录（不占用 `tests/fixtures/`，避免与实现者争用写域）；
//! - 超时/崩溃用例用 `try_wait` **有界轮询**（≤2s，20ms 步进）断言子进程已回收，不用裸 `sleep` 断言；
//! - 每个负例一个独立 `#[test]`，不合并成批量断言；
//! - 临时目录由 `TempDir` RAII 回收，运行结束后无残留。

use pony_agent_core::agent::plugin_host::{
    PluginHostConfig, PluginManifest, PluginSupervisor, ENV_ALLOWLIST,
};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Fixture plumbing
// ─────────────────────────────────────────────────────────────────────────────────────────────

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// 每个用例独占的临时根；`Drop` 时整树删除（谁创建谁清理）。
struct TempRoot {
    path: PathBuf,
}

impl TempRoot {
    fn new(tag: &str) -> Self {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "pony-plugin-contract-{tag}-{}-{seq}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(path.join("workspace")).expect("create plugin contract workspace");
        Self { path }
    }

    fn workspace(&self) -> PathBuf {
        self.path.join("workspace")
    }

    /// 写一个可执行的 fixture 插件脚本（POSIX 可执行脚本 + shebang）。
    fn fixture(&self, mode: &str) -> PathBuf {
        let script = FIXTURE_BODY.replace("@MODE@", mode);
        let path = self.path.join(format!("plugin-{mode}.py"));
        fs::write(&path, script).expect("write plugin fixture script");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
                .expect("chmod plugin fixture executable");
        }
        path
    }

    fn config(&self, timeout_ms: u64) -> PluginHostConfig {
        PluginHostConfig {
            workspace_root: self.workspace(),
            timeout_ms,
        }
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// fixture 插件：按行分隔的 JSON-RPC 2.0（与宿主协议同构）。
///
/// 行为由 `@MODE@` 决定；只有 `echo` 一个工具是 manifest 声明过的，
/// 因此每个模式的差异都来自「协议/生命周期行为」，而不是工具集合变化。
const FIXTURE_BODY: &str = r#"#!/usr/bin/env python3
import json
import os
import sys
import time

MODE = "@MODE@"
MARKER = "FIXTURE_ECHO_MARKER"


def emit(payload):
    sys.stdout.write(json.dumps(payload) + "\n")
    sys.stdout.flush()


def respond(request_id, result):
    emit({"jsonrpc": "2.0", "id": request_id, "result": result})


for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        message = json.loads(line)
    except json.JSONDecodeError:
        continue
    method = message.get("method")
    request_id = message.get("id")
    params = message.get("params") or {}

    if method == "initialize":
        api = "pony-plugin-v2" if MODE == "badapi" else "pony-plugin-v1"
        respond(
            request_id,
            {
                "apiVersion": api,
                "capabilities": ["tools"],
                "name": "echo_fixture",
                "version": "0.1.0",
            },
        )
    elif method == "notifications/initialized":
        continue
    elif method == "tools/list":
        tools = [
            {
                "name": "echo",
                "description": "fixture echo tool",
                "inputSchema": {"type": "object"},
            }
        ]
        if MODE == "undeclared":
            tools.append(
                {
                    "name": "undeclared_tool",
                    "description": "not declared in manifest",
                    "inputSchema": {"type": "object"},
                }
            )
        respond(request_id, {"tools": tools})
    elif method == "tools/call":
        if MODE == "hang":
            time.sleep(3600)
            continue
        if MODE == "crash":
            sys.exit(1)
        if MODE == "env":
            respond(
                request_id,
                {
                    "env": sorted(os.environ.keys()),
                    "cwd": os.getcwd(),
                    "marker": MARKER,
                },
            )
            continue
        respond(request_id, {"echoed": params.get("arguments"), "marker": MARKER})
    elif method == "shutdown":
        respond(request_id, {})
        break
    else:
        continue
"#;

const MANIFEST_JSON: &str = r#"{
  "apiVersion": "pony-plugin-v1",
  "name": "echo_fixture",
  "version": "0.1.0",
  "capabilities": ["tools"],
  "tools": [
    { "name": "echo", "description": "fixture echo tool", "inputSchema": { "type": "object" } }
  ],
  "hookPoints": []
}"#;

fn manifest() -> PluginManifest {
    PluginManifest::parse(MANIFEST_JSON).expect("fixture manifest must parse under plugin-v1")
}

/// 有界轮询：子进程必须在预算内被回收。返回真实等待耗时（毫秒），便于失败时给出证据。
fn wait_for_child_exit(supervisor: &mut PluginSupervisor, budget: Duration) -> Option<u128> {
    let started = Instant::now();
    while started.elapsed() < budget {
        match supervisor.try_wait() {
            Ok(Some(_status)) => return Some(started.elapsed().as_millis()),
            // 已回收 / 进程不存在：`try_wait` 以 `None` 表示「仍在运行」，错误同样视为未回收。
            Ok(None) | Err(_) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn proc_entry_exists(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

fn spawn_for(root: &TempRoot, mode: &str, timeout_ms: u64) -> PluginSupervisor {
    let command = root.fixture(mode);
    let mut supervisor = PluginSupervisor::spawn(manifest(), command, root.config(timeout_ms))
        .unwrap_or_else(|err| panic!("plugin fixture `{mode}` 必须能 spawn，实际错误：{err}"));
    supervisor
        .initialize()
        .unwrap_or_else(|err| panic!("plugin fixture `{mode}` 必须能 initialize，实际错误：{err}"));
    supervisor
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// 1. 全链路正例
// ─────────────────────────────────────────────────────────────────────────────────────────────

#[test]
fn t3_fixture_plugin_runs_spawn_initialize_call_shutdown_lifecycle() {
    // Arrange
    let root = TempRoot::new("lifecycle");
    let command = root.fixture("echo");
    let mut supervisor = PluginSupervisor::spawn(manifest(), command, root.config(5_000))
        .expect("spawn 必须成功（fixture 存在且可执行）");

    // Act — initialize（协商 + 注册）
    supervisor.initialize().expect("initialize 必须成功");

    // Assert — 注册面来自 manifest 声明，且模型可见名带 `plugin__<name>__<tool>` 命名空间
    let registered: Vec<String> = supervisor
        .tools()
        .iter()
        .map(|tool| tool.name.clone())
        .collect();
    assert_eq!(
        registered,
        vec!["echo".to_string()],
        "initialize 后注册面必须精确等于 manifest 声明的工具集合，实际 {registered:?}"
    );

    // Act — tools/call
    let arguments = json!({ "text": "pony-plugin-contract", "nested": { "n": 7 } });
    let result = supervisor
        .call_tool("echo", arguments.clone())
        .expect("tools/call 必须成功");

    // Assert — fixture 的标记与入参必须原样回来（外壳包装形状不作要求）
    let serialized = serde_json::to_string(&result).expect("call result 必须可序列化");
    assert!(
        serialized.contains("FIXTURE_ECHO_MARKER"),
        "调用结果必须来自 fixture 插件（缺少标记），实际 {serialized}"
    );
    assert!(
        serialized.contains("pony-plugin-contract"),
        "fixture 必须收到并回显调用参数，实际 {serialized}"
    );

    // Act — shutdown
    let pid = supervisor.child_pid().expect("已 initialize 的插件必须有子进程 pid");
    supervisor.shutdown().expect("shutdown 必须成功");

    // Assert — shutdown 后进程必须被回收（有界轮询 /proc，非裸 sleep）
    #[cfg(target_os = "linux")]
    {
        let deadline = Instant::now() + Duration::from_secs(2);
        while proc_entry_exists(pid) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            !proc_entry_exists(pid),
            "shutdown 后插件进程 {pid} 仍残留在进程表中"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// 2. 隔离：子进程 env 精确等于白名单
// ─────────────────────────────────────────────────────────────────────────────────────────────

#[test]
fn t3_plugin_child_env_is_exactly_the_allowlist() {
    // Arrange — 父进程注入一个绝不允许透传的哨兵变量（模拟 provider key）
    let secret_name = "PONY_PLUGIN_CONTRACT_SECRET";
    // 哨兵值：`sk-mock-` 前缀仅为满足敏感信息门禁的良性占位符约定（scripts/check-secrets.mjs:73-76）。
    let secret_value = "sk-mock-pony-plugin-contract-dummy";
    std::env::set_var(secret_name, secret_value);
    assert!(
        !ENV_ALLOWLIST.contains(&secret_name),
        "测试前提不成立：哨兵变量 `{secret_name}` 不得出现在 ENV_ALLOWLIST 中"
    );
    // Arrange — 父进程自身必须存在白名单之外的变量（否则"清空"断言会空转）
    let parent_keys: BTreeSet<String> = std::env::vars().map(|(key, _)| key).collect();
    let allowlist: BTreeSet<&str> = ENV_ALLOWLIST.iter().copied().collect();
    let outside_allowlist: Vec<&String> = parent_keys
        .iter()
        .filter(|key| !allowlist.contains(key.as_str()))
        .collect();
    assert!(
        !outside_allowlist.is_empty(),
        "测试前提不成立：父进程没有白名单之外的变量，env 隔离断言将空转"
    );

    let root = TempRoot::new("env");
    let mut supervisor = spawn_for(&root, "env", 5_000);

    // Act
    let result = supervisor
        .call_tool("echo", json!({}))
        .expect("env fixture 必须能返回自己的环境快照");

    // Assert — 子进程看到的环境变量集合必须精确等于「白名单 ∩ 父进程实际变量」
    let child_keys: Vec<String> = result
        .get("env")
        .and_then(Value::as_array)
        .expect("fixture 必须回传 env 键集合")
        .iter()
        .map(|key| key.as_str().unwrap_or_default().to_string())
        .collect();
    let child_set: BTreeSet<String> = child_keys.iter().cloned().collect();

    let leaked: Vec<&String> = child_set
        .iter()
        .filter(|key| !allowlist.contains(key.as_str()))
        .collect();
    assert!(
        leaked.is_empty(),
        "插件子进程继承了白名单之外的环境变量（多一个即 fail）：{leaked:?}"
    );
    assert!(
        !child_set.contains(secret_name),
        "哨兵变量 `{secret_name}` 泄漏进插件子进程（env_clear 未生效）"
    );

    let expected: BTreeSet<String> = parent_keys
        .iter()
        .filter(|key| allowlist.contains(key.as_str()))
        .cloned()
        .collect();
    assert_eq!(
        child_set, expected,
        "子进程 env 必须精确等于白名单（父进程存在的白名单变量必须透传，白名单外的必须全部丢弃）"
    );

    // Assert — cwd 必须限定在会话 workspace（ADR 0067 决策 3）
    let child_cwd = result
        .get("cwd")
        .and_then(Value::as_str)
        .expect("fixture 必须回传 cwd");
    let expected_cwd = root
        .workspace()
        .canonicalize()
        .expect("workspace 目录必须存在");
    assert_eq!(
        Path::new(child_cwd)
            .canonicalize()
            .expect("子进程 cwd 必须存在"),
        expected_cwd,
        "插件子进程 cwd 必须限定在会话 workspace，实际 {child_cwd}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// 3–6. manifest 负例（逐例独立）
// ─────────────────────────────────────────────────────────────────────────────────────────────

#[test]
fn t3_manifest_rejects_unknown_api_version() {
    // Arrange
    let json_text = MANIFEST_JSON.replace("pony-plugin-v1", "pony-plugin-v2");

    // Act
    let parsed = PluginManifest::parse(&json_text);

    // Assert
    let error = parsed.err().expect("apiVersion 不符的 manifest 必须被拒绝");
    assert!(
        error.contains("api_version") || error.contains("apiVersion"),
        "拒绝原因必须可辨识为 apiVersion 不符，实际错误：{error}"
    );
}

#[test]
fn t3_manifest_rejects_unknown_capability() {
    // Arrange — 未声明能力必须拒绝（能力默认拒绝，不是静默忽略）
    let json_text = MANIFEST_JSON.replace(r#""capabilities": ["tools"]"#, r#""capabilities": ["root_shell"]"#);
    assert!(
        json_text.contains("root_shell"),
        "测试前提不成立：fixture manifest 未能注入未知能力"
    );

    // Act
    let parsed = PluginManifest::parse(&json_text);

    // Assert
    let error = parsed.err().expect("未知能力的 manifest 必须被拒绝");
    assert!(
        error.contains("root_shell") && error.contains("capability"),
        "拒绝原因必须点名未知能力，实际错误：{error}"
    );
}

#[test]
fn t3_manifest_rejects_malformed_json() {
    // Arrange
    let json_text = r#"{ "apiVersion": "pony-plugin-v1", "name": "broken", "#;

    // Act
    let parsed = PluginManifest::parse(json_text);

    // Assert
    let error = parsed.err().expect("畸形 manifest 必须被拒绝");
    assert!(
        error.contains("manifest"),
        "拒绝原因必须可辨识为 manifest 非法，实际错误：{error}"
    );
}

#[test]
fn t3_manifest_rejects_empty_tool_less_namespace_name() {
    // Arrange — 插件名是工具命名空间 `plugin__<name>__<tool>` 的中段，不得含分隔符
    let json_text = MANIFEST_JSON.replace(r#""name": "echo_fixture""#, r#""name": "echo__fixture""#);

    // Act
    let parsed = PluginManifest::parse(&json_text);

    // Assert
    let error = parsed.err().expect("含 `__` 的插件名必须被拒绝");
    assert!(
        error.contains("__"),
        "拒绝原因必须点名非法命名空间分隔符，实际错误：{error}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// 7–8. 握手与调用面的负例（逐例独立）
// ─────────────────────────────────────────────────────────────────────────────────────────────

#[test]
fn t3_handshake_rejects_plugin_api_version_mismatch() {
    // Arrange — manifest 合法，但插件在 handshake 里协商出不同的 apiVersion
    let root = TempRoot::new("badapi");
    let command = root.fixture("badapi");
    let mut supervisor = PluginSupervisor::spawn(manifest(), command, root.config(5_000))
        .expect("spawn 必须成功（apiVersion 协商发生在 initialize）");

    // Act
    let initialized = supervisor.initialize();

    // Assert
    let error = initialized
        .err()
        .expect("插件协商 apiVersion 不符时 initialize 必须拒绝");
    assert!(
        error.contains("api_version") || error.contains("apiVersion"),
        "拒绝原因必须可辨识为 apiVersion 不符，实际错误：{error}"
    );
}

#[test]
fn t3_initialize_rejects_undeclared_tool_registration() {
    // Arrange — 插件 tools/list 返回 manifest 未声明的工具
    let root = TempRoot::new("undeclared");
    let command = root.fixture("undeclared");
    let mut supervisor = PluginSupervisor::spawn(manifest(), command, root.config(5_000))
        .expect("spawn 必须成功");

    // Act
    let initialized = supervisor.initialize();

    // Assert — 声明外一律拒绝（不得静默过滤后继续注册）
    let error = initialized
        .err()
        .expect("插件注册 manifest 未声明的工具时 initialize 必须拒绝");
    assert!(
        error.contains("undeclared_tool"),
        "拒绝原因必须点名越界注册的工具，实际错误：{error}"
    );
}

#[test]
fn t3_call_tool_rejects_undeclared_tool_name() {
    // Arrange
    let root = TempRoot::new("unknown-call");
    let mut supervisor = spawn_for(&root, "echo", 5_000);

    // Act
    let called = supervisor.call_tool("undeclared_tool", json!({}));

    // Assert
    let error = called.err().expect("调用未注册的工具必须 fail-closed");
    assert!(
        error.contains("undeclared_tool"),
        "拒绝原因必须点名未知工具，实际错误：{error}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// 9. 超时回收：无残留子进程（有界轮询 try_wait，不用裸 sleep 断言）
// ─────────────────────────────────────────────────────────────────────────────────────────────

#[test]
fn t3_call_timeout_kills_process_group_without_residue() {
    // Arrange — 150ms deadline + 永不回应的 fixture
    let root = TempRoot::new("timeout");
    let mut supervisor = spawn_for(&root, "hang", 150);
    let pid = supervisor
        .child_pid()
        .expect("已 initialize 的插件必须有子进程 pid");

    // Act
    let started = Instant::now();
    let called = supervisor.call_tool("echo", json!({ "text": "never returns" }));
    let elapsed = started.elapsed();

    // Assert 1 — 调用必须以超时失败，而不是悬挂
    let error = called.err().expect("超时调用必须返回错误而不是悬挂");
    assert!(
        error.contains("timeout") || error.contains("超时"),
        "超时失败原因必须可辨识，实际错误：{error}"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "超时必须在 deadline 附近返回（实测 {elapsed:?}），不得退化成等进程自然退出"
    );

    // Assert 2 — 进程组必须被杀且被回收（有界轮询 try_wait）
    let waited = wait_for_child_exit(&mut supervisor, Duration::from_secs(2));
    assert!(
        waited.is_some(),
        "超时后插件子进程 {pid} 未被回收（2s 内 try_wait 始终无退出状态）"
    );

    // Assert 3 — 进程表里不得残留（Linux）：证明确实 kill 了，而不是只把 fd 关掉
    #[cfg(target_os = "linux")]
    {
        assert!(
            !proc_entry_exists(pid),
            "超时后插件子进程 {pid} 仍残留在进程表中"
        );
    }

    // Assert 4 — 进入 fail-closed 后不得重启/重试（不重试风暴）
    let after = supervisor.call_tool("echo", json!({}));
    let after_error = after.err().expect("超时 fail-closed 后的再次调用必须失败");
    assert!(
        after_error.contains("unavailable") || after_error.contains("plugin_"),
        "fail-closed 后的再次调用必须以 plugin 来源错误码拒绝，实际错误：{after_error}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// 10. 崩溃 fail-closed：不重启、不悬挂
// ─────────────────────────────────────────────────────────────────────────────────────────────

#[test]
fn t3_plugin_crash_fails_closed_without_restart() {
    // Arrange
    let root = TempRoot::new("crash");
    let mut supervisor = spawn_for(&root, "crash", 5_000);
    let pid = supervisor
        .child_pid()
        .expect("已 initialize 的插件必须有子进程 pid");

    // Act
    let called = supervisor.call_tool("echo", json!({ "text": "boom" }));

    // Assert 1 — 崩溃以错误返回，不 panic、不悬挂
    let error = called.err().expect("插件崩溃时调用必须 fail-closed");
    assert!(
        !error.trim().is_empty(),
        "崩溃必须以可读错误返回，实际为空错误"
    );

    // Assert 2 — 崩溃进程必须已被回收
    assert!(
        wait_for_child_exit(&mut supervisor, Duration::from_secs(2)).is_some(),
        "崩溃后插件子进程 {pid} 未被回收"
    );

    // Assert 3 — 崩溃后不重启（同一 supervisor 不得再拉起新进程）
    assert_eq!(
        supervisor.child_pid(),
        None,
        "崩溃后不得留下仍在运行的子进程句柄（不重试、不重启）"
    );
}
