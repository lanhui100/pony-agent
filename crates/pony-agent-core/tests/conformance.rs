//! 契约 conformance kit 进程内套件（ADR 0068 决策 2 的骨架，W2 前置）。
//!
//! 设计约束（task-4 骨架）：
//! - **逐例独立**：每个用例单独放进一个临时 kit 目录、单独跑一次 `--bin conformance`，
//!   禁止「一整批套件跑一次、有一个失败就算全过/全不过」的整批断言；
//! - **负例断言失败原因**：负例的 `reason` 必须命中实测错误文本，否则二进制以 2 退出，
//!   本套件即 FAIL（防止「随便拒了就算对」）；
//! - 语义用例（越权能力/畸形 payload/超时/取消/apiVersion 协商/未知能力）在 T3 落地后用
//!   **运行期内联生成**的 fixture 插件转正，无 `#[ignore]` 残留，也不写 `todo!()` 假装通过。
//!
//! 覆盖事实：①纯 schema 校验（五份 v1 契约）；②插件宿主的语义边界（真实 stdio 进程）；
//! ③golden 对齐 —— 用真实 Rust 产物（registry descriptor / fixture manifest / hooks 配置 /
//! MCP 配置）反向校验 schema，证明「schema 是代码的投影」而不是手写第二真值。

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// cargo 为同包 bin target 注入的路径（无需手工拼 target/ 目录）。
const BIN: &str = env!("CARGO_BIN_EXE_conformance");

fn schemas_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/contracts")
}

/// 一次性的 kit 目录；Drop 时清理，避免污染 temp。
struct KitDir {
    path: PathBuf,
}

