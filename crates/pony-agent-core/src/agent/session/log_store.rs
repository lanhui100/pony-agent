//! ADR 0099 **阶段 1-A/1-B**：**每会话一份 append-only 日志文件**（写入真源）。
//!
//! 阶段 1-A 交付本模块（未接线）；阶段 1-B 起由 [`super::log_sink::SessionLogSink`]
//! 接线到 `SqliteSessionBackend::persist_commands_batch`：**日志先落、SQLite 后写**。
//!
//! 契约（ADR 0099 修订后的 D1 / D2 / 阶段 1）：
//!
//! ```text
//! <root>/--<normalized-cwd>--/<encoded-session-id>/session.jsonl
//! ```
//!
//! 1. **单写者**：文件级独占锁统一走 `std::fs::File::try_lock`（跨平台：Unix 即
//!    `flock(LOCK_EX|LOCK_NB)`，Windows 即 `LockFileEx`）。**只有一条代码路径**：
//!    不存在「非 Unix 分支未编译验证」的死角，也不再直接调用 `libc`。
//!    抢不到锁 ⇒ [`LogStoreError::Locked`]，**绝不静默降级为写**。
//! 2. **append-only**：每批 `append` = 追加若干 JSONL 行 + `fsync`；失败回滚到
//!    先前字节长度（`set_len`）；已提交事件永不重写。
//! 3. **seq 单调**：`seq` 从 0 起连续，只由该文件的唯一写者分配。
//! 4. **header 自描述**：首行 `{"type":"session","version":1,"id":…,"cwd":…,"createdAt":…}`；
//!    [`read_header`] 返回四态 [`HeaderClass`]，取代「版本不等即拒」的布尔拒绝。
//! 5. **撕裂尾行**：读取遇到无法解析的尾行即**丢弃该行及之后**，保留已提交前缀，
//!    不 panic、不报错（崩溃只丢尾行是设计行为）。`open_write` 会在拿锁后把撕裂尾行
//!    截断掉，保证后续 append 从行边界继续。
//! 6. **原子创建**：目录 `create_dir_all`、文件 `create_new`；`id` 单射转义为安全
//!    路径段，`../` / 绝对路径 / `..` / 盘符 / 控制字符一律**拒绝**（不是转义后放行）。
//! 7. **两种已提交行形态**（阶段 1-B 新增第二种）：
//!    - 通用事件 [`SessionLogEvent`]：`{"seq","type","ts","payload"}`；
//!    - 持久化命令信封 **`{"seq","ts","cmd"}`**（[`LogWriter::append_commands`] 写，
//!      `cmd` 是序列化后的 `PersistCommand`）——写入真源实际用的是这一种。
//!    判「已提交」只看行内 `seq`（[`SeqProbe`]），两种形态因此可以共存且 `seq`
//!    严格连续、单调追加。
//!
//! 本期不做（阶段后续）：`manifest.json`、附件、zstd 帧、远程 HTTP 实现、fold 投影。

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// 当前实现的会话日志格式版本。
///
/// header 里的 `version` 与之比较即得三态：低于 ⇒ `migration-required`，
/// 等于 ⇒ `current`，高于 ⇒ `unsupported`。
pub const SESSION_LOG_FORMAT_VERSION: u64 = 1;

/// 会话日志文件名（当前代际）。
pub const SESSION_LOG_FILE_NAME: &str = "session.jsonl";

/// 转义后路径段的字节上限（保证单段远小于主流文件系统的 255 字节限制）。
const MAX_SEGMENT_BYTES: usize = 200;

/// 会话 id 的原始字符上限（转义最坏膨胀 3 倍，仍远小于 [`MAX_SEGMENT_BYTES`]）。
const MAX_SESSION_ID_CHARS: usize = 128;

/// 项目目录前缀/后缀标记：`--<normalized-cwd>--`。
const CWD_DIR_MARKER: &str = "--";

/// 转义引导符。正常字面量集合不含 `~`，故转义输出与字面量不会混淆（保证单射）。
const ESCAPE_PREFIX: char = '~';

// ---------------------------------------------------------------------------
// 错误类型
// ---------------------------------------------------------------------------

/// 会话日志存储错误。
///
/// 每个变体都是**显式失败**（fail loud）；没有任何一只「静默降级为写」的旁路。
#[derive(Debug)]
pub enum LogStoreError {
    /// session id 不能安全地转义为单个路径段（路径遍历 / 绝对路径 / 控制字符等）。
    InvalidSessionId { session_id: String, reason: String },
    /// 会话目录或 `session.jsonl` 不存在。
    SessionNotFound { session_id: String },
    /// 同一个 session id 出现在多个 cwd 分组目录下，无法判定读哪个（不猜）。
    SessionDirAmbiguous { session_id: String, count: usize },
    /// header 版本低于当前实现：需要迁移后才能读写。
    HeaderMigrationRequired { found: u64, current: u64 },
    /// header 版本高于当前实现：本二进制不认识，拒绝打开。
    HeaderUnsupported { found: u64, current: u64 },
    /// header JSON / 结构损坏，无法分类。
    HeaderMalformed { reason: String },
    /// 文件级独占锁已被他人持有（另一 writer 正在写该会话）。
    Locked { path: PathBuf },
    /// 底层 IO 失败。
    Io { context: String, source: std::io::Error },
    /// 事件序列化失败。
    Serialize { reason: String },
    /// 回滚目标偏移已过期：文件在「append 完成 → 回滚」之间被别人追加过字节
    /// （`expected` = 回执记录的追加后长度，`actual` = 抢到锁后看到的长度）。
    /// 语义：**不猜、不截断**（否则会截掉他人字节），调用方按可观测残留处理。
    RollbackStaleOffset { expected: u64, actual: u64 },
}

impl std::fmt::Display for LogStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSessionId { session_id, reason } => {
                write!(f, "invalid session id `{session_id}`: {reason}")
            }
            Self::SessionNotFound { session_id } => {
                write!(f, "session log not found for `{session_id}`")
            }
            Self::SessionDirAmbiguous { session_id, count } => write!(
                f,
                "session id `{session_id}` is ambiguous: found in {count} cwd groups"
            ),
            Self::HeaderMigrationRequired { found, current } => write!(
                f,
                "session log header version {found} requires migration (current {current})"
            ),
            Self::HeaderUnsupported { found, current } => write!(
                f,
                "session log header version {found} is unsupported (current {current})"
            ),
            Self::HeaderMalformed { reason } => {
                write!(f, "malformed session log header: {reason}")
            }
            Self::Locked { path } => {
                write!(f, "session log is locked by another writer: `{}`", path.display())
            }
            Self::Io { context, source } => write!(f, "{context}: {source}"),
            Self::Serialize { reason } => write!(f, "failed to encode session log event: {reason}"),
            Self::RollbackStaleOffset { expected, actual } => write!(
                f,
                "rollback offset is stale: expected file length {expected}, actual {actual}"
            ),
        }
    }
}

