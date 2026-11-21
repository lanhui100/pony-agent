//! `/resume` 列表行渲染 + 归档动作的**纯函数契约层**（C1~C5）。
//!
//! **契约真源 = `.dev-team/contract-matrix-tui-resume-archive.md`**（C1 行格式 / C2 可见性 /
//! C3 空态计数 / C4 跨区提示 / C5 归档三态；验收断言 T1~T6）。本文件在冻结期只放**空桩**：
//! 每个函数体 = `todo!()`，实现归 Executor；任何逻辑都不得写在这里以外的"顺手位置"。
//!
//! 纯度约束（铁律）：本模块**不得**触终端、时钟、文件、环境变量或任何全局状态；
//! 输入即全部信息，输出即结果（可用 `resume_row_parts` 在无 TTY 环境逐列断言）。
//!
//! 过渡豁免：契约冻结期这些函数尚无调用者（`todo!()` 也不消费入参），接线完成后应删除本行。
#![allow(dead_code, unused_variables)]

/// 单行 `/resume` 渲染的**全部**输入（数据面 + 视图面 + 当前工作区基准）。
///
/// 字段口径与 `SessionItem` 加渲染期视图位一一对应；任何"当前会话是否活跃""是否选中"
/// 的判定都**不得**在本层偷看 `TuiState`，一律由调用方作为字段传入。
#[derive(Debug, Clone, Copy)]
pub struct ResumeRowInput<'a> {
    /// 会话 id（完整 id；短 id 由 `resume_row_parts` 内部按 `-` 末段取前 8 char）。
    pub session_id: &'a str,
    /// 会话标题（用户可见正文，未截断；截断口径见 C1）。
    pub title: &'a str,
    /// 会话归属工作区 id；`None` ⇒ 默认工作区（不参与跨区前缀判定）。
    pub workspace_id: Option<&'a str>,
    /// 工作区别名；缺失或空白时跨区前缀回落到 `workspace_id`。
    pub workspace_label: Option<&'a str>,
    /// 已格式化的最近活动时间（`HH:MM` / `MM-DD HH:MM` / `YYYY-MM-DD` / 缺失回退 `—`）。
    pub updated_at: &'a str,
    /// 是否已归档（可见性由 C2 决定；本字段只影响可见性，不进入行文本）。
    pub archived: bool,
    /// 是否是**当前**会话（为真时右列追加 ` · 当前`）。
    pub is_active: bool,
    /// 是否是**列表选中行**（为真时右列追加 ` · <短id>`）。
    pub is_selected: bool,
    /// 当前工作区 id（跨区前缀的基准；`None` ⇒ 不出现前缀）。
    pub current_workspace_id: Option<&'a str>,
}

/// 一行拆成左右两列：左列（跨区前缀 + 标题）、右列（时间 + 状态标记）。
///
/// 分开返回是为了让渲染层复用既有 `list_row(selected, &left, &right, width)`：
/// 列内截断与配色优先级各归其位，纯函数只负责"文本与宽度"。
/// （`Debug/PartialEq` 仅供参考实现与断言逐列比较，不构成行为契约。）
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResumeRowParts {
    /// 左列文本（跨区前缀 + 已截断标题，宽字符按 2 列计，溢出以 `…` 收尾）。
    pub left: String,
    /// 右列文本（时间打头，随后按 C1 追加 ` · 当前` / ` · <短id>`）。
    pub right: String,
}