impl KitDir {
    fn new(name: &str) -> Self {
        let mut path = std::env::temp_dir();
        path.push(format!("pony-conformance-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("创建临时 kit 目录");
        Self { path }
    }

    fn with_case(self, file: &str, json: &str) -> Self {
        std::fs::write(self.path.join(file), json).expect("写入用例文件");
        self
    }

    fn run(&self, extra: &[&str]) -> Output {
        let mut command = Command::new(BIN);
        command
            .arg("--kit")
            .arg(&self.path)
            .arg("--schemas")
            .arg(schemas_dir());
        command.args(extra);
        command.output().expect("运行 conformance 二进制")
    }
}

impl Drop for KitDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn case_json(id: &str, contract: &str, expect: &str, reason: Option<&str>, instance: &str) -> String {
    let reason = match reason {
        Some(reason) => format!(",\"reason\":\"{reason}\""),
        None => String::new(),
    };
    format!(
        "{{\"id\":\"{id}\",\"contract\":\"{contract}\",\"expect\":\"{expect}\"{reason},\"instance\":{instance}}}"
    )
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

// ── 正例（6）：契约应接受 ────────────────────────────────────────────────────

const TOOL_DESCRIPTOR_BUILTIN: &str = r#"{
  "identity": {"descriptorId":"builtin:workspace_read_file","modelName":"Read","canonicalName":"Read","primitiveName":"workspace_read_file","source":"builtin"},
  "description": "read a file",
  "inputSchema": {"type":"object"},
  "kind": "read",
  "exposure": "model_visible",
  "permissionDeclaration": {"scopes":["workspace_read"],"requiresApproval":false,"hostMediated":false},
  "executionPolicy": {"concurrentSafe":true,"cancellable":true,"defaultTimeoutMs":10000,"resultBudgetBytes":1024},
  "handlerProvenance": {"handlerKind":"router_primitive","sourceId":"builtin-tools"},
  "sourceRevision": "builtin-45"
}"#;

const TOOL_DESCRIPTOR_MCP: &str = r#"{
  "identity": {"descriptorId":"mcp:github","modelName":"mcp__github__list_repos","canonicalName":"list_repos","primitiveName":"list_repos","source":"mcp"},
  "description": "list repos",
  "inputSchema": {"type":"object"},
  "kind": "external",
  "exposure": "model_visible",
  "permissionDeclaration": {"scopes":[],"requiresApproval":true,"hostMediated":false},
  "executionPolicy": {"concurrentSafe":false,"cancellable":true,"defaultTimeoutMs":3000,"resultBudgetBytes":4096},
  "handlerProvenance": {"handlerKind":"external","sourceId":"mcp:github"},
  "sourceRevision": "mcp-1"
}"#;

const HOOKS_COMMAND: &str = r#"{"hooks":{"fmt":{"command":"npx prettier --write .","timeoutMs":5000,"hookPoints":["tool_call_end"],"onFailure":"fail_turn"}}}"#;

const MCP_CONFIG_ENV_REF: &str = r#"{"mcpServers":{"github":{"command":"npx","args":["-y","@mcp/github"],"env":{"GITHUB_TOKEN":"${env:GITHUB_TOKEN}"},"disabled":false}}}"#;

const SKILL_MANIFEST: &str = r#"{"name":"write-adr","description":"ADR helper","user-invocable":true,"allowed-tools":["Read","Write"]}"#;

const PLUGIN_TOOLS: &str = r#"{"apiVersion":"pony-plugin-v1","name":"demo","version":"1.0.0","capabilities":["tools"],"tools":[{"name":"echo","description":"echo back","inputSchema":{"type":"object"}}]}"#;

#[test]
fn positive_1_tool_descriptor_builtin_accepted() {
    let kit = KitDir::new("p1").with_case(
        "case.json",
        &case_json("tool-descriptor/builtin-read", "tool-descriptor", "valid", None, TOOL_DESCRIPTOR_BUILTIN),
    );
    let output = kit.run(&[]);
    assert_eq!(output.status.code(), Some(0), "stdout={} stderr={}", stdout_of(&output), stderr_of(&output));
    assert!(stdout_of(&output).contains("PASS tool-descriptor/builtin-read"));
}

#[test]
fn positive_2_tool_descriptor_mcp_namespaced_accepted() {
    let kit = KitDir::new("p2").with_case(
        "case.json",
        &case_json("tool-descriptor/mcp-namespaced", "tool-descriptor", "valid", None, TOOL_DESCRIPTOR_MCP),
    );
    let output = kit.run(&[]);
    assert_eq!(output.status.code(), Some(0), "stdout={} stderr={}", stdout_of(&output), stderr_of(&output));
}

#[test]
fn positive_3_hooks_legacy_command_accepted() {
    let kit = KitDir::new("p3").with_case(
        "case.json",
        &case_json("hooks/legacy-command", "hooks", "valid", None, HOOKS_COMMAND),
    );
    let output = kit.run(&[]);
    assert_eq!(output.status.code(), Some(0), "stdout={} stderr={}", stdout_of(&output), stderr_of(&output));
}

#[test]
fn positive_4_mcp_config_env_indirection_accepted() {
    let kit = KitDir::new("p4").with_case(
        "case.json",
        &case_json("mcp-config/env-indirection", "mcp-config", "valid", None, MCP_CONFIG_ENV_REF),
    );
    let output = kit.run(&[]);
    assert_eq!(output.status.code(), Some(0), "stdout={} stderr={}", stdout_of(&output), stderr_of(&output));
}

#[test]
fn positive_5_skill_manifest_accepted() {
    let kit = KitDir::new("p5").with_case(
        "case.json",
        &case_json("skill-manifest/basic", "skill-manifest", "valid", None, SKILL_MANIFEST),
    );
    let output = kit.run(&[]);
    assert_eq!(output.status.code(), Some(0), "stdout={} stderr={}", stdout_of(&output), stderr_of(&output));
}

#[test]
fn positive_6_plugin_manifest_accepted() {
    let kit = KitDir::new("p6").with_case(
        "case.json",
        &case_json("plugin/tools", "plugin", "valid", None, PLUGIN_TOOLS),
    );
    let output = kit.run(&[]);
    assert_eq!(output.status.code(), Some(0), "stdout={} stderr={}", stdout_of(&output), stderr_of(&output));
}

// ── 负例（6）：契约必须拒绝，且原因必须命中 ─────────────────────────────────

const NEG_TOOL_DESCRIPTOR_MCP_UNNAMESPACED: &str = r#"{
  "identity": {"descriptorId":"mcp:github","modelName":"list_repos","canonicalName":"list_repos","primitiveName":"list_repos","source":"mcp"},
  "description": "list repos",
  "inputSchema": {"type":"object"},
  "kind": "external",
  "exposure": "model_visible",
  "permissionDeclaration": {"scopes":[],"requiresApproval":true,"hostMediated":false},
  "executionPolicy": {"concurrentSafe":false,"cancellable":true,"defaultTimeoutMs":3000,"resultBudgetBytes":4096},
  "handlerProvenance": {"handlerKind":"external","sourceId":"mcp:github"},
  "sourceRevision": "mcp-1"
}"#;

