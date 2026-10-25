//! 最小 JSON 日志面（Wave43 D8）。
//!
//! 契约：只服务本次新增的 trace 相关代码路径，**不替换**存量 `eprintln!`
//! （PA-119 债务豁免范围不得借本次扩大）。
//!
//! Wave-4 C4 增量：
//! - `trace_id` 在存在活跃 turn 时必须写入真实的 32 位十六进制 trace_id（无活跃 turn 才写 `null`）；
//! - 桌面端可经 [`set_log_file_path`] 注入落盘路径（追加 + 512KiB 轮转上限），
//!   落盘异常一律降级为「不落盘」，绝不阻塞、绝不 panic；
//! - 输出/落盘前统一过 [`crate::agent::ponysentry::sanitize`] 脱敏。
//!
//! Wave-4.1 修复（L3-A #3/#4）：
//! - 文件 sink 全程持有**同一把进程级 Mutex**（取路径 → 判长度 → 轮转 → 写整行），
//!   且行尾并入**同一次** `write_all`，并发行不会交错成非法 JSON；
//! - 轮转失败时**保留原文件并放弃本轮落盘**，绝不 truncate 销毁既有日志。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

/// 日志单文件体积上限（字节）= 512KiB（Wave-4 C4.2；与 NFR `observability.log_file_max_bytes` 对齐）。
const LOG_FILE_MAX_BYTES: u64 = 524_288;

/// 文件 sink 的全局状态：路径 + 串行化锁（`None` ⇒ 仅输出 stdout）。
static LOG_FILE: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

fn log_file_slot() -> MutexGuard<'static, Option<PathBuf>> {
    LOG_FILE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 注入桌面端日志落盘路径（追加写；父目录不存在则按需创建）。
///
/// 调用方：Tauri 启动装配点（`app.path().app_data_dir()` 解析后注入
/// `<app_data_dir>/logs/pony-agent.log`）。
/// 语义：幂等，重复注入以最后一次为准；创建父目录的失败只降级为「后续落盘静默失败」，
/// 不返回错误、不 panic（日志不可用绝不阻断主链路）。
pub fn set_log_file_path(path: impl Into<PathBuf>) {
    let path = path.into();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    *log_file_slot() = Some(path);
}

/// 读取当前注入的日志落盘路径（诊断/测试用）。
pub fn log_file_path() -> Option<PathBuf> {
    log_file_slot().clone()
}

/// 输出一行 JSON 日志。
///
/// 调用方：trace 装配与导出失败降级路径（例如导出错误时 `warn` 一条）。
/// 语义：单行 JSON，至少含 `level` / `message` / `ts` / `trace_id` 四个字段；
/// 输出目标为进程 **stdout**（供宿主/运维按行采集）并在注入了落盘路径时同步追加到日志文件；
/// 不做异步排队、不依赖日志框架。
/// `level` 取字面量 `"debug"` / `"info"` / `"warn"` / `"error"`——**调用点必须直接写
/// 字面量**（本模块不提供 level 常量，以免静态扫描断言失效）。
/// `trace_id` 参数为 `None` 时**先**取当前活跃 turn 的真实 trace_id（Wave-4 C4.1），
/// 仍取不到才输出 JSON `null`，不得省略键。
/// 失败语义：本函数不返回错误、不 panic；写 stdout/文件失败时静默丢弃。
pub fn log_json(level: &str, message: &str, trace_id: Option<&str>) {
    let timestamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    // C4.3：落盘与输出前统一脱敏（调用方可能传入未经清洗的外部内容）。
    let sanitized = crate::agent::ponysentry::sanitize(message);
    // C4.1：显式未提供时回落到当前活跃 turn 的真实 trace_id；取不到才为 null。
    let resolved_trace_id = trace_id
        .map(str::to_string)
        .or_else(crate::agent::otel::current_trace_id);
    let record = serde_json::json!({
        "level": level,
        "message": sanitized,
        "ts": timestamp,
        // Option 序列化为 null 时键仍然存在（契约：不得省略 trace_id 键）。
        "trace_id": resolved_trace_id,
    });

    let Ok(line) = serde_json::to_string(&record) else {
        return;
    };
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    let _ = writeln!(handle, "{line}");
    let _ = handle.flush();
    drop(handle);

    append_to_file(&line);
}

/// 追加一行到注入的日志文件；全程持锁，超限先轮转。任何异常都静默降级。
///
/// 并发安全（L3-A #3）：路径读取、长度判定、轮转与写入在同一把 Mutex 内完成；
/// 行尾 `\n` 与正文合并为**同一次** `write_all`，因此并发写不会产生交错行。
fn append_to_file(line: &str) {
    let slot = log_file_slot();
    let Some(path) = slot.as_ref() else {
        return;
    };

    let mut payload = String::with_capacity(line.len() + 1);
    payload.push_str(line);
    payload.push('\n');
    let incoming = payload.len() as u64;

    if current_len(path) + incoming > LOG_FILE_MAX_BYTES && !rotate(path) {
        // 轮转失败：保留原文件、放弃本轮落盘（绝不 truncate 销毁既有日志）。
        return;
    }

    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path.as_path())
    {
        let _ = file.write_all(payload.as_bytes());
        let _ = file.flush();
    }
}

