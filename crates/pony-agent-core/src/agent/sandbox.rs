//! Platform sandbox support matrix (PA-076 phase 5, tasks 5.1-5.2).
//!
//! Design.md Decision 7 keeps the sandbox port (`SandboxBackend`) independent from the process
//! lifecycle port (`ProcessBackend`). The former constrains workspace files, network, environment
//! and handle inheritance; the latter is responsible for lifecycle and containment. This module
//! records which containment strategy a platform can offer, and provides the fail-closed gate an
//! autonomous `Run` must pass before it may execute at all.
//!
//! Important: containment is *never* a substitute for approval. A Windows Job Object that forbids
//! breakaway still does not authorize an unsandboxed command; a Unix process group is best-effort
//! containment and cannot claim to stop `setsid`/double-fork escapees. Where no real sandbox
//! backend exists, autonomous `Run` fails closed (`NoSandboxBackend` reports `Unavailable`).
//!
//! Adjudication record (2026-08-04, PA-076 remaining item 4): the real backend is split out as
//! follow-up work, and staying fail-closed is the design-compliant terminal state for this card.
//! PA-077 adds Windows Job Object lifecycle containment in `process.rs` / `process/windows_job.rs`:
//! kill-on-close, no breakaway permission flags, and explicit tree termination. Assignment occurs
//! after std spawn, so pre-assignment descendants can escape (scheduler-dependent window).
//! This best-effort protection is NOT a real SandboxBackend. Full filesystem/network isolation
//! remains separate follow-up work; the historical evaluation is in
//! `management/task-system/02_REVIEWS/2026-08-04-pa076-sandbox-backend-evaluation.md`.
//! Until a real backend is registered, `NoSandboxBackend` + `enforce_sandbox` keep autonomous
//! `Run` at `sandbox_unavailable`, never silently downgrading to an unsandboxed shell.

pub use crate::agent::tool_runtime::{SandboxAvailability, SandboxBackend, SandboxRequest};

/// The containment strategy a platform can offer. This is a support matrix, not an enforcement
/// guarantee: it documents intent and capability so the runtime can decide whether an autonomous
/// `Run` is even allowed to start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SandboxSupportMatrix {
    /// Windows Job Object with breakaway forbidden. Intended containment for Windows children.
    ///
    /// PA-077 implements best-effort Job lifecycle containment in `ProcessManager`; the
    /// post-spawn assignment window is not an escape-proof boundary. This enum is a platform
    /// strategy, NOT `SandboxBackend` availability. Job Objects do not mediate file or network
    /// access and never replace the independent approval/sandbox gate.
    WindowsJobObject,
    /// Unix process group. Best-effort containment only: killing a process group does not stop a
    /// child that calls `setsid` or double-forks. This variant explicitly does not promise to
    /// contain such escapees.
    UnixProcessGroup,
    /// No containment strategy is available on this platform/configuration. Autonomous `Run`
    /// must fail closed.
    NoSandbox,
}

impl SandboxSupportMatrix {
    /// The containment strategy this binary can offer on its host platform. Returns
    /// `WindowsJobObject` on Windows and `UnixProcessGroup` elsewhere; never `NoSandbox` for a
    /// known platform (a real `SandboxBackend` may still be unavailable at runtime).
    pub fn platform_default() -> Self {
        if cfg!(windows) {
            SandboxSupportMatrix::WindowsJobObject
        } else {
            SandboxSupportMatrix::UnixProcessGroup
        }
    }
}

/// Fail-closed sandbox gate shared by the runtime and the `Run` migration (task 5.5).
///
/// An autonomous `Run` may proceed only when the backend reports the sandbox as available (or the
/// host explicitly approved an unsandboxed run) AND the backend accepts the concrete request. An
/// unavailable backend rejects every request: no silent downgrade to unsandboxed execution.
pub fn enforce_sandbox(
    backend: &dyn SandboxBackend,
    request: &SandboxRequest,
) -> Result<(), String> {
    match backend.availability() {
        SandboxAvailability::Unavailable => {
            Err("sandbox is unavailable; autonomous process execution fails closed".to_string())
        }
        SandboxAvailability::Available | SandboxAvailability::HostApprovedUnsandboxed => {
            backend.validate(request)
        }
    }
}

