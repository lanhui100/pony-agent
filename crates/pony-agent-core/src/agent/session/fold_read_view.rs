//! B190：`fold` 的**读路径预算**——字节预检、降级判定与定向补折。
//!
//! 从 `fold.rs` 抽出（文件规模基线），抽出的是同一条判定链上的三件事：
//!
//! 1. 预算常量（[`DEFAULT_READ_FOLD_BUDGET`] / [`DEFAULT_READ_FOLD_LOG_BYTES`] /
//!    [`DEFAULT_READ_FOLD_SESSION_BYTES`]）；
//! 2. 全量折叠（[`fold_read_view`]）与它的字节预检降级（B047）；
//! 3. 预检降级后的定向补折（[`fold_missing_sessions_from_logs`]）与索引合并
//!    （[`merge_log_truth_into_sessions`]）（B190）。
//!
//! 折叠骨架（`fold_session` / `read_log_events` / `apply_index_projection`）仍在 `fold.rs`：
//! 本模块只决定「折谁、折多少、降级后补谁」，不复制任何落位规则。
//!
//! 父模块 `fold` 通过 `pub use` / `pub(crate) use` 保留原有寻址契约
//! （`fold::merge_log_truth_into_sessions` 等），调用方零改动。

use std::collections::{HashMap, HashSet};
use std::path::Path;

use super::fold::{apply_index_projection, fold_session, read_log_events};
use super::log_store::{SessionLogMeta, SessionLogStore};
use super::types::SessionState;

// ---------------------------------------------------------------------------
// ADR 0099 阶段 2-B：读路径切到日志真源
// ---------------------------------------------------------------------------

/// 读路径折叠的**耗时预算**（ADR 0099 阶段 2 门禁：启动装载 ≤800ms）。
///
/// 超预算不是错误，是**降级信号**：本轮保持索引视图（grandfather），记
/// `session_fold_slow` 日志，下轮再补。绝不因为「快了才可信」而在读路径上阻塞启动。
pub const DEFAULT_READ_FOLD_BUDGET: std::time::Duration = std::time::Duration::from_millis(800);

/// 读路径折叠的**日志字节预算**：折叠开始前先按日志总字节做一次预检，超出即整轮降级。
///
/// 为什么需要它（B047 实测）：`DEFAULT_READ_FOLD_BUDGET` 是**时**预算，超时后的语义是
/// 「整轮丢弃已折叠结果、回落索引视图」（见 [`fold_read_view`] 的不变量）。真实库
/// 682MB 日志 / 154 会话时，折叠读到 800ms 只折完 10 个会话就被丢弃——
/// `session_fold_slow: elapsed=880ms folded=10 skipped=144`，**这 880ms 是纯损耗**：
/// 折出来的东西一行都没用上，随后 `fill_index_only_rows_from_index_projection` 又把
/// 153 个会话从索引投影全量读一遍。
///
/// 换成字节预检后，同一份库的判定是**确定性**的（同一批日志两次装载给同一结论，不会
/// 因为机器快慢而跳变），且判定时**不花任何日志 IO**：总量超预算 ⇒ 直接按索引视图。
/// 索引视图的完整性由 `fill_index_only_rows_from_index_projection` 保证（它不受预算约束）。
///
/// 取值依据：实测读路径折叠吞吐 ≈ 100MB/s（10 会话 / 88MB / 880ms）⇒ 32MiB ≈ 320ms，
/// 既留出「小库照常走日志真源」的空间，也不让大库白付秒级代价。
pub const DEFAULT_READ_FOLD_LOG_BYTES: u64 = 32 * 1024 * 1024;

/// 定向补折的**单会话**字节上限（只在字节预检降级后使用，见
/// [`fold_missing_sessions_from_logs`]）。
///
/// 取 8MiB 的依据：读路径折叠吞吐实测 ≈ 100MB/s，单会话日志超过 8MiB 时它自己就要吃掉
/// 800ms 时预算的大半（B047 实测口径），补折的收益/成本比极差；这类超大日志交给显式
/// `pony resume <id>` 的定向装载（`load_session_targeted`）更划算。此处宁缺不慢。
pub const DEFAULT_READ_FOLD_SESSION_BYTES: u64 = 8 * 1024 * 1024;

