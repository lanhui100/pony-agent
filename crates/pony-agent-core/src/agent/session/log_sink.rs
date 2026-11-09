//! ADR 0099 阶段 1-B：把 `PersistCommand` 流**接线**到每会话日志文件（写入真源）。
//!
//! 语义（用户裁决：直接切写入，不做双写灰度）：
//!
//! - 写路径以**日志文件为真源**，SQLite 降为索引/缓存；
//! - 一批命令里按会话分组，逐会话追加命令信封 + `fsync`；
//! - 任何一步失败都返回 `Err`，由调用方直接判 `PersistCommandOutcome::Failed`，
//!   **SQLite 一行都不改**（绝不静默成功）。
//!
//! 路径约定沿用既有事实源：日志根目录 = 库文件所在目录下的 `session-logs/`
//! （与 `attachments/` 同款派生方式），文件位置由 [`SessionLogStore`] 决定：
//!
//! ```text
//! <db 所在目录>/session-logs/--<normalized-cwd>--/<encoded-session-id>/session.jsonl
//! ```
//!
//! cwd 分组段是**可读、可损失**的展示标签（见 `normalize_cwd_segment`）：唯一性由单射的
//! session id 路径段保证，故本模块用进程当前目录作为该标签，不同进程从不同目录启动
//! 也不会串会话（读取侧 `resolve_session_dir` 跨分组扫描即可找到）。
//!
//! ## 为什么每个会话持一把**长命**写者句柄
//!
//! 1. **独占锁只抢一次**：`open_write` 的锁（`File::try_lock`）与「扫已提交尾行」都是
//!    文件级代价；每条命令都重开重扫会把持久化放大成 O(文件长度)。缓存 `LogWriter`
//!    之后，一次 `persist_commands_batch` 只剩「写 + fsync」。
//! 2. **同进程并发不再互相误判**：独占锁的语义按「打开文件描述」计，同进程另开一个 fd
//!    也会互斥。缓存句柄把「同进程多线程写同一会话」变成对同一把句柄的串行写，
//!    `Locked` 因此只表示**真的另一个写者**（其它进程持有该会话）。
//! 3. **另一写者短暂占用时有限等待**：锁冲突按 2ms × 100 次（约 200ms）有界重试，
//!    吸收「另一个写者正在写」的瞬时窗口；预算耗尽仍返回 `Err`（不静默降级为写）。

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::Value;

use super::backend::PersistCommand;
use super::log_command::{command_session_id, command_to_value};
use super::log_store::{now_ms, LogStoreError, LogWriter, SessionLogMeta, SessionLogStore};

/// 会话日志根目录名（置于应用数据目录下，与库文件同级）。
pub const SESSION_LOG_ROOT_DIR_NAME: &str = "session-logs";

/// 抢锁重试预算：每次 2ms，共尝试 `1 + LOCK_RETRY_ATTEMPTS` 次（约 200ms 上限）。
const LOCK_RETRY_ATTEMPTS: u32 = 100;

/// 抢锁重试的退避间隔。
const LOCK_RETRY_SLEEP: std::time::Duration = std::time::Duration::from_millis(2);

/// 日志根目录：`<db 所在目录>/session-logs/`。
///
/// 与 `sqlite_session::build` 里 `attachment_root = db_path.parent().join("attachments")`
/// 同一条既有路径解析约定：库文件所在目录即应用数据目录（生产为
/// `dirs::data_local_dir()/PonyAgent/`），因此不硬编码任何绝对路径。
pub fn session_log_root_for_db(db_path: &Path) -> PathBuf {
    match db_path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(SESSION_LOG_ROOT_DIR_NAME),
        _ => PathBuf::from(SESSION_LOG_ROOT_DIR_NAME),
    }
}

