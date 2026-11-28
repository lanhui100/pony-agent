//! `/resume` 会话列表与 provider/model 选择面板的视图模型（B1795831358336 Phase 2）。
//!
//! 从 `tui.rs` 整块切出的纯数据 + 纯格式化域：不依赖 `TuiState`，不触碰渲染与输入，
//! 因此可独立编译与单测（Locality of Change：改 provider 表单只开这一个文件）。
//!
//! 切出契约：类型名、字段名、字段顺序、`pub` 可见性逐字保留——`resume_list.rs` /
//! `model_panel.rs` / `model_window_editor.rs` 与外部 `pony_tui::tui::*` 消费者都按原路径导入。

#[derive(Debug, Clone)]
pub struct SessionItem {
    pub id: String,
    pub title: String,
    /// 已格式化的最近活动时间（同日 `HH:MM` / 同年 `MM-DD HH:MM` / 跨年 `YYYY-MM-DD`）。
    /// 契约 C1 起时间独占右列、轮次不再展示（轮次只保留在本字段以备其它入口读取）。
    pub updated_at: String,
    /// 真实用户轮数（已剔除宿主续轮提示）。
    pub turn_count: usize,
    /// 会话归属工作区 id（契约 C1 跨区前缀的判定输入；`None` = 默认工作区/未归属）。
    pub workspace_id: Option<String>,
    /// 工作区别名（契约 C1：缺失或空白时跨区前缀回落到 `workspace_id`）。
    pub workspace_label: Option<String>,
    /// 是否已归档（契约 C2：默认隐藏，列表内 `a` 切换显示）。
    pub archived: bool,
}

/// `/resume` 列表的最近活动时间显示口径：同日只显示时刻，同年显示月日+时刻，
/// 跨年显示日期。时间戳缺失/越界时回退为 `—`，绝不 panic。
pub(crate) fn format_session_row_time(updated_at_ms: u64) -> String {
    use chrono::{Datelike, Local, TimeZone};
    if updated_at_ms == 0 {
        return "—".to_string();
    }
    let Ok(updated_at_ms) = i64::try_from(updated_at_ms) else {
        return "—".to_string();
    };
    let Some(dt) = Local.timestamp_millis_opt(updated_at_ms).single() else {
        return "—".to_string();
    };
    let now = Local::now();
    if dt.date_naive() == now.date_naive() {
        dt.format("%H:%M").to_string()
    } else if dt.year() == now.year() {
        dt.format("%m-%d %H:%M").to_string()
    } else {
        dt.format("%Y-%m-%d").to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderItem {
    pub id: String,
    pub name: String,
    pub protocol: String,
    pub base_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelItem {
    pub id: String,
    pub name: String,
    /// `providers.json` 里显式声明的上下文窗口（`None` = 未声明，跟随能力预设）。
    /// 与 `context_window_tokens` 必须分开：面板要能说清「这个数是我填的，还是预设补的」。
    pub declared_context_window: Option<u32>,
    /// 生效窗口（声明值 > 预设补缺）；渲染直接用它，不在每帧读盘。
    pub context_window_tokens: Option<u32>,
}

impl ModelItem {
    /// 只带 id/name 的条目（窗口未知）。
    ///
    /// 供未接注册表的调用方与测试构造：窗口字段必须显式建模，但这里的「不知道」是合法状态
    /// （例如面板用空列表占位），不该逼每个调用方各写两个 `None`。
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            declared_context_window: None,
            context_window_tokens: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddingProviderStep {
    Name,
    BaseUrl,
    ApiKey,
    Commit,
}

#[derive(Debug, Clone, Default)]
pub struct AddProviderForm {
    pub name: String,
    pub base_url: String,
    pub api_key: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AddProviderPayload {
    pub id: String,
    pub name: String,
    pub protocol: String,
    pub base_url: String,
    pub api_key: Option<String>,
}

impl AddProviderPayload {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.id.trim().is_empty() {
            return Err("Provider ID 不能为空");
        }
        if self.name.trim().is_empty() {
            return Err("Provider 名称不能为空");
        }
        let url = self.base_url.trim();
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err("Base URL 必须是以 http:// 或 https:// 开头的合法地址");
        }
        match self.protocol.trim().to_lowercase().as_str() {
            "openai" | "anthropic" | "gemini" | "ollama" => Ok(()),
            _ => Err("不支持的协议类型"),
        }
    }
}