/// 读路径折叠报告（供调用方记录 `session_fold_slow` 与装载耗时）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReadFoldOutcome {
    /// 成功 fold 且**有已提交命令**的会话（日志真源侧）。
    pub folded: Vec<String>,
    /// 折叠出 `RemoveSession` 墓碑的会话（D3：读路径视其为不存在）。
    pub deleted: Vec<String>,
    /// fold / 读日志失败的会话（`(session_id, 原因)`）：**保留索引视图**，不丢会话。
    pub failed: Vec<(String, String)>,
    /// 因预算耗尽而**跳过折叠**的会话（`degraded()` 为真 ⇔ 非空）。
    pub skipped_due_to_budget: Vec<String>,
    /// 日志文件存在但没有任何已提交命令（只有 header）的会话：**不覆盖索引行**。
    pub empty_log: Vec<String>,
    /// 本轮降级是否由**字节预检**判定（区别与时预算耗尽）。
    ///
    /// 只有预检降级会触发 [`fold_missing_sessions_from_logs`] 的定向补折：预检语义是
    /// 「一次日志 IO 都不做」，而「日志有、索引无」的会话**只有日志一个来源**——
    /// 整轮不读即等于永久丢失（`/resume` 里再也看不到，索引也永远不会自愈）。
    pub precheck_degraded: bool,
    /// 本轮折叠总耗时（毫秒）。
    pub elapsed_ms: u128,
}

impl ReadFoldOutcome {
    /// 是否触发了「超预算 ⇒ 降级为读索引」。
    pub fn degraded(&self) -> bool {
        !self.skipped_due_to_budget.is_empty()
    }
}

/// 日志根下各会话日志文件的字节总量（`stat` 级别，不读内容）。
///
/// 只服务 [`fold_read_view`] 的**字节预检**：判定「本轮折叠能否在预算内跑完」。
/// 单条 `stat` / 路径推导失败按 0 计——该会话会在折叠阶段按 `failed` 收口，预检不重复报错。
pub(crate) fn total_log_bytes(store: &SessionLogStore, metas: &[SessionLogMeta]) -> u64 {
    metas
        .iter()
        .filter_map(|meta| store.log_path(&meta.session_id, &meta.cwd).ok())
        .filter_map(|path| std::fs::metadata(path).ok())
        .map(|meta| meta.len())
        .sum()
}

/// 读路径的**日志侧视图**：扫日志根，逐会话 fold 出 `SessionState`。
///
/// 与 [`rebuild_index`] 的区别：这里**只读**、不写索引、单会话失败不中断整轮
/// （该会话保留索引视图，等价于「下轮再补」）。
///
/// 超预算时返回**空视图**并把名单记进 `skipped_due_to_budget`——降级语义是「本轮按索引
/// 读」，不是「读一半日志 + 一半索引」：半态会让 `load_store` 的同一份结果在两次调用
/// 之间跳变，而启动路径只需要「可读且完整」。
///
/// 为避免「读满 800ms 又被整轮丢弃」的纯损耗，进入循环前先做**字节预检**
/// （[`DEFAULT_READ_FOLD_LOG_BYTES`]）：日志总量超预算 ⇒ 一次日志 IO 都不做，直接降级。
pub fn fold_read_view(
    log_root: &Path,
    budget: std::time::Duration,
) -> (HashMap<String, SessionState>, ReadFoldOutcome) {
    fold_read_view_with_byte_budget(log_root, budget, DEFAULT_READ_FOLD_LOG_BYTES)
}