impl std::error::Error for LogStoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

fn io_err(context: impl Into<String>, source: std::io::Error) -> LogStoreError {
    LogStoreError::Io {
        context: context.into(),
        source,
    }
}

// ---------------------------------------------------------------------------
// header 四态
// ---------------------------------------------------------------------------

/// 会话日志 header（首行自描述块）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionLogHeader {
    /// 文件格式版本（与 [`SESSION_LOG_FORMAT_VERSION`] 比较）。
    pub version: u64,
    /// 会话 id（原始未转义形态）。
    pub session_id: String,
    /// 会话创建时的 cwd（原始形态）。
    pub cwd: String,
    /// 创建时间，Unix 毫秒。
    pub created_at: i64,
}

/// header 分类结果：**四态**，不是布尔化拒绝。
///
/// * [`HeaderClass::Current`] —— 版本等于当前实现，可读写。
/// * [`HeaderClass::MigrationRequired`] —— 版本低于当前实现（旧格式），需先迁移。
/// * [`HeaderClass::Unsupported`] —— 版本高于当前实现（新二进制写的文件），本进程不认识。
/// * [`HeaderClass::Malformed`] —— JSON / 结构坏（非 JSON、非 object、`type` 不是
///   `"session"`、`version` 不是整数、缺 id/cwd/createdAt）。**不可比较的版本号**无从
///   判断「高/低」，因此归入 malformed 而不是硬塞成 unsupported。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HeaderClass {
    Current(SessionLogHeader),
    MigrationRequired(SessionLogHeader),
    Unsupported(SessionLogHeader),
    Malformed { reason: String },
}

impl HeaderClass {
    /// 是否可直接读写。
    pub fn is_current(&self) -> bool {
        matches!(self, Self::Current(_))
    }

    /// 可取出的 header（`Malformed` 无 header）。
    pub fn header(&self) -> Option<&SessionLogHeader> {
        match self {
            Self::Current(h) | Self::MigrationRequired(h) | Self::Unsupported(h) => Some(h),
            Self::Malformed { .. } => None,
        }
    }

    /// 四态的可读标签，便于日志与错误信息。
    pub fn label(&self) -> &'static str {
        match self {
            Self::Current(_) => "current",
            Self::MigrationRequired(_) => "migration-required",
            Self::Unsupported(_) => "unsupported",
            Self::Malformed { .. } => "malformed",
        }
    }
}

/// header 序列化形态（`type` / `createdAt` 为 JSON 侧命名）。
#[derive(Serialize)]
struct HeaderLine<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
    version: u64,
    id: &'a str,
    cwd: &'a str,
    #[serde(rename = "createdAt")]
    created_at: i64,
    /// 迁移来源自描述（ADR 0099 阶段 3-B）。**正常写入路径不写**（`skip_serializing_if`），
    /// 只有「从 blob 合成的日志」才带 ⇒ 合成流永不伪装成原始过程流。
    #[serde(default, rename = "source", skip_serializing_if = "Option::is_none")]
    source: Option<&'a SessionLogSource>,
}

/// 会话日志的**来源自描述**（日志 header 的可选 `source` 字段）。
///
/// 契约（ADR 0099 阶段 3-B / 方案 A）：
/// - 只有「从存量 blob 合成的日志」写它，`kind = "migrated-from-blob"`；
/// - 正常写入路径（`SessionLogSink`）**不写**该字段（保持 `None`）——合成流不得伪装成
///   原始过程流，读方/审计方据此可区分「真实过程事件」与「按状态反推的事件」；
/// - 字段可选且向后兼容：旧读方（含本仓 `classify_header_line`）遇到未知键**忽略**，
///   四态分类不变。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionLogSource {
    /// 来源类型；目前只有 `migrated-from-blob`。
    pub kind: String,
    /// 合成时刻（Unix 毫秒）。
    #[serde(default)]
    pub migrated_at_ms: Option<i64>,
    /// 合成方应用版本。
    #[serde(default)]
    pub app_version: Option<String>,
}

impl SessionLogSource {
    /// `source.kind` 的唯一取值：由存量 blob 合成。
    pub const KIND_MIGRATED_FROM_BLOB: &'static str = "migrated-from-blob";

    /// 「从 blob 迁移」来源标记。
    pub fn migrated_from_blob(app_version: impl Into<String>, migrated_at_ms: i64) -> Self {
        Self {
            kind: Self::KIND_MIGRATED_FROM_BLOB.to_string(),
            migrated_at_ms: Some(migrated_at_ms),
            app_version: Some(app_version.into()),
        }
    }

    /// 是否是从 blob 合成的日志。
    pub fn is_migrated_from_blob(&self) -> bool {
        self.kind == Self::KIND_MIGRATED_FROM_BLOB
    }
}

/// 读取指定文件的 header 并分类。
///
/// IO 失败（文件不存在 / 无权限）返回 `Err(Io)`——四态枚举只表达「文件读得到但格式
/// 有话说」的四种情况，不把 IO 失败伪装成格式问题。
pub fn read_header(path: &Path) -> Result<HeaderClass, LogStoreError> {
    let file = File::open(path).map_err(|e| io_err(format!("open `{}`", path.display()), e))?;
    let mut line = String::new();
    BufReader::new(file)
        .read_line(&mut line)
        .map_err(|e| io_err(format!("read header of `{}`", path.display()), e))?;
    Ok(classify_header_line(&line))
}

/// 读日志 header 里的**来源自描述**（`source` 字段）。
///
/// 与 [`read_header`] 互补而不是替代：`SessionLogHeader` 只承载「会话身份 + 格式版本」
/// 这层必需信息（加字段会破坏仓库内其它模块的结构体字面量构造），来源标记是**可选元数据**，
/// 由本函数单独读出 ⇒ 旧读方零影响、四态分类不变（`read_header` 仍按 version 四态判定）。
///
/// - 文件缺失 / IO 失败 ⇒ `Err`；
/// - header 可读但无 `source` 键（正常写入路径 / 旧文件）⇒ `Ok(None)`；
/// - `source` 存在但结构坏 ⇒ `Ok(None)`＋不静默：调用方若要强判可用
///   [`read_header_source_strict`]。
pub fn read_header_source(path: &Path) -> Result<Option<SessionLogSource>, LogStoreError> {
    let file = File::open(path).map_err(|e| io_err(format!("open `{}`", path.display()), e))?;
    let mut line = String::new();
    BufReader::new(file)
        .read_line(&mut line)
        .map_err(|e| io_err(format!("read header of `{}`", path.display()), e))?;
    Ok(source_from_header_line(&line))
}

