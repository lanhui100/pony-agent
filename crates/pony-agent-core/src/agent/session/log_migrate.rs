//! ADR 0099 **阶段 3-B**：存量会话日志回填（grandfather 转正，方案 A）。
//!
//! ## 问题
//!
//! 真实库 139 个索引会话里只有 3 个有日志文件，其余都是「只有 blob、没有事件流」的
//! grandfather 会话 —— 对它们，「删掉索引行仍可读」这条承诺**还不成立**。
//!
//! ## 方案 A（用户批准）：从 blob **合成**一份日志
//!
//! 合成 = 把既有 `SessionState` 反推成 `PersistCommand` 流（顺序固定、可重放），写进
//! 临时目录后用 `fold` 折回来，**与 blob 状态逐字段比对**：一致才原子发布到正式路径。
//!
//! 三条不可妥协的规矩：
//!
//! 1. **合成流必须自证身份**：日志 header 写
//!    `source: {kind: "migrated-from-blob", migratedAtMs, appVersion}`（见
//!    [`super::log_store::SessionLogSource`]），正常写入路径**不写**该字段。合成事件是
//!    「按状态反推」的，不是原始过程事件；不标记就是伪装。
//! 2. **不一致就不落盘**：逐字段比对失败 ⇒ 该会话计入 `mismatched`，**不写日志、
//!    不改 blob**（原数据一个字节都不动），报告里列明差异。
//! 3. **保留 blob 作回滚面**：迁移**不删** `sessions` 行；出问题删掉日志目录即可回到
//!    老读路径（`load_store` 的 grandfather 分支）。
//!
//! ## 幂等
//!
//! 已有日志的会话直接跳过（`skipped_has_log`），所以第二次跑必然是
//! `migrated=0 / skipped_has_log=全部`。`force` 只允许**重跑上次自己的迁移产物**
//! （header `source.kind == migrated-from-blob`），**永不覆盖原始过程流**。
//!
//! ## 比对口径（哪些字段必须相等，哪些不比）
//!
//! 复用 [`super::fold::session_parity_issues`]（与 `rebuild_index` 的双读比对同一口径），
//! 它对 `trace_migration_state` / `turn_trace_refs` 已不比（那两个是写路径的**投影形态**，
//! 日志里没有承载它们的命令）。本模块再显式登记两条例外（[`MIGRATION_PARITY_EXEMPTIONS`]）：
//!
//! - `workspace_id`：日志格式**没有**承载它的命令（ADR 0099 阶段 2-D 已把它定为索引独有
//!   投影，读路径从索引行继承）⇒ 合成流不可能复现，不比；
//! - `last_commit_watermark`：日志里唯一的水位载体是 `cursor.event_watermark`，而 `fold`
//!   会把 `event_watermark` 与 `last_commit_watermark` 都对齐到它 ⇒ 该派生字段不比
//!   （`event_watermark` 本体**仍然比**，合成时写进 cursor）。
//!
//! 其余字段（history / title / turn_count / nodes / cursor / branches / traces / 记忆四件套
//! / transcript / evidence / 两个 watermark 中的 event_watermark）**逐个严格比对**。
//! 注意：`provider_native_transcript` / 记忆四件套 / `history_state_evidence` 目前**没有**
//! 事件载体 ⇒ 含这些状态的会话会如实落进 `mismatched`（不落盘），这是诚实结果，不是缺陷。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::backend::{PersistCommand, SessionBackend, SessionMetaPatch};
use super::fold::read_session_state;
use super::log_command::command_to_value;
use super::log_store::{now_ms, SessionLogMeta, SessionLogSource, SessionLogStore};
use super::types::SessionState;
use crate::agent::sqlite_session::SqliteSessionBackend;

/// 迁移比对时**显式豁免**的字段（理由见模块文档；其余字段一律严格比对）。
///
/// **只是 `fold::LOG_UNCARRIED_STATE_FIELDS` 的别名**：豁免清单只有一个定义，
/// `rebuild-index` 的 `mismatched` 与迁移共用它（避免「迁移说一致、重建说不一致」）。
pub const MIGRATION_PARITY_EXEMPTIONS: [&str; 2] = super::fold::LOG_UNCARRIED_STATE_FIELDS;