const NEG_HOOKS_BOTH_FORMS: &str = r#"{"hooks":{"x":{"command":"echo hi","argv":["echo","hi"]}}}"#;

const NEG_MCP_CONFIG_LITERAL_SECRET: &str = r#"{"mcpServers":{"g":{"command":"npx","env":{"GITHUB_TOKEN":"ghp_deadbeef"}}}}"#;

const NEG_SKILL_MANIFEST_NO_NAME: &str = r#"{"description":"no name"}"#;

const NEG_PLUGIN_BAD_API_VERSION: &str = r#"{"apiVersion":"pony-plugin-v2","name":"demo","version":"1.0.0","capabilities":["tools"],"tools":[{"name":"e","description":"d","inputSchema":{}}]}"#;

const NEG_PLUGIN_UNKNOWN_CAPABILITY: &str = r#"{"apiVersion":"pony-plugin-v1","name":"demo","version":"1.0.0","capabilities":["telepathy"]}"#;

fn assert_negative_case(name: &str, contract: &str, reason: &str, instance: &str) {
    let kit = KitDir::new(name).with_case(
        "case.json",
        &case_json(&format!("negative/{name}"), contract, "invalid", Some(reason), instance),
    );
    let output = kit.run(&[]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "负例应被 schema 拒绝且原因命中，实际 stdout={} stderr={}",
        stdout_of(&output),
        stderr_of(&output)
    );
    assert!(stdout_of(&output).contains("PASS negative/"), "{}", stdout_of(&output));
}

#[test]
fn negative_1_tool_descriptor_mcp_model_name_not_namespaced() {
    assert_negative_case(
        "tool-descriptor-mcp-model-name",
        "tool-descriptor",
        "string does not match pattern",
        NEG_TOOL_DESCRIPTOR_MCP_UNNAMESPACED,
    );
}

#[test]
fn negative_2_hooks_two_forms_at_once() {
    assert_negative_case(
        "hooks-two-forms",
        "hooks",
        "must match exactly one of the 2 oneOf branches",
        NEG_HOOKS_BOTH_FORMS,
    );
}

#[test]
fn negative_3_mcp_config_literal_secret() {
    assert_negative_case(
        "mcp-config-literal-secret",
        "mcp-config",
        "string does not match pattern",
        NEG_MCP_CONFIG_LITERAL_SECRET,
    );
}

#[test]
fn negative_4_skill_manifest_missing_name() {
    assert_negative_case(
        "skill-manifest-missing-name",
        "skill-manifest",
        "missing required field 'name'",
        NEG_SKILL_MANIFEST_NO_NAME,
    );
}

#[test]
fn negative_5_plugin_api_version_not_in_enum() {
    assert_negative_case(
        "plugin-api-version",
        "plugin",
        "value is not one of",
        NEG_PLUGIN_BAD_API_VERSION,
    );
}

#[test]
fn negative_6_plugin_unknown_capability() {
    // 与 manifest.rs::validate 同向：未知能力一律拒绝（plugin_capability_unsupported）。
    assert_negative_case(
        "plugin-unknown-capability",
        "plugin",
        "value is not one of",
        NEG_PLUGIN_UNKNOWN_CAPABILITY,
    );
}

// ── 退出码语义（0 / 2 / 3）的自证用例 ────────────────────────────────────────

#[test]
fn expectation_mismatch_exits_2() {
    // 声明 expect=invalid 但实例其实合法 → 用例失败（退出码 2），不是「配平了就算过」。
    let kit = KitDir::new("exit2").with_case(
        "case.json",
        &case_json("mismatch/declared-invalid", "skill-manifest", "invalid", Some("missing required field 'name'"), SKILL_MANIFEST),
    );
    let output = kit.run(&[]);
    assert_eq!(output.status.code(), Some(2), "stdout={} stderr={}", stdout_of(&output), stderr_of(&output));
    assert!(stdout_of(&output).contains("期望被拒但校验通过"), "{}", stdout_of(&output));
}

#[test]
fn negative_reason_mismatch_exits_2() {
    // 拒绝是正确行为，但原因不命中 → 逐例断言失败（退出码 2）。
    let kit = KitDir::new("reason2").with_case(
        "case.json",
        &case_json("negative/wrong-reason", "skill-manifest", "invalid", Some("unexpected additional field"), NEG_SKILL_MANIFEST_NO_NAME),
    );
    let output = kit.run(&[]);
    assert_eq!(output.status.code(), Some(2), "stdout={} stderr={}", stdout_of(&output), stderr_of(&output));
    assert!(stdout_of(&output).contains("原因不匹配"), "{}", stdout_of(&output));
}

