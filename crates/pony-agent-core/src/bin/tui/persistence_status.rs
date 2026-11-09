//! 单实例锁的**启动预检** + 退出提示需要的**持久化事实采集**（P0 数据丢失事故的防线）。
//!
//! 真实事故：第二个 `pony` TUI 抢不到 `sessions.db.lock` → 旧实现只往 stderr 打一句
//! 「已被其他进程独占」（而 TUI 启动后 stderr 被 `dup2` 进 `tui-runtime.log`，用户永远
//! 看不到）→ 它照常进 TUI、照常接收输入、每轮在提交边界失败 → 退出时还打印
//! `pony resume <id>` 的假承诺。本模块负责两件事：
//!
//! 1. [`startup_permission`]：锁被**别的进程**占着就 fail-closed（不静默降级为可交互只读）；
//! 2. [`session_persistence_evidence`] / [`SaveFailureWitness`]：退出提示所需的**事实**
//!    （该 id 到底落没落库、本次运行期间有没有保存失败），文案骨架在 `tui/terminal.rs`。
//!
//! 纪律：判定函数是**纯函数**（可单测），IO 只出现在「采集事实」的薄函数里。

use std::path::{Path, PathBuf};

use pony_agent_core::agent::instance_lock::{
    lock_file_path, single_instance_lock_status, InstanceLockStatus,
};
use pony_agent_core::agent::session::SessionStore;
use pony_agent_core::agent::sqlite_session::default_sqlite_path;

use super::PersistenceEvidence;

/// 启动预检结论：继续，或**拒绝启动**并附可执行的诊断文案。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupPermission {
    Proceed,
    Refuse(String),
}

/// 纯判定：`HeldByOther` ⇒ 拒绝启动；`Writable` / `Unavailable` ⇒ 继续既有流程。
///
/// 为什么 `Unavailable` 也放行：那是「探测本身失败」（锁文件打不开等），与「别人正占着」
/// 不是一回事，拒绝启动会把偶发的权限问题升级成完全不可用。
pub fn startup_permission(
    db_path: &Path,
    status: &InstanceLockStatus,
    runtime_log_display: &str,
) -> StartupPermission {
    match status {
        InstanceLockStatus::HeldByOther {
            pid,
            command,
            since_unix_ms,
        } => StartupPermission::Refuse(refusal_message(
            db_path,
            *pid,
            command.as_deref(),
            *since_unix_ms,
            runtime_log_display,
        )),
        InstanceLockStatus::Writable | InstanceLockStatus::Unavailable { .. } => {
            StartupPermission::Proceed
        }
    }
}

/// 采集当前会话库的锁状态并判定（宿主在 `main` 分发里调用，早于任何 `SessionStore`）。
pub fn tui_startup_permission() -> StartupPermission {
    let db_path = default_sqlite_path();
    let status = single_instance_lock_status(&db_path);
    startup_permission(&db_path, &status, &runtime_log_display(None))
}

/// 启动预检的**执行**形态：拒绝启动时向真实 stderr 打印诊断并 `exit(1)`。
///
/// 调用点必须在「创建任何 `SessionStore` / 进入 raw mode」之前：那时 stderr 还没被 `dup2`
/// 进运行日志，用户当场就能看到；一旦进了 TUI，诊断就只能落进日志文件（=事故原来的样子）。
pub fn enforce_startup_permission() {
    if let StartupPermission::Refuse(message) = tui_startup_permission() {
        eprintln!("{message}");
        std::process::exit(1);
    }
}