/// 迁移选项。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MigrateOptions {
    /// 只合成 + 校验 + 报告，**零写入**（日志根不出现任何新文件）。
    pub dry_run: bool,
    /// 允许**重跑自己的迁移产物**（header `source.kind == migrated-from-blob`）。
    /// 原始过程流永不被覆盖，即使加了 `force`。
    pub force: bool,
}

impl MigrateOptions {
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

/// 迁移报告（CLI 输出契约）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MigrateReport {
    /// 索引里的会话总数（`load_index_store` 的可见集合）。
    pub scanned: usize,
    /// 成功转正（本次真正写出日志）的会话。
    pub migrated: Vec<String>,
    /// 已有日志被跳过的会话数（幂等口径：第二次跑 == `scanned`）。
    pub skipped_has_log: usize,
    /// `force` 下被覆盖重写的**迁移产物**会话（原始过程流不计入此表）。
    pub remigrated: Vec<String>,
    /// 无法解析归属 cwd、退回**进程 cwd** 的会话（逐条记录，不静默）。
    pub cwd_fallback: Vec<String>,
    /// 无法忠实合成、因此**未落盘**的会话：`(session_id, 差异说明)`。
    pub mismatched: Vec<(String, String)>,
    /// 其它失败（IO / 锁 / 会话 id 非法等）：`(session_id, 原因)`。
    pub failed: Vec<(String, String)>,
    /// 是否只是预演。
    pub dry_run: bool,
}

impl MigrateReport {
    /// 汇总一行（CLI 与日志共用）。
    pub fn summary_line(&self) -> String {
        format!(
            "mode={} scanned={} migrated={} skipped_has_log={} remigrated={} cwd_fallback={} mismatched={} failed={}",
            if self.dry_run { "dry-run" } else { "apply" },
            self.scanned,
            self.migrated.len(),
            self.skipped_has_log,
            self.remigrated.len(),
            self.cwd_fallback.len(),
            self.mismatched.len(),
            self.failed.len()
        )
    }

    /// CLI 退出码：`0` 全部成功或无事可做；`5` 存在 mismatched / failed。
    pub fn exit_code(&self) -> i32 {
        if self.mismatched.is_empty() && self.failed.is_empty() {
            0
        } else {
            5
        }
    }

    /// 需要人工关注的会话（CLI 逐条打印）。
    pub fn problem_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for (session_id, reason) in &self.mismatched {
            lines.push(format!("mismatched {session_id}: {reason}"));
        }
        for (session_id, reason) in &self.failed {
            lines.push(format!("failed {session_id}: {reason}"));
        }
        lines
    }
}

// ---------------------------------------------------------------------------
// A. 合成器（纯函数：状态 → 命令流）
// ---------------------------------------------------------------------------