/// 日志的 cwd 分组标签：进程当前目录（取不到则空串 ⇒ `--root--` 分组）。
pub fn current_log_cwd() -> String {
    std::env::current_dir()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// 某个库文件下、某会话的日志文件路径（读侧/测试/排障用的纯函数）。
///
/// session id 不安全（路径遍历等）时返回 `None`——与 `LogStoreError::InvalidSessionId`
/// 同一条拒绝口径，不做任何转义后放行。
pub fn session_log_path_for_db(db_path: &Path, session_id: &str) -> Option<PathBuf> {
    SessionLogStore::new(session_log_root_for_db(db_path))
        .log_path(session_id, &current_log_cwd())
        .ok()
}

/// 一次日志追加的回执（ADR 0107 P0.3）。
///
/// - `written`：本批是否**实际写入字节**（R5：零写入批不得被判 `LogPersisted`）；
/// - `restored`：每会话**追加前**的已提交长度——供「事务拒 stale-epoch 后回滚本批日志」
///   精确截断（一次批通常只涉及一个会话）。
#[derive(Debug, Default, Clone)]
pub struct LogAppendReceipt {
    pub written: bool,
    /// 每会话**追加前**长度（回滚要截回的目标）。
    pub restored: Vec<(String, u64)>,
    /// 每会话**追加后**长度（与 `restored` 同序）。回滚前用它做偏移校验：
    /// 文件若在「append 完成 → 回滚」之间被别人追加过，长度不等 ⇒ 放弃截断（绝不猜）。
    pub post_append_len: Vec<(String, u64)>,
}

/// 会话日志写入端：一个 `SqliteSessionBackend` 持有一个实例。
#[derive(Debug)]
pub struct SessionLogSink {
    store: SessionLogStore,
    cwd: String,
    /// 每会话一把长命写者句柄（独占锁持有到 backend drop 或该会话被删除）。
    ///
    /// 这把 `Mutex` 同时是进程内的写序列化闸门：同一批命令里的多个会话按 `BTreeMap`
    /// 顺序串行落盘（一次 `persist_commands_batch` 本身是单个 SQLite 事务，无需并行）。
    writers: Mutex<HashMap<String, LogWriter>>,
}

impl SessionLogSink {
    /// 以日志根目录与 cwd 分组标签构造。
    pub fn new(root: impl Into<PathBuf>, cwd: impl Into<String>) -> Self {
        Self {
            store: SessionLogStore::new(root),
            cwd: cwd.into(),
            writers: Mutex::new(HashMap::new()),
        }
    }

    /// 按库文件路径构造（根目录 = 库文件所在目录下的 `session-logs/`）。
    pub fn for_db(db_path: &Path) -> Self {
        Self::new(session_log_root_for_db(db_path), current_log_cwd())
    }

    /// 日志根目录。
    pub fn root(&self) -> &Path {
        self.store.root()
    }

    /// cwd 分组标签。
    pub fn cwd(&self) -> &str {
        &self.cwd
    }

    /// 某会话的日志文件路径（**仅当前 cwd 分组**；session id 不安全时 `None`）。
    ///
    /// 判「有没有日志」请用 [`Self::find_log_path`]：分组是创建时的 cwd 标签，跨工作区
    /// 会漏判。
    pub fn log_path(&self, session_id: &str) -> Option<PathBuf> {
        self.store.log_path(session_id, &self.cwd).ok()
    }

    /// **跨 cwd 分组**查找某会话的日志文件（判「有没有日志」的唯一正确入口）。
    pub fn find_log_path(&self, session_id: &str) -> Option<PathBuf> {
        self.store.find_log_path(session_id)
    }

    /// 跨分组查找某会话的摘要（迁移解析归属 cwd 用）。
    pub fn find_session_meta(&self, session_id: &str) -> Option<SessionLogMeta> {
        self.store.find_session_meta(session_id)
    }

    /// 某会话的日志目录（session id 不安全时 `None`）。
    pub fn session_dir(&self, session_id: &str) -> Option<PathBuf> {
        self.store.session_dir(session_id, &self.cwd).ok()
    }

    /// 追加一批持久化命令（真源写入）。
    ///
    /// - 按 `session_id` 分组，每会话一次 `fsync`；
    /// - `PublishMetadata` 无会话归属 ⇒ 不进日志（全局元数据，日志按会话分文件）；
    /// - `RemoveSession` ⇒ 不进日志：删除语义由 [`Self::delete_session`] 在 SQLite 提交后
    ///   执行（把事件写进正要被删掉的文件没有意义）；
    /// - 任一会话失败即整体 `Err`：调用方必须把整个批次判失败，不得部分成功；
    /// - 返回 [`LogAppendReceipt`]：`written` = 是否实际写入字节（P0.1 / R5：零写入批不得被判
    ///   `LogPersisted`）；`restored` = 每会话追加前长度（P0.3：供 stale-epoch 回滚截断）。
    pub fn append_commands(&self, commands: &[PersistCommand]) -> Result<LogAppendReceipt, String> {
        self.append_commands_with(commands, false)
    }

    /// ADR 0107 P0.1 / R3：**锁降级实例**按批开/放 writer——append 完立即 `drop`
    /// （释放文件级独占锁），不长期缓存句柄。
    ///
    /// 为什么必须区分：降级实例若长期持有同一会话的 writer 句柄，主（可写）实例写该会话会
    /// 被挡成 `Locked → append 失败 → 整批 Failed → 整轮回滚`，破坏「可写实例零回归」。
    /// 可写实例仍走 [`Self::append_commands`]（长命缓存句柄，避免每批重开重扫）。
    pub fn append_commands_releasing(
        &self,
        commands: &[PersistCommand],
    ) -> Result<LogAppendReceipt, String> {
        self.append_commands_with(commands, true)
    }

    /// 公共实现：`release` = 按批开/放 writer（不复用缓存句柄）。
    fn append_commands_with(
        &self,
        commands: &[PersistCommand],
        release: bool,
    ) -> Result<LogAppendReceipt, String> {
        let mut grouped: BTreeMap<&str, Vec<Value>> = BTreeMap::new();
        for command in commands {
            if matches!(command, PersistCommand::RemoveSession { .. }) {
                continue;
            }
            let Some(session_id) = command_session_id(command) else {
                continue;
            };
            let value = command_to_value(command)
                .map_err(|error| format!("encode session log command for `{session_id}`: {error}"))?;
            grouped.entry(session_id).or_default().push(value);
        }
        if grouped.is_empty() {
            return Ok(LogAppendReceipt::default());
        }
        let mut writers = self
            .writers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let ts = now_ms();
        let mut receipt = LogAppendReceipt::default();
        for (session_id, values) in grouped {
            if release {
                // 先丢弃可能存在的缓存句柄（释放锁），再按批开/放。
                writers.remove(session_id);
                let meta = SessionLogMeta::new(session_id, self.cwd.clone());
                let mut writer = self.open_writer(session_id, &meta)?;
                // 追加前长度：回滚时按它 set_len（含 header / 已提交命令）。
                let pre_append_len = writer.committed_len();
                writer
                    .append_commands(ts, &values)
                    .map_err(|error| format!("append session log for `{session_id}`: {error}"))?;
                receipt.restored.push((session_id.to_string(), pre_append_len));
                receipt
                    .post_append_len
                    .push((session_id.to_string(), writer.committed_len()));
                drop(writer); // 立即释放 flock（不缓存）
                continue;
            }
            if !writers.contains_key(session_id) {
                let meta = SessionLogMeta::new(session_id, self.cwd.clone());
                let writer = self.open_writer(session_id, &meta)?;
                writers.insert(session_id.to_string(), writer);
            }
            let writer = writers
                .get_mut(session_id)
                .expect("writer inserted in the same iteration");
            let pre_append_len = writer.committed_len();
            if let Err(error) = writer.append_commands(ts, &values) {
                // 写失败即丢弃缓存句柄（下次重新拿锁 + 重扫撕裂尾行），并整体失败。
                writers.remove(session_id);
                return Err(format!("append session log for `{session_id}`: {error}"));
            }
            receipt.restored.push((session_id.to_string(), pre_append_len));
            receipt
                .post_append_len
                .push((session_id.to_string(), writer.committed_len()));
        }
        receipt.written = true;
        Ok(receipt)
    }

    /// ADR 0107 P0.3：回滚一批**已写入**的日志（stale-epoch 批不得留在真源里）。
    ///
    /// 对每个 `(session_id, pre_append_len)`：先丢弃缓存 writer，再走 [`Self::open_writer`]
    /// **重新取得该会话的日志独占锁**（`try_lock_exclusive`），持锁后调用
    /// [`LogWriter::rollback_to`]（`set_len` + `sync_all` + 重扫校正 `committed_len`/`next_seq`）。
    /// 抢不到锁 ⇒ 打 `stale_epoch_log_rollback_failed` 并跳过（不重试、绝不无锁截断）。
    /// 回滚句柄不缓存：drop 即释放 flock，后续 append 重新 open + scan。
    pub fn rollback(&self, restored: &[(String, u64)], post_append_len: &[(String, u64)]) {
        let expected_by_session: HashMap<&str, u64> = post_append_len
            .iter()
            .map(|(session_id, len)| (session_id.as_str(), *len))
            .collect();
        for (session_id, pre_append_len) in restored {
            // 先丢弃缓存 writer（否则自己会挡自己的锁），再**走 open_writer 重新抢独占锁**：
            // 截断必须在持锁前提下做——无锁 `set_len` 会截掉并发写者（另一降级实例）的字节。
            if let Ok(mut writers) = self.writers.lock() {
                writers.remove(session_id.as_str());
            }
            let meta = SessionLogMeta::new(session_id, self.cwd.clone());
            let mut writer = match self.open_writer(session_id, &meta) {
                Ok(writer) => writer,
                Err(error) => {
                    eprintln!(
                        "[pony-agent][session] stale_epoch_log_rollback_failed: session={session_id}（抢不到日志独占锁，未截断）：{error}"
                    );
                    continue;
                }
            };
            let Some(expected) = expected_by_session.get(session_id.as_str()).copied() else {
                eprintln!(
                    "[pony-agent][session] stale_epoch_log_rollback_skipped: session={session_id} expected=? actual=?（回执缺 post_append_len，不猜、不截断）"
                );
                continue;
            };
            match writer.rollback_to(*pre_append_len, expected) {
                Ok(()) => {}
                Err(LogStoreError::RollbackStaleOffset { expected, actual }) => {
                    eprintln!(
                        "[pony-agent][session] stale_epoch_log_rollback_skipped: session={session_id} expected={expected} actual={actual}（追加后文件又变长，偏移已过期；不猜、不截断，stale 批残留在日志中）"
                    );
                    continue;
                }
                Err(error) => {
                    eprintln!(
                        "[pony-agent][session] stale_epoch_log_rollback_failed: session={session_id} path={} pre_append_len={pre_append_len}：{error}",
                        writer.path().display()
                    );
                    continue;
                }
            }
            // 不缓存该句柄：drop ⇒ 释放 flock，后续 append 重新 open + scan（seq 以重扫为准）。
            drop(writer);
        }
    }

    /// 删除某会话的整个日志目录（已在 SQLite 提交之后调用；幂等）。
    ///
    /// 先丢弃缓存句柄（drop ⇒ 释放独占锁），再删目录；否则自己的写者句柄会把删除挡成
    /// `Locked`。失败只返回 `Err` 供调用方记日志——「先删 SQLite 行、再删目录」的顺序
    /// 保证最坏只剩「索引已删、日志目录还在」这一种半态。
    pub fn delete_session(&self, session_id: &str) -> Result<(), String> {
        let mut writers = self
            .writers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        writers.remove(session_id);
        self.store
            .delete(session_id)
            .map_err(|error| format!("delete session log for `{session_id}`: {error}"))
    }

    /// 打开（必要时创建）某会话的写者句柄，锁冲突按有界预算重试。
    fn open_writer(&self, session_id: &str, meta: &SessionLogMeta) -> Result<LogWriter, String> {
        let mut attempt = 0u32;
        loop {
            match self.store.open_write(session_id, meta) {
                Ok(writer) => return Ok(writer),
                Err(LogStoreError::Locked { .. }) if attempt < LOCK_RETRY_ATTEMPTS => {
                    attempt += 1;
                    std::thread::sleep(LOCK_RETRY_SLEEP);
                }
                Err(error) => {
                    return Err(format!("open session log for `{session_id}`: {error}"));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        let unique = format!(
            "pony-0099-1b-sink-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        );
        let path = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    fn flush_command(session_id: &str) -> PersistCommand {
        PersistCommand::FlushEvents {
            epoch: 1,
            session_id: session_id.to_string(),
            turn_id: "t".to_string(),
            branch_id: "main".to_string(),
            events: Vec::new(),
        }
    }

    #[test]
    fn log_root_is_derived_next_to_the_database_file() {
        let root = session_log_root_for_db(Path::new("/data/PonyAgent/sessions.db"));
        assert_eq!(root, PathBuf::from("/data/PonyAgent/session-logs"));
        // 无父目录时退化为相对根，不 panic、不硬编码绝对路径。
        assert_eq!(
            session_log_root_for_db(Path::new("sessions.db")),
            PathBuf::from(SESSION_LOG_ROOT_DIR_NAME)
        );
    }

    #[test]
    fn append_skips_commands_without_a_session_and_deletes_afterwards() {
        let dir = temp_dir("skip");
        let db = dir.join("sessions.db");
        let sink = SessionLogSink::for_db(&db);
        let command = PersistCommand::PublishMetadata {
            epoch: 1,
            key: "k".to_string(),
            value: "v".to_string(),
        };
        // R5：零写入批必须回报 `written=false`（否则上层会把索引失败误判成 `LogPersisted`）。
        let receipt = sink.append_commands(&[command]).expect("no-op append");
        assert!(!receipt.written, "无会话归属的命令 ⇒ 零字节写入");
        assert!(receipt.restored.is_empty(), "零写入批不得记录回滚目标");
        assert!(receipt.post_append_len.is_empty(), "零写入批不得记录追加后长度");
        assert!(
            !sink.root().exists(),
            "no session-scoped command ⇒ 不创建任何日志目录"
        );
        // 幂等：从未存在过的会话删除不报错。
        sink.delete_session("pa_never").expect("idempotent delete");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn append_message_command(session_id: &str, content: &str, ordinal: usize) -> PersistCommand {
        PersistCommand::AppendMessage {
            epoch: 1,
            session_id: session_id.to_string(),
            message: crate::agent::session::TurnHistoryMessage {
                role: "user".to_string(),
                content: content.to_string(),
                ..Default::default()
            },
            ordinal,
        }
    }

    /// ADR 0107 P0.3：事务拒 stale-epoch 后，按回执把本批日志**回滚**——文件长度回到追加前、
    /// 后续 append 的 seq 连续、fold 读出的历史不含被回滚消息。
    #[test]
    fn rollback_removes_the_stale_batch_from_the_log() {
        let dir = temp_dir("rollback");
        let sink = SessionLogSink::for_db(&dir.join("sessions.db"));
        let session_id = "pa_rb";

        let first = sink
            .append_commands(&[append_message_command(session_id, "第一问", 0)])
            .expect("first append");
        assert!(first.written, "第一批必须写入字节");
        let path = sink.log_path(session_id).expect("log path");
        let len_after_first = std::fs::metadata(&path).expect("metadata").len();
        assert!(len_after_first > 0);

        // 第二批模拟「事务拒 stale-epoch」：先写进日志，随后按回执回滚。
        let stale = sink
            .append_commands(&[append_message_command(session_id, "被拒消息", 1)])
            .expect("stale append");
        assert!(stale.written, "第二批确实写过字节（否则回滚无从谈起）");
        assert_eq!(stale.restored.len(), 1, "回执必须记录一个会话的回滚目标");
        assert_eq!(stale.restored[0].0, session_id);
        assert_eq!(
            stale.restored[0].1, len_after_first,
            "回执里的 pre_append_len 必须是追加前的已提交长度"
        );
        assert!(
            std::fs::metadata(&path).expect("metadata").len() > len_after_first,
            "回滚前文件必须已变长（否则本用例证明不了回滚）"
        );

        assert_eq!(
            stale.post_append_len.len(),
            1,
            "回执必须记录一个会话的追加后长度（偏移校验用）"
        );
        sink.rollback(&stale.restored, &stale.post_append_len);
        assert_eq!(
            std::fs::metadata(&path).expect("metadata").len(),
            len_after_first,
            "rollback 必须把日志截回追加前长度"
        );

        // 回滚后继续 append：seq 连续（无空洞/重复），fold 读出的历史不含被回滚消息。
        let third = sink
            .append_commands(&[append_message_command(session_id, "回滚后的新消息", 2)])
            .expect("append after rollback");
        assert!(third.written);

        let raw = std::fs::read_to_string(&path).expect("read log");
        let mut seqs: Vec<u64> = Vec::new();
        for line in raw.lines().filter(|line| !line.trim().is_empty()) {
            let value: Value = serde_json::from_str(line).expect("log line is json");
            if let Some(seq) = value.get("seq").and_then(Value::as_u64) {
                seqs.push(seq);
            }
        }
        assert!(!seqs.is_empty(), "回滚后日志必须还有命令行");
        for pair in seqs.windows(2) {
            assert_eq!(pair[1], pair[0] + 1, "回滚后 seq 必须连续：{seqs:?}");
        }
        assert!(!raw.contains("被拒消息"), "被回滚的消息不得残留在日志里");

        let mut sessions = HashMap::new();
        crate::agent::session::fold::merge_log_truth_into_sessions(sink.root(), &mut sessions);
        let history: Vec<String> = sessions
            .get(session_id)
            .expect("folded session")
            .history
            .iter()
            .map(|entry| entry.content.clone())
            .collect();
        assert!(
            history.iter().any(|content| content == "第一问"),
            "fold 必须保留第一批消息：{history:?}"
        );
        assert!(
            history.iter().any(|content| content == "回滚后的新消息"),
            "fold 必须包含回滚后的新消息：{history:?}"
        );
        assert!(
            !history.iter().any(|content| content.contains("被拒消息")),
            "fold 不得读回被回滚的消息：{history:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ADR 0107 P0.5：回滚偏移过期时**必须跳过**——「append 完成 → 回滚」之间别人追加了字节，
    /// 此时按旧偏移 `set_len` 会截掉他人写入，并让对方 writer 的 `committed_len` 指向文件外。
    #[test]
    fn rollback_is_skipped_when_the_file_grew_after_append() {
        let dir = temp_dir("rollback-stale-offset");
        let sink = SessionLogSink::for_db(&dir.join("sessions.db"));
        let session_id = "pa_rb_stale_offset";

        // 用 releasing 路径（append 完即释放 flock），模拟降级实例：窗口内别人可以追加。
        sink.append_commands_releasing(&[append_message_command(session_id, "偏移-第一问", 0)])
            .expect("first append");
        let stale = sink
            .append_commands_releasing(&[append_message_command(session_id, "偏移-被拒消息", 1)])
            .expect("stale append");
        assert!(stale.written);
        assert_eq!(stale.post_append_len.len(), 1);
        assert_eq!(stale.post_append_len[0].0, session_id);
        let post_len = stale.post_append_len[0].1;

        // 另一句柄在「append 完成 → 回滚」窗口里追加了字节。
        let other = SessionLogStore::new(sink.root());
        let meta = SessionLogMeta::new(session_id, current_log_cwd());
        {
            let mut writer = other.open_write(session_id, &meta).expect("other writer");
            writer
                .append_commands(
                    now_ms(),
                    &[command_to_value(&append_message_command(session_id, "偏移-他人字节", 2))
                        .expect("encode")],
                )
                .expect("other append");
        }
        let path = sink.log_path(session_id).expect("log path");
        let len_with_foreign = std::fs::metadata(&path).expect("metadata").len();
        assert!(
            len_with_foreign > post_len,
            "前置：他人追加后文件必须长于回执的 post_append_len（{len_with_foreign} vs {post_len}）"
        );
        let before = std::fs::read(&path).expect("read log");

        // 回滚：偏移已过期 ⇒ 必须跳过（不猜、不截断）。
        sink.rollback(&stale.restored, &stale.post_append_len);

        let after = std::fs::read(&path).expect("read log");
        assert_eq!(
            before, after,
            "偏移过期时回滚必须跳过：绝不截掉他人字节（{} → {} 字节）",
            before.len(),
            after.len()
        );
        assert!(
            String::from_utf8_lossy(&after).contains("偏移-他人字节"),
            "他人写入的字节必须完整保留"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ADR 0107 P0.3：回滚必须**持该会话日志独占锁**（绝不无锁 `set_len`）。
    ///
    /// 同进程两个 `LogStore` 句柄 = 两个独立 open file description ⇒ flock 相互生效，
    /// 正好可以验证锁语义：写者持锁期间，另一个句柄 `open_write` 必须拿到 `Locked`；
    /// 回滚结束后长度/seq 一致，另一句柄也能正常接手写入（seq 连续）。
    #[test]
    fn rollback_takes_the_session_log_exclusive_lock() {
        let dir = temp_dir("rollback-lock");
        let db = dir.join("sessions.db");
        let sink = SessionLogSink::for_db(&db);
        let session_id = "pa_rb_lock";

        sink.append_commands(&[append_message_command(session_id, "锁语义-第一问", 0)])
            .expect("first append");
        let len_after_first = sink
            .log_path(session_id)
            .and_then(|path| std::fs::metadata(path).ok())
            .map(|meta| meta.len())
            .expect("log metadata");

        let stale = sink
            .append_commands(&[append_message_command(session_id, "锁语义-被拒消息", 1)])
            .expect("stale append");
        assert!(stale.written);

        // ① 锁语义：另一个句柄（不同 open file description）拿不到同一会话的写者。
        let other_store = SessionLogStore::new(sink.root());
        let meta = SessionLogMeta::new(session_id, current_log_cwd());
        match other_store.open_write(session_id, &meta) {
            Err(LogStoreError::Locked { .. }) => {}
            other => panic!(
                "flock 未生效：写者持锁期间另一句柄必须 Locked，实际 {other:?}"
            ),
        }

        // ② 回滚（内部重新抢锁 + 偏移校验 + set_len + 重扫）。
        sink.rollback(&stale.restored, &stale.post_append_len);
        let path = sink.log_path(session_id).expect("log path");
        assert_eq!(
            std::fs::metadata(&path).expect("metadata").len(),
            len_after_first,
            "回滚后文件长度必须回到追加前"
        );

        // ③ 回滚句柄已释放锁 ⇒ 另一句柄能接手写入，且 seq 与被截断前缀连续。
        let mut other_writer = other_store
            .open_write(session_id, &meta)
            .expect("rollback 后必须能重新取得写者（锁已释放）");
        assert_eq!(
            other_writer.committed_len(),
            len_after_first,
            "重新 open 时 committed_len 必须与回滚后的文件长度一致"
        );
        let seqs = other_writer
            .append_commands(now_ms(), &[command_to_value(&append_message_command(session_id, "锁语义-接手消息", 2)).expect("encode")])
            .expect("append after rollback");
        assert_eq!(seqs.len(), 1);
        drop(other_writer);

        let raw = std::fs::read_to_string(&path).expect("read log");
        let seq_numbers: Vec<u64> = raw
            .lines()
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter_map(|value| value.get("seq").and_then(Value::as_u64))
            .collect();
        for pair in seq_numbers.windows(2) {
            assert_eq!(pair[1], pair[0] + 1, "seq 必须连续：{seq_numbers:?}");
        }
        assert!(
            !raw.contains("锁语义-被拒消息"),
            "被回滚的消息不得残留在日志里"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unsafe_session_id_is_rejected_not_escaped() {
        let dir = temp_dir("unsafe");
        let sink = SessionLogSink::for_db(&dir.join("sessions.db"));
        assert!(sink.log_path("../evil").is_none());
        assert!(sink.session_dir("../evil").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn concurrent_appends_through_one_sink_serialize_on_the_same_session() {
        use std::sync::Arc;
        let dir = temp_dir("concurrent");
        let sink = Arc::new(SessionLogSink::for_db(&dir.join("sessions.db")));
        let mut handles = Vec::new();
        for thread_id in 0..4u32 {
            let sink = Arc::clone(&sink);
            let session_id = format!("pa_s{thread_id}");
            handles.push(std::thread::spawn(move || {
                for _ in 0..25u32 {
                    sink.append_commands(&[flush_command(&session_id)])
                        .expect("per-session append must not fail");
                    // 同一会话被多线程写入：长命句柄串行化，不得报 Locked。
                    sink.append_commands(&[flush_command("pa_shared")])
                        .expect("same-session append must be serialized by the cached writer");
                }
            }));
        }
        for handle in handles {
            handle.join().expect("thread join");
        }
        // 共享会话日志：4 线程 × 25 次 = 100 条命令，seq 必须严格连续。
        let path = sink.log_path("pa_shared").expect("log path");
        let raw = std::fs::read_to_string(&path).expect("read shared log");
        let seqs: Vec<u64> = raw
            .lines()
            .skip(1)
            .map(|line| {
                serde_json::from_str::<serde_json::Value>(line).expect("json line")["seq"]
                    .as_u64()
                    .expect("seq")
            })
            .collect();
        assert_eq!(seqs, (0..100u64).collect::<Vec<_>>());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_session_evicts_the_cached_writer_so_the_directory_can_go() {
        let dir = temp_dir("delete");
        let sink = SessionLogSink::for_db(&dir.join("sessions.db"));
        sink.append_commands(&[flush_command("pa_del")])
            .expect("append");
        let session_dir = sink.session_dir("pa_del").expect("session dir");
        assert!(session_dir.is_dir());
        sink.delete_session("pa_del").expect("delete after evict");
        assert!(
            !session_dir.exists(),
            "删除后目录必须消失（不能被自己的写者锁住）"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