/// 拒绝启动的诊断文案：库路径 / 持有者 pid / 命令行 / 启动时间 + 两条明确处置。
fn refusal_message(
    db_path: &Path,
    pid: Option<u32>,
    command: Option<&str>,
    since_unix_ms: Option<u64>,
    runtime_log_display: &str,
) -> String {
    let lock_path = lock_file_path(db_path);
    let pid = pid
        .map(|pid| pid.to_string())
        .unwrap_or_else(|| "未知（锁文件里没有身份信息）".to_string());
    let command = command
        .filter(|command| !command.trim().is_empty())
        .unwrap_or("未知");
    let since = format_since_unix_ms(since_unix_ms);
    let mut message = format!(
        "pony 拒绝启动：会话库正被另一个 pony 进程独占（单实例锁）。\n\
         \n\
         \x20 会话库：{}\n\
         \x20 锁文件：{}\n\
         \x20 持有者：pid={pid}\n\
         \x20 命令行：{command}\n\
         \x20 启动于：{since}\n\
         \n\
         为什么拒绝：并发写同一个会话库会让本实例静默降级为只读——你输入的内容不会落库，\n\
         退出时的续接提示也会变成假承诺（「聊完退出、什么都没保存」就是这么发生的）。\n\
         \n\
         怎么处理（任选其一）：\n\
         1. 回到那个 pony 所在的终端退出它（Ctrl-C 或 /quit），再重新运行本命令；\n\
         2. 只想在这个实例里换会话：在 TUI 内用 `/resume`，不要再开第二个 pony 进程。\n",
        db_path.display(),
        lock_path.display()
    );
    if !runtime_log_display.trim().is_empty() {
        message.push_str(&format!("\n更多上下文：{}\n", runtime_log_display.trim()));
    }
    message
}

/// `since_unix_ms` → 本地可读时间（读不到就明说「未知」，绝不 panic）。
fn format_since_unix_ms(since_unix_ms: Option<u64>) -> String {
    use chrono::TimeZone;
    match since_unix_ms {
        None => "未知".to_string(),
        Some(ms) => match chrono::Local.timestamp_millis_opt(ms as i64).single() {
            Some(at) => format!("{}（unix_ms={ms}）", at.format("%Y-%m-%d %H:%M:%S %:z")),
            None => format!("unix_ms={ms}"),
        },
    }
}

/// 该会话 id 是否**真的**在会话库里（有可恢复历史）。
///
/// 数据源是既有 API `SessionStore::new().list_sessions()`（只读；`list_sessions` 只返回
/// `session_is_persistable` 的行，因此「0 轮、无任何内容」的会话会被判 `NotPersisted`——
/// 那本来也没有可恢复历史，与用户直觉一致）。
///
/// **接线点**：另一位队友正在加廉价的 `session_exists(id)`；切换时只改这一处（函数签名与
/// 语义不变），调用点 `run_tui` 无需改动。
pub fn session_persistence_evidence(session_id: &str) -> PersistenceEvidence {
    // P1：退出路径同样不能整库装载（一次 `SessionStore::new()` = 日志 fold + 自愈 + 可能的
    // 整包回写）。这里只发一条只读 SELECT；失败才退回整库视图（慢但语义不变）。
    let db_path = default_sqlite_path();
    let cheap = pony_agent_core::agent::session::load_session_meta_rows(&db_path)
        .map(|rows| rows.iter().any(|row| row.conversation_id == session_id));
    let persisted = match cheap {
        Ok(persisted) => persisted,
        Err(error) => {
            eprintln!(
                "[pony-agent][session] persistence_evidence_fallback：快速查询失败，退回整库视图（{error}）"
            );
            SessionStore::new()
                .list_sessions()
                .iter()
                .any(|session| session.conversation_id == session_id)
        }
    };
    if persisted {
        PersistenceEvidence::Persisted
    } else {
        PersistenceEvidence::NotPersisted
    }
}

/// 运行期「保存失败」的目击者：只扫描**本次运行新增**的那段运行日志。
///
/// 为什么用日志而不是计数器：提交失败目前只 `eprintln!`（`store_stage0::commit_store`），
/// 而 TUI 的 stderr 已被 dup2 进运行日志——那段日志就是现成的、零侵入的事实源。记录启动
/// 时的文件长度，退出时只看新增部分，历史运行里的旧失败不会被算进本次。
pub struct SaveFailureWitness {
    path: Option<PathBuf>,
    offset: u64,
}

/// 提交/落盘失败在现场留下的字样（中文是 `commit_store` 的原文，英文是 SQLite 的原文）。
const SAVE_FAILURE_MARKERS: [&str; 5] = [
    "数据可能丢失",
    "commit_boundary_failed",
    "attempt to write a readonly database",
    "single_instance_lock_denied",
    "save_to_backend 失败",
];

/// 单次扫描上限：日志轮转异常时不要把退出流程拖成一次整文件读。
const MAX_SCAN_BYTES: u64 = 1024 * 1024;

