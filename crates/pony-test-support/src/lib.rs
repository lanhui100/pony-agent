//! 共享测试夹具与内存存储桩（供工作区各 crate 集成测试复用，避免 `pony-agent-core`
//! 通过 dev-dependencies 自引用 `test-support` 特性触发二次编译）。

use pony_storage::session::{PersistedStore, SessionBackend, SessionStore};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

static PROCESS_ENV_LOCK: Mutex<()> = Mutex::new(());

/// 同一集成测试套件二进制内所有修改或读取共享进程环境 / 全局 OTel 状态的用例共用一把锁。
pub fn env_lock() -> MutexGuard<'static, ()> {
    PROCESS_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// B230：跨平台测试路径工具（剥离 Windows `\\?\` verbatim 前缀）。
pub fn deverbatim(path: &Path) -> PathBuf {
    let raw = path.to_string_lossy();
    let normalized = if let Some(rest) = raw.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = raw.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        raw.into_owned()
    };
    PathBuf::from(normalized)
}

/// 将模块内测试函数名解析为当前集成测试二进制在 `cargo test -- --exact` 下的完整过滤名。
pub fn exact_filter(module_path: &str, fn_name: &str) -> String {
    match module_path.split_once("::") {
        Some((_, rel)) => format!("{rel}::{fn_name}"),
        None => fn_name.to_string(),
    }
}

/// 纯内存会话后端（仅保留临时附件目录并在 Drop 时回收）。
pub struct MemorySessionBackend {
    attachment_root: PathBuf,
}

impl MemorySessionBackend {
    pub fn new(attachment_root: PathBuf) -> Self {
        Self { attachment_root }
    }
}

impl SessionBackend for MemorySessionBackend {
    fn load_store(&self) -> Option<PersistedStore> {
        None
    }

    fn save_store(&self, _store: &PersistedStore) -> Result<(), String> {
        Ok(())
    }

    fn attachment_root(&self) -> Option<PathBuf> {
        Some(self.attachment_root.clone())
    }
}

impl Drop for MemorySessionBackend {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.attachment_root);
    }
}

fn unique_test_session_dir(prefix: &str) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
    std::env::temp_dir()
        .join(format!("{prefix}-{stamp}"))
        .join("sessions")
}

/// 构造隔离的内存会话存储（不落盘、不触及真实用户会话库）。
pub fn memory_only_session_store() -> SessionStore {
    SessionStore::with_backend(Box::new(MemorySessionBackend::new(
        unique_test_session_dir("pony-agent-memory-attachments"),
    )))
}

/// 解析 `ponycli` 可执行文件路径（供跨 crate 集成测试复用，避免 `env!("CARGO_BIN_EXE_ponycli")` 编译期绑定）。
///
/// 优先读取 `CARGO_BIN_EXE_ponycli`；其次检查当前测试二进制同级 target profile 目录；
/// 若尚未构建则按当前 `target-dir` 触发一次 `cargo build -p pony-cli --bin ponycli`。
pub fn ponycli_bin() -> PathBuf {
    static BIN_PATH: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    BIN_PATH
        .get_or_init(|| {
            if let Some(explicit) = std::env::var_os("CARGO_BIN_EXE_ponycli") {
                let p = PathBuf::from(explicit);
                if p.exists() {
                    return p;
                }
            }
            let exe_name = if cfg!(windows) { "ponycli.exe" } else { "ponycli" };
            let current = std::env::current_exe().expect("resolve current_exe");
            // current_exe: <target-dir>/<profile>/deps/<test-bin>
            let profile_dir = current
                .parent()
                .and_then(|p| p.parent())
                .expect("resolve target profile dir");
            let candidate = profile_dir.join(exe_name);
            if candidate.exists() {
                return candidate;
            }
            let target_dir = profile_dir.parent().expect("resolve target dir");
            let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(|p| p.parent())
                .expect("resolve workspace root");
            let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
            let status = std::process::Command::new(cargo)
                .current_dir(workspace_root)
                .arg("build")
                .arg("--manifest-path")
                .arg(workspace_root.join("crates/pony-cli/Cargo.toml"))
                .arg("--target-dir")
                .arg(target_dir)
                .arg("--bin")
                .arg("ponycli")
                .status()
                .expect("spawn cargo build -p pony-cli --bin ponycli");
            assert!(
                status.success(),
                "构建 ponycli 失败（status={status:?}）"
            );
            assert!(
                candidate.exists(),
                "构建后仍未找到 ponycli 二进制: {}",
                candidate.display()
            );
            candidate
        })
        .clone()
}
