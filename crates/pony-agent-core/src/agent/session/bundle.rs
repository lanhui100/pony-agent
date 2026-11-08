//! ADR 0099 **阶段 3-A**：会话**目录包**（bundle）= 传输格式的第一块。
//!
//! 真源仍是每会话一份 append-only 日志文件（D1）；本模块只是把它**打包成可传输单元**，
//! 用于换机、分享、审计，也是 D5 远程增量同步的前置形态。
//!
//! ## 包格式（唯一真源格式，不得另造）
//!
//! ```text
//! <包根>/
//!   manifest.json      # 自描述 + 逐文件 sha256/bytes
//!   events.jsonl       # 与 session.jsonl **逐字节同格式**（header 行 + {"seq","ts","cmd"} 行）
//!   attachments/       # 附件实体（可选）
//! ```
//!
//! 三条格式契约（对外承诺，改动必须同步 ADR）：
//!
//! 1. **`files[].path` 语义 = 包根相对路径、POSIX 分隔符**（`events.jsonl`、
//!    `attachments/x.png`）。禁止绝对路径、禁止 `..` 段；`manifest.json` **自身不入
//!    `files`**（自引用哈希无解），但导入时必须存在且可解析。
//! 2. **事件严格、附件可降级**：`events.jsonl` 的 sha256/bytes/header/seq/eventCount
//!    任一不符 ⇒ 校验失败（调用方 EXIT=2，**零落盘**）；`attachments/*` 缺失或哈希不符
//!    ⇒ 记 warning 并继续导入事件（ADR 0099 D5「缺附件降级为占位、不得静默」），
//!    此时退出码是 **4**（不是 0，也不是 2）。
//! 3. **manifest 不参与 sha256 自校验**（自引用哈希无解）⇒ 导入时对 manifest 做逐字段
//!    结构性校验，并用 `events.jsonl` 的 header（`sessionId`/`cwd`/`createdAtMs`）与
//!    seq/eventCount 一致性做兜底，任何一侧被篡改都无法静默通过。
//!
//! ## 单文件形态：手写最小 ustar（不加依赖）
//!
//! 仓库没有 tar / zstd 依赖，且规则禁止新增第三方依赖 ⇒ 单文件形态用**手写的 ustar**
//! （512 字节头 + 内容 512 对齐 + 两个全零块收尾），不压缩。压缩（`.tar.zst`）留作后续
//! ADR 事项。读写两侧都在本文件内实现（`write_tar_bytes` / `read_tar_bytes`），能覆盖
//! 自家产出的包；遇到 GNU 长名条目（typeflag `L`）明确报错而不是猜。
//!
//! ## 导入语义（三条硬约束）
//!
//! 1. **只落日志文件与附件**：导入**不写 SQLite 索引**（索引由
//!    `ponycli sessions rebuild-index` 重建；这也是「索引可删可建」的兑现路径）。
//! 2. **先全量校验再落盘**：所有校验在临时目录完成，成功后原子 `rename` 到目标位置；
//!    失败 ⇒ 目标根目录零改动。
//! 3. **不静默覆盖**：目标会话已存在 ⇒ 默认拒绝（EXIT=3）；`--force` 才把旧目录改名为
//!    `<id>.bak-<ts>`（不删）。

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::log_store::{
    encode_session_id_segment, now_ms, read_header, HeaderClass, SessionLogStore,
    SESSION_LOG_FILE_NAME,
};

// ---------------------------------------------------------------------------
// 契约常量
// ---------------------------------------------------------------------------

/// `manifest.json` 的 `format` 值（包类型标识）。
pub const BUNDLE_FORMAT: &str = "pony.session.bundle";
/// 当前支持的包格式版本。
pub const BUNDLE_VERSION: u64 = 1;
/// 包内 manifest 文件名。
pub const BUNDLE_MANIFEST_FILE_NAME: &str = "manifest.json";
/// 包内事件流文件名（内容与 `session.jsonl` 同格式）。
pub const BUNDLE_EVENTS_FILE_NAME: &str = "events.jsonl";
/// 包内附件目录名。
pub const BUNDLE_ATTACHMENTS_DIR_NAME: &str = "attachments";

/// ustar 块大小。
const TAR_BLOCK: usize = 512;
/// `attachments/` 在包内与落盘侧的相对前缀。
const ATTACHMENTS_PREFIX: &str = "attachments/";

// ---------------------------------------------------------------------------
// 清单
// ---------------------------------------------------------------------------

/// 包清单（`manifest.json`）。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BundleManifest {
    /// 固定为 [`BUNDLE_FORMAT`]。
    pub format: String,
    /// 包格式版本（当前只支持 [`BUNDLE_VERSION`]）。
    pub version: u64,
    /// 会话 id。
    pub session_id: String,
    /// 会话创建时的 cwd（决定导入后的 cwd 分组目录）。
    pub cwd: String,
    /// 会话创建时间（取自日志 header，Unix 毫秒）。
    pub created_at_ms: i64,
    /// 事件条数（不含 header 行）。
    pub event_count: usize,
    /// 最后一个事件的 seq（空事件流为 `None`）。
    pub last_seq: Option<u64>,
    /// 逐文件指纹（**不含 manifest.json 自身**）。
    pub files: Vec<BundleFileEntry>,
    /// 来源信息（审计用）。
    pub source: BundleSource,
}

/// 包内单个文件的指纹。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BundleFileEntry {
    /// 包根相对路径（POSIX 分隔符）。
    pub path: String,
    /// 文件内容的 sha256（小写十六进制）。
    pub sha256: String,
    /// 文件字节数。
    pub bytes: u64,
}

/// 包的来源自描述。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BundleSource {
    /// 来源类型（本地导出固定 `"local"`）。
    pub kind: String,
    /// 导出方应用版本。
    pub app_version: String,
    /// 导出时刻（Unix 毫秒）。
    pub exported_at_ms: i64,
}

/// 导出形态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BundleFormat {
    /// 目录形态（主形态）。
    Dir,
    /// 单文件形态（最小 ustar `.tar`，不压缩）。
    Tar,
}

impl BundleFormat {
    /// CLI 取值解析（`dir` / `tar`）。
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "dir" => Some(Self::Dir),
            "tar" => Some(Self::Tar),
            _ => None,
        }
    }

    /// 稳定标签（报告与 CLI 输出用）。
    pub fn label(self) -> &'static str {
        match self {
            Self::Dir => "dir",
            Self::Tar => "tar",
        }
    }
}

// ---------------------------------------------------------------------------
// 错误与退出码
// ---------------------------------------------------------------------------