/// 从既有会话状态**反推**事件流（ADR 0099 阶段 3-B 方案 A）。
///
/// 顺序固定且幂等（同一状态必得同一命令序列）：
/// `PublishMetadata`（事件 schema 版本，最小必要）→ `UpdateSessionMeta` → 逐条
/// `AppendMessage`（ordinal 递增）→ 逐节点 `UpdateHistoryNode` → `UpdateCursor`
/// （水位写进 cursor）→ 逐分支 `UpdateBranch` → 逐 trace `AppendTrace`。
///
/// 为什么水位写进 cursor：日志格式里唯一的水位载体就是 `cursor.event_watermark`
/// （`fold` 用它同时得出 `event_watermark` 与 `last_commit_watermark`）。
///
/// 注意：本函数**只产出命令**，不决定成败——能否「忠实合成」由 [`migrate_session_state`]
/// 的逐字段比对判定。
pub fn synthesize_log_events(state: &SessionState, log_meta: &SessionLogMeta) -> Vec<PersistCommand> {
    let session_id = log_meta.session_id.clone();
    let mut commands: Vec<PersistCommand> = Vec::new();

    // 1) 最小必要元数据：事件 schema 版本（读路径的 validate_event_schema 需要它）。
    commands.push(PersistCommand::PublishMetadata {
        epoch: 1,
        key: "events.schema_version".to_string(),
        value: crate::agent::turn_event::EVENT_SCHEMA_VERSION.to_string(),
    });

    // 2) 会话级元数据：三态 patch（`Some(None)` 清空 / `Some(Some(v))` 赋值）。
    commands.push(PersistCommand::UpdateSessionMeta {
        epoch: 1,
        session_id: session_id.clone(),
        meta_patch: SessionMetaPatch {
            title: Some(state.title.clone()),
            title_override: Some(state.title_override.clone()),
            archived: Some(state.archived),
            summary: Some(state.summary.clone()),
            turn_count: Some(state.turn_count),
            last_referenced_file: state.last_referenced_file.clone(),
            updated_at_ms: Some(state.updated_at_ms),
        },
    });

    // 3) ADR 0102：transcript / 记忆四件套 / `history_state_evidence` 的载体。
    //
    // 位置：紧跟 `UpdateSessionMeta`（同属「会话级状态」），排在消息/图/trace 之前——这些
    // 命令彼此独立、fold 可任意重放，固定顺序只为让「合成流」与「真实写路径的命令次序」
    // 保持一致（meta → 会话级内容 → 消息 → 图 → trace），便于人工对读日志。
    //
    // 语义：transcript 是**追加**、记忆三项**整份替换**、evidence **追加**。日志此刻为空，
    // 整份状态恰好等于「一次追加 / 一次替换 + 一批追加」，不会重复累加。
    if !state.provider_native_transcript.is_empty() {
        commands.push(PersistCommand::AppendProviderTranscript {
            epoch: 1,
            session_id: session_id.clone(),
            messages: state.provider_native_transcript.clone(),
        });
    }
    let memory_present = !state.long_term_memory_entries.is_empty()
        || !state.memory_write_evidence.is_empty()
        || !state.memory_write_hook_trace_records.is_empty()
        || !state.history_state_evidence.is_empty();
    if memory_present {
        commands.push(PersistCommand::UpdateSessionMemory {
            epoch: 1,
            session_id: session_id.clone(),
            entries: state.long_term_memory_entries.clone(),
            evidence: state.memory_write_evidence.clone(),
            hook_trace_records: state.memory_write_hook_trace_records.clone(),
            history_state_evidence: state.history_state_evidence.clone(),
        });
    }

    // 4) 历史消息：ordinal 递增（fold 按 ordinal 排序，同 ordinal 用到达顺序）。
    for (ordinal, message) in state.history.iter().enumerate() {
        commands.push(PersistCommand::AppendMessage {
            epoch: 1,
            session_id: session_id.clone(),
            message: message.clone(),
            ordinal,
        });
    }

    // 5) 历史节点（含节点快照：refs 为空时 fold 会保留内嵌 trace）。
    for node in &state.history_nodes {
        commands.push(PersistCommand::UpdateHistoryNode {
            epoch: 1,
            session_id: session_id.clone(),
            node: node.clone(),
        });
    }

    // 6) 光标（水位写进 cursor —— 日志格式里唯一的载体）。
    let mut cursor = state.history_cursor.clone();
    cursor.session_id = session_id.clone();
    cursor.event_watermark = state.event_watermark;
    commands.push(PersistCommand::UpdateCursor {
        epoch: 1,
        session_id: session_id.clone(),
        cursor,
    });

    // 7) 分支。
    for branch in &state.history_branches {
        commands.push(PersistCommand::UpdateBranch {
            epoch: 1,
            session_id: session_id.clone(),
            branch: branch.clone(),
        });
    }

    // 8) trace（`trace_order` = 当前顺序，fold 按它排序）。
    for (trace_order, trace) in state.turn_trace_history.iter().enumerate() {
        commands.push(PersistCommand::AppendTrace {
            epoch: 1,
            session_id: session_id.clone(),
            trace: trace.clone(),
            trace_order,
        });
    }

    commands
}