/// 与 [`fold_read_view`] 同义，字节预算由调用方给出（测试用：单测要能在不造 32MiB 夹具的
/// 前提下驱动预检分支）。
pub(crate) fn fold_read_view_with_byte_budget(
    log_root: &Path,
    budget: std::time::Duration,
    byte_budget: u64,
) -> (HashMap<String, SessionState>, ReadFoldOutcome) {
    let started = std::time::Instant::now();
    let store = SessionLogStore::new(log_root.to_path_buf());
    let mut outcome = ReadFoldOutcome::default();
    let metas = match store.list_sessions() {
        Ok(metas) => metas,
        Err(error) => {
            // 扫不动日志根（权限 / IO）⇒ 整轮降级为读索引，不阻塞启动。
            outcome.skipped_due_to_budget.push(format!("<log-root: {error}>"));
            return (HashMap::new(), outcome);
        }
    };
    // 预算为 0 ⇒ 立即降级（测试与「极端保护」入口用同一条判定，不做特殊分支）。
    let mut over_budget = budget.is_zero();
    // B047 预检：日志总量超出字节预算 ⇒ 折叠注定无法在时预算内完成，而超时的语义是
    // **整轮丢弃**。此时连一次日志读都不做，直接整轮降级为读索引（确定性、零损耗）。
    let log_bytes_total = total_log_bytes(&store, &metas);
    if log_bytes_total > byte_budget {
        outcome.precheck_degraded = true;
        outcome.skipped_due_to_budget = metas
            .iter()
            .map(|meta| meta.session_id.clone())
            .collect();
        outcome.elapsed_ms = started.elapsed().as_millis();
        eprintln!(
            "[pony-agent][session] session_fold_precheck: log_bytes={}MB byte_budget={}MB sessions={} — 日志总量超出读路径折叠字节预算，本轮整轮按索引视图（不读日志）",
            log_bytes_total / (1024 * 1024),
            byte_budget / (1024 * 1024),
            metas.len()
        );
        return (HashMap::new(), outcome);
    }
    let mut folded: HashMap<String, SessionState> = HashMap::new();
    for meta in metas {
        if over_budget || started.elapsed() > budget {
            over_budget = true;
            outcome.skipped_due_to_budget.push(meta.session_id.clone());
            continue;
        }
        fold_meta_into_view(&store, &meta, &mut folded, &mut outcome);
    }
    outcome.elapsed_ms = started.elapsed().as_millis();
    if outcome.degraded() {
        eprintln!(
            "[pony-agent][session] session_fold_slow: elapsed={}ms budget={}ms folded={} skipped={} — 降级为读索引（下轮再补）",
            outcome.elapsed_ms,
            budget.as_millis(),
            outcome.folded.len(),
            outcome.skipped_due_to_budget.len()
        );
        // 降级即整轮回落索引视图（见上方「半态」说明）。
        outcome.folded.clear();
        return (HashMap::new(), outcome);
    }
    (folded, outcome)
}

/// 折一个会话日志，把结果收进 `view` 与 `outcome`。
///
/// 全量折叠（[`fold_read_view_with_byte_budget`]）与定向补折
/// （[`fold_missing_sessions_from_logs`]）共用这一条落位规则：落位规则只写一份，
/// 两条读路径对同一份日志给出的状态才不会分叉。
fn fold_meta_into_view(
    store: &SessionLogStore,
    meta: &SessionLogMeta,
    view: &mut HashMap<String, SessionState>,
    outcome: &mut ReadFoldOutcome,
) {
    let session_id = meta.session_id.clone();
    let events = match read_log_events(store, meta) {
        Ok(events) => events,
        Err(error) => {
            outcome
                .failed
                .push((session_id, format!("read log: {error}")));
            return;
        }
    };
    if events.is_empty() {
        // 只有 header：日志还没有任何已提交命令。用它覆盖索引行会把真实历史读成空。
        outcome.empty_log.push(session_id);
        return;
    }
    match fold_session(&events) {
        Ok(result) if result.deleted => outcome.deleted.push(session_id),
        Ok(result) => {
            let mut state = result.state;
            if state.conversation_id.is_empty() {
                state.conversation_id = session_id.clone();
            }
            outcome.folded.push(session_id.clone());
            view.insert(session_id, state);
        }
        Err(error) => outcome
            .failed
            .push((session_id, format!("fold: {error}"))),
    }
}