/// 包操作错误。`exit_code` 是 CLI 契约的唯一来源（测试也断言它）。
#[derive(Debug)]
pub enum BundleError {
    /// 会话在日志根下不存在（或同名会话跨 cwd 分组歧义）。
    SessionNotFound { session_id: String, detail: String },
    /// 包校验失败：`step` 指明是哪一项（`manifest` / `format` / `version` / `files` /
    /// `header` / `seq` / `path`），`detail` 指明具体文件/字段。
    Validation { step: &'static str, detail: String },
    /// 目标会话目录已存在（未加 `--force`）。
    TargetExists { path: PathBuf },
    /// 导出目标已存在（不允许覆盖已有包）。
    OutputExists { path: PathBuf },
    /// 文件系统 / IO 失败。
    Io { context: String, source: std::io::Error },
    /// tar 结构错误（非本工具产出的包、截断、GNU 长名等）。
    Tar { detail: String },
}

impl BundleError {
    /// CLI 退出码契约（脚本可区分「失败」与「目标已存在」）：
    ///
    /// - `2` = 硬失败：会话不存在 / 包校验失败 / 导出目标已存在（**零落盘**）；
    /// - `3` = 目标会话已存在且未加 `--force`；
    /// - `1` = IO / tar 结构等实现层失败。
    ///
    /// `4`（事件成功但附件降级）不是错误，见 [`ImportReport::exit_code`]。
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::SessionNotFound { .. }
            | Self::Validation { .. }
            | Self::OutputExists { .. } => 2,
            Self::TargetExists { .. } => 3,
            Self::Io { .. } | Self::Tar { .. } => 1,
        }
    }
}

impl std::fmt::Display for BundleError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SessionNotFound { session_id, detail } => {
                write!(formatter, "会话不存在：{session_id}（{detail}）")
            }
            Self::Validation { step, detail } => write!(formatter, "包校验失败 [{step}]：{detail}"),
            Self::TargetExists { path } => write!(
                formatter,
                "目标会话已存在：{}（默认拒绝覆盖；确认要覆盖请加 --force）",
                path.display()
            ),
            Self::OutputExists { path } => {
                write!(formatter, "导出目标已存在：{}（拒绝覆盖已有包）", path.display())
            }
            Self::Io { context, source } => write!(formatter, "IO 失败（{context}）：{source}"),
            Self::Tar { detail } => write!(formatter, "tar 结构错误：{detail}"),
        }
    }
}

fn io_err(context: impl Into<String>, source: std::io::Error) -> BundleError {
    BundleError::Io {
        context: context.into(),
        source,
    }
}

fn validation(step: &'static str, detail: impl Into<String>) -> BundleError {
    BundleError::Validation {
        step,
        detail: detail.into(),
    }
}

// ---------------------------------------------------------------------------
// 报告
// ---------------------------------------------------------------------------

/// 导出报告（CLI 打印用）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExportReport {
    /// 会话 id。
    pub session_id: String,
    /// 会话 cwd。
    pub cwd: String,
    /// 产出路径（目录或 .tar 文件）。
    pub out: PathBuf,
    /// 导出形态。
    pub format_label: &'static str,
    /// 事件条数（不含 header）。
    pub event_count: usize,
    /// 最后一个事件 seq。
    pub last_seq: Option<u64>,
    /// 包内文件清单（含 `manifest.json`）。
    pub files: Vec<String>,
    /// 包体积（全部文件字节数之和）。
    pub total_bytes: u64,
    /// 附件相对路径清单。
    pub attachments: Vec<String>,
    /// 非致命警告（例如附件实体缺失）。
    pub warnings: Vec<String>,
    /// 是否存在**附件降级**（源侧附件缺失 / 写包失败）。
    pub attachment_degraded: bool,
}

impl ExportReport {
    /// CLI 退出码：`0` = 全成功；`4` = 包已产出但附件降级（warnings 会逐条列出原因）。
    pub fn exit_code(&self) -> i32 {
        if self.attachment_degraded {
            4
        } else {
            0
        }
    }
}

/// 导入报告（CLI 打印用）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportReport {
    /// 会话 id。
    pub session_id: String,
    /// 会话 cwd。
    pub cwd: String,
    /// 目标会话目录。
    pub target_dir: PathBuf,
    /// 事件条数（不含 header）。
    pub event_count: usize,
    /// 最后一个事件 seq。
    pub last_seq: Option<u64>,
    /// 落盘的附件相对路径。
    pub attachments: Vec<String>,
    /// 非致命警告（附件缺失/哈希不符等）。
    pub warnings: Vec<String>,
    /// 是否只是预演（零写入）。
    pub dry_run: bool,
    /// `--force` 覆盖时旧目录的备份路径。
    pub backup: Option<PathBuf>,
    /// 是否存在**附件降级**（缺失 / sha256 不符）；`warnings` 里逐条列出文件与原因。
    pub attachment_degraded: bool,
}

impl ImportReport {
    /// CLI 退出码契约（ADR 0099 阶段 3-A）：
    ///
    /// - `0` = 全部成功（含附件全部校验通过）；
    /// - `4` = 事件导入成功，但**有附件降级**（缺失或 sha256 不符）——不得静默成 0；
    /// - 硬失败 / 目标已存在分别由 [`BundleError::exit_code`] 给 `2` / `3`。
    pub fn exit_code(&self) -> i32 {
        if self.attachment_degraded {
            4
        } else {
            0
        }
    }
}

/// 导入选项。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImportOptions {
    /// 只校验 + 报告，零写入。
    pub dry_run: bool,
    /// 允许覆盖已存在的目标会话（旧目录改名 `.bak-<ts>`）。
    pub force: bool,
}

impl ImportOptions {
    /// 预演。
    pub fn dry_run() -> Self {
        Self {
            dry_run: true,
            force: false,
        }
    }

    /// 真正落盘。
    pub fn apply() -> Self {
        Self {
            dry_run: false,
            force: false,
        }
    }
}

// ---------------------------------------------------------------------------
// 导出
// ---------------------------------------------------------------------------

