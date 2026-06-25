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