/// 字节预检降级后的**定向补折**：只折「日志里有、视图里没有」的会话。
///
/// 为什么需要它：`/resume` 的列表来自索引视图，而 `known` 之外的会话**只有日志一个来源**。
/// 本机实测日志总量 1583MB ≫ 32MiB 预检预算 ⇒ 预检每轮都整轮降级、一次日志都不读，
/// 于是这些会话在 `/resume` 里永久消失（索引也不会自愈：没有任何路径把它们写回索引）。
/// 补折把它们按「日志为唯一来源」的唯一副本装载回来，成本有明确上界：
///
/// - **近者优先**：按日志 mtime 降序，预算耗尽时丢掉的是最老的会话；
/// - **单会话上限**（`per_session_bytes`）：超大日志不补（见
///   [`DEFAULT_READ_FOLD_SESSION_BYTES`]）；
/// - **总字节上限**（`byte_budget`）与**时预算**（`budget`）：与全量折叠同一口径。
///
/// 与全量折叠的降级语义一致：预算内能折的会话结果完整，超出的记进
/// `skipped_due_to_budget`（下轮再补），绝不返回「同一会话折一半」的半态。
pub(crate) fn fold_missing_sessions_from_logs(
    log_root: &Path,
    known: &HashSet<String>,
    budget: std::time::Duration,
    byte_budget: u64,
    per_session_bytes: u64,
) -> (HashMap<String, SessionState>, ReadFoldOutcome) {
    let started = std::time::Instant::now();
    let store = SessionLogStore::new(log_root.to_path_buf());
    let mut outcome = ReadFoldOutcome::default();
    let mut view: HashMap<String, SessionState> = HashMap::new();
    let metas = match store.list_sessions() {
        Ok(metas) => metas,
        Err(error) => {
            // 扫不动日志根：与全量折叠同口径，不阻塞启动、不丢索引视图。
            outcome
                .failed
                .push(("<log-root>".to_string(), error.to_string()));
            return (view, outcome);
        }
    };
    let mut candidates: Vec<(u128, u64, SessionLogMeta)> = Vec::new();
    for meta in metas {
        if known.contains(&meta.session_id) {
            continue;
        }
        let Ok(path) = store.log_path(&meta.session_id, &meta.cwd) else {
            continue;
        };
        let Ok(stat) = std::fs::metadata(&path) else {
            continue;
        };
        candidates.push((log_modified_ms(&stat), stat.len(), meta));
    }
    candidates.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.2.session_id.cmp(&right.2.session_id))
    });
    let mut used_bytes = 0u64;
    for (_, bytes, meta) in candidates {
        if started.elapsed() > budget
            || bytes > per_session_bytes
            || used_bytes.saturating_add(bytes) > byte_budget
        {
            outcome.skipped_due_to_budget.push(meta.session_id);
            continue;
        }
        used_bytes = used_bytes.saturating_add(bytes);
        fold_meta_into_view(&store, &meta, &mut view, &mut outcome);
    }
    outcome.elapsed_ms = started.elapsed().as_millis();
    (view, outcome)
}