/// A `SandboxBackend` for platforms/configuration with no real sandbox. It always reports
/// `Unavailable`, so autonomous `Run` fails closed wherever this backend is registered.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoSandboxBackend;

impl SandboxBackend for NoSandboxBackend {
    fn availability(&self) -> SandboxAvailability {
        SandboxAvailability::Unavailable
    }

    fn validate(&self, _request: &SandboxRequest) -> Result<(), String> {
        Err(
            "no real sandbox backend is available on this platform; autonomous Run fails closed"
                .to_string(),
        )
    }
}

/// Configurable `SandboxBackend` for dispatcher-level and `Run` migration tests. Availability and
/// the validation verdict are fixed at construction so a test can deterministically exercise the
/// fail-closed gate.
#[derive(Clone, Debug)]
pub struct TestSandboxBackend {
    availability: SandboxAvailability,
    validate_result: Result<(), String>,
}

impl TestSandboxBackend {
    pub fn new(availability: SandboxAvailability, validate_result: Result<(), String>) -> Self {
        Self {
            availability,
            validate_result,
        }
    }

    /// A backend that reports the sandbox as available and accepts every request.
    pub fn available() -> Self {
        Self::new(SandboxAvailability::Available, Ok(()))
    }

    /// A backend that reports the sandbox as unavailable (fails closed).
    pub fn unavailable() -> Self {
        Self::new(
            SandboxAvailability::Unavailable,
            Err("sandbox is unavailable".to_string()),
        )
    }

    /// A backend that reports an explicit host-approved unsandboxed run and accepts the request.
    pub fn host_approved_unsandboxed() -> Self {
        Self::new(SandboxAvailability::HostApprovedUnsandboxed, Ok(()))
    }

    /// A backend that reports the sandbox as available but rejects the concrete request.
    pub fn rejecting(reason: impl Into<String>) -> Self {
        Self::new(SandboxAvailability::Available, Err(reason.into()))
    }

    pub fn availability(&self) -> SandboxAvailability {
        self.availability.clone()
    }
}

impl SandboxBackend for TestSandboxBackend {
    fn availability(&self) -> SandboxAvailability {
        self.availability.clone()
    }

    fn validate(&self, _request: &SandboxRequest) -> Result<(), String> {
        self.validate_result.clone()
    }
}

/// 信任透传后端，对齐 DSH danger-full-access / bash-local，报告 HostApprovedUnsandboxed 且放行命令
#[derive(Clone, Copy, Debug, Default)]
pub struct HostApprovedUnsandboxedBackend;

impl HostApprovedUnsandboxedBackend {
    pub fn new() -> Self {
        Self
    }
}

impl SandboxBackend for HostApprovedUnsandboxedBackend {
    fn availability(&self) -> SandboxAvailability {
        SandboxAvailability::HostApprovedUnsandboxed
    }

    fn validate(&self, _request: &SandboxRequest) -> Result<(), String> {
        Ok(())
    }
}

/// 平台原生沙箱探活与门禁后端
#[derive(Clone, Debug)]
pub struct NativeSandboxBackend {
    availability: SandboxAvailability,
}

impl NativeSandboxBackend {
    /// 依据当前平台支持能力与系统工具探活
    pub fn detect() -> Self {
        // 诚实性（T2-2）：`Available` 在本项目的语义是**「已强制隔离」**，不是
        // 「本机存在某个同名二进制」。本 build 内没有任何代码用 `bwrap` / `sandbox-exec` /
        // Windows Job Object 去 wrap 待执行命令（真正的强制实现属后续工作），
        // 因此这里恒报 `Unavailable`：上层据此 fail-closed，绝不静默降级成
        // 「看起来隔离了、其实直接跑」。等真实 wrap 落地后才允许按真实探活声明 `Available`。
        Self {
            availability: SandboxAvailability::Unavailable,
        }
    }
}

impl SandboxBackend for NativeSandboxBackend {
    fn availability(&self) -> SandboxAvailability {
        self.availability.clone()
    }