/// 与 [`read_header_source`] 同源，但 `source` 结构坏时返回 `Err`（审计/排查入口用）。
pub fn read_header_source_strict(path: &Path) -> Result<Option<SessionLogSource>, LogStoreError> {
    let file = File::open(path).map_err(|e| io_err(format!("open `{}`", path.display()), e))?;
    let mut line = String::new();
    BufReader::new(file)
        .read_line(&mut line)
        .map_err(|e| io_err(format!("read header of `{}`", path.display()), e))?;
    let value: serde_json::Value =
        serde_json::from_str(line.trim()).map_err(|e| LogStoreError::Serialize {
            reason: format!("header is not valid JSON: {e}"),
        })?;
    match value.get("source") {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(raw) => serde_json::from_value(raw.clone())
            .map(Some)
            .map_err(|e| LogStoreError::Serialize {
                reason: format!("header `source` is malformed: {e}"),
            }),
    }
}

/// 从 header 首行文本里取 `source`（坏 JSON / 坏结构 ⇒ `None`）。
fn source_from_header_line(line: &str) -> Option<SessionLogSource> {
    let value: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    let raw = value.get("source")?;
    if raw.is_null() {
        return None;
    }
    serde_json::from_value(raw.clone()).ok()
}

/// 把首行文本分类为四态之一。
fn classify_header_line(line: &str) -> HeaderClass {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return HeaderClass::Malformed {
            reason: "missing header line".to_string(),
        };
    }
    let value: serde_json::Value = match serde_json::from_str(trimmed) {
        Ok(value) => value,
        Err(err) => {
            return HeaderClass::Malformed {
                reason: format!("header is not valid JSON: {err}"),
            }
        }
    };
    if !value.is_object() {
        return HeaderClass::Malformed {
            reason: "header is not a JSON object".to_string(),
        };
    }
    match value.get("type").and_then(serde_json::Value::as_str) {
        Some("session") => {}
        other => {
            return HeaderClass::Malformed {
                reason: format!("header `type` must be \"session\", got {other:?}"),
            }
        }
    }
    let Some(version) = value.get("version").and_then(serde_json::Value::as_u64) else {
        return HeaderClass::Malformed {
            reason: "header `version` is missing or not an unsigned integer".to_string(),
        };
    };
    let (Some(id), Some(cwd), Some(created_at)) = (
        value.get("id").and_then(serde_json::Value::as_str),
        value.get("cwd").and_then(serde_json::Value::as_str),
        value.get("createdAt").and_then(serde_json::Value::as_i64),
    ) else {
        return HeaderClass::Malformed {
            reason: "header must carry string `id`, string `cwd` and integer `createdAt`"
                .to_string(),
        };
    };
    let header = SessionLogHeader {
        version,
        session_id: id.to_string(),
        cwd: cwd.to_string(),
        created_at,
    };
    match version.cmp(&SESSION_LOG_FORMAT_VERSION) {
        std::cmp::Ordering::Equal => HeaderClass::Current(header),
        std::cmp::Ordering::Less => HeaderClass::MigrationRequired(header),
        std::cmp::Ordering::Greater => HeaderClass::Unsupported(header),
    }
}

// ---------------------------------------------------------------------------
// 事件模型
// ---------------------------------------------------------------------------

/// 日志中的一条事件（已分配 `seq`）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionLogEvent {
    /// 单调递增序号，从 0 开始，由该文件唯一写者分配。
    pub seq: u64,
    /// 事件类型（JSON 字段名 `type`）。
    #[serde(rename = "type")]
    pub kind: String,
    /// 事件时间戳，Unix 毫秒。
    #[serde(default)]
    pub ts: i64,
    /// 事件载荷（不透明 JSON）。
    #[serde(default)]
    pub payload: serde_json::Value,
}

/// 调用方提供的、尚未分配 `seq` 的事件。
#[derive(Clone, Debug, PartialEq)]
pub struct NewSessionLogEvent {
    /// 事件类型。
    pub kind: String,
    /// 事件时间戳，Unix 毫秒。
    pub ts: i64,
    /// 事件载荷。
    pub payload: serde_json::Value,
}

impl NewSessionLogEvent {
    /// 用当前时刻作为 `ts` 构造事件。
    pub fn new(kind: impl Into<String>, payload: serde_json::Value) -> Self {
        Self {
            kind: kind.into(),
            ts: now_ms(),
            payload,
        }
    }

    /// 指定 `ts` 构造事件（测试与重放用）。
    pub fn with_ts(kind: impl Into<String>, ts: i64, payload: serde_json::Value) -> Self {
        Self {
            kind: kind.into(),
            ts,
            payload,
        }
    }
}

/// 会话摘要（用于 `list_sessions`）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionLogMeta {
    /// 会话 id。
    pub session_id: String,
    /// 会话 cwd。
    pub cwd: String,
    /// 创建时间，Unix 毫秒。
    pub created_at: i64,
    /// 迁移来源自描述（只有「从 blob 合成」的日志才写；正常写入为 `None`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SessionLogSource>,
}

impl SessionLogMeta {
    /// 以当前时刻为 `created_at` 构造摘要。
    pub fn new(session_id: impl Into<String>, cwd: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            cwd: cwd.into(),
            created_at: now_ms(),
            source: None,
        }
    }

    /// 附上「从 blob 迁移」来源标记（header 会写 `source` 字段）。
    pub fn with_source(mut self, source: SessionLogSource) -> Self {
        self.source = Some(source);
        self
    }

    /// 指定 `created_at`（迁移时用会话自身的 `updated_at_ms` 作近似）。
    pub fn with_created_at(mut self, created_at: i64) -> Self {
        self.created_at = created_at;
        self
    }

    fn from_header(header: &SessionLogHeader) -> Self {
        Self {
            session_id: header.session_id.clone(),
            cwd: header.cwd.clone(),
            created_at: header.created_at,
            source: None,
        }
    }
}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// 路径段编码
// ---------------------------------------------------------------------------

/// 可原样出现在路径段里的字节（unchecked 语义：由调用方保证位置约束）。
///
/// 只放 `a-z 0-9 - _` 与小写之外全转义：大写也不放行，避免在大小写不敏感文件系统
/// （Windows / 默认 macOS APFS）上把 `pa_AB` 与 `pa_ab` 映射到同一目录。
fn is_literal_byte(byte: u8) -> bool {
    matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_')
}

fn push_escape(out: &mut String, byte: u8) {
    out.push(ESCAPE_PREFIX);
    out.push(char::from_digit((byte >> 4) as u32, 16).unwrap_or('0'));
    out.push(char::from_digit((byte & 0x0f) as u32, 16).unwrap_or('0'));
}

