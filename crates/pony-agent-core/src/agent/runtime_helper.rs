use std::sync::OnceLock;
use tokio::runtime::{Handle, Runtime};

pub fn block_on<F: std::future::Future>(f: F) -> F::Output {
    match Handle::try_current() {
        Ok(handle) => handle.block_on(f),
        Err(_) => {
            static RUNTIME: OnceLock<Runtime> = OnceLock::new();
            RUNTIME
                .get_or_init(|| Runtime::new().expect("failed to create tokio runtime"))
                .block_on(f)
        }
    }
}

/// A guard that keeps a tokio runtime context active on the current thread.
/// Drop it to exit the runtime context. Uses a static shared runtime.
pub struct TestRuntimeGuard {
    _guard: tokio::runtime::EnterGuard<'static>,
    _rt: &'static Runtime,
}

impl TestRuntimeGuard {
    pub fn new() -> Self {
        static RUNTIME: OnceLock<Runtime> = OnceLock::new();
        let rt = RUNTIME.get_or_init(|| Runtime::new().expect("failed to create tokio runtime"));
        Self {
            _guard: rt.enter(),
            _rt: rt,
        }
    }

    /// Create a guard and leak it, keeping the runtime context active
    /// for the rest of the process lifetime.
    pub fn leak() {
        std::mem::forget(Self::new());
    }
}