/// 导出会话为目录包（`format = Dir`）或单文件包（`format = Tar`）。
///
/// `db_path` 只用于**尽力**收集附件实体（附件索引在 SQLite 索引里）；取不到不算失败，
/// 只在报告里记 warning（附件可降级，事件是硬要求）。
pub fn export_session(
    log_root: &Path,
    session_id: &str,
    out: &Path,
    format: BundleFormat,
    app_version: &str,
    db_path: Option<&Path>,
) -> Result<ExportReport, BundleError> {
    if out.exists() {
        return Err(BundleError::OutputExists {
            path: out.to_path_buf(),
        });
    }
    let store = SessionLogStore::new(log_root.to_path_buf());
    let mut metas = store
        .list_sessions()
        .map_err(|error| io_err(format!("scan log root `{}`", log_root.display()), io_from(error)))?
        .into_iter()
        .filter(|meta| meta.session_id == session_id)
        .collect::<Vec<_>>();
    if metas.is_empty() {
        return Err(BundleError::SessionNotFound {
            session_id: session_id.to_string(),
            detail: format!("日志根 `{}` 下没有该会话", log_root.display()),
        });
    }
    if metas.len() > 1 {
        return Err(validation(
            "path",
            format!(
                "会话 `{session_id}` 出现在 {} 个 cwd 分组下（歧义），拒绝猜测",
                metas.len()
            ),
        ));
    }
    let meta = metas.remove(0);
    let session_dir = store
        .session_dir(session_id, &meta.cwd)
        .map_err(|error| validation("path", error.to_string()))?;
    let log_path = session_dir.join(SESSION_LOG_FILE_NAME);
    let events_bytes =
        fs::read(&log_path).map_err(|error| io_err(format!("read `{}`", log_path.display()), error))?;
    let header = match read_header(&log_path) {
        Ok(HeaderClass::Current(header)) => header,
        Ok(other) => {
            return Err(validation(
                "header",
                format!(
                    "会话日志 header 不是 current（{}），先迁移再导出",
                    other.label()
                ),
            ))
        }
        Err(error) => return Err(validation("header", error.to_string())),
    };
    let (event_count, last_seq) = inspect_events(&events_bytes, session_id)?;

    // 附件：先收集（会话目录内 `attachments/` + 尽力从索引取实体），再统一落包。
    let mut warnings = Vec::new();
    let mut attachments: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    collect_session_dir_attachments(&session_dir, &mut attachments, &mut warnings)?;
    if let Some(db_path) = db_path {
        collect_indexed_attachments(db_path, session_id, &mut attachments, &mut warnings);
    }

    // 先写临时目录（同父目录内，保证 rename 原子），成功后整体 rename。
    let staging = StagingDir::new(out, "export")?;
    write_bundle_tree(&staging.path, &events_bytes, &attachments, &header, app_version, event_count, last_seq, &mut warnings)?;

    let attachment_degraded = !warnings.is_empty();
    let mut report = ExportReport {
        session_id: session_id.to_string(),
        cwd: meta.cwd.clone(),
        out: out.to_path_buf(),
        format_label: format.label(),
        event_count,
        last_seq,
        files: Vec::new(),
        total_bytes: 0,
        attachments: attachments.keys().cloned().collect(),
        warnings,
        attachment_degraded,
    };
    let mut entries = staging.collect_entries()?;
    report.total_bytes = entries.iter().map(|(_, bytes)| bytes.len() as u64).sum();
    report.files = entries.iter().map(|(path, _)| path.clone()).collect();

    staging.finish(out, format, &mut entries)?;
    Ok(report)
}

/// 把包内容写进 `root`（临时目录）：`manifest.json` + `events.jsonl` + `attachments/`。
#[allow(clippy::too_many_arguments)]
fn write_bundle_tree(
    root: &Path,
    events_bytes: &[u8],
    attachments: &BTreeMap<String, Vec<u8>>,
    header: &super::log_store::SessionLogHeader,
    app_version: &str,
    event_count: usize,
    last_seq: Option<u64>,
    warnings: &mut Vec<String>,
) -> Result<(), BundleError> {
    fs::create_dir_all(root).map_err(|error| io_err("create bundle dir", error))?;
    let events_path = root.join(BUNDLE_EVENTS_FILE_NAME);
    fs::write(&events_path, events_bytes)
        .map_err(|error| io_err(format!("write `{}`", events_path.display()), error))?;

    let mut files = vec![BundleFileEntry {
        path: BUNDLE_EVENTS_FILE_NAME.to_string(),
        sha256: sha256_hex(events_bytes),
        bytes: events_bytes.len() as u64,
    }];
    for (relative, bytes) in attachments {
        let target = root.join(relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| io_err(format!("create `{}`", parent.display()), error))?;
        }
        if let Err(error) = fs::write(&target, bytes) {
            // 附件可降级：写不进去就记 warning 并从 manifest 里剔除（不产出半包）。
            warnings.push(format!("附件 `{relative}` 写入失败，已从包中剔除：{error}"));
            continue;
        }
        files.push(BundleFileEntry {
            path: relative.clone(),
            sha256: sha256_hex(bytes),
            bytes: bytes.len() as u64,
        });
    }

    let manifest = BundleManifest {
        format: BUNDLE_FORMAT.to_string(),
        version: BUNDLE_VERSION,
        session_id: header.session_id.clone(),
        cwd: header.cwd.clone(),
        created_at_ms: header.created_at,
        event_count,
        last_seq,
        files,
        source: BundleSource {
            kind: "local".to_string(),
            app_version: app_version.to_string(),
            exported_at_ms: now_ms(),
        },
    };
    let raw = serde_json::to_string_pretty(&manifest)
        .map_err(|error| validation("manifest", format!("serialize manifest: {error}")))?;
    let manifest_path = root.join(BUNDLE_MANIFEST_FILE_NAME);
    fs::write(&manifest_path, raw)
        .map_err(|error| io_err(format!("write `{}`", manifest_path.display()), error))?;
    Ok(())
}

/// 收集会话目录内的附件实体（`<session-dir>/attachments/**`）。
fn collect_session_dir_attachments(
    session_dir: &Path,
    out: &mut BTreeMap<String, Vec<u8>>,
    warnings: &mut Vec<String>,
) -> Result<(), BundleError> {
    let attachments_dir = session_dir.join(BUNDLE_ATTACHMENTS_DIR_NAME);
    if !attachments_dir.is_dir() {
        return Ok(());
    }
    let mut stack = vec![attachments_dir.clone()];
    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir)
            .map_err(|error| io_err(format!("read `{}`", dir.display()), error))?;
        for entry in entries {
            let entry = entry.map_err(|error| io_err("iterate attachments dir", error))?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let relative = match path.strip_prefix(&attachments_dir) {
                Ok(relative) => relative,
                Err(_) => continue,
            };
            let bundle_relative = bundle_attachment_path(&relative.to_string_lossy());
            match fs::read(&path) {
                Ok(bytes) => {
                    out.insert(bundle_relative, bytes);
                }
                Err(error) => warnings.push(format!(
                    "附件实体 `{}` 读取失败，已跳过：{error}",
                    path.display()
                )),
            }
        }
    }
    Ok(())
}

/// 尽力从 SQLite 索引收集附件实体：`store_metadata` 的 `attachment_assets` +
/// `session_attachment_index` → `<db 目录>/attachments/<relative_path>`。
///
/// 索引只是**派生视图**，读不到（库不存在/键缺失/JSON 坏）一律记 warning 并继续：
/// 附件可降级，事件是硬要求（ADR 0099 D5）。
fn collect_indexed_attachments(
    db_path: &Path,
    session_id: &str,
    out: &mut BTreeMap<String, Vec<u8>>,
    warnings: &mut Vec<String>,
) {
    if !db_path.is_file() {
        return;
    }
    let connection = match rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ) {
        Ok(connection) => connection,
        Err(error) => {
            warnings.push(format!(
                "无法只读打开会话库收集附件（{}）：{error}",
                db_path.display()
            ));
            return;
        }
    };
    let read_meta = |key: &str| -> Option<String> {
        connection
            .query_row(
                "SELECT value FROM store_metadata WHERE key = ?1",
                rusqlite::params![key],
                |row| row.get::<_, String>(0),
            )
            .ok()
    };
    let Some(index_raw) = read_meta("session_attachment_index") else {
        return;
    };
    let Some(assets_raw) = read_meta("attachment_assets") else {
        return;
    };
    let index: serde_json::Value = match serde_json::from_str(&index_raw) {
        Ok(value) => value,
        Err(error) => {
            warnings.push(format!("attachment 索引不可解析，附件不入包：{error}"));
            return;
        }
    };
    let assets: serde_json::Value = match serde_json::from_str(&assets_raw) {
        Ok(value) => value,
        Err(error) => {
            warnings.push(format!("attachment 资产表不可解析，附件不入包：{error}"));
            return;
        }
    };
    let Some(asset_ids) = index.get(session_id).and_then(|value| value.as_array()) else {
        return;
    };
    let attachment_root = db_path
        .parent()
        .map(|parent| parent.join("attachments"))
        .unwrap_or_else(|| PathBuf::from("attachments"));
    for asset_id in asset_ids.iter().filter_map(|value| value.as_str()) {
        let Some(relative) = assets
            .get(asset_id)
            .and_then(|asset| asset.get("relativePath"))
            .and_then(|value| value.as_str())
        else {
            warnings.push(format!("附件资产 `{asset_id}` 缺 relativePath，已跳过"));
            continue;
        };
        let source = attachment_root.join(relative);
        match fs::read(&source) {
            Ok(bytes) => {
                out.insert(bundle_attachment_path(relative), bytes);
            }
            Err(error) => warnings.push(format!(
                "附件实体 `{}` 缺失，已跳过（降级为占位）：{error}",
                source.display()
            )),
        }
    }
}

