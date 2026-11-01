//! 红相验收：沙箱「诚实性」契约（T2-2）。
//!
//! 缺陷面（`crates/pony-agent-core/src/agent/sandbox.rs`）：
//! 1. `NativeSandboxBackend::detect()` 仅凭 PATH 里**存在同名文件**就报 `Available`
//!    （`has_command` 只调 `Path::is_file()`，连可执行位都不检查），Windows 分支更是恒报
//!    `Available`；而全仓没有任何代码用 bwrap / sandbox-exec 去 wrap 命令，也没有 Windows
//!    隔离执行实现。即：未真正隔离却声称「已强制隔离」。
//! 2. `validate()` 只做 `workspace_root.contains("..")` 的字符串匹配，不做路径规范化，
//!    因此绝对路径 `/etc/passwd`（以及任何不含 `..` 的逃逸形式）会被判为合规。
//!
//! 本文件断言的是**契约**而不是某个枚举取值：只要实现选择诚实（检测到真实强制能力才声明
//! 已隔离），或真的接上 wrap 实现，断言就会转绿；不依赖本机是否安装 bwrap、不启动任何进程。

use pony_agent_core::agent::sandbox::{
    LocalProcessSandboxBackend, NativeSandboxBackend, SandboxAvailability, SandboxBackend,
    SandboxRequest,
};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// PATH 是进程级全局状态：本文件内串行化，避免并发用例互相污染。
static PATH_LOCK: Mutex<()> = Mutex::new(());

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Self {
        let unique = format!(
            "pony-t2-{prefix}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock before epoch")
                .as_nanos()
        );
        let path = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn write(&self, name: &str, content: &str) -> PathBuf {
        let file = self.path.join(name);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).expect("create parent dir");
        }
        std::fs::write(&file, content).expect("write temp file");
        file
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn request(workspace_root: &str) -> SandboxRequest {
    SandboxRequest {
        workspace_root: workspace_root.to_string(),
        allow_network: false,
        environment_allowlist: Vec::new(),
        isolate_environment: true,
    }
}

/// 契约口径：`Available` 意味着「已强制隔离」。任何其他取值都不构成该声明。
fn claims_enforced_isolation(availability: &SandboxAvailability) -> bool {
    matches!(availability, SandboxAvailability::Available)
}

/// 「PATH 里有同名文件」不等于「有强制隔离能力」。
/// 用例在 PATH 里放一个**普通（不可执行）文件** `bwrap` / `sandbox-exec`：
/// 现状 detect() 只看 `is_file()` ⇒ 立刻声称 Available（红相）；
/// 诚实的实现要么做真实能力探活（执行探针会失败），要么不提供该后端。
#[test]
fn detect_must_not_claim_enforced_isolation_from_a_mere_path_entry() {
    let _guard = PATH_LOCK.lock().unwrap_or_else(|error| error.into_inner());

    let fake_bin_dir = TempDir::new("sandbox-fake-path");
    // 普通文件，没有可执行位：既不是 bwrap，也不是 sandbox-exec
    fake_bin_dir.write("bwrap", "this file only pretends to be bubblewrap");
    fake_bin_dir.write("sandbox-exec", "this file only pretends to be sandbox-exec");

    let previous_path = std::env::var_os("PATH");
    std::env::set_var("PATH", fake_bin_dir.path());
    let probe = NativeSandboxBackend::detect();
    let availability = probe.availability();
    match previous_path {
        Some(value) => std::env::set_var("PATH", value),
        None => std::env::remove_var("PATH"),
    }

    assert!(
        !claims_enforced_isolation(&availability),
        "T2-2：PATH 里只有一个不可执行的同名文件（未真正隔离）时，不得声称已强制隔离；\
         实际 availability={availability:?}。detect() 必须做真实能力探活，或诚实地不提供该后端"
    );
}

/// 反向钉子：PATH 为空时必须**仍然**不声称隔离（现有行为，防止后续修复把探活写反）。
#[test]
fn detect_without_any_sandbox_binary_must_not_claim_isolation() {
    let _guard = PATH_LOCK.lock().unwrap_or_else(|error| error.into_inner());

    let empty_bin_dir = TempDir::new("sandbox-empty-path");
    let previous_path = std::env::var_os("PATH");
    std::env::set_var("PATH", empty_bin_dir.path());
    let availability = NativeSandboxBackend::detect().availability();
    match previous_path {
        Some(value) => std::env::set_var("PATH", value),
        None => std::env::remove_var("PATH"),
    }

    assert!(
        !claims_enforced_isolation(&availability),
        "T2-2：PATH 中不存在任何沙箱二进制时不得声称已强制隔离，实际 availability={availability:?}"
    );
}

/// 路径规范化穿越：凡是解析后落到沙箱 root 之外的 workspace_root 都必须被拒。
/// 现状 `contains(\"..\")` 只能拦到字面 `..`，绝对路径逃逸直接放行（红相）。
#[test]
fn validate_must_reject_every_escape_that_normalizes_outside_the_root() {
    let root = TempDir::new("sandbox-root");
    let backend = LocalProcessSandboxBackend::new(root.path().to_path_buf());

    let escapes = [
        // 字面穿越（现状已拦，作为契约回归钉子）
        "a/../../etc/passwd",
        "../../etc/passwd",
        "./a/../../../etc/passwd",
        // 不含 `..` 的逃逸：绝对路径直接指向 root 之外
        "/etc/passwd",
        "/etc",
    ];

    for escape in escapes {
        let verdict = backend.validate(&request(escape));
        assert!(
            verdict.is_err(),
            "T2-2：解析后落在沙箱 root 之外的 workspace_root 必须被拒，实际放行 {:?}（contains(\"..\") 不是路径规范化）",
            escape
        );
    }

    // 反向钉子：root 内的正常相对路径必须仍然放行（防止“一刀切拒绝”把功能打死）
    for inside in [".", "a/b", "./a/b", "a/b/c.txt"] {
        assert!(
            backend.validate(&request(inside)).is_ok(),
            "T2-2：root 内的正常路径必须放行，实际被拒 {:?}",
            inside
        );
    }
}