/// 拒绝明显不安全的 session id（**拒绝，不是转义后放行**）。
///
/// 拒绝面覆盖：空串、`.`、`..`、任意路径分隔符（含 Windows `\`）、冒号（盘符与
/// NTFS 备用数据流）、控制字符、超长。
fn validate_session_id(session_id: &str) -> Result<(), LogStoreError> {
    let reject = |reason: &str| {
        Err(LogStoreError::InvalidSessionId {
            session_id: session_id.to_string(),
            reason: reason.to_string(),
        })
    };
    if session_id.is_empty() {
        return reject("session id must not be empty");
    }
    if session_id == "." || session_id == ".." {
        return reject("session id must not be a dot segment");
    }
    if session_id.len() > MAX_SESSION_ID_CHARS {
        return reject("session id exceeds the supported length");
    }
    if session_id.contains('/') || session_id.contains('\\') {
        return reject("session id must not contain path separators");
    }
    if session_id.contains(':') {
        return reject("session id must not contain `:` (drive prefix / alternate data stream)");
    }
    if session_id.bytes().any(|b| b < 0x20 || b == 0x7f) {
        return reject("session id must not contain control characters");
    }
    Ok(())
}

/// session id → **单射**安全路径段。
///
/// 字面量集合 = `a-z 0-9 - _`，以及非首位的 `.`；其余字节（大写、`~`、空格、
/// 非 ASCII 等）转义为 `~hh`（小写十六进制，逐字节）。因为 `~` 本身不在字面量集合，
/// 且转义长度固定 3 字节，映射可逆 ⇒ 单射 ⇒ 不同 id 永不共享目录。
pub fn encode_session_id_segment(session_id: &str) -> Result<String, LogStoreError> {
    validate_session_id(session_id)?;
    let mut out = String::with_capacity(session_id.len());
    for (index, byte) in session_id.bytes().enumerate() {
        let literal_dot = byte == b'.' && index > 0;
        if is_literal_byte(byte) || literal_dot {
            out.push(byte as char);
        } else {
            push_escape(&mut out, byte);
        }
    }
    if out.len() > MAX_SEGMENT_BYTES {
        return Err(LogStoreError::InvalidSessionId {
            session_id: session_id.to_string(),
            reason: format!("encoded segment exceeds {MAX_SEGMENT_BYTES} bytes"),
        });
    }
    Ok(out)
}

/// cwd → 可读的项目目录名（`--<normalized-cwd>--` 的中间段）。
///
/// 这里是**故意可读、可损失**的展示层标签：首尾分隔符剥掉，内部 `/` 与 `\` 都映射成
/// `-`，大写原样保留（`/Users/dm/Code` 要看得懂）。它**不承担唯一性**——唯一性由单射的
/// session id 段保证：即便大小写不敏感文件系统上两个仅大小写不同的 cwd 共用同一个分组
/// 目录，它们的会话 id 路径段也不同，不会混写。
pub fn normalize_cwd_segment(cwd: &str) -> String {
    let trimmed = cwd.trim_matches(['/', '\\']);
    let mut out = String::with_capacity(trimmed.len() + 2);
    for byte in trimmed.bytes() {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'/' | b'\\' => {
                out.push(if byte == b'/' || byte == b'\\' {
                    '-'
                } else {
                    byte as char
                });
            }
            _ => push_escape(&mut out, byte),
        }
    }
    // 首字符 `.` 会造出隐藏目录，与「项目目录可读」相悖，转义掉。
    if out.starts_with('.') {
        let mut escaped = String::with_capacity(out.len() + 2);
        push_escape(&mut escaped, b'.');
        escaped.push_str(&out[1..]);
        out = escaped;
    }
    if out.is_empty() {
        out.push_str("root");
    }
    format!("{CWD_DIR_MARKER}{out}{CWD_DIR_MARKER}")
}

// ---------------------------------------------------------------------------
// 文件级独占锁
// ---------------------------------------------------------------------------

/// 尝试对已打开的文件取得**独占、非阻塞**锁。
///
/// **统一为 `std::fs::File::try_lock`**（阶段 1-B 收口 Windows 遗留）：Unix 下它就是
/// `flock(LOCK_EX|LOCK_NB)`（语义按打开文件描述计，故同一进程内另开一个 fd 也会互斥，
/// 正好是「同会话第二个 writer 必须被拒」要的语义）；Windows 下是 `LockFileEx`。
/// 一条代码路径 ⇒ 两个平台都经过编译验证，不再有「等 Windows CI」的死角；本模块
/// 也不再直接调用 `libc`。锁随文件句柄关闭释放。
fn try_lock_exclusive(file: &File, path: &Path) -> Result<(), LogStoreError> {
    match file.try_lock() {
        Ok(()) => Ok(()),
        Err(std::fs::TryLockError::WouldBlock) => Err(LogStoreError::Locked {
            path: path.to_path_buf(),
        }),
        Err(std::fs::TryLockError::Error(source)) => {
            Err(io_err(format!("try_lock `{}`", path.display()), source))
        }
    }
}

/// 一行已提交日志的最小探针：**只认 `seq`**。
///
/// 阶段 1-B 起同一文件可能承载两种已提交行形态（通用事件 [`SessionLogEvent`] /
/// 命令信封 `{"seq","ts","cmd"}`），故「这一行是否已提交」的判据收敛为
/// 「合法 JSON 对象 + 无符号整数 `seq`」，不绑定某一行的完整 schema。
#[derive(Deserialize)]
struct SeqProbe {
    seq: u64,
}

/// 扫一片日志文件，返回「最后一个完整且可解析行的结束偏移」与「最后一个已提交 `seq`」。
///
/// 用于 `open_write` 的崩溃恢复：撕裂尾行（不完整 / 解析失败）被识别为不可提交后缀，
/// 由调用方 `set_len` 截断，保证后续 append 从行边界继续，且 `seq` 接着已提交前缀。
fn scan_committed_tail(
    file: &mut File,
    expect_session_id: &str,
) -> Result<(u64, Option<u64>), LogStoreError> {
    file.seek(SeekFrom::Start(0))
        .map_err(|e| io_err("rewind session log", e))?;
    let mut reader = BufReader::new(&*file);
    let mut committed: u64 = 0;
    let mut last_seq: Option<u64> = None;
    let mut header_seen = false;
    let mut line = String::new();
    loop {
        line.clear();
        let read = reader
            .read_line(&mut line)
            .map_err(|e| io_err("scan session log", e))?;
        if read == 0 {
            break;
        }
        if !line.ends_with('\n') {
            // 撕裂尾行：没有换行结束符 ⇒ 崩溃在写入中途，丢弃。
            break;
        }
        if !header_seen {
            let class = classify_header_line(&line);
            match class {
                HeaderClass::Current(header) if header.session_id == expect_session_id => {
                    header_seen = true;
                    committed += read as u64;
                    continue;
                }
                HeaderClass::Current(header) => {
                    return Err(LogStoreError::HeaderMalformed {
                        reason: format!(
                            "header id `{}` does not match `{expect_session_id}`",
                            header.session_id
                        ),
                    })
                }
                HeaderClass::MigrationRequired(header) => {
                    return Err(LogStoreError::HeaderMigrationRequired {
                        found: header.version,
                        current: SESSION_LOG_FORMAT_VERSION,
                    })
                }
                HeaderClass::Unsupported(header) => {
                    return Err(LogStoreError::HeaderUnsupported {
                        found: header.version,
                        current: SESSION_LOG_FORMAT_VERSION,
                    })
                }
                HeaderClass::Malformed { reason } => {
                    return Err(LogStoreError::HeaderMalformed { reason })
                }
            }
        }
        match serde_json::from_str::<SeqProbe>(line.trim()) {
            Ok(probe) => {
                committed += read as u64;
                last_seq = Some(probe.seq);
            }
            // 解析失败 ⇒ 该行及之后都不可提交（阶段 1 恢复语义：保留前缀）。
            Err(_) => break,
        }
    }
    Ok((committed, last_seq))
}