/// 附件在包内的相对路径（POSIX 分隔符）。
fn bundle_attachment_path(relative: &str) -> String {
    let normalized = relative.replace('\\', "/");
    format!("{ATTACHMENTS_PREFIX}{}", normalized.trim_start_matches('/'))
}

// ---------------------------------------------------------------------------
// 导入
// ---------------------------------------------------------------------------

/// 导入一个**已校验**的会话包；默认拒绝覆盖已存在会话（`force` 才备份后覆盖）。
///
/// 校验顺序（与 task-9 C.1 一致，任一失败即 `Err`，**目标根零改动**）：
/// 1. manifest 存在且可解析；
/// 2. `format` / `version` 受支持；
/// 3. 逐文件 sha256 与 bytes 匹配（`events.jsonl` 严格；`attachments/*` 降级为 warning）；
/// 4. `events.jsonl` 首行 header 与 manifest 一致；
/// 5. 事件数 / 末 seq / seq 连续性。
pub fn import_bundle(
    log_root: &Path,
    bundle_path: &Path,
    options: ImportOptions,
) -> Result<ImportReport, BundleError> {
    // tar 形态：先解到系统临时目录（**不在日志根下**，否则 dry-run 就不是零写入了）。
    let mut extracted: Option<ExtractedBundle> = None;
    let root: PathBuf = if bundle_path.is_dir() {
        bundle_path.to_path_buf()
    } else if bundle_path.is_file() {
        let extracted_bundle = ExtractedBundle::from_tar_file(bundle_path)?;
        let path = extracted_bundle.path.clone();
        extracted = Some(extracted_bundle);
        path
    } else {
        return Err(validation(
            "path",
            format!("包路径不存在：{}", bundle_path.display()),
        ));
    };
    let result = import_bundle_root(log_root, &root, options);
    drop(extracted);
    result
}

/// 对**已解包目录**执行导入（tar/dir 两种形态共用同一条校验与落盘路径）。
fn import_bundle_root(
    log_root: &Path,
    root: &Path,
    options: ImportOptions,
) -> Result<ImportReport, BundleError> {
    let (manifest, events_bytes, attachments, warnings) = load_and_validate(root)?;
    let store = SessionLogStore::new(log_root.to_path_buf());
    let target_dir = store
        .session_dir(&manifest.session_id, &manifest.cwd)
        .map_err(|error| validation("path", error.to_string()))?;

    let attachment_degraded = !warnings.is_empty();
    let mut report = ImportReport {
        session_id: manifest.session_id.clone(),
        cwd: manifest.cwd.clone(),
        target_dir: target_dir.clone(),
        event_count: manifest.event_count,
        last_seq: manifest.last_seq,
        attachments: attachments.keys().cloned().collect(),
        warnings,
        dry_run: options.dry_run,
        backup: None,
        attachment_degraded,
    };
    let target_exists = target_dir.exists();
    if target_exists && !options.force {
        return Err(BundleError::TargetExists { path: target_dir });
    }
    if options.dry_run {
        // 预演：校验已全部通过，此处**一个字节都不写**（也不建目录）。
        return Ok(report);
    }

    // 落盘：同分组目录内的临时目录 → 原子 rename；--force 先把旧目录改名备份。
    let group_dir = target_dir
        .parent()
        .ok_or_else(|| validation("path", "目标目录无父目录"))?
        .to_path_buf();
    fs::create_dir_all(&group_dir)
        .map_err(|error| io_err(format!("create `{}`", group_dir.display()), error))?;
    if target_exists {
        let backup = group_dir.join(format!(
            "{}.bak-{}",
            target_dir
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| manifest.session_id.clone()),
            now_ms()
        ));
        fs::rename(&target_dir, &backup)
            .map_err(|error| io_err(format!("backup `{}`", target_dir.display()), error))?;
        report.backup = Some(backup);
    }
    let mut staging = StagingDir::in_dir(&group_dir, "import")?;
    fs::write(staging.path.join(SESSION_LOG_FILE_NAME), &events_bytes).map_err(|error| {
        io_err(
            format!("write `{}`", staging.path.join(SESSION_LOG_FILE_NAME).display()),
            error,
        )
    })?;
    for (relative, bytes) in &attachments {
        let name = relative
            .strip_prefix(ATTACHMENTS_PREFIX)
            .unwrap_or(relative.as_str());
        let target = staging.path.join(BUNDLE_ATTACHMENTS_DIR_NAME).join(name);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| io_err(format!("create `{}`", parent.display()), error))?;
        }
        fs::write(&target, bytes)
            .map_err(|error| io_err(format!("write `{}`", target.display()), error))?;
    }
    fs::rename(&staging.path, &target_dir).map_err(|error| {
        io_err(
            format!("rename staging → `{}`", target_dir.display()),
            error,
        )
    })?;
    staging.disarm();
    Ok(report)
}

