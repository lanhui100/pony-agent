//! `/resume` 列表行渲染 + 归档动作的**纯函数契约层**（C1~C5）。
//!
//! **契约真源 = `.dev-team/contract-matrix-tui-resume-archive.md`**（C1 行格式 / C2 可见性 /
//! C3 空态计数 / C4 跨区提示 / C5 归档三态；验收断言 T1~T6）。行格式/可见性/裁决逻辑**只**
//! 落在本模块（纯函数），渲染与事件循环的接线在 `tui.rs`；不得在别处另写一份行格式。
//!
//! 纯度约束（铁律）：本模块**不得**触终端、时钟、文件、环境变量或任何全局状态；
//! 输入即全部信息，输出即结果（可用 `resume_row_parts` 在无 TTY 环境逐列断言）。

use super::{ellipsize_width, is_main_session_id, str_width};

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

/// 去空白后仍非空的 `Option<&str>`（C1/C4 的「非空」口径统一走这里，避免两处分歧）。
fn non_blank(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

/// C1 短 id：`session_id` 按 `-` 末段再取前 8 个 **char**（不按字节，CJK 不被切半个字）。
fn short_session_id(session_id: &str) -> String {
    session_id
        .rsplit('-')
        .next()
        .unwrap_or(session_id)
        .chars()
        .take(8)
        .collect()
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
pub fn resume_row_parts(input: &ResumeRowInput<'_>, width: usize) -> ResumeRowParts {
    // C1：`width == 0` ⇒ 左右皆空（不 panic、不越界）。
    if width == 0 {
        return ResumeRowParts::default();
    }

    // 右列 = 时间打头；仅 active 追加「 · 当前」、仅 selected 追加「 · <短id>」，顺序固定。
    // 轮次（`N 轮`）**不再进入**右列（旧实现作废，回归点见 T1）。
    let mut right = input.updated_at.to_string();
    if input.is_active {
        right.push_str(" · 当前");
    }
    if input.is_selected {
        right.push_str(" · ");
        right.push_str(&short_session_id(input.session_id));
    }

    // 跨区前缀：两个 id 去空白后都非空且不相等时才出现，`label` 空白回落 `workspace_id`。
    let mut left = String::new();
    if let (Some(workspace_id), Some(current_id)) = (
        non_blank(input.workspace_id),
        non_blank(input.current_workspace_id),
    ) {
        if workspace_id != current_id {
            let label = non_blank(input.workspace_label).unwrap_or(workspace_id);
            left.push_str(label);
            left.push_str(" · ");
        }
    }
    left.push_str(input.title);

    // 截断顺序是契约：右列先按 `width - 2` 收口，左列再按「剩余预算 - 1 个间隔」收口，
    // 保证 `宽度(left) + 1 + 宽度(right) ≤ width`。
    let right_budget = width.saturating_sub(2);
    let right = ellipsize_width(&right, right_budget);
    let left_budget = width.saturating_sub(str_width(&right) + 1);
    let left = ellipsize_width(&left, left_budget);

    ResumeRowParts { left, right }
}

/// C2：单行 `/resume` 可见性谓词（「这一行该不该出现在列表里」）。
///
/// 契约（`.dev-team/contract-matrix-tui-resume-archive.md` C2）：
/// `is_main_session_id(session_id) && !continuation_only && (show_archived || !archived)`。
/// - `subagent-*` / `teammate-*` 前缀恒不可见（空 id 按主会话处理，沿用 core 口径）；
/// - 只减行不改序：调用方按输入序（`updatedAt desc`）输出，**禁止**在本函数内排序/分组/去重。
pub fn session_visible(
    session_id: &str,
    archived: bool,
    continuation_only: bool,
    show_archived: bool,
) -> bool {
    // 口径逐字照契约 C2：主会话 && 非续轮碎片 && (显式看归档 || 未归档)。
    // 只做判定不做排序/分组/去重 —— 顺序由调用方按输入序保持。
    is_main_session_id(session_id) && !continuation_only && (show_archived || !archived)
}

/// C4：切换工作区后的提示文案（`None` = 不需要提示）。
///
/// 契约（`.dev-team/contract-matrix-tui-resume-archive.md` C4）：
/// 仅当 `old_id != new_id` 且 `new_id` 为 `Some(非空)` 时返回
/// `Some("已切换到工作区 <label>（<root_path>）")`；否则 `None`。
/// `<label>` = `label`（去空白后非空）否则回落 `new_id`；`root_path` 原样内嵌。
/// `old_id == None` 视为与任何有效 `new_id` 不相等。
pub fn workspace_switch_notice(
    old_id: Option<&str>,
    new_id: Option<&str>,
    label: Option<&str>,
    root_path: &str,
) -> Option<String> {
    // `new_id` 无效（None / 空白）⇒ 无从提示；`old_id == None` 视为与任何有效新 id 不相等。
    let new_id = non_blank(new_id)?;
    if non_blank(old_id) == Some(new_id) {
        return None;
    }
    let label = non_blank(label).unwrap_or(new_id);
    Some(format!("已切换到工作区 {label}（{root_path}）"))
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
pub fn archive_plan(is_streaming: bool, turn_in_flight: bool, arg: Option<&str>) -> ArchivePlan {
    // 阻塞优先：参数不参与判定（`/archive <id>` 同样被拦，C5 第 1 条）。
    if is_streaming || turn_in_flight {
        return ArchivePlan::BlockedDuringStreaming;
    }
    match non_blank(arg) {
        Some(target) => ArchivePlan::ArchiveOther(target.to_string()),
        None => ArchivePlan::ArchiveCurrentThenRotate,
    }
}