// ---------------------------------------------------------------------------
// 存储
// ---------------------------------------------------------------------------

/// 本地会话日志存储：一个 root 目录下按 `<cwd 分组>/<会话 id>` 组织。
///
/// 本类型不做缓存、不持有全局状态：每次 `open_write` / `open_read` 都直接落到文件系统，
/// 与「文件即真源」一致。
#[derive(Clone, Debug)]
pub struct SessionLogStore {
    root: PathBuf,
}

impl SessionLogStore {
    /// 以 `root` 为根创建存储句柄（不创建目录；目录在 `open_write` 时按需创建）。
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// 根目录。
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 会话目录：`<root>/--<normalized-cwd>--/<encoded-id>`。
    pub fn session_dir(&self, session_id: &str, cwd: &str) -> Result<PathBuf, LogStoreError> {
        let encoded = encode_session_id_segment(session_id)?;
        Ok(self.root.join(normalize_cwd_segment(cwd)).join(encoded))
    }

    /// 会话日志文件路径。
    pub fn log_path(&self, session_id: &str, cwd: &str) -> Result<PathBuf, LogStoreError> {
        Ok(self.session_dir(session_id, cwd)?.join(SESSION_LOG_FILE_NAME))
    }

    /// **跨 cwd 分组**按会话 id 查找日志文件（不看进程 cwd）。
    ///
    /// 为什么需要：会话的 cwd 分组是**创建时**的目录标签，进程换个工作目录启动后
    /// `open_write`/`log_path(id, current_cwd())` 会指向另一个分组 ⇒ 用进程 cwd 判
    /// 「有没有日志」必然误判（ADR 0099 阶段 3-B 实测：跨工作区会话被判成 grandfather，
    /// 于是 2-D 写路径继续给它写整包 blob、迁移也会重复造分组）。
    ///
    /// 快路径：先按**当前进程 cwd** 直接 stat（绝大多数情况一次命中，不扫目录）；
    /// 冷路径：遍历 `cwd_groups()`（分组数很少，线性扫可接受）返回第一个命中。
    /// 返回 `None` = 该 id 在任何分组下都没有日志文件。
    pub fn find_log_path(&self, session_id: &str) -> Option<PathBuf> {
        let encoded = encode_session_id_segment(session_id).ok()?;
        // 快路径：进程 cwd 分组（写路径的常态）。
        if let Some(cwd) = std::env::current_dir().ok() {
            let candidate = self
                .root
                .join(normalize_cwd_segment(&cwd.to_string_lossy()))
                .join(&encoded)
                .join(SESSION_LOG_FILE_NAME);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        // 冷路径：跨分组线性扫。
        for group in self.cwd_groups().ok()? {
            let candidate = group.join(&encoded).join(SESSION_LOG_FILE_NAME);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        None
    }

    /// 跨所有 cwd 分组扫描全部已有日志的编码 session id 集合（消除 O(N) 跨分组逐个 stat）。
    pub fn existing_log_encoded_ids(&self) -> std::collections::HashSet<String> {
        let mut ids = std::collections::HashSet::new();
        let Ok(groups) = self.cwd_groups() else {
            return ids;
        };
        for group in groups {
            let Ok(entries) = fs::read_dir(&group) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() && path.join(SESSION_LOG_FILE_NAME).is_file() {
                    if let Some(file_name) = path.file_name().and_then(|s| s.to_str()) {
                        ids.insert(file_name.to_string());
                    }
                }
            }
        }
        ids
    }

    /// **跨 cwd 分组**查找会话摘要（`current` header 的第一个命中）。
    ///
    /// 迁移用它解析「该会话已有日志时属于哪个 cwd 分组」，从而把产物写回**原分组**，
    /// 不制造重复分组（见 `log_migrate` 的 cwd 归属解析）。
    pub fn find_session_meta(&self, session_id: &str) -> Option<SessionLogMeta> {
        let encoded = encode_session_id_segment(session_id).ok()?;
        for group in self.cwd_groups().ok()? {
            let path = group.join(&encoded).join(SESSION_LOG_FILE_NAME);
            if let Ok(HeaderClass::Current(header)) = read_header(&path) {
                return Some(SessionLogMeta::from_header(&header));
            }
        }
        None
    }

    /// 读取某个会话的 header 分类。
    pub fn read_session_header(&self, session_id: &str) -> Result<HeaderClass, LogStoreError> {
        let dir = self.resolve_session_dir(session_id)?;
        read_header(&dir.join(SESSION_LOG_FILE_NAME))
    }

    /// 取得会话目录（跨 cwd 分组扫描）。
    ///
    /// 同一 id 出现在多个 cwd 分组下 ⇒ [`LogStoreError::SessionDirAmbiguous`]：宁可失败
    /// 也不猜，避免读到错误项目的同名会话。
    fn resolve_session_dir(&self, session_id: &str) -> Result<PathBuf, LogStoreError> {
        let encoded = encode_session_id_segment(session_id)?;
        let groups = self.cwd_groups()?;
        let mut hits = Vec::new();
        for group in groups {
            let candidate = group.join(&encoded);
            if candidate.is_dir() {
                hits.push(candidate);
            }
        }
        match hits.len() {
            0 => Err(LogStoreError::SessionNotFound {
                session_id: session_id.to_string(),
            }),
            1 => Ok(hits.remove(0)),
            count => Err(LogStoreError::SessionDirAmbiguous {
                session_id: session_id.to_string(),
                count,
            }),
        }
    }

    /// 列举 root 下的 cwd 分组目录（不存在 root ⇒ 空）。
    fn cwd_groups(&self) -> Result<Vec<PathBuf>, LogStoreError> {
        let mut groups = Vec::new();
        let entries = match fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(groups),
            Err(err) => {
                return Err(io_err(
                    format!("read session log root `{}`", self.root.display()),
                    err,
                ))
            }
        };
        for entry in entries {
            let entry = entry.map_err(|e| io_err("iterate session log root", e))?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with(CWD_DIR_MARKER) && name.ends_with(CWD_DIR_MARKER) {
                groups.push(path);
            }
        }
        groups.sort();
        Ok(groups)
    }