// ---------------------------------------------------------------------------
// B. 迁移主流程
// ---------------------------------------------------------------------------

/// 把索引里**尚无日志**的会话转正为日志真源。
///
/// 逐会话：合成 → 写临时目录 → fold 回来逐字段比对 → 一致才发布。
pub fn migrate_grandfathered_sessions(
    backend: &SqliteSessionBackend,
    options: MigrateOptions,
) -> Result<MigrateReport, String> {
    // ADR 0107 §5 P1：破坏性维护（非 dry-run）必须独占 `<db>.lock`；抢不到 ⇒ 明确失败。
    // dry-run 零写入、不需要锁（也绝不能因为别的进程在做维护就被拒）。
    let _maintenance_guard = if options.dry_run {
        None
    } else {
        Some(
            crate::agent::sqlite_session::commit::acquire_maintenance_lock(backend.db_path())
                .map_err(|busy| busy.to_string())?,
        )
    };
    // 源状态取**运行时读路径**（`load_store`）：对 grandfather 会话它等于 blob 状态
    // （没有日志可合并）；对已有日志的会话我们本来就会跳过。
    // 刻意**不**用 `load_index_store`：那是「索引原样视图」的诊断口径，`session_read_from_log`
    // 有一条守卫用例钉死它只允许出现在 `sqlite_session.rs` 与 `fold.rs`（防止运行时路径
    // 误用索引视图），迁移属于运行时维护路径，必须走 `load_store`。
    //
    // **只做一次快照**（ADR 0099 阶段 3-B 缺陷 3）：循环中途再调 `load_store` 会把刚写好的
    // 日志合并进来，`skipped_has_log` / `migrated` 计数随写入顺序漂移（实测 dry-run 50 /
    // apply 52）。这里进循环前一次性取定 `(id, state)` 与 workspace 注册表。
    let source_store = backend
        .load_store()
        .ok_or_else(|| "load_store() 返回 None（会话库不可读）".to_string())?;
    let workspaces = source_store.workspaces.clone();
    let mut snapshot: Vec<(String, SessionState)> = source_store
        .sessions_view()
        .iter()
        .map(|(session_id, state)| (session_id.clone(), state.clone()))
        .collect();
    // 按 id 排序：报告与落盘顺序稳定（幂等断言与人工核对都靠它）。
    snapshot.sort_by(|left, right| left.0.cmp(&right.0));
    let log_root = backend.session_log_root();
    let mut report = MigrateReport {
        scanned: snapshot.len(),
        dry_run: options.dry_run,
        ..Default::default()
    };
    let app_version = env!("CARGO_PKG_VERSION");
    let migrated_at_ms = now_ms();

    // 暂存根：apply 用**日志根内部**的隐藏目录（同文件系统 ⇒ rename 原子）；dry-run 用
    // 系统临时目录（日志根必须零写入）。两者都按 cwd 分组 + 编码后的 id 布局。
    let staging_root = if options.dry_run {
        std::env::temp_dir().join(format!(
            "pony-migrate-dryrun-{}-{}",
            std::process::id(),
            migrated_at_ms
        ))
    } else {
        log_root.join(MIGRATION_STAGING_DIR_NAME)
    };
    let mut staging_guard = StagingGuard::new(staging_root.clone(), options.dry_run);

    for (session_id, state) in &snapshot {
        let session_id = session_id.as_str();
        // 已有日志 ⇒ 跳过（幂等）；force 只允许重跑自己的迁移产物。
        if backend.session_has_log(session_id) {
            match existing_log_source(&log_root, session_id) {
                Some(source) if source.is_migrated_from_blob() && options.force => {
                    // 落到下面重跑；旧产物会被原子覆盖。
                }
                _ => {
                    report.skipped_has_log += 1;
                    continue;
                }
            }
        }
        // 产物归属（缺陷 2）：按会话自身归属解析 cwd，而不是运行时的进程 cwd。
        let cwd = resolve_session_cwd(
            &state.workspace_id,
            session_id,
            &workspaces,
            &log_root,
            &mut report.cwd_fallback,
        );
        match migrate_one(
            state,
            session_id,
            &cwd,
            &staging_root,
            &log_root,
            app_version,
            migrated_at_ms,
            options,
        ) {
            Ok(outcome) => match outcome {
                MigrationOutcome::Migrated { replaced } => {
                    if replaced {
                        report.remigrated.push(session_id.to_string());
                    } else {
                        report.migrated.push(session_id.to_string());
                    }
                }
                MigrationOutcome::DryRunVerified { replaced } => {
                    if replaced {
                        report.remigrated.push(session_id.to_string());
                    } else {
                        report.migrated.push(session_id.to_string());
                    }
                }
            },
            Err(MigrationError::Mismatched { detail }) => {
                report.mismatched.push((session_id.to_string(), detail));
            }
            Err(MigrationError::Failed { detail }) => {
                report.failed.push((session_id.to_string(), detail));
            }
        }
    }
    staging_guard.cleanup();
    eprintln!(
        "[pony-agent][session] migrate_logs_summary: {}",
        report.summary_line()
    );
    Ok(report)
}