#[test]
fn missing_kit_argument_exits_3() {
    let output = Command::new(BIN).output().expect("运行 conformance 二进制");
    assert_eq!(output.status.code(), Some(3), "stderr={}", stderr_of(&output));
    assert!(stderr_of(&output).contains("配置非法"), "{}", stderr_of(&output));
}

#[test]
fn empty_kit_directory_exits_3() {
    let kit = KitDir::new("empty");
    let output = kit.run(&[]);
    assert_eq!(output.status.code(), Some(3), "stdout={} stderr={}", stdout_of(&output), stderr_of(&output));
}

#[test]
fn unknown_contract_exits_3() {
    let kit = KitDir::new("unknown-contract").with_case(
        "case.json",
        &case_json("x/y", "not-a-contract", "valid", None, "{}"),
    );
    let output = kit.run(&[]);
    assert_eq!(output.status.code(), Some(3), "stdout={} stderr={}", stdout_of(&output), stderr_of(&output));
}

// ── 语义用例（T3 落地后由 #[ignore] 转正；fixture 在运行期内联生成）──────────────
//
// 这 6 条原先因「纯 schema kit 看不到 transport 语义」被 ignore。T3 插件宿主落地后，
// 用**运行期内联生成**的 fixture 插件转正（不写 tests/fixtures/，避免争用 T3 写域）；
// 仍逐例独立断言，负例只断言「必须失败 + 来源码家族可辨识」，不冻结具体错误串。

#[cfg(unix)]
mod plugin_fixture {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    const SCRIPT_TEMPLATE: &str = r##"#!/usr/bin/env python3
import json, sys, time

API_VERSION = "__API_VERSION__"
REGISTER_SHADOW = __SHADOW__
GARBAGE = __GARBAGE__

def emit(text):
    sys.stdout.write(text + "\n")
    sys.stdout.flush()

def respond(mid, result):
    emit(json.dumps({"jsonrpc": "2.0", "id": mid, "result": result}))

def respond_error(mid, code, message):
    emit(json.dumps({"jsonrpc": "2.0", "id": mid, "error": {"code": code, "message": message}}))

TOOLS = [
    {"name": "echo", "description": "echo", "inputSchema": {"type": "object"}},
    {"name": "sleep", "description": "sleep", "inputSchema": {"type": "object"}},
]
if REGISTER_SHADOW:
    TOOLS.append({"name": "shadow", "description": "undeclared", "inputSchema": {"type": "object"}})

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        message = json.loads(line)
    except ValueError:
        continue
    method = message.get("method")
    mid = message.get("id")
    if mid is None:
        continue
    if method == "initialize":
        if GARBAGE:
            emit("this-is-not-a-json-rpc-envelope")
        else:
            respond(mid, {"apiVersion": API_VERSION, "capabilities": ["tools"]})
    elif method == "tools/list":
        respond(mid, {"tools": TOOLS})
    elif method == "tools/call":
        params = message.get("params") or {}
        name = params.get("name")
        arguments = params.get("arguments") or {}
        if name == "echo":
            respond(mid, {"echo": arguments})
        elif name == "sleep":
            time.sleep(max(int(arguments.get("ms", 0)), 0) / 1000.0)
            respond(mid, {"slept": True})
        else:
            respond_error(mid, -32601, "unknown tool")
    elif method == "shutdown":
        respond(mid, {"ok": True})
        sys.exit(0)
    else:
        respond_error(mid, -32601, "method not found")
"##;

    /// 每个用例独占的临时根；`Drop` 时整树删除。
    pub struct TempRoot {
        path: PathBuf,
    }

    impl TempRoot {
        pub fn new(tag: &str) -> Self {
            let seq = SEQ.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "pony-conformance-{tag}-{}-{seq}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(path.join("workspace")).expect("创建 fixture 临时根");
            Self { path }
        }

        pub fn workspace(&self) -> PathBuf {
            self.path.join("workspace")
        }