/// C1：把一行 `/resume` 会话投影为「左列 + 右列」，宽度单位为**列**（宽字符按 2 计）。
///
/// 契约（逐条对应 `.dev-team/contract-matrix-tui-resume-archive.md` C1）：
/// - 右列 = 时间（`updated_at` 原样打头）；仅 `is_active` 追加 ` · 当前`；仅 `is_selected`
///   追加 ` · <短id>`，顺序固定为 `时间 · 当前 · 短id`；
/// - **轮次不得出现**（右列不得含 `轮`）；
/// - 跨区前缀 `<label> · ` 仅当 `workspace_id` 与 `current_workspace_id` 都非空且不相等时出现，
///   `label` 缺失/空白时回落 `workspace_id`，前缀属于左列；
/// - 短 id = `session_id` 按 `-` 末段取前 8 char；
/// - 截断：`right` 先按 `width.saturating_sub(2)` 收口，`left` 再按
///   `width.saturating_sub(宽度(right) + 1)` 收口，溢出统一以 `…` 收尾；
/// - 不变量：`宽度(left) + 1 + 宽度(right) ≤ width`（`right` 非空时）；`width == 0` ⇒ 两者皆空；
///   任意 `width` 与任意超长标题（>200 字）**不得 panic、不得越界**。
///
/// 待实现（Executor）：当前为冻结空桩。
pub fn resume_row_parts(input: &ResumeRowInput<'_>, width: usize) -> ResumeRowParts {
    todo!()
}

/// C2：单行 `/resume` 可见性谓词（「这一行该不该出现在列表里」）。
///
/// 契约（`.dev-team/contract-matrix-tui-resume-archive.md` C2）：
/// `is_main_session_id(session_id) && !continuation_only && (show_archived || !archived)`。
/// - `subagent-*` / `teammate-*` 前缀恒不可见（空 id 按主会话处理，沿用 core 口径）；
/// - 只减行不改序：调用方按输入序（`updatedAt desc`）输出，**禁止**在本函数内排序/分组/去重。
///
/// 待实现（Executor）：当前为冻结空桩。
pub fn session_visible(
    session_id: &str,
    archived: bool,
    continuation_only: bool,
    show_archived: bool,
) -> bool {
    todo!()
}

/// C4：切换工作区后的提示文案（`None` = 不需要提示）。
///
/// 契约（`.dev-team/contract-matrix-tui-resume-archive.md` C4）：
/// 仅当 `old_id != new_id` 且 `new_id` 为 `Some(非空)` 时返回
/// `Some("已切换到工作区 <label>（<root_path>）")`；否则 `None`。
/// `<label>` = `label`（去空白后非空）否则回落 `new_id`；`root_path` 原样内嵌。
/// `old_id == None` 视为与任何有效 `new_id` 不相等。
///
/// 待实现（Executor）：当前为冻结空桩。
pub fn workspace_switch_notice(
    old_id: Option<&str>,
    new_id: Option<&str>,
    label: Option<&str>,
    root_path: &str,
) -> Option<String> {
    todo!()
}

/// C5：`/archive` 的动作裁决（三态）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArchivePlan {
    /// 流式输出中或本轮轮次在飞：**任何参数都拒绝**（含 `/archive <id>`）。
    BlockedDuringStreaming,
    /// 归档指定会话 id（已 `trim`）；不换当前 session_id、不复位作用域。
    ArchiveOther(String),
    /// 无参归档：归档当前会话后**换新 session_id**（与 `/clear` 同法
    /// `new_session_id()` + `reset_session_scope()`），并同步 goal 续轮取消登记。
    ArchiveCurrentThenRotate,
}

/// C5：归档动作三态裁决（判定顺序即优先级）。
///
/// 契约（`.dev-team/contract-matrix-tui-resume-archive.md` C5）：
/// 1. `is_streaming || turn_in_flight` ⇒ [`ArchivePlan::BlockedDuringStreaming`]（参数不参与判定）；
/// 2. 否则 `arg.trim()` 非空 ⇒ [`ArchivePlan::ArchiveOther`]（携带 trim 后的 id）；
/// 3. 否则（`None` 或全空白）⇒ [`ArchivePlan::ArchiveCurrentThenRotate`]。
///
/// 阻塞态必须由调用方给出可见回执，不得静默吞掉。
///
/// 待实现（Executor）：当前为冻结空桩。
pub fn archive_plan(is_streaming: bool, turn_in_flight: bool, arg: Option<&str>) -> ArchivePlan {
    todo!()
}
