pub struct BlockingHelper;

impl BlockingHelper {
    pub async fn spawn<T: Send + 'static>(
        f: impl FnOnce() -> T + Send + 'static,
    ) -> Result<T, String> {
        tauri::async_runtime::spawn_blocking(f)
            .await
            .map_err(|e| format!("blocking helper spawn error: {e}"))
    }
}