impl SaveFailureWitness {
    /// 记录「本次运行的日志起点」（在 stderr 重定向**之前**调用）。
    pub fn capture(path: Option<&Path>) -> Self {
        let offset = path
            .and_then(|path| std::fs::metadata(path).ok())
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        Self {
            path: path.map(Path::to_path_buf),
            offset,
        }
    }

    /// 本次运行期间是否出现过保存失败（读不到日志 ⇒ `false`，不阻塞退出）。
    pub fn observed_save_failure(&self) -> bool {
        let Some(path) = self.path.as_deref() else {
            return false;
        };
        let Ok(file) = std::fs::File::open(path) else {
            return false;
        };
        let len = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
        if len <= self.offset {
            return false;
        }
        use std::io::{Read, Seek, SeekFrom};
        let mut file = file;
        if self.offset > 0 && file.seek(SeekFrom::Start(self.offset)).is_err() {
            return false;
        }
        let take = (len - self.offset).min(MAX_SCAN_BYTES) as usize;
        let mut buffer = vec![0u8; take];
        if file.read_exact(&mut buffer).is_err() {
            return false;
        }
        segment_reports_save_failure(&String::from_utf8_lossy(&buffer))
    }
}

/// 纯判定：这段日志里有没有保存失败的痕迹。
fn segment_reports_save_failure(segment: &str) -> bool {
    SAVE_FAILURE_MARKERS
        .iter()
        .any(|marker| segment.contains(marker))
}

/// 退出提示里要展示的运行日志路径（缺省日志路径也告诉用户，便于自查）。
pub fn runtime_log_display(path: Option<&Path>) -> String {
    match path {
        Some(path) => path.display().to_string(),
        None => super::tui_runtime_log_path()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "(未启用运行日志)".to_string()),
    }
}

/// 退出提示的**唯一接线点**（顺序敏感）。
///
/// 先取「本次运行有没有保存失败」，再查库：查库本身可能往运行日志追加诊断行，顺序颠倒会把
/// 自查噪声算成运行期失败（= 对用户谎报数据丢失）。
pub fn emit_exit_hint(
    session_id: &str,
    save_failure_witness: &SaveFailureWitness,
    runtime_log_path: Option<&Path>,
) {
    let save_failure_observed = save_failure_witness.observed_save_failure();
    let evidence = session_persistence_evidence(session_id);
    super::print_resume_hint_with_evidence(
        session_id,
        evidence,
        save_failure_observed,
        &runtime_log_display(runtime_log_path),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_failure_segment_detection_matches_commit_store_messages() {
        assert!(segment_reports_save_failure(
            "[pony-agent][session] save_to_backend 失败：commit_boundary_failed（数据可能丢失）"
        ));
        assert!(segment_reports_save_failure(
            "commit_boundary_failed: begin immediate tx: attempt to write a readonly database"
        ));
        assert!(segment_reports_save_failure(
            "single_instance_lock_denied: /x/sessions.db.lock holder pid=1 cmd=pony since=1"
        ));
        assert!(
            !segment_reports_save_failure(
                "[pony-agent][session] commit_boundary_committed: label=save_to_backend session_count=1 elapsed_ms=3"
            ),
            "成功提交的日志不得被判成保存失败"
        );
    }

    #[test]
    fn witness_only_scans_bytes_written_after_capture() {
        let dir = std::env::temp_dir().join(format!(
            "pony-save-failure-witness-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let log = dir.join("tui-runtime.log");
        // 上一次运行留下的失败痕迹必须**不**算进本次。
        std::fs::write(&log, "旧记录：commit_boundary_failed（数据可能丢失）\n").expect("seed log");
        let witness = SaveFailureWitness::capture(Some(&log));
        assert!(
            !witness.observed_save_failure(),
            "启动前已有的旧痕迹不得算作本次运行失败"
        );
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&log)
            .expect("open log");
        writeln!(file, "本次：save_to_backend 失败：commit_boundary_failed（数据可能丢失）")
            .expect("append log");
        assert!(
            witness.observed_save_failure(),
            "本次运行新增的失败痕迹必须被识别"
        );
        let _ = std::fs::remove_file(&log);
        let _ = std::fs::remove_dir(&dir);
    }
}