        /// 写出可执行插件脚本（POSIX 可执行位 + shebang）。
        pub fn write_script(&self, body: &str) -> PathBuf {
            let path = self.path.join("plugin.py");
            std::fs::write(&path, body).expect("写 fixture 脚本");
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&path).expect("读 fixture 元数据").permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&path, permissions).expect("设置 fixture 可执行位");
            path
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    pub fn script(api_version: &str, shadow: bool, garbage: bool) -> String {
        SCRIPT_TEMPLATE
            .replace("__API_VERSION__", api_version)
            .replace("__SHADOW__", if shadow { "True" } else { "False" })
            .replace("__GARBAGE__", if garbage { "True" } else { "False" })
    }

    pub fn manifest_json(name: &str, tool_names: &[&str]) -> String {
        let tools: Vec<serde_json::Value> = tool_names
            .iter()
            .map(|tool| {
                serde_json::json!({
                    "name": tool,
                    "description": format!("fixture tool {tool}"),
                    "inputSchema": {"type": "object"}
                })
            })
            .collect();
        serde_json::json!({
            "apiVersion": "pony-plugin-v1",
            "name": name,
            "version": "0.1.0",
            "capabilities": ["tools"],
            "tools": tools,
            "hookPoints": []
        })
        .to_string()
    }

    pub fn config(root: &TempRoot) -> pony_agent_core::agent::plugin_host::PluginHostConfig {
        pony_agent_core::agent::plugin_host::PluginHostConfig::default()
            .with_workspace_root(root.workspace())
            .with_timeout_ms(500)
    }
}

/// 把若干 (id, contract, instance) 交给 `--bin conformance` 校验（golden 对齐真值）。
fn assert_contract_instances_valid(tag: &str, cases: &[(String, String, serde_json::Value)]) {
    let kit = KitDir::new(tag);
    for (index, (id, contract, instance)) in cases.iter().enumerate() {
        let case = serde_json::json!({
            "id": id,
            "contract": contract,
            "expect": "valid",
            "instance": instance
        });
        std::fs::write(
            kit.path.join(format!("case-{index:02}.json")),
            serde_json::to_string_pretty(&case).expect("序列化用例"),
        )
        .expect("写 golden 用例");
    }
    let output = kit.run(&[]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout={} stderr={}",
        stdout_of(&output),
        stderr_of(&output)
    );
    let passed = stdout_of(&output).matches("PASS ").count();
    assert_eq!(passed, cases.len(), "{}", stdout_of(&output));
}

// ── 语义用例（6）：越权能力 / 畸形 payload / 超时 / 取消 / apiVersion / 未知能力 ──

#[test]
#[cfg(unix)]
fn semantic_api_version_mismatch_rejected_at_initialize() {
    use pony_agent_core::agent::plugin_host::{PluginManifest, PluginSupervisor};
    let root = plugin_fixture::TempRoot::new("api-version");
    let command = root.write_script(&plugin_fixture::script("pony-plugin-v2", false, false));
    let manifest = PluginManifest::parse(&plugin_fixture::manifest_json("api-version-fixture", &["echo"]))
        .expect("manifest 本身合法");
    let mut supervisor = PluginSupervisor::spawn(manifest, command, plugin_fixture::config(&root))
        .expect("fixture 应可启动");
    let error = supervisor.initialize().expect_err("apiVersion 不符必须拒绝握手");
    assert!(
        error.starts_with("plugin_api_version_mismatch:"),
        "必须报 apiVersion 家族错误，实得: {error}"
    );
    assert!(!supervisor.is_initialized(), "握手失败不得标记为已注册");
}

#[test]
fn semantic_unknown_capability_rejected() {
    use pony_agent_core::agent::plugin_host::PluginManifest;
    let manifest = r#"{
        "apiVersion": "pony-plugin-v1",
        "name": "unknown-cap-fixture",
        "version": "0.1.0",
        "capabilities": ["telepathy"],
        "tools": []
    }"#;
    let error = PluginManifest::parse(manifest).expect_err("未知能力必须拒绝");
    assert!(
        error.starts_with("plugin_capability_unsupported:"),
        "必须报未知能力家族错误，实得: {error}"
    );
}