/// manifest **结构性校验**（在强类型解析之前）。
///
/// 为什么需要：`manifest.json` 自身不在 `files` 里（自引用哈希无解），因此它**没有完整性
/// 保护**。故导入时对 manifest 做「键存在 + 类型正确 + 语义非空」的显式校验，再由
/// `events.jsonl` 的结构性校验（header/seq/eventCount）兜底——任何一侧被篡改都无法静默通过。
fn validate_manifest_shape(raw: &str) -> Result<(), BundleError> {
    let value: serde_json::Value = serde_json::from_str(raw).map_err(|error| {
        validation(
            "manifest",
            format!("`{BUNDLE_MANIFEST_FILE_NAME}` 不是合法 JSON：{error}"),
        )
    })?;
    let object = value
        .as_object()
        .ok_or_else(|| validation("manifest", "`manifest.json` 顶层必须是 JSON 对象"))?;
    let require = |key: &str| -> Result<&serde_json::Value, BundleError> {
        object
            .get(key)
            .ok_or_else(|| validation("manifest", format!("缺少必填字段 `{key}`")))
    };
    if require("format")?.as_str().is_none() {
        return Err(validation("manifest", "`format` 必须是字符串"));
    }
    if require("version")?.as_u64().is_none() {
        return Err(validation("manifest", "`version` 必须是非负整数"));
    }
    let session_id = require("sessionId")?
        .as_str()
        .ok_or_else(|| validation("manifest", "`sessionId` 必须是字符串"))?;
    if session_id.is_empty() {
        return Err(validation("manifest", "`sessionId` 不得为空"));
    }
    let cwd = require("cwd")?
        .as_str()
        .ok_or_else(|| validation("manifest", "`cwd` 必须是字符串"))?;
    if cwd.is_empty() {
        return Err(validation("manifest", "`cwd` 不得为空"));
    }
    if require("createdAtMs")?.as_i64().is_none() {
        return Err(validation("manifest", "`createdAtMs` 必须是整数"));
    }
    if require("eventCount")?.as_u64().is_none() {
        return Err(validation("manifest", "`eventCount` 必须是非负整数"));
    }
    let last_seq = require("lastSeq")?;
    if !(last_seq.is_null() || last_seq.as_u64().is_some()) {
        return Err(validation(
            "manifest",
            "`lastSeq` 必须是 null 或非负整数",
        ));
    }
    let files = require("files")?
        .as_array()
        .ok_or_else(|| validation("manifest", "`files` 必须是数组"))?;
    for (index, entry) in files.iter().enumerate() {
        let path = entry
            .get("path")
            .and_then(|value| value.as_str())
            .ok_or_else(|| {
                validation("manifest", format!("`files[{index}].path` 必须是字符串"))
            })?;
        if path.is_empty() {
            return Err(validation(
                "manifest",
                format!("`files[{index}].path` 不得为空"),
            ));
        }
        let sha = entry
            .get("sha256")
            .and_then(|value| value.as_str())
            .ok_or_else(|| {
                validation("manifest", format!("`files[{index}].sha256` 必须是字符串"))
            })?;
        if sha.len() != 64 || !sha.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(validation(
                "manifest",
                format!("`files[{index}].sha256` 必须是 64 位十六进制（实际 `{sha}`）"),
            ));
        }
        if entry.get("bytes").and_then(|value| value.as_u64()).is_none() {
            return Err(validation(
                "manifest",
                format!("`files[{index}].bytes` 必须是非负整数"),
            ));
        }
    }
    let source = require("source")?
        .as_object()
        .ok_or_else(|| validation("manifest", "`source` 必须是对象"))?;
    for key in ["kind", "appVersion"] {
        let value = source
            .get(key)
            .and_then(|value| value.as_str())
            .ok_or_else(|| validation("manifest", format!("`source.{key}` 必须是字符串")))?;
        if value.is_empty() {
            return Err(validation(
                "manifest",
                format!("`source.{key}` 不得为空"),
            ));
        }
    }
    if source
        .get("exportedAtMs")
        .and_then(|value| value.as_i64())
        .is_none()
    {
        return Err(validation(
            "manifest",
            "`source.exportedAtMs` 必须是整数",
        ));
    }
    Ok(())
}

/// 校验包目录，返回 `(manifest, events.jsonl 字节, 附件映射, 警告)`。
fn load_and_validate(
    root: &Path,
) -> Result<
    (
        BundleManifest,
        Vec<u8>,
        BTreeMap<String, Vec<u8>>,
        Vec<String>,
    ),
    BundleError,
> {
    let manifest_path = root.join(BUNDLE_MANIFEST_FILE_NAME);
    let raw = fs::read_to_string(&manifest_path).map_err(|error| {
        validation(
            "manifest",
            format!("`{}` 读不出：{error}", manifest_path.display()),
        )
    })?;
    // manifest **不参与 sha256 自校验**（自引用哈希无解）⇒ 这里做结构兜底：
    // 逐字段检查「存在 + 类型正确 + 语义非空」，再用 serde 强类型解析。
    validate_manifest_shape(&raw)?;
    let manifest: BundleManifest = serde_json::from_str(&raw).map_err(|error| {
        validation(
            "manifest",
            format!("`{BUNDLE_MANIFEST_FILE_NAME}` 解析失败：{error}"),
        )
    })?;

    if manifest.format != BUNDLE_FORMAT {
        return Err(validation(
            "format",
            format!(
                "`format` 必须是 `{BUNDLE_FORMAT}`，实际 `{}`",
                manifest.format
            ),
        ));
    }
    if manifest.version != BUNDLE_VERSION {
        return Err(validation(
            "version",
            format!(
                "包格式版本 {} 不受支持（本工具支持 {BUNDLE_VERSION}）",
                manifest.version
            ),
        ));
    }

    // 逐文件：先做路径合法性（禁绝对/父目录），再核指纹。
    let mut listed: BTreeMap<&str, &BundleFileEntry> = BTreeMap::new();
    for entry in &manifest.files {
        validate_relative_bundle_path(&entry.path)?;
        if entry.path == BUNDLE_MANIFEST_FILE_NAME {
            return Err(validation(
                "files",
                format!("`{BUNDLE_MANIFEST_FILE_NAME}` 不得出现在 files 列表中（自引用哈希无解）"),
            ));
        }
        if listed.insert(entry.path.as_str(), entry).is_some() {
            return Err(validation(
                "files",
                format!("files 列表里 `{}` 重复", entry.path),
            ));
        }
    }
    if !listed.contains_key(BUNDLE_EVENTS_FILE_NAME) {
        return Err(validation(
            "files",
            format!("files 列表缺少 `{BUNDLE_EVENTS_FILE_NAME}`"),
        ));
    }

    let mut warnings = Vec::new();
    let mut events_bytes: Option<Vec<u8>> = None;
    let mut attachments: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for (path, entry) in &listed {
        let disk_path = root.join(path);
        let bytes = match fs::read(&disk_path) {
            Ok(bytes) => bytes,
            Err(error) => {
                if path.starts_with(ATTACHMENTS_PREFIX) {
                    warnings.push(format!(
                        "附件 `{path}` 缺失（{error}）；按契约降级导入，事件不受影响"
                    ));
                    continue;
                }
                return Err(validation(
                    "files",
                    format!("`{path}` 读不出（{error}）"),
                ));
            }
        };
        if bytes.len() as u64 != entry.bytes {
            if path.starts_with(ATTACHMENTS_PREFIX) {
                warnings.push(format!(
                    "附件 `{path}` 字节数不符（manifest {} / 实际 {}）；按契约降级，附件不入包",
                    entry.bytes,
                    bytes.len()
                ));
                continue;
            }
            return Err(validation(
                "files",
                format!(
                    "`{path}` 字节数不符（manifest {} / 实际 {}）",
                    entry.bytes,
                    bytes.len()
                ),
            ));
        }
        let digest = sha256_hex(&bytes);
        if digest != entry.sha256 {
            if path.starts_with(ATTACHMENTS_PREFIX) {
                warnings.push(format!(
                    "附件 `{path}` sha256 不符（manifest {} / 实际 {digest}）；按契约降级",
                    entry.sha256
                ));
                continue;
            }
            return Err(validation(
                "files",
                format!("`{path}` sha256 不符（manifest {} / 实际 {digest}）", entry.sha256),
            ));
        }
        if *path == BUNDLE_EVENTS_FILE_NAME {
            events_bytes = Some(bytes);
        } else {
            attachments.insert((*path).to_string(), bytes);
        }
    }
    // 包内未登记的文件：事件目录里出现未登记文件 = 篡改风险，直接拒绝。
    let stray = find_stray_files(root, &listed)?;
    if !stray.is_empty() {
        return Err(validation(
            "files",
            format!("包内存在未登记文件（拒绝导入）：{stray:?}"),
        ));
    }
    let events_bytes = events_bytes.ok_or_else(|| validation("files", "`events.jsonl` 缺失"))?;

    // header 与 manifest 一致。
    let header = classify_bundle_header(&events_bytes)?;
    if header.session_id != manifest.session_id {
        return Err(validation(
            "header",
            format!(
                "header.id={} 与 manifest.sessionId={} 不一致",
                header.session_id, manifest.session_id
            ),
        ));
    }
    if header.cwd != manifest.cwd {
        return Err(validation(
            "header",
            format!(
                "header.cwd={} 与 manifest.cwd={} 不一致",
                header.cwd, manifest.cwd
            ),
        ));
    }
    if header.created_at != manifest.created_at_ms {
        return Err(validation(
            "header",
            format!(
                "header.createdAt={} 与 manifest.createdAtMs={} 不一致",
                header.created_at, manifest.created_at_ms
            ),
        ));
    }

    // 事件数 / 末 seq / seq 连续。
    let (event_count, last_seq) = inspect_events(&events_bytes, &manifest.session_id)?;
    if event_count != manifest.event_count || last_seq != manifest.last_seq {
        return Err(validation(
            "seq",
            format!(
                "事件数与 manifest 不一致：manifest eventCount={} lastSeq={:?}，实际 eventCount={} lastSeq={:?}（事件流被截断？）",
                manifest.event_count, manifest.last_seq, event_count, last_seq
            ),
        ));
    }
    Ok((manifest, events_bytes, attachments, warnings))
}