    /// 列举全部会话（只返回 header 为 `current` 的会话，按 session id 排序）。
    ///
    /// 非 current 的 header（迁移中 / 更高版本 / 损坏）不在列表中出现：列表是派生视图，
    /// 它们的正文仍留在磁盘上，由显式 `read_session_header` 暴露分类。
    pub fn list_sessions(&self) -> Result<Vec<SessionLogMeta>, LogStoreError> {
        let mut metas = Vec::new();
        for group in self.cwd_groups()? {
            let entries = fs::read_dir(&group)
                .map_err(|e| io_err(format!("read cwd group `{}`", group.display()), e))?;
            for entry in entries {
                let entry = entry.map_err(|e| io_err("iterate cwd group", e))?;
                let dir = entry.path();
                if !dir.is_dir() {
                    continue;
                }
                let header = match read_header(&dir.join(SESSION_LOG_FILE_NAME)) {
                    Ok(class) => class,
                    // 缺文件 / header 读不出 ⇒ 不是可列举会话，跳过（不因单个坏目录失败整表）。
                    Err(_) => continue,
                };
                if let HeaderClass::Current(header) = header {
                    metas.push(SessionLogMeta::from_header(&header));
                }
            }
        }
        metas.sort_by(|a, b| a.session_id.cmp(&b.session_id));
        Ok(metas)
    }

    /// 打开**独占写**句柄。
    ///
    /// 步骤：校验 id → 建目录（幂等）→ `create_new` 建文件（已存在则打开）→ 抢独占锁
    /// → 空文件写 header+fsync → 扫已提交前缀截断撕裂尾行 → 返回 writer。
    /// 抢不到锁即 [`LogStoreError::Locked`]，**不降级为写**。
    pub fn open_write(
        &self,
        session_id: &str,
        meta: &SessionLogMeta,
    ) -> Result<LogWriter, LogStoreError> {
        let path = self.log_path(session_id, &meta.cwd)?;
        let dir = path
            .parent()
            .ok_or_else(|| LogStoreError::InvalidSessionId {
                session_id: session_id.to_string(),
                reason: "session log path has no parent directory".to_string(),
            })?;
        fs::create_dir_all(dir).map_err(|e| io_err(format!("create `{}`", dir.display()), e))?;

        let mut file = open_or_create(&path)?;
        try_lock_exclusive(&file, &path)?;
        let len = file
            .metadata()
            .map_err(|e| io_err(format!("stat `{}`", path.display()), e))?
            .len();
        if len == 0 {
            let header = HeaderLine {
                kind: "session",
                version: SESSION_LOG_FORMAT_VERSION,
                id: session_id,
                cwd: &meta.cwd,
                created_at: meta.created_at,
                source: meta.source.as_ref(),
            };
            let line = serde_json::to_string(&header).map_err(|e| LogStoreError::Serialize {
                reason: e.to_string(),
            })?;
            let mut bytes = line.into_bytes();
            bytes.push(b'\n');
            file.seek(SeekFrom::Start(0))
                .map_err(|e| io_err("rewind new session log", e))?;
            file.write_all(&bytes)
                .map_err(|e| io_err(format!("write header `{}`", path.display()), e))?;
            file.sync_all()
                .map_err(|e| io_err(format!("fsync header `{}`", path.display()), e))?;
        }
        finish_writer(&path, session_id, file)
    }

    /// 打开**只读**句柄，返回 `seq > after_seq` 的事件流。
    ///
    /// `after_seq = None` 表示从头读（`seq` 起点是 0，用排他比较的 `Some(0)` 表达不了
    /// 「从头读」，故用 `Option` 而不是裸 `u64`）。
    pub fn open_read(
        &self,
        session_id: &str,
        after_seq: Option<u64>,
    ) -> Result<LogReader, LogStoreError> {
        let dir = self.resolve_session_dir(session_id)?;
        let path = dir.join(SESSION_LOG_FILE_NAME);
        let class = read_header(&path)?;
        match class {
            HeaderClass::Current(header) if header.session_id == session_id => {}
            HeaderClass::Current(header) => {
                return Err(LogStoreError::HeaderMalformed {
                    reason: format!(
                        "header id `{}` does not match requested `{session_id}`",
                        header.session_id
                    ),
                })
            }
            HeaderClass::MigrationRequired(header) => {
                return Err(LogStoreError::HeaderMigrationRequired {
                    found: header.version,
                    current: SESSION_LOG_FORMAT_VERSION,
                })
            }
            HeaderClass::Unsupported(header) => {
                return Err(LogStoreError::HeaderUnsupported {
                    found: header.version,
                    current: SESSION_LOG_FORMAT_VERSION,
                })
            }
            HeaderClass::Malformed { reason } => return Err(LogStoreError::HeaderMalformed { reason }),
        }
        let file = File::open(&path).map_err(|e| io_err(format!("open `{}`", path.display()), e))?;
        LogReader::skip_header(path, BufReader::new(file), after_seq)
    }

    /// 删除整个会话目录（idempotent：不存在视为已删除）。
    ///
    /// 若该会话当前被某个 writer 持锁，返回 [`LogStoreError::Locked`]，不执行删除。
    ///
    /// 已知窗口：锁检查与 `remove_dir_all` 之间不是原子的（Unix 上已打开的 fd 也能被
    /// unlink），故这是**尽力而为**的活跃写者保护。真正的删除语义按 ADR 0099 D3 走墓碑
    /// 事件（阶段 1-C）；本方法只服务「明确的物理清理」。
    pub fn delete(&self, session_id: &str) -> Result<(), LogStoreError> {
        let dir = match self.resolve_session_dir(session_id) {
            Ok(dir) => dir,
            Err(LogStoreError::SessionNotFound { .. }) => return Ok(()),
            Err(err) => return Err(err),
        };
        let log_path = dir.join(SESSION_LOG_FILE_NAME);
        if log_path.exists() {
            let file = File::open(&log_path)
                .map_err(|e| io_err(format!("open `{}`", log_path.display()), e))?;
            try_lock_exclusive(&file, &log_path)?;
            // 锁随句柄关闭释放；删除拒绝发生在任何 remove 之前。
            drop(file);
        }
        fs::remove_dir_all(&dir).map_err(|e| io_err(format!("remove `{}`", dir.display()), e))
    }
}

/// 打开已存在的日志文件，或原子创建它（`create_new`，避免并发创建互踩）。
fn open_or_create(path: &Path) -> Result<File, LogStoreError> {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    match options.clone().create_new(true).open(path) {
        Ok(file) => Ok(file),
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => options
            .open(path)
            .map_err(|e| io_err(format!("open existing `{}`", path.display()), e)),
        Err(err) => Err(io_err(format!("create `{}`", path.display()), err)),
    }
}