#[test]
#[cfg(unix)]
fn semantic_capability_escalation_rejected() {
    use pony_agent_core::agent::plugin_host::{PluginManifest, PluginSupervisor};
    let root = plugin_fixture::TempRoot::new("escalation");
    // 插件在 tools/list 里注册了 manifest 未声明的 `shadow` 工具。
    let command = root.write_script(&plugin_fixture::script("pony-plugin-v1", true, false));
    let manifest = PluginManifest::parse(&plugin_fixture::manifest_json("escalation-fixture", &["echo"]))
        .expect("manifest 本身合法");
    let mut supervisor = PluginSupervisor::spawn(manifest, command, plugin_fixture::config(&root))
        .expect("fixture 应可启动");
    let error = supervisor.initialize().expect_err("声明外工具必须拒绝注册");
    assert!(
        error.starts_with("plugin_protocol_error:") || error.contains("未声明"),
        "必须明确指出声明外注册，实得: {error}"
    );
    assert!(supervisor.tools().is_empty(), "越权注册不得留下任何已注册工具");
}

#[test]
#[cfg(unix)]
fn semantic_malformed_payload_rejected() {
    use pony_agent_core::agent::plugin_host::{PluginManifest, PluginSupervisor};
    let root = plugin_fixture::TempRoot::new("malformed");
    let command = root.write_script(&plugin_fixture::script("pony-plugin-v1", false, true));
    let manifest = PluginManifest::parse(&plugin_fixture::manifest_json("malformed-fixture", &["echo"]))
        .expect("manifest 本身合法");
    let mut supervisor = PluginSupervisor::spawn(manifest, command, plugin_fixture::config(&root))
        .expect("fixture 应可启动");
    let error = supervisor.initialize().expect_err("非 JSON-RPC 响应必须 fail-closed");
    // 握手失败会同时标记会话不可用（`plugin_unavailable` 前缀），但**来源码家族必须保留**
    // （`plugin_protocol_error` 出现在诊断里），不得塌缩成笼统 handler_error。
    assert!(
        error.contains("plugin_protocol_error"),
        "畸形 payload 必须保留协议错误家族，实得: {error}"
    );
    assert!(
        error.starts_with("plugin_unavailable:") || error.starts_with("plugin_protocol_error:"),
        "畸形 payload 必须以不可用/协议错误家族报出，实得: {error}"
    );
}

#[test]
#[cfg(unix)]
fn semantic_timeout_maps_to_source_specific_error() {
    use pony_agent_core::agent::plugin_host::{PluginManifest, PluginSupervisor};
    use serde_json::json;
    use std::time::Duration;
    let root = plugin_fixture::TempRoot::new("timeout");
    let command = root.write_script(&plugin_fixture::script("pony-plugin-v1", false, false));
    let manifest = PluginManifest::parse(&plugin_fixture::manifest_json("timeout-fixture", &["echo", "sleep"]))
        .expect("manifest 本身合法");
    let mut supervisor = PluginSupervisor::spawn(manifest, command, plugin_fixture::config(&root))
        .expect("fixture 应可启动");
    supervisor.initialize().expect("正向握手应成功");
    let error = supervisor
        .call_tool_with_deadline("sleep", json!({"ms": 800}), Duration::from_millis(120))
        .expect_err("超过 deadline 必须失败");
    assert!(
        error.starts_with("plugin_timeout:"),
        "超时必须映射 plugin_timeout（不得塌缩成 handler_error），实得: {error}"
    );
}