/// 暂存目录名（在日志根内，`cwd_groups` 只认 `--…--` ⇒ 它对 `list_sessions` 不可见）。
const MIGRATION_STAGING_DIR_NAME: &str = ".pony-migrate-staging";

/// 单会话迁移结果（成功路径的两种形态）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MigrationOutcome {
    /// 真正写入了正式路径。
    Migrated { replaced: bool },
    /// 预演：校验通过但没有落盘。
    DryRunVerified { replaced: bool },
}

/// 单会话迁移错误：`Mismatched` = 无法忠实合成（不落盘）；`Failed` = IO/锁等。
#[derive(Debug)]
enum MigrationError {
    Mismatched { detail: String },
    Failed { detail: String },
}

/// 迁移一个会话：合成 → 暂存写 → fold 比对 → （非 dry-run）原子发布。
#[allow(clippy::too_many_arguments)]
fn migrate_one(
    state: &SessionState,
    session_id: &str,
    cwd: &str,
    staging_root: &Path,
    log_root: &Path,
    app_version: &str,
    migrated_at_ms: i64,
    options: MigrateOptions,
) -> Result<MigrationOutcome, MigrationError> {
    let already_present = log_root
        .join(super::log_store::normalize_cwd_segment(&cwd))
        .join(
            super::log_store::encode_session_id_segment(session_id)
                .map_err(|error| MigrationError::Failed {
                    detail: format!("会话 id 非法：{error}"),
                })?,
        )
        .join(super::log_store::SESSION_LOG_FILE_NAME)
        .is_file();
    // 暂存目录（每次独立，避免上一次失败残留污染）。
    let staging_dir = staging_root.join(format!(
        "stage-{}-{}",
        std::process::id(),
        migrated_at_ms
    ));
    let _ = std::fs::remove_dir_all(&staging_dir);

    let meta = SessionLogMeta::new(session_id, cwd.to_string())
        .with_created_at(state.updated_at_ms as i64)
        .with_source(SessionLogSource::migrated_from_blob(
            app_version,
            migrated_at_ms,
        ));
    let commands = synthesize_log_events(state, &meta);
    let mut encoded: Vec<serde_json::Value> = Vec::with_capacity(commands.len());
    for command in &commands {
        encoded.push(command_to_value(command).map_err(|error| MigrationError::Failed {
            detail: format!("命令编码失败：{error}"),
        })?);
    }
    let staging_store = SessionLogStore::new(staging_dir.clone());
    {
        let mut writer = staging_store
            .open_write(session_id, &meta)
            .map_err(|error| MigrationError::Failed {
                detail: format!("暂存日志写入失败：{error}"),
            })?;
        writer
            .append_commands(now_ms(), &encoded)
            .map_err(|error| MigrationError::Failed {
                detail: format!("暂存事件追加失败：{error}"),
            })?;
    }

    // —— 逐字段比对：fold 回来 vs blob 状态 ——
    let folded = read_session_state(&staging_store, session_id).map_err(|error| {
        MigrationError::Mismatched {
            detail: format!("合成流 fold 失败：{error}"),
        }
    })?;
    if folded.deleted {
        return Err(MigrationError::Mismatched {
            detail: "合成流 fold 出墓碑（源状态不该含 RemoveSession）".to_string(),
        });
    }
    let expected = normalize_for_parity(state, session_id);
    let actual = normalize_for_parity(&folded.state, session_id);
    // 与 `rebuild-index` 的 mismatched **同一个**判定函数（豁免清单只有一处定义）。
    let issues: Vec<String> =
        super::fold::session_parity_issues_ignoring_uncarried(&expected, &actual);
    if !issues.is_empty() {
        let _ = std::fs::remove_dir_all(&staging_dir);
        return Err(MigrationError::Mismatched {
            detail: format!("合成流与 blob 状态不一致：{}", summarize_issues(&issues)),
        });
    }
    if options.dry_run {
        let _ = std::fs::remove_dir_all(&staging_dir);
        return Ok(MigrationOutcome::DryRunVerified {
            replaced: already_present,
        });
    }

    // —— 原子发布：把校验通过的日志文件 rename 到正式路径 ——
    let target_dir = SessionLogStore::new(log_root.to_path_buf())
        .session_dir(session_id, cwd)
        .map_err(|error| MigrationError::Failed {
            detail: format!("目标路径解析失败：{error}"),
        })?;
    std::fs::create_dir_all(&target_dir).map_err(|error| MigrationError::Failed {
        detail: format!("创建目标目录失败：{error}"),
    })?;
    let staged_log = staging_store.session_dir(session_id, cwd).map_err(|error| {
        MigrationError::Failed {
            detail: format!("暂存路径解析失败：{error}"),
        }
    })?;
    std::fs::rename(
        staged_log.join(super::log_store::SESSION_LOG_FILE_NAME),
        target_dir.join(super::log_store::SESSION_LOG_FILE_NAME),
    )
    .map_err(|error| MigrationError::Failed {
        detail: format!("发布日志失败（rename）：{error}"),
    })?;
    let _ = std::fs::remove_dir_all(&staging_dir);
    Ok(MigrationOutcome::Migrated {
        replaced: already_present,
    })
}