/// 日志文件的 mtime（Unix 毫秒）；取不到按 0 计（排序退化为按 id，仍确定）。
fn log_modified_ms(stat: &std::fs::Metadata) -> u128 {
    stat.modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

/// 把**日志真源**合并进索引视图（`load_store` 的读切换）。
///
/// 合并规则（ADR 0099 D1 / 阶段 2 / 2-D）：
/// 1. 有日志的会话 ⇒ **日志无条件为准**（同 id 覆盖索引行）。阶段 2-D 停写整包 blob 后，
///    索引行只剩索引字段（`__index_only` 轻量行），日志是唯一内容来源；原先那条
///    「索引 `updated_at_ms` 更新就保留索引」的过渡定序已删除——留着它会把轻量行
///    （内容为空、时间戳却随写路径前进）当成比日志更新，从而把会话读成空壳；
/// 2. 无日志的会话 ⇒ 保持索引视图（存量会话，grandfather，不得丢失）；
/// 3. 墓碑（`RemoveSession`）⇒ 从视图里移除（D3：删除不复活）；
/// 4. 折叠失败的会话 ⇒ 保持索引视图（不丢会话，下一轮再补）；
/// 5. fold 无法推导的索引侧字段（`INDEX_ONLY_STATE_FIELDS` + `workspace_id`）从索引行继承，
///    否则读切换会凭空丢掉「日志不承载」的真实状态。
pub fn merge_log_truth_into_sessions(
    log_root: &Path,
    sessions: &mut HashMap<String, SessionState>,
) -> ReadFoldOutcome {
    merge_log_truth_into_sessions_with_budget(log_root, sessions, DEFAULT_READ_FOLD_BUDGET)
}

/// 与 [`merge_log_truth_into_sessions`] 同语义，但预算由调用方给出（测试/维护入口用）。
pub fn merge_log_truth_into_sessions_with_budget(
    log_root: &Path,
    sessions: &mut HashMap<String, SessionState>,
    budget: std::time::Duration,
) -> ReadFoldOutcome {
    merge_log_truth_into_sessions_with_caps(
        log_root,
        sessions,
        budget,
        DEFAULT_READ_FOLD_LOG_BYTES,
        DEFAULT_READ_FOLD_LOG_BYTES,
        DEFAULT_READ_FOLD_SESSION_BYTES,
    )
}

/// 与 [`merge_log_truth_into_sessions_with_budget`] 同语义，三项预算全部由调用方给出。
///
/// 测试/维护入口：单测要能在不造 32MiB 夹具的前提下分别驱动「预检降级」与
/// 「定向补折的单会话/总量上限」两条分支。
pub(crate) fn merge_log_truth_into_sessions_with_caps(
    log_root: &Path,
    sessions: &mut HashMap<String, SessionState>,
    budget: std::time::Duration,
    precheck_byte_budget: u64,
    targeted_byte_budget: u64,
    per_session_bytes: u64,
) -> ReadFoldOutcome {
    // 生产路径走「默认字节预算」的入口（`fold_read_view`）；只有测试注入非默认预算时
    // 才直接调带参版本——两条分支的判定逻辑同一份，不存在第二套预检。
    let (folded, mut outcome) = if precheck_byte_budget == DEFAULT_READ_FOLD_LOG_BYTES {
        fold_read_view(log_root, budget)
    } else {
        fold_read_view_with_byte_budget(log_root, budget, precheck_byte_budget)
    };
    apply_folded_into_sessions(folded, &outcome.deleted, sessions);
    if outcome.precheck_degraded {
        // 预检降级 ⇒ 一次日志都不读。索引视图之外的会话**只有日志一个来源**，
        // 不补折就永久消失（`/resume` 看不到，索引也不会自愈）。
        let known: HashSet<String> = sessions.keys().cloned().collect();
        let (targeted, targeted_outcome) = fold_missing_sessions_from_logs(
            log_root,
            &known,
            budget,
            targeted_byte_budget,
            per_session_bytes,
        );
        let targeted_folded = targeted_outcome.folded.clone();
        apply_folded_into_sessions(targeted, &targeted_outcome.deleted, sessions);
        outcome.folded.extend(targeted_outcome.folded);
        outcome.deleted.extend(targeted_outcome.deleted);
        outcome.failed.extend(targeted_outcome.failed);
        outcome.empty_log.extend(targeted_outcome.empty_log);
        outcome
            .skipped_due_to_budget
            .extend(targeted_outcome.skipped_due_to_budget);
        // 名单语义收敛：补折成功的会话不算「本轮未读」，重复项去重（预检曾把它们全量列进去）。
        if !targeted_folded.is_empty() {
            outcome
                .skipped_due_to_budget
                .retain(|session_id| !targeted_folded.contains(session_id));
        }
        let mut seen: HashSet<String> = HashSet::new();
        outcome
            .skipped_due_to_budget
            .retain(|session_id| seen.insert(session_id.clone()));
        outcome.elapsed_ms = outcome.elapsed_ms.saturating_add(targeted_outcome.elapsed_ms);
        eprintln!(
            "[pony-agent][session] session_fold_targeted_fill: folded={} skipped={} elapsed={}ms — 预检降级后补折「日志有、索引无」的会话（近者优先）",
            targeted_folded.len(),
            outcome.skipped_due_to_budget.len(),
            outcome.elapsed_ms
        );
    }
    outcome
}

/// 把折叠结果落进会话视图：索引里没有 ⇒ 日志为唯一来源；有 ⇒ 按索引投影规则合并。
///
/// 全量折叠与定向补折共用本函数：合并规则只写一份，两条路径才对同一会话给出同一结果。
fn apply_folded_into_sessions(
    folded: HashMap<String, SessionState>,
    deleted: &[String],
    sessions: &mut HashMap<String, SessionState>,
) {
    for (session_id, state) in folded {
        match sessions.get(&session_id) {
            None => {
                // 索引里没有这一行 ⇒ 日志是唯一来源（阶段 2-B 的核心兑现点）。
                sessions.insert(session_id, state);
            }
            Some(indexed) => {
                sessions.insert(session_id, apply_index_projection(indexed, state));
            }
        }
    }
    for session_id in deleted {
        sessions.remove(session_id);
    }
}