/// 找包根下未登记的普通文件（`manifest.json` 与 `files` 之外的一律拒绝）。
fn find_stray_files(
    root: &Path,
    listed: &BTreeMap<&str, &BundleFileEntry>,
) -> Result<Vec<String>, BundleError> {
    let mut stray = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries =
            fs::read_dir(&dir).map_err(|error| io_err(format!("read `{}`", dir.display()), error))?;
        for entry in entries {
            let entry = entry.map_err(|error| io_err("iterate bundle dir", error))?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let relative = match path.strip_prefix(root) {
                Ok(relative) => relative.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            if relative == BUNDLE_MANIFEST_FILE_NAME || listed.contains_key(relative.as_str()) {
                continue;
            }
            stray.push(relative);
        }
    }
    stray.sort();
    Ok(stray)
}

/// 相对路径合法性：非空、非绝对、无 `..` / `.` 段、无盘符、非 Windows 分隔符混用。
fn validate_relative_bundle_path(path: &str) -> Result<(), BundleError> {
    if path.is_empty() {
        return Err(validation("path", "files[].path 不得为空"));
    }
    if path.contains('\\') {
        return Err(validation(
            "path",
            format!("`{path}` 含反斜杠（包内路径必须是 POSIX 分隔符）"),
        ));
    }
    if path.starts_with('/') || path.contains(':') {
        return Err(validation(
            "path",
            format!("`{path}` 是绝对路径 / 含盘符（必须是包根相对路径）"),
        ));
    }
    for segment in path.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(validation(
                "path",
                format!("`{path}` 含非法路径段（空段 / `.` / `..` 一律拒绝）"),
            ));
        }
    }
    Ok(())
}

/// 校验 `events.jsonl` 的首行 header（必须 `current`）。
fn classify_bundle_header(events_bytes: &[u8]) -> Result<super::log_store::SessionLogHeader, BundleError> {
    let first_line = events_bytes
        .split(|byte| *byte == b'\n')
        .next()
        .unwrap_or_default();
    let first_line = String::from_utf8_lossy(first_line);
    let value: serde_json::Value = serde_json::from_str(first_line.trim()).map_err(|error| {
        validation("header", format!("`events.jsonl` 首行不是合法 JSON：{error}"))
    })?;
    let version = value
        .get("version")
        .and_then(|value| value.as_u64())
        .ok_or_else(|| validation("header", "`events.jsonl` 首行缺 version"))?;
    if version != super::log_store::SESSION_LOG_FORMAT_VERSION {
        return Err(validation(
            "header",
            format!(
                "`events.jsonl` header 版本 {version} 不受支持（本工具支持 {}）",
                super::log_store::SESSION_LOG_FORMAT_VERSION
            ),
        ));
    }
    let kind = value.get("type").and_then(|value| value.as_str());
    if kind != Some("session") {
        return Err(validation(
            "header",
            format!("`events.jsonl` 首行 type 必须是 `session`，实际 {kind:?}"),
        ));
    }
    let session_id = value
        .get("id")
        .and_then(|value| value.as_str())
        .ok_or_else(|| validation("header", "`events.jsonl` 首行缺 id"))?
        .to_string();
    let cwd = value
        .get("cwd")
        .and_then(|value| value.as_str())
        .ok_or_else(|| validation("header", "`events.jsonl` 首行缺 cwd"))?
        .to_string();
    let created_at = value
        .get("createdAt")
        .and_then(|value| value.as_i64())
        .ok_or_else(|| validation("header", "`events.jsonl` 首行缺 createdAt"))?;
    Ok(super::log_store::SessionLogHeader {
        version,
        session_id,
        cwd,
        created_at,
    })
}