/// 已拿锁的 writer 收尾：校验 header 为 current、截断撕裂尾行、算下一个 seq。
fn finish_writer(path: &Path, session_id: &str, mut file: File) -> Result<LogWriter, LogStoreError> {
    let (committed, last_seq) = scan_committed_tail(&mut file, session_id)?;
    let file_len = file
        .metadata()
        .map_err(|e| io_err(format!("stat `{}`", path.display()), e))?
        .len();
    if file_len > committed {
        // 撕裂尾行：截断到最后一个已提交行边界，已提交前缀一个字节都不动。
        file.set_len(committed)
            .map_err(|e| io_err(format!("truncate torn tail of `{}`", path.display()), e))?;
        file.sync_all()
            .map_err(|e| io_err(format!("fsync truncation of `{}`", path.display()), e))?;
    }
    Ok(LogWriter {
        path: path.to_path_buf(),
        file,
        session_id: session_id.to_string(),
        committed_len: committed,
        next_seq: last_seq.map(|seq| seq + 1).unwrap_or(0),
    })
}

// ---------------------------------------------------------------------------
// 写者
// ---------------------------------------------------------------------------

/// 阶段 1-B：持久化命令的日志信封行 `{"seq":N,"ts":ms,"cmd":{…}}`。
///
/// `cmd` 是**序列化后的 `PersistCommand`**（由 `log_command` 编码），日志因此无损
/// 保存现有命令流，可据此重建状态。
#[derive(Serialize)]
struct CommandEnvelope<'a> {
    seq: u64,
    ts: i64,
    cmd: &'a serde_json::Value,
}

/// 独占写句柄：持有文件级独占锁，`seq` 由它单调分配。
///
/// Drop 时文件句柄关闭 ⇒ 锁释放。已提交事件永不重写：本类型只做「追加 + fsync」，
/// 唯一的 `set_len` 用途是失败回滚与打开时截断撕裂尾行。
#[derive(Debug)]
pub struct LogWriter {
    path: PathBuf,
    file: File,
    /// 会话 id（`rollback_to` 重扫已提交尾部时做 header 校验）。
    session_id: String,
    committed_len: u64,
    next_seq: u64,
}

impl LogWriter {
    /// 日志文件路径。
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 下一个将被分配的 `seq`。
    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    /// 已提交字节长度（最后一次成功 append 之后的位置）。
    pub fn committed_len(&self) -> u64 {
        self.committed_len
    }

    /// 追加一条事件，返回分配的 `seq`。
    pub fn append_one(&mut self, event: NewSessionLogEvent) -> Result<u64, LogStoreError> {
        let mut seqs = self.append(std::slice::from_ref(&event))?;
        seqs.pop().ok_or_else(|| LogStoreError::Serialize {
            reason: "append produced no seq".to_string(),
        })
    }

    /// ADR 0107 P0.3：把**已提交前缀截断回 `len`**，用于回滚被事务拒收（stale-epoch）的批次。
    ///
    /// 前置：调用方已持有本会话日志的**独占锁**（本类型即锁持有者）——绝不允许无锁 `set_len`，
    /// 否则会截掉并发写者（另一降级实例）刚写入的字节。
    ///
    /// 截断后**重新扫描已提交尾部**校正 `committed_len` / `next_seq`：保证后续 append 的
    /// `seq` 从被截断前缀之后继续、无空洞无重复。
    pub fn rollback_to(&mut self, len: u64, expected_len: u64) -> Result<(), LogStoreError> {
        // 偏移校验（ADR 0107 P0.5）：文件长度必须仍是回执记录的「追加后长度」。
        // 不等 ⇒ 期间有人追加过字节，`len` 会截掉对方的写入 ⇒ 拒绝，交调用方按残留处理。
        let actual = self
            .file
            .metadata()
            .map_err(|e| io_err(format!("stat `{}` before rollback", self.path.display()), e))?
            .len();
        if actual != expected_len {
            return Err(LogStoreError::RollbackStaleOffset {
                expected: expected_len,
                actual,
            });
        }
        if len > self.committed_len {
            return Err(LogStoreError::Serialize {
                reason: format!(
                    "rollback target {len} is beyond committed length {} of `{}`",
                    self.committed_len,
                    self.path.display()
                ),
            });
        }
        self.file
            .set_len(len)
            .map_err(|e| io_err(format!("truncate `{}` for rollback", self.path.display()), e))?;
        self.file
            .sync_all()
            .map_err(|e| io_err(format!("fsync rollback truncation of `{}`", self.path.display()), e))?;
        let (committed, last_seq) = scan_committed_tail(&mut self.file, &self.session_id)?;
        self.committed_len = committed;
        self.next_seq = last_seq.map(|seq| seq + 1).unwrap_or(0);
        Ok(())
    }

    /// 追加一批事件（一次 `write_all` + 一次 `fsync`），返回分配的 `seq` 列表。
    ///
    /// 失败时把文件回滚到先前字节长度：不留下半批事件，也不动已提交前缀。
    pub fn append(&mut self, events: &[NewSessionLogEvent]) -> Result<Vec<u64>, LogStoreError> {
        if events.is_empty() {
            return Ok(Vec::new());
        }
        let start_seq = self.next_seq;
        let mut buf: Vec<u8> = Vec::new();
        let mut seqs = Vec::with_capacity(events.len());
        for (offset, event) in events.iter().enumerate() {
            let seq = start_seq + offset as u64;
            let record = SessionLogEvent {
                seq,
                kind: event.kind.clone(),
                ts: event.ts,
                payload: event.payload.clone(),
            };
            let line = serde_json::to_string(&record).map_err(|e| LogStoreError::Serialize {
                reason: e.to_string(),
            })?;
            buf.extend_from_slice(line.as_bytes());
            buf.push(b'\n');
            seqs.push(seq);
        }
        self.commit(&buf, events.len())?;
        Ok(seqs)
    }

    /// 追加一批**持久化命令**（阶段 1-B 写入真源的形态），返回分配的 `seq` 列表。
    ///
    /// 每行是一个自描述信封 `{"seq":N,"ts":ms,"cmd":{…PersistCommand…}}`：`cmd` 无损
    /// 承载命令本身，故日志可据此重建状态。与 [`Self::append`] 共用同一提交原语
    /// （一次 `write_all` + 一次 `fsync`，失败回滚到先前长度）。
    pub fn append_commands(
        &mut self,
        ts: i64,
        commands: &[serde_json::Value],
    ) -> Result<Vec<u64>, LogStoreError> {
        if commands.is_empty() {
            return Ok(Vec::new());
        }
        let start_seq = self.next_seq;
        let mut buf: Vec<u8> = Vec::new();
        let mut seqs = Vec::with_capacity(commands.len());
        for (offset, cmd) in commands.iter().enumerate() {
            let seq = start_seq + offset as u64;
            let line = serde_json::to_string(&CommandEnvelope { seq, ts, cmd }).map_err(|e| {
                LogStoreError::Serialize {
                    reason: e.to_string(),
                }
            })?;
            buf.extend_from_slice(line.as_bytes());
            buf.push(b'\n');
            seqs.push(seq);
        }
        self.commit(&buf, commands.len())?;
        Ok(seqs)
    }