/// 差异摘要：**字段名 + 截断后的值**（单个会话的 transcript 差异可达数 MB，报告不能原样喷出）。
///
/// 规则：每条差异截到 [`ISSUE_PREVIEW_CHARS`] 字符，最多列 [`ISSUE_PREVIEW_COUNT`] 条，
/// 余下以「…（共 N 项差异）」收口——报告可读，且**不隐藏**差异规模。
const ISSUE_PREVIEW_CHARS: usize = 180;
const ISSUE_PREVIEW_COUNT: usize = 6;

fn summarize_issues(issues: &[String]) -> String {
    let mut parts: Vec<String> = issues
        .iter()
        .take(ISSUE_PREVIEW_COUNT)
        .map(|issue| {
            let taken: String = issue.chars().take(ISSUE_PREVIEW_CHARS).collect();
            if issue.chars().count() > ISSUE_PREVIEW_CHARS {
                format!("{taken}…")
            } else {
                taken
            }
        })
        .collect();
    if issues.len() > ISSUE_PREVIEW_COUNT {
        parts.push(format!(
            "…（共 {} 项差异，已截断显示前 {ISSUE_PREVIEW_COUNT} 项）",
            issues.len()
        ));
    }
    parts.join("; ")
}

/// 比对前的归一：`fold` 会把 `cursor.session_id` 归一为行键（既有口径），源侧做同一归一，
/// 否则会拿「空 session_id」和「行键」比出假差异。
///
/// 另外把 `last_commit_watermark` 对齐到 `event_watermark`：日志格式只有一个水位载体，
/// 该字段已在 [`MIGRATION_PARITY_EXEMPTIONS`] 里豁免，这里只是让差异不会二次放大。
fn normalize_for_parity(state: &SessionState, session_id: &str) -> SessionState {
    let mut normalized = state.clone();
    normalized.conversation_id = session_id.to_string();
    normalized.history_cursor.session_id = session_id.to_string();
    normalized.last_commit_watermark = normalized.event_watermark;
    normalized
}