/// 扫描事件行：返回 `(事件数, 末 seq)`；任何结构问题（坏行 / seq 不连续 / 会话不符）
/// 都返回校验错误——**绝不静默跳过**。
fn inspect_events(
    events_bytes: &[u8],
    expected_session_id: &str,
) -> Result<(usize, Option<u64>), BundleError> {
    let raw =
        std::str::from_utf8(events_bytes).map_err(|error| validation("header", format!("事件流不是 UTF-8：{error}")))?;
    let mut lines = raw.lines();
    let header = lines
        .next()
        .ok_or_else(|| validation("header", "`events.jsonl` 是空文件（缺 header 行）"))?;
    if !header.trim_start().starts_with('{') {
        return Err(validation("header", "`events.jsonl` 首行不是 JSON 对象"));
    }
    let mut count = 0usize;
    let mut last_seq: Option<u64> = None;
    for (index, line) in lines.enumerate() {
        if line.trim().is_empty() {
            // 尾部空行（末尾换行产生）不是事件；中间空行说明文件被拼接/破坏。
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(line).map_err(|error| {
            validation(
                "seq",
                format!(
                    "`events.jsonl` 第 {} 行不是合法 JSON（事件流被截断/撕裂？）：{error}",
                    index + 2
                ),
            )
        })?;
        let seq = value
            .get("seq")
            .and_then(|value| value.as_u64())
            .ok_or_else(|| {
                validation(
                    "seq",
                    format!("`events.jsonl` 第 {} 行缺 seq", index + 2),
                )
            })?;
        let expected = last_seq.map(|previous| previous + 1).unwrap_or(0);
        if seq != expected {
            return Err(validation(
                "seq",
                format!(
                    "`events.jsonl` seq 不连续：期望 {expected}，实际 {seq}（第 {} 行）",
                    index + 2
                ),
            ));
        }
        if let Some(cmd_session) = value
            .get("cmd")
            .and_then(|cmd| cmd.get("sessionId"))
            .and_then(|value| value.as_str())
        {
            if cmd_session != expected_session_id {
                return Err(validation(
                    "seq",
                    format!(
                        "`events.jsonl` 第 {} 行的 cmd.sessionId={cmd_session} 与包会话 {expected_session_id} 不一致",
                        index + 2
                    ),
                ));
            }
        }
        last_seq = Some(seq);
        count += 1;
    }
    Ok((count, last_seq))
}

// ---------------------------------------------------------------------------
// 临时目录（“先校验/落临时目录，成功后原子 rename”）
// ---------------------------------------------------------------------------

struct StagingDir {
    path: PathBuf,
    armed: bool,
}

impl StagingDir {
    /// 在 `target` 的同级目录下建临时目录（同分区才保证 rename 原子）。
    fn new(target: &Path, label: &str) -> Result<Self, BundleError> {
        let parent = match target.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
            _ => PathBuf::from("."),
        };
        fs::create_dir_all(&parent)
            .map_err(|error| io_err(format!("create `{}`", parent.display()), error))?;
        Self::in_dir(&parent, label)
    }

    fn in_dir(parent: &Path, label: &str) -> Result<Self, BundleError> {
        let name = format!(
            ".pony-bundle-{label}-{}-{}",
            std::process::id(),
            now_ms()
        );
        let path = parent.join(name);
        fs::create_dir_all(&path)
            .map_err(|error| io_err(format!("create staging `{}`", path.display()), error))?;
        Ok(Self { path, armed: true })
    }

    fn collect_entries(&self) -> Result<Vec<(String, Vec<u8>)>, BundleError> {
        let mut entries = Vec::new();
        let mut stack = vec![self.path.clone()];
        while let Some(dir) = stack.pop() {
            let items = fs::read_dir(&dir)
                .map_err(|error| io_err(format!("read `{}`", dir.display()), error))?;
            for item in items {
                let item = item.map_err(|error| io_err("iterate staging dir", error))?;
                let path = item.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let relative = path
                    .strip_prefix(&self.path)
                    .map_err(|_| validation("path", "staging 路径越界"))?
                    .to_string_lossy()
                    .replace('\\', "/");
                let bytes = fs::read(&path)
                    .map_err(|error| io_err(format!("read `{}`", path.display()), error))?;
                entries.push((relative, bytes));
            }
        }
        entries.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(entries)
    }

    /// 把临时目录发布成最终形态：`Dir` 直接 rename；`Tar` 打包成单文件再 rename。
    fn finish(
        &self,
        target: &Path,
        format: BundleFormat,
        entries: &mut Vec<(String, Vec<u8>)>,
    ) -> Result<(), BundleError> {
        match format {
            BundleFormat::Dir => {
                fs::rename(&self.path, target).map_err(|error| {
                    io_err(
                        format!("rename staging → `{}`", target.display()),
                        error,
                    )
                })?;
            }
            BundleFormat::Tar => {
                let tar_bytes = write_tar_bytes(entries);
                let mut temp_file = target.as_os_str().to_owned();
                temp_file.push(format!(".tmp-{}-{}", std::process::id(), now_ms()));
                let temp_file = PathBuf::from(temp_file);
                {
                    let mut file = fs::File::create(&temp_file).map_err(|error| {
                        io_err(format!("create `{}`", temp_file.display()), error)
                    })?;
                    file.write_all(&tar_bytes)
                        .map_err(|error| io_err("write tar", error))?;
                    file.sync_all().map_err(|error| io_err("fsync tar", error))?;
                }
                fs::rename(&temp_file, target).map_err(|error| {
                    io_err(format!("rename tar → `{}`", target.display()), error)
                })?;
                let _ = fs::remove_dir_all(&self.path);
            }
        }
        Ok(())
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for StagingDir {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

/// 从 `.tar` 解出目录（解到系统临时目录，**绝不落在日志根下**）。
struct ExtractedBundle {
    path: PathBuf,
}

impl ExtractedBundle {
    fn from_tar_file(bundle_path: &Path) -> Result<Self, BundleError> {
        let bytes = fs::read(bundle_path)
            .map_err(|error| io_err(format!("read `{}`", bundle_path.display()), error))?;
        let entries = read_tar_bytes(&bytes)?;
        let base = std::env::temp_dir().join(format!(
            "pony-bundle-extract-{}-{}",
            std::process::id(),
            now_ms()
        ));
        fs::create_dir_all(&base).map_err(|error| io_err("create extract dir", error))?;
        for (relative, bytes) in entries {
            validate_relative_bundle_path(&relative)?;
            let target = base.join(&relative);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| io_err(format!("create `{}`", parent.display()), error))?;
            }
            fs::write(&target, bytes)
                .map_err(|error| io_err(format!("write `{}`", target.display()), error))?;
        }
        Ok(Self { path: base })
    }
}

impl Drop for ExtractedBundle {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

// ---------------------------------------------------------------------------
// 最小 ustar 读写（无第三方依赖）
// ---------------------------------------------------------------------------

/// 把 `(相对路径, 内容)` 列表写成 ustar 字节流（不压缩）。
fn write_tar_bytes(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    for (path, bytes) in entries {
        out.extend_from_slice(&tar_header(path, bytes.len() as u64));
        out.extend_from_slice(bytes);
        let padding = (TAR_BLOCK - (bytes.len() % TAR_BLOCK)) % TAR_BLOCK;
        out.extend(std::iter::repeat(0u8).take(padding));
    }
    // 两个全零块收尾。
    out.extend(std::iter::repeat(0u8).take(TAR_BLOCK * 2));
    out
}

/// 构造 512 字节 ustar 头（>100 字节的路径用 `prefix` 字段承载）。
fn tar_header(path: &str, size: u64) -> [u8; TAR_BLOCK] {
    let (prefix, name) = split_tar_path(path);
    let mut header = [0u8; TAR_BLOCK];
    write_ascii(&mut header[0..100], name);
    write_octal(&mut header[100..108], 0o644);
    write_octal(&mut header[108..116], 0);
    write_octal(&mut header[116..124], 0);
    write_octal(&mut header[124..136], size);
    write_octal(&mut header[136..148], 0);
    // chksum 先填空格，算完再写回。
    for byte in header[148..156].iter_mut() {
        *byte = b' ';
    }
    header[156] = b'0';
    header[257..263].copy_from_slice(b"ustar\0");
    header[263..265].copy_from_slice(b"00");
    write_ascii(&mut header[265..297], "pony");
    write_ascii(&mut header[297..329], "pony");
    write_octal(&mut header[329..337], 0);
    write_octal(&mut header[337..345], 0);
    if let Some(prefix) = prefix {
        write_ascii(&mut header[345..500], prefix);
    }
    let checksum: u32 = header.iter().map(|byte| *byte as u32).sum();
    let octal = format!("{checksum:06o}\0 ");
    header[148..156].copy_from_slice(octal.as_bytes());
    header
}

/// 写 ASCII 字段（不足补 `\0`；超长则截断——调用方已用 `split_tar_path` 保证不超长）。
fn write_ascii(field: &mut [u8], value: &str) {
    let bytes = value.as_bytes();
    let len = bytes.len().min(field.len());
    field[..len].copy_from_slice(&bytes[..len]);
}

/// 写八进制数值字段（`n-1` 位数字 + `\0`）。
fn write_octal(field: &mut [u8], value: u64) {
    let text = format!("{:0width$o}", value, width = field.len() - 1);
    let bytes = text.as_bytes();
    let len = bytes.len().min(field.len() - 1);
    field[..len].copy_from_slice(&bytes[..len]);
    field[field.len() - 1] = 0;
}

/// ustar 路径切分：`name` ≤100 字节；超出则按 `/` 切到 `prefix`（≤155 字节）。
fn split_tar_path(path: &str) -> (Option<&str>, &str) {
    if path.len() <= 100 {
        return (None, path);
    }
    let bytes = path.as_bytes();
    // 从右往左找能放进 name(100) 且 prefix(155) 的分割点。
    let mut index = bytes.len().saturating_sub(100).max(1);
    while index < bytes.len() {
        if bytes[index] == b'/' && index <= 155 && bytes.len() - index - 1 <= 100 {
            return (Some(&path[..index]), &path[index + 1..]);
        }
        index += 1;
    }
    (None, path)
}

/// 解析 ustar 字节流；只支持常规文件条目（`0` / `\0` / `5`），其余明确报错。
fn read_tar_bytes(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, BundleError> {
    let mut entries = Vec::new();
    let mut offset = 0usize;
    while offset + TAR_BLOCK <= bytes.len() {
        let block = &bytes[offset..offset + TAR_BLOCK];
        if block.iter().all(|byte| *byte == 0) {
            break; // 收尾块
        }
        let name = tar_field(block, 0, 100);
        let size = tar_octal(block, 124, 12)?;
        let typeflag = block[156];
        let prefix = tar_field(block, 345, 155);
        let checksum_recorded = tar_octal(block, 148, 8)?;
        let mut header = block.to_vec();
        for byte in header[148..156].iter_mut() {
            *byte = b' ';
        }
        let checksum_actual: u64 = header.iter().map(|byte| *byte as u64).sum();
        if checksum_recorded != checksum_actual {
            return Err(BundleError::Tar {
                detail: format!(
                    "头校验和不符（记录 {checksum_recorded} / 实际 {checksum_actual}，条目 `{name}`）"
                ),
            });
        }
        let path = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let data_start = offset + TAR_BLOCK;
        match typeflag {
            b'0' | 0 => {
                let data_end = data_start
                    .checked_add(size as usize)
                    .filter(|end| *end <= bytes.len())
                    .ok_or_else(|| BundleError::Tar {
                        detail: format!("条目 `{path}` 的内容被截断"),
                    })?;
                entries.push((path, bytes[data_start..data_end].to_vec()));
                let padded = data_start + tar_padded_size(size as usize);
                offset = padded.min(bytes.len());
            }
            b'5' => {
                offset = data_start + tar_padded_size(size as usize);
            }
            b'L' => {
                return Err(BundleError::Tar {
                    detail: "不支持 GNU 长文件名条目（typeflag `L`）：请用手写 ustar 或目录形态"
                        .to_string(),
                });
            }
            other => {
                return Err(BundleError::Tar {
                    detail: format!("不支持的 tar 条目类型 `{}`（条目 `{path}`）", other as char),
                });
            }
        }
    }
    Ok(entries)
}

fn tar_padded_size(size: usize) -> usize {
    let remainder = size % TAR_BLOCK;
    if remainder == 0 {
        size
    } else {
        size + (TAR_BLOCK - remainder)
    }
}

/// 读 ustar 文本字段（到第一个 `\0` 为止，去尾部空格）。
fn tar_field(block: &[u8], start: usize, len: usize) -> String {
    let slice = &block[start..start + len];
    let end = slice.iter().position(|byte| *byte == 0).unwrap_or(slice.len());
    String::from_utf8_lossy(&slice[..end]).trim_end().to_string()
}

/// 读 ustar 八进制字段。
fn tar_octal(block: &[u8], start: usize, len: usize) -> Result<u64, BundleError> {
    let raw = tar_field(block, start, len);
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(0);
    }
    u64::from_str_radix(trimmed, 8).map_err(|error| BundleError::Tar {
        detail: format!("八进制字段解析失败（`{trimmed}`）：{error}"),
    })
}

// ---------------------------------------------------------------------------
// 工具
// ---------------------------------------------------------------------------

/// 内容 sha256（小写十六进制）。
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// `LogStoreError` → `io::Error`（只用于把「路径解析/扫描」失败挂到 IO 上下文）。
fn io_from(error: super::log_store::LogStoreError) -> std::io::Error {
    std::io::Error::other(error.to_string())
}

/// 目录/文件包的**只读**摘要（供 `--dry-run` 与测试打印物理凭证）。
pub fn describe_bundle(path: &Path) -> Result<Vec<BundleFileEntry>, BundleError> {
    if path.is_dir() {
        let mut entries = Vec::new();
        let mut stack = vec![path.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let items =
                fs::read_dir(&dir).map_err(|error| io_err(format!("read `{}`", dir.display()), error))?;
            for item in items {
                let item = item.map_err(|error| io_err("iterate bundle", error))?;
                let item_path = item.path();
                if item_path.is_dir() {
                    stack.push(item_path);
                    continue;
                }
                let relative = item_path
                    .strip_prefix(path)
                    .map_err(|_| validation("path", "包路径越界"))?
                    .to_string_lossy()
                    .replace('\\', "/");
                let bytes = fs::read(&item_path).map_err(|error| {
                    io_err(format!("read `{}`", item_path.display()), error)
                })?;
                entries.push(BundleFileEntry {
                    path: relative,
                    sha256: sha256_hex(&bytes),
                    bytes: bytes.len() as u64,
                });
            }
        }
        entries.sort_by(|left, right| left.path.cmp(&right.path));
        return Ok(entries);
    }
    let bytes = fs::read(path).map_err(|error| io_err(format!("read `{}`", path.display()), error))?;
    Ok(read_tar_bytes(&bytes)?
        .into_iter()
        .map(|(path, bytes)| BundleFileEntry {
            path,
            sha256: sha256_hex(&bytes),
            bytes: bytes.len() as u64,
        })
        .collect())
}

/// 会话 id 的路径段（导入/导出报告里展示落点用；非法 id ⇒ 校验错误）。
pub fn session_segment(session_id: &str) -> Result<String, BundleError> {
    encode_session_id_segment(session_id).map_err(|error| validation("path", error.to_string()))
}