    fn validate(&self, request: &SandboxRequest) -> Result<(), String> {
        match self.availability {
            SandboxAvailability::Available => {
                validate_workspace_root(&request.workspace_root, None)
            }
            SandboxAvailability::Unavailable => {
                Err("Native sandbox backend is unavailable on this host system".to_string())
            }
            SandboxAvailability::HostApprovedUnsandboxed => Ok(()),
        }
    }
}

/// workspace_root 的路径穿越校验（fail-closed）。
///
/// 语义（替代旧的 `contains("..")` 字符串匹配 —— 那既不规范化，也拦不住绝对路径逃逸）：
/// 1. 先做**词法规范化**（逐 `Component` 处理 `.` / `..`，纯路径语义，不碰文件系统、不跟随符号链接）；
/// 2. 规范化过程中若 `..` 越过基准根 ⇒ 拒绝；
/// 3. 若后端持有沙箱 root，规范化后的路径必须**按组件**落在该 root 之下
///    （`Path::starts_with` 是组件级比较，`/tmp/ab` 不会被误判为 `/tmp/a` 的子路径）；
/// 4. 无 root 的后端以请求自身为基准，只拒绝越界与空值。
fn validate_workspace_root(
    workspace_root: &str,
    sandbox_root: Option<&std::path::Path>,
) -> Result<(), String> {
    if workspace_root.trim().is_empty() {
        return Err(
            "Sandbox security policy violation: empty workspace root is not allowed".to_string(),
        );
    }

    let normalized = normalize_lexically(std::path::Path::new(workspace_root))?;
    let Some(root) = sandbox_root else {
        // ADVISORY 修复：没有配置沙箱 root 的后端**无法证明**绝对路径落在隔离范围内
        // （`/etc/passwd` 规范化后依然"合法"）。此处 fail-closed 拒绝绝对路径，
        // 而不是像旧实现那样直接放行 —— 一旦该分支被真后端启用，旧写法就是 fail-open。
        if normalized.is_absolute() {
            return Err(format!(
                "Sandbox security policy violation: workspace root `{workspace_root}` is absolute but this backend has no sandbox root to contain it"
            ));
        }
        return Ok(());
    };

    let normalized_root = normalize_lexically(root)?;
    let candidate = if normalized.is_absolute() {
        normalized
    } else {
        normalized_root.join(normalized)
    };
    let candidate = normalize_lexically(&candidate)?;
    if candidate.starts_with(&normalized_root) {
        Ok(())
    } else {
        Err(format!(
            "Sandbox security policy violation: workspace root `{workspace_root}` resolves outside the sandbox root `{}`",
            normalized_root.display()
        ))
    }
}

/// 词法路径规范化：消掉 `.`、按栈处理 `..`；`..` 越过基准根时返回错误。
fn normalize_lexically(path: &std::path::Path) -> Result<std::path::PathBuf, String> {
    use std::path::Component;

    let mut normalized = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(std::path::MAIN_SEPARATOR.to_string()),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(format!(
                        "Sandbox security policy violation: path traversal detected in workspace root `{}`",
                        path.display()
                    ));
                }
            }
            Component::Normal(part) => normalized.push(part),
        }
    }
    Ok(normalized)
}

/// Linux/Unix 环境下的轻量级命名空间与工作区受限沙箱实现
#[derive(Clone, Debug, Default)]
pub struct LocalProcessSandboxBackend {
    pub workspace_root: Option<std::path::PathBuf>,
}

impl LocalProcessSandboxBackend {
    pub fn new(root: impl Into<std::path::PathBuf>) -> Self {
        Self {
            workspace_root: Some(root.into()),
        }
    }
}

impl SandboxBackend for LocalProcessSandboxBackend {
    fn availability(&self) -> SandboxAvailability {
        // ADVISORY 修复（与 T2-2 诚实性口径一致）：本后端只做 workspace root 的**路径门禁**
        // （`validate` 的词法规范化比较），并没有任何命名空间/文件系统/网络的强制包裹。
        // `Available` 在本项目语义是"已强制隔离"，所以这里只能报 `Unavailable`，
        // 让 `enforce_sandbox` fail-closed；等真正的 wrap 实现落地后再改回对应取值。
        SandboxAvailability::Unavailable
    }

