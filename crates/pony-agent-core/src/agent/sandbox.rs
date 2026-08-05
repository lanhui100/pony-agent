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
//! A Windows Job Object containment backend is feasible — `windows-sys` 0.61.2 is already in the
//! dependency tree (transitive) and exposes `CreateJobObjectW` / `SetInformationJobObject`
//! (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, breakaway flags off) / `AssignProcessToJobObject` /
//! `TerminateJobObject` under the `Win32_System_JobObjects` feature. A full sandbox (workspace
//! file + network containment plus Job Object) is a larger, separate effort. Both are follow-up
//! cards; see `management/task-system/02_REVIEWS/2026-08-04-pa076-sandbox-backend-evaluation.md`.
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
    /// Implementation note: Job Objects are a documented spike decision for this phase and are
    /// NOT implemented as full containment yet. Even when implemented, a Job Object limits what a
    /// process tree can do (kill-on-close, breakaway prevention); it does NOT mediate file or
    /// network access and is therefore never a substitute for the `SandboxBackend` approval gate.
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
        SandboxAvailability::Unavailable => Err(
            "sandbox is unavailable; autonomous process execution fails closed".to_string(),
        ),
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
        Self::new(SandboxAvailability::Unavailable, Err(
            "sandbox is unavailable".to_string(),
        ))
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
        let error = rejecting.validate(&sample_request()).expect_err("must reject");
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
        assert!(enforce_sandbox(
            &TestSandboxBackend::host_approved_unsandboxed(),
            &request
        )
        .is_ok());
        assert!(enforce_sandbox(&TestSandboxBackend::rejecting("denied"), &request).is_err());
    }
}