    /// 单一提交原语：写入 + `fsync`，成功则推进 `committed_len` / `next_seq`，
    /// 失败回滚到先前字节长度。
    fn commit(&mut self, buf: &[u8], count: usize) -> Result<(), LogStoreError> {
        let previous_len = self.committed_len;
        match self.write_batch(buf, previous_len) {
            Ok(len) => {
                self.committed_len = len;
                self.next_seq += count as u64;
                Ok(())
            }
            Err(err) => {
                self.rollback(previous_len);
                Err(err)
            }
        }
    }

    fn write_batch(&mut self, buf: &[u8], previous_len: u64) -> Result<u64, LogStoreError> {
        let context = format!("append to `{}`", self.path.display());
        self.file
            .seek(SeekFrom::Start(previous_len))
            .map_err(|e| io_err(context.clone(), e))?;
        self.file
            .write_all(buf)
            .map_err(|e| io_err(context.clone(), e))?;
        self.file
            .sync_all()
            .map_err(|e| io_err(format!("fsync `{}`", self.path.display()), e))?;
        Ok(previous_len + buf.len() as u64)
    }

    /// 回滚到先前字节长度。回滚自身失败也返回原错误（不让次生错误盖住首因）。
    fn rollback(&mut self, previous_len: u64) {
        let _ = self.file.set_len(previous_len);
        let _ = self.file.sync_all();
        let _ = self.file.seek(SeekFrom::Start(previous_len));
    }
}

// ---------------------------------------------------------------------------
// 读者
// ---------------------------------------------------------------------------

/// 只读事件流：**丢弃撕裂尾行**（不完整或解析失败的最后一行），保留已提交前缀。
///
/// 只读不加锁：日志 append-only，读到的永远是一个已提交前缀的镜像。
///
/// 注意：本读取器只解出[`SessionLogEvent`] 形态的行。阶段 1-B 写进真源的命令信封
/// （`{"seq","ts","cmd"}`）不是 `SessionLogEvent`，会像撕裂尾行一样终止读取——命令流的
/// 折叠投影（`fold`）按 ADR 0099 是**后续阶段**的交付，本阶段不做，也不在这里
/// 「猜」一个投影出来。
#[derive(Debug)]
pub struct LogReader {
    path: PathBuf,
    reader: BufReader<File>,
    after_seq: Option<u64>,
    done: bool,
}

impl LogReader {
    fn skip_header(
        path: PathBuf,
        mut reader: BufReader<File>,
        after_seq: Option<u64>,
    ) -> Result<Self, LogStoreError> {
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .map_err(|e| io_err(format!("read header of `{}`", path.display()), e))?;
        Ok(Self {
            path,
            reader,
            after_seq,
            done: false,
        })
    }

    /// 下一条事件；流结束（含撕裂尾行）返回 `Ok(None)`。
    ///
    /// 撕裂尾行不是错误：`open_write` 下次打开会把它截断，读侧只需保留前缀。
    pub fn next_event(&mut self) -> Result<Option<SessionLogEvent>, LogStoreError> {
        let mut line = String::new();
        loop {
            if self.done {
                return Ok(None);
            }
            line.clear();
            let read = self
                .reader
                .read_line(&mut line)
                .map_err(|e| io_err(format!("read `{}`", self.path.display()), e))?;
            if read == 0 {
                self.done = true;
                return Ok(None);
            }
            if !line.ends_with('\n') {
                // 撕裂尾行（无换行结束符）：丢弃该行及之后。
                self.done = true;
                return Ok(None);
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let event = match serde_json::from_str::<SessionLogEvent>(trimmed) {
                Ok(event) => event,
                Err(_) => {
                    // 解析失败的最后一行：同样按撕裂尾行丢弃（不 panic、不报错）。
                    self.done = true;
                    return Ok(None);
                }
            };
            if let Some(floor) = self.after_seq {
                if event.seq <= floor {
                    continue;
                }
            }
            return Ok(Some(event));
        }
    }

    /// 读完全部剩余事件。
    pub fn collect_events(&mut self) -> Result<Vec<SessionLogEvent>, LogStoreError> {
        let mut events = Vec::new();
        while let Some(event) = self.next_event()? {
            events.push(event);
        }
        Ok(events)
    }
}

impl Iterator for LogReader {
    type Item = SessionLogEvent;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_event().ok().flatten()
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn session_id_encoding_is_injective_and_rejects_traversal() {
        assert_eq!(encode_session_id_segment("pa_abc-123").unwrap(), "pa_abc-123");
        // 大写被转义 ⇒ 大小写不敏感文件系统上也不会撞目录。
        assert_eq!(encode_session_id_segment("pa_A").unwrap(), "pa_~41");
        assert_eq!(encode_session_id_segment("pa_a").unwrap(), "pa_a");
        assert_ne!(
            encode_session_id_segment("pa_A").unwrap(),
            encode_session_id_segment("pa_a").unwrap()
        );
        for bad in ["../evil", "/abs/path", "..", ".", "", "a\\b", "c:evil"] {
            assert!(
                encode_session_id_segment(bad).is_err(),
                "`{bad}` 必须被拒绝"
            );
        }
    }

    #[test]
    fn header_classification_covers_four_states() {
        let current = format!(
            r#"{{"type":"session","version":{SESSION_LOG_FORMAT_VERSION},"id":"pa_x","cwd":"/tmp","createdAt":1}}"#
        );
        assert!(matches!(
            classify_header_line(&current),
            HeaderClass::Current(_)
        ));
        assert!(matches!(
            classify_header_line(&current.replace(r#""version":1"#, r#""version":0"#)),
            HeaderClass::MigrationRequired(_)
        ));
        assert!(matches!(
            classify_header_line(&current.replace(r#""version":1"#, r#""version":2"#)),
            HeaderClass::Unsupported(_)
        ));
        assert!(matches!(
            classify_header_line("{\"type\":\"session\""),
            HeaderClass::Malformed { .. }
        ));
    }

    #[test]
    fn cwd_segment_is_readable_and_marked() {
        assert_eq!(normalize_cwd_segment("/Users/dm/Code"), "--Users-dm-Code--");
        assert_eq!(normalize_cwd_segment("/"), "--root--");
        assert_eq!(normalize_cwd_segment(".hidden"), "--~2ehidden--");
    }
}