/// 解析会话的**归属 cwd**（决定产物落在哪个 `--<cwd>--` 分组）。
///
/// 优先级（找不到就降级，且降级必须留痕）：
/// 1. 会话的 workspace → 注册表（`store_metadata.workspaces`）的 `root_path`。
///    **`workspace_id == None` 按产品既有归一化视为默认工作区**（与
///    `workspace::normalize_session_workspace_id` 同一口径：None → `default`），
///    否则真实库里 91 个未盖章的老会话会全部退化成进程 cwd（实测会产生 `--tmp--`）；
/// 2. 该 id **已有日志**的分组（`SessionLogStore::find_session_meta`）——保证重跑/迁移
///    不会把它挪到别的分组（缺陷 2 的核心：不得按运行时进程 cwd 伪造归属）；
/// 3. 退回进程 cwd，并把会话 id 记进 `cwd_fallback`（不静默）。
fn resolve_session_cwd(
    workspace_id: &Option<String>,
    session_id: &str,
    workspaces: &[crate::agent::workspace::WorkspaceRecord],
    log_root: &Path,
    cwd_fallback: &mut Vec<String>,
) -> String {
    let resolved_workspace_id = workspace_id
        .as_deref()
        .unwrap_or(crate::agent::workspace::DEFAULT_WORKSPACE_ID);
    if let Some(record) = workspaces
        .iter()
        .find(|record| record.id == resolved_workspace_id)
    {
        if !record.root_path.trim().is_empty() {
            return record.root_path.clone();
        }
    }
    if let Some(meta) = SessionLogStore::new(log_root.to_path_buf()).find_session_meta(session_id) {
        return meta.cwd;
    }
    cwd_fallback.push(session_id.to_string());
    super::log_sink::current_log_cwd()
}

/// 读某会话已有日志的 `source` 标记（无日志 / 读取失败 ⇒ `None`）。
fn existing_log_source(log_root: &Path, session_id: &str) -> Option<SessionLogSource> {
    let store = SessionLogStore::new(log_root.to_path_buf());
    // 跨分组查找（同 `session_has_log` 口径），避免进程 cwd 与创建分组不一致时漏判。
    let path = store.find_log_path(session_id)?;
    super::log_store::read_header_source(&path).ok().flatten()
}

/// 暂存根守卫：drop 时清理（dry-run 与 apply 都要清，避免残留）。
struct StagingGuard {
    root: PathBuf,
    cleanup_on_drop: bool,
    _created: bool,
}

impl StagingGuard {
    fn new(root: PathBuf, dry_run: bool) -> Self {
        Self {
            root,
            cleanup_on_drop: true,
            _created: dry_run,
        }
    }

    fn cleanup(&mut self) {
        if self.cleanup_on_drop {
            let _ = std::fs::remove_dir_all(&self.root);
            self.cleanup_on_drop = false;
        }
    }
}

impl Drop for StagingGuard {
    fn drop(&mut self) {
        if self.cleanup_on_drop {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
}

/// 迁移后核对：日志根下这些会话的 header 是否都带 `migrated-from-blob` 标记
/// （CLI/测试的审计入口；`None` = 会话无日志）。
pub fn migrated_session_ids(log_root: &Path) -> Vec<String> {
    let store = SessionLogStore::new(log_root.to_path_buf());
    let mut ids = BTreeSet::new();
    for meta in store.list_sessions().unwrap_or_default() {
        let Some(path) = store.find_log_path(&meta.session_id) else {
            continue;
        };
        if super::log_store::read_header_source(&path)
            .ok()
            .flatten()
            .is_some_and(|source| source.is_migrated_from_blob())
        {
            ids.insert(meta.session_id);
        }
    }
    ids.into_iter().collect()
}