    fn validate(&self, request: &SandboxRequest) -> Result<(), String> {
        validate_workspace_root(&request.workspace_root, self.workspace_root.as_deref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> SandboxRequest {
        SandboxRequest {
            workspace_root: ".".to_string(),
            allow_network: false,
            environment_allowlist: Vec::new(),
            isolate_environment: false,
        }
    }

    /// ADVISORY 修复：无配置 root 的后端必须拒绝绝对路径（旧实现直接放行 = fail-open）。
    #[test]
    fn validate_without_sandbox_root_rejects_absolute_paths() {
        assert!(
            validate_workspace_root("/etc/passwd", None).is_err(),
            "无 root 时无法证明绝对路径被隔离，必须 fail-closed"
        );
        assert!(
            validate_workspace_root("a/b", None).is_ok(),
            "root 内相对路径应放行"
        );
        assert!(
            validate_workspace_root("a/../../etc/passwd", None).is_err(),
            "词法穿越必须被拒"
        );
    }

    /// ADVISORY 修复：只做路径门禁的后端不得声称"已强制隔离"。
    #[test]
    fn local_process_backend_must_not_claim_enforced_isolation() {
        let backend = LocalProcessSandboxBackend::new(".");
        assert_eq!(
            backend.availability(),
            SandboxAvailability::Unavailable,
            "没有 wrap 实现就不能报 Available；否则 enforce_sandbox 会放行未隔离的执行"
        );
        assert!(
            enforce_sandbox(&backend, &sample_request()).is_err(),
            "诚实化后 enforce_sandbox 必须 fail-closed"
        );
    }

    #[test]
    fn platform_default_is_sane_and_never_no_sandbox() {
        let default = SandboxSupportMatrix::platform_default();
        if cfg!(windows) {
            assert_eq!(default, SandboxSupportMatrix::WindowsJobObject);
        } else {
            assert_eq!(default, SandboxSupportMatrix::UnixProcessGroup);
        }
        assert_ne!(default, SandboxSupportMatrix::NoSandbox);
    }

    #[test]
    fn no_sandbox_backend_reports_unavailable_and_rejects() {
        let backend = NoSandboxBackend;
        assert_eq!(backend.availability(), SandboxAvailability::Unavailable);
        assert!(backend.validate(&sample_request()).is_err());
    }

    #[test]
    fn test_sandbox_backend_honors_configured_availability() {
        assert_eq!(
            TestSandboxBackend::available().availability(),
            SandboxAvailability::Available
        );
        assert_eq!(
            TestSandboxBackend::unavailable().availability(),
            SandboxAvailability::Unavailable
        );
        assert_eq!(
            TestSandboxBackend::host_approved_unsandboxed().availability(),
            SandboxAvailability::HostApprovedUnsandboxed
        );
    }

    #[test]
    fn test_sandbox_backend_honors_configured_validation() {
        let accepting = TestSandboxBackend::available();
        assert!(accepting.validate(&sample_request()).is_ok());

        let rejecting = TestSandboxBackend::rejecting("workspace path outside root");
        let error = rejecting
            .validate(&sample_request())
            .expect_err("must reject");
        assert!(error.contains("workspace path outside root"));
    }

    #[test]
    fn enforce_sandbox_fails_closed_when_backend_is_unavailable() {
        let request = sample_request();
        assert!(enforce_sandbox(&NoSandboxBackend, &request).is_err());
        assert!(enforce_sandbox(&TestSandboxBackend::unavailable(), &request).is_err());
    }

    #[test]
    fn enforce_sandbox_accepts_available_backend_and_rejects_denied_request() {
        let request = sample_request();
        assert!(enforce_sandbox(&TestSandboxBackend::available(), &request).is_ok());
        assert!(
            enforce_sandbox(&TestSandboxBackend::host_approved_unsandboxed(), &request).is_ok()
        );
        assert!(enforce_sandbox(&TestSandboxBackend::rejecting("denied"), &request).is_err());
    }
}