/// 现有文件长度；不存在或不可读时按 0 处理（下一次直接新建）。
fn current_len(path: &Path) -> u64 {
    std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
}

/// 轮转：把当前文件改名为 `<name>.1`（覆盖旧备份），返回是否成功。
///
/// 语义（L3-A #4）：**只做 rename**，失败返回 `false` 由调用方放弃本轮落盘；
/// 绝不 truncate —— 日志卷不可写/被占用时宁可丢这一行，也不销毁已落盘的证据。
fn rotate(path: &Path) -> bool {
    let backup = path.with_extension(rotated_extension(path));
    std::fs::rename(path, &backup).is_ok()
}

/// 轮转备份文件后缀（保留原扩展名语义：`pony-agent.log` → `pony-agent.log.1`）。
fn rotated_extension(path: &Path) -> String {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) if !ext.is_empty() => format!("{ext}.1"),
        _ => "1".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// 日志 sink 是进程级全局态，本模块单测必须串行（避免互相覆盖路径/轮转）。
    static LOG_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn lock() -> MutexGuard<'static, ()> {
        LOG_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn temp_log_path(tag: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("pony-otel-log-{tag}-{stamp}.log"))
    }

    /// L3-A #3：多线程并发写日志时，文件里每一行都必须是可解析的完整 JSON（不得交错）。
    #[test]
    fn concurrent_writes_produce_parseable_single_line_json() {
        let _guard = lock();
        let path = temp_log_path("concurrent");
        set_log_file_path(path.clone());

        let mut handles = Vec::new();
        for thread_index in 0..8 {
            handles.push(std::thread::spawn(move || {
                for line_index in 0..200 {
                    log_json(
                        "warn",
                        &format!("thread={thread_index} line={line_index}"),
                        Some("4bf92f3577b34da6a3ce929d0e0e4736"),
                    );
                }
            }));
        }
        for handle in handles {
            handle.join().expect("writer thread must not panic");
        }

        let content = std::fs::read_to_string(&path).expect("log file readable");
        let lines: Vec<&str> = content.lines().collect();
        assert!(
            lines.len() >= 8 * 200,
            "至少应写入 1600 行，实际 {}",
            lines.len()
        );
        for (index, line) in lines.iter().enumerate() {
            let parsed: Result<serde_json::Value, _> = serde_json::from_str(line);
            assert!(
                parsed.is_ok(),
                "L3-A#3：第 {index} 行不是完整 JSON（并发写交错）：{line}"
            );
            assert_eq!(
                parsed.expect("checked")["level"], "warn",
                "第 {index} 行 level 字段异常"
            );
        }

        let _ = std::fs::remove_file(&path);
    }

    /// L3-A #4：轮转 rename 失败时**不得** truncate 既有日志（只放弃本轮落盘）。
    #[test]
    fn rotation_failure_keeps_existing_log_intact() {
        let _guard = lock();
        let path = temp_log_path("rotate-fail");
        std::fs::write(&path, "EXISTING-LOG-LINE\n").expect("seed log");

        // 让备份目标成为「非空目录」⇒ rename 在类 unix 上返回 EISDIR/ENOTEMPTY。
        let backup = path.with_extension(rotated_extension(&path));
        std::fs::create_dir_all(&backup).expect("mkdir backup");
        std::fs::write(backup.join("keep.txt"), "keep").expect("seed backup content");

        assert!(!rotate(&path), "rename 到非空目录必须失败");
        let content = std::fs::read_to_string(&path).expect("原文件必须仍可读");
        assert_eq!(
            content, "EXISTING-LOG-LINE\n",
            "L3-A#4：轮转失败时原日志必须原样保留（严禁 truncate 销毁）"
        );

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&backup);
    }

    /// C4.2：写入超限时轮转为 `.1`，新旧文件都保留且内容完整。
    #[test]
    fn rotation_creates_backup_and_keeps_writing() {
        let _guard = lock();
        let path = temp_log_path("rotate");
        std::fs::write(&path, vec![b'x'; LOG_FILE_MAX_BYTES as usize - 16])
            .expect("seed near-limit log");
        set_log_file_path(path.clone());

        log_json("warn", "after rotation", None);

        let backup = path.with_extension(rotated_extension(&path));
        assert!(backup.exists(), "超限后必须产生轮转备份文件");
        assert_eq!(
            std::fs::metadata(&backup).expect("backup meta").len(),
            LOG_FILE_MAX_BYTES - 16,
            "备份文件必须完整保留轮转前内容"
        );
        let current = std::fs::read_to_string(&path).expect("new log readable");
        let parsed: serde_json::Value =
            serde_json::from_str(current.trim_end()).expect("轮转后的新文件必须是合法 JSON 行");
        assert_eq!(parsed["message"], "after rotation");

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&backup);
    }
}