#[test]
#[cfg(unix)]
fn semantic_cancellation_releases_budget() {
    use pony_agent_core::agent::plugin_host::{PluginManifest, PluginSupervisor};
    use serde_json::json;
    use std::time::{Duration, Instant};
    let root = plugin_fixture::TempRoot::new("cancel");
    let command = root.write_script(&plugin_fixture::script("pony-plugin-v1", false, false));
    let manifest = PluginManifest::parse(&plugin_fixture::manifest_json("cancel-fixture", &["echo", "sleep"]))
        .expect("manifest 本身合法");
    let mut supervisor = PluginSupervisor::spawn(manifest, command, plugin_fixture::config(&root))
        .expect("fixture 应可启动");
    supervisor.initialize().expect("正向握手应成功");
    let pid = supervisor.child_pid().expect("已启动的插件应有 pid");
    let _ = supervisor
        .call_tool_with_deadline("sleep", json!({"ms": 800}), Duration::from_millis(120))
        .expect_err("超过 deadline 必须失败");

    // 取消/超时后必须回收子进程（有界轮询，不用裸 sleep 断言）。
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut reaped = false;
    while Instant::now() < deadline {
        if matches!(supervisor.try_wait(), Ok(Some(_))) {
            reaped = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(reaped, "超时/取消后插件进程 {pid} 必须被回收，不得残留");

    // fail-closed：不重启、不重试；后续调用立即失败，不占用新的预算/进程。
    let follow_up = supervisor
        .call_tool("echo", json!({"after": "cancel"}))
        .expect_err("取消后必须保持 fail-closed");
    assert!(
        follow_up.starts_with("plugin_unavailable:"),
        "取消后应报 plugin_unavailable（不重试、不重启），实得: {follow_up}"
    );
}

// ── golden 对齐（schema 是投影：用真实 Rust 产物反向校验契约）─────────────────

#[test]
fn golden_builtin_tool_descriptors_match_contract_schema() {
    let snapshot = pony_agent_core::agent::tools::ToolRegistrySnapshot::builtin()
        .expect("内置 registry 应可构造");
    assert!(!snapshot.descriptors.is_empty(), "内置 registry 不得为空");
    let cases: Vec<(String, String, serde_json::Value)> = snapshot
        .descriptors
        .iter()
        .enumerate()
        .map(|(index, descriptor)| {
            (
                format!("golden/builtin-{index:02}"),
                "tool-descriptor".to_string(),
                serde_json::to_value(descriptor).expect("descriptor 应可序列化"),
            )
        })
        .collect();
    assert_contract_instances_valid("golden-builtin", &cases);
}

#[test]
fn golden_fixture_plugin_manifest_matches_contract_schema() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/plugin/plugin.json");
    let text = std::fs::read_to_string(&path).expect("读取 T3 fixture manifest");
    let manifest: serde_json::Value = serde_json::from_str(&text).expect("fixture manifest 是 JSON");
    assert_contract_instances_valid(
        "golden-plugin-manifest",
        &[(
            "golden/fixture-plugin-manifest".to_string(),
            "plugin".to_string(),
            manifest,
        )],
    );
}

#[test]
fn golden_hooks_config_forms_match_contract_schema() {
    let legacy = serde_json::json!({
        "hooks": {"fmt": {"command": "npx prettier --write .", "timeoutMs": 5000, "hookPoints": ["tool_call_end"]}}
    });
    let argv = serde_json::json!({
        "hooks": {"fmt": {"argv": ["npx", "prettier", "--write", "."], "cwd": ".", "timeoutMs": 1000}}
    });
    assert_contract_instances_valid(
        "golden-hooks",
        &[
            ("golden/hooks-legacy-command".to_string(), "hooks".to_string(), legacy),
            ("golden/hooks-argv".to_string(), "hooks".to_string(), argv),
        ],
    );
}

#[test]
fn golden_mcp_config_env_indirection_matches_contract_schema() {
    let config = serde_json::json!({
        "mcpServers": {
            "github": {
                "command": "npx",
                "args": ["-y", "@mcp/github"],
                "env": {"GITHUB_TOKEN": "${env:GITHUB_TOKEN}"},
                "disabled": false
            }
        }
    });
    assert_contract_instances_valid(
        "golden-mcp-config",
        &[(
            "golden/mcp-config-env-ref".to_string(),
            "mcp-config".to_string(),
            config,
        )],
    );
}

#[test]
#[cfg(unix)]
fn golden_plugin_tool_descriptors_match_contract_schema() {
    use pony_agent_core::agent::plugin_host::{PluginHostProvider, PluginManifest, PluginSupervisor};
    use pony_agent_core::agent::tools::ToolProvider;
    let root = plugin_fixture::TempRoot::new("golden-descriptor");
    let command = root.write_script(&plugin_fixture::script("pony-plugin-v1", false, false));
    let manifest = PluginManifest::parse(&plugin_fixture::manifest_json("golden-descriptor-fixture", &["echo", "sleep"]))
        .expect("manifest 本身合法");
    let supervisor = PluginSupervisor::spawn(manifest, command, plugin_fixture::config(&root))
        .expect("fixture 应可启动");
    let provider = PluginHostProvider::new(supervisor);
    provider.initialize().expect("正向握手应成功");
    let descriptors = provider.list_tools();
    assert!(!descriptors.is_empty(), "注册后应产出外部 descriptor");
    let cases: Vec<(String, String, serde_json::Value)> = descriptors
        .iter()
        .enumerate()
        .map(|(index, descriptor)| {
            (
                format!("golden/plugin-descriptor-{index:02}"),
                "tool-descriptor".to_string(),
                serde_json::to_value(descriptor).expect("descriptor 应可序列化"),
            )
        })
        .collect();
    assert_contract_instances_valid("golden-plugin-descriptor", &cases);
}
