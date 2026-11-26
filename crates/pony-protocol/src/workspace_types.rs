//! Workspace 注册表（PA-079）：WorkspaceRecord + 注册逻辑（create/list/get/default）。
//! 持久化由调用方（SessionStore）经 PersistedStore / store_metadata 完成，本模块保持纯数据 + 校验。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 默认 workspace id（保留字，豁免 `ws-<slug>` 规则；PA-081 前端消费同一常量）。
pub const DEFAULT_WORKSPACE_ID: &str = "default";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRecord {
    /// 稳定 id：默认 `"default"`，其余 `ws-<slug>-<ts>`
    pub id: String,
    pub name: String,
    /// canonical 绝对路径（Windows 去 `\\?\` 前缀）
    pub root_path: String,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn slugify(name: &str) -> String {
    let mut slug = String::new();
    for ch in name.trim().to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_matches('-').to_string()
}

/// Windows canonicalize 输出去 `\\?\` / `\\?\UNC\` 前缀，统一为可读绝对路径。
pub fn normalize_win_prefix(path: &str) -> String {
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    if let Some(rest) = path.strip_prefix(r"\\?\") {
        return rest.to_string();
    }
    path.to_string()
}

/// 规范化 workspace root：trim → canonicalize（存在）→ 必须是目录 → Windows 去前缀。
pub fn normalize_workspace_root(root: &str) -> Result<String, String> {
    let trimmed = root.trim();
    if trimmed.is_empty() {
        return Err("workspace root 不能为空".into());
    }
    let input = PathBuf::from(trimmed);
    let canonical = input
        .canonicalize()
        .map_err(|error| format!("workspace root 不可解析：{error}"))?;
    if !canonical.is_dir() {
        return Err("workspace root 不是目录".into());
    }
    Ok(normalize_win_prefix(&canonical.display().to_string()))
}

/// 校验 workspace id（默认保留字或 `ws-<slug>`）。
pub fn validate_workspace_id(id: &str) -> Result<String, String> {
    let trimmed = id.trim();
    if trimmed == DEFAULT_WORKSPACE_ID {
        return Ok(trimmed.to_string());
    }
    if trimmed.len() > 3
        && trimmed.starts_with("ws-")
        && !trimmed.contains('/')
        && !trimmed.contains('\\')
    {
        return Ok(trimmed.to_string());
    }
    Err(format!("workspace id 非法：{id}"))
}

/// Windows 上 root 比较大小写不敏感（canonicalize 不统一大小写，`C:\WS` vs `c:\ws` 应判重）；
/// Unix 上大小写敏感。
pub fn roots_match(left: &str, right: &str) -> bool {
    if cfg!(windows) {
        left.to_lowercase() == right.to_lowercase()
    } else {
        left == right
    }
}

/// 默认 workspace 是否存在于注册表；缺则返回"待创建"占位。
pub fn default_workspace_exists(records: &[WorkspaceRecord]) -> bool {
    records.iter().any(|record| record.id == DEFAULT_WORKSPACE_ID)
}

/// 解析默认工作区基目录（纯逻辑，无 IO，可注入 `windows` 与 `path_exists` 便于确定性测试）。
/// 返回的 base 最终由调用方 `join("pony_agent")` 得到默认工作区根目录。
///
/// `windows == true`（Windows 形态）：
/// - `document_dir Some` → `Some(document_dir)`
/// - 否则 `home_dir Some` 且 `path_exists(home_dir/Documents)` → `Some(home_dir/Documents)`
/// - 否则 `home_dir Some` → `Some(home_dir)`
/// - 全 None → `None`
/// `windows == false`（Unix 形态）：`home_dir Some → Some(home_dir)`；`None → None`。
///
/// R2-1：以参数而非 `cfg!(windows)` 表达平台分支，使 Windows 分支可在 Linux 上编译与测。
pub fn resolve_default_workspace_base(
    document_dir: Option<&Path>,
    home_dir: Option<&Path>,
    windows: bool,
    path_exists: &dyn Fn(&Path) -> bool,
) -> Option<PathBuf> {
    if !windows {
        return home_dir.map(Path::to_path_buf);
    }
    if let Some(document) = document_dir {
        return Some(document.to_path_buf());
    }
    if let Some(home) = home_dir {
        let documents = home.join("Documents");
        // 仅当 home/Documents 实际存在时才采用该形态（存在性由 path_exists 注入判定）；
        // 否则回退到 home 本身。
        if path_exists(&documents) {
            return Some(documents);
        }
        return Some(home.to_path_buf());
    }
    None
}

/// 默认工作区根目录（三级树安装期行为）：
/// - Windows：`...\Documents\pony_agent`（Known-Folder 解析，尊重 OneDrive 重定向）
/// - macOS/Linux：`$HOME/pony_agent`
/// 纯逻辑解析（复用 `resolve_default_workspace_base`），目录创建由 `bootstrap` 负责；
/// dirs 全失败返回 None，由调用方决定兜底策略（不再静默回退进程 cwd）。
pub fn compute_default_workspace_root() -> Option<PathBuf> {
    let base = resolve_default_workspace_base(
        dirs::document_dir().as_deref(),
        dirs::home_dir().as_deref(),
        cfg!(windows),
        &|candidate| candidate.is_dir(),
    )?;
    Some(base.join("pony_agent"))
}

/// 确保注册表存在默认工作区记录且其根目录可用：
/// 记录缺失 → 以 Documents~/pony_agent（不存在即建）登记并返回 true；
/// 已存在则不动（用户既有登记永不静默迁移）。调用方负责落盘。
pub fn bootstrap_default_workspace(records: &mut Vec<WorkspaceRecord>) -> bool {
    bootstrap_default_workspace_with_base(records, None)
}

/// 测试/注入变体：`base_override` 显式给定时跳过系统 Known-Folder 解析（不触碰真实用户目录）。
///
/// R2 修复：base 解析失败（系统 dirs 全 None）→ 传入 `None` 给 inner → 返回 false 且不登记
/// （删除旧的"回退进程 cwd 并持久化"分支——安装包进程 cwd=AppData 会产生错误默认工作区）。
pub fn bootstrap_default_workspace_with_base(
    records: &mut Vec<WorkspaceRecord>,
    base_override: Option<PathBuf>,
) -> bool {
    let resolved_base = match base_override {
        Some(base) => Some(base),
        None => resolve_default_workspace_base(
            dirs::document_dir().as_deref(),
            dirs::home_dir().as_deref(),
            cfg!(windows),
            &|candidate| candidate.is_dir(),
        ),
    };
    bootstrap_default_workspace_inner(records, resolved_base)
}

/// 内部登记层（R2-2/R2-5）：`resolved_base: None`（解析失败）→ 返回 false 且不注册、无任何
/// 回退；`Some(base)` → 创建 `base/pony_agent`，创建失败 → false（记日志），成功 → 注册 default
/// 记录并返回 true。幂等：已有 default 记录时返回 false 且不动注册表。
pub fn bootstrap_default_workspace_inner(
    records: &mut Vec<WorkspaceRecord>,
    resolved_base: Option<PathBuf>,
) -> bool {
    if default_workspace_exists(records) {
        return false;
    }
    let Some(base) = resolved_base else {
        return false;
    };
    let root = base.join("pony_agent");
    if let Err(error) = std::fs::create_dir_all(&root) {
        eprintln!(
            "[pony-agent] 默认工作区目录创建失败，不注册默认工作区：{error}"
        );
        return false;
    }
    // P2-6：默认 root 走与 create 相同的规范化（canonicalize + 去 \\?\ 前缀），
    // 避免与 create_workspace_entry 存储形式不一致导致 PA-080 前缀比较失配。
    let canonical = normalize_workspace_root(&root.display().to_string())
        .unwrap_or_else(|_| root.display().to_string());
    records.push(WorkspaceRecord {
        id: DEFAULT_WORKSPACE_ID.to_string(),
        name: "默认工作区".to_string(),
        root_path: canonical,
    });
    true
}

/// 按 root 查 workspace（Windows 大小写归一；与 create 的重复判定一致）。
pub fn find_workspace_by_root(records: &[WorkspaceRecord], canonical_root: &str) -> Option<WorkspaceRecord> {
    records
        .iter()
        .find(|record| roots_match(&record.root_path, canonical_root))
        .cloned()
}

/// 显示名长度上限（create/rename 共用）。
pub const MAX_DISPLAY_NAME_CHARS: usize = 64;

/// 显示名校验（工作区/会话共用）：trim 后非空、≤64 字符；返回 trim 结果。
/// create 与 rename 共用同一规则，避免同一注册表两条写入路径不变量分叉。
pub fn validate_display_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("名称不能为空".into());
    }
    if trimmed.chars().count() > MAX_DISPLAY_NAME_CHARS {
        return Err(format!("名称不能超过 {} 个字符", MAX_DISPLAY_NAME_CHARS));
    }
    Ok(trimmed.to_string())
}

/// 创建 workspace 条目并推入注册表（调用方负责持久化）。id 由 `ws-<slug>-<ts>` 生成。
pub fn create_workspace_entry(
    records: &mut Vec<WorkspaceRecord>,
    name: &str,
    root_path: &str,
) -> Result<WorkspaceRecord, String> {
    let trimmed_name = validate_display_name(name).map_err(|error| format!("workspace {error}"))?;
    let canonical = normalize_workspace_root(root_path)?;
    if records.iter().any(|record| roots_match(&record.root_path, &canonical)) {
        return Err("workspace root 已存在（重复 root 拒绝）".into());
    }
    if records.iter().any(|record| record.name == trimmed_name) {
        return Err("已存在同名工作区".into());
    }

    let slug = slugify(&trimmed_name);
    let id = if slug.is_empty() {
        format!("ws-{}", now_ms())
    } else {
        format!("ws-{slug}-{}", now_ms())
    };
    let record = WorkspaceRecord {
        id,
        name: trimmed_name,
        root_path: canonical,
    };
    records.push(record.clone());
    Ok(record)
}

/// 确保 root 已在 workspace 注册表中（wave-38 契约 R1.1）：
/// - **幂等**：已存在同 root 记录（`roots_match`）→ 原样返回该既有记录，`records` 不变、
///   不产生任何写盘（持久化由调用方负责）；
/// - **新登记**：复用 `create_workspace_entry`（id 规则 `ws-<slug>-<ts>` 与名称校验单源），
///   root 规范化复用 `normalize_workspace_root`（canonicalize + 必须为目录 + Windows 去 `\\?\` 前缀）；
/// - **显示名规则**：取 canonical root 的 `file_name`；空/非 UTF-8/根路径（如 `/`）→ `"workspace"`；
///   按 char 截断到 ≤64 字符；与既有记录重名（不同 root）→ 依次追加 `" (2)"`…`" (99)"`；
///   全部占用则 `Err`；
/// - 不改动既有 default 记录、不做任何迁移（R1.5 语义边界）。
pub fn ensure_workspace_for_root(
    records: &mut Vec<WorkspaceRecord>,
    root_path: &str,
) -> Result<WorkspaceRecord, String> {
    let canonical = normalize_workspace_root(root_path)?;
    // 幂等优先：同 root 命中既有记录 → 原样返回（不新增、不改名、不迁移）。
    if let Some(existing) = find_workspace_by_root(records, &canonical) {
        return Ok(existing);
    }
    let base = display_name_from_root(&canonical);
    let name = unique_display_name(records, &base)?;
    // 名称/root → id 的生成与校验单源复用 create_workspace_entry（入参 root 已是 canonical）。
    create_workspace_entry(records, &name, &canonical)
}

/// 由 canonical root 派生显示名：`file_name`；空/非 UTF-8/根路径（如 `/`）→ `"workspace"`；
/// 按 char 截断到 `MAX_DISPLAY_NAME_CHARS`。纯函数（无 IO），供 ensure 复用。
fn display_name_from_root(canonical_root: &str) -> String {
    let candidate = Path::new(canonical_root)
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("workspace");
    let truncated: String = candidate.chars().take(MAX_DISPLAY_NAME_CHARS).collect();
    if truncated.trim().is_empty() {
        "workspace".to_string()
    } else {
        truncated
    }
}

/// 为 `base` 分配注册表内唯一的显示名：原名可用则原名；与既有记录重名（不同 root）→
/// 依次追加 `" (2)"`…`" (99)"`（追加后仍需 ≤64 char，必要时先截断 base）；全部占用 → `Err`。
fn unique_display_name(records: &[WorkspaceRecord], base: &str) -> Result<String, String> {
    let trimmed = base.trim();
    let base: String = if trimmed.is_empty() {
        "workspace".to_string()
    } else {
        trimmed.chars().take(MAX_DISPLAY_NAME_CHARS).collect()
    };
    if !records.iter().any(|record| record.name == base) {
        return Ok(base);
    }
    for ordinal in 2..=99usize {
        let suffix = format!(" ({ordinal})");
        let keep = MAX_DISPLAY_NAME_CHARS
            .saturating_sub(suffix.chars().count())
            .max(1);
        let truncated: String = base.chars().take(keep).collect();
        let candidate = format!("{truncated}{suffix}");
        if !records.iter().any(|record| record.name == candidate) {
            return Ok(candidate);
        }
    }
    Err(format!(
        "无法为 workspace 分配唯一显示名：{base:?} 的 (2)-(99) 去重后缀均已被占用"
    ))
}

/// 重命名工作区：仅改显示名；id 与 root 不变。校验规则与 create 一致，
/// 重名判定排除自身（改名回当前名为 no-op 成功）。未知 id 报错。
pub fn rename_workspace_entry(
    records: &mut Vec<WorkspaceRecord>,
    workspace_id: &str,
    name: &str,
) -> Result<WorkspaceRecord, String> {
    let trimmed_name = validate_display_name(name).map_err(|error| format!("workspace {error}"))?;
    // 先做只读校验（重名判定排除自身），再取可变借用，避免借用重叠。
    if records
        .iter()
        .filter(|other| other.id != workspace_id)
        .any(|other| other.name == trimmed_name)
    {
        return Err("已存在同名工作区".into());
    }
    let record = records
        .iter_mut()
        .find(|record| record.id == workspace_id)
        .ok_or_else(|| format!("workspace 不存在：{workspace_id}"))?;
    record.name = trimmed_name.clone();
    Ok(WorkspaceRecord {
        id: record.id.clone(),
        name: trimmed_name,
        root_path: record.root_path.clone(),
    })
}

/// 删除工作区注册：default 保留字拒绝；未知 id 报错。只移除注册表记录——
/// 目录、会话日志与会话数据均归调用方/其他机制所有，本函数不触碰。
/// 调用方（SessionStore::delete_workspace）负责把名下会话归属重写为 default。
pub fn delete_workspace_entry(
    records: &mut Vec<WorkspaceRecord>,
    workspace_id: &str,
) -> Result<(), String> {
    if workspace_id == DEFAULT_WORKSPACE_ID {
        return Err("默认工作区不可删除".into());
    }
    match records.iter().position(|record| record.id == workspace_id) {
        Some(index) => {
            records.remove(index);
            Ok(())
        }
        None => Err(format!("workspace 不存在：{workspace_id}")),
    }
}

/// 解析 workspace id → root；id 为 None/缺省 → 默认 workspace root；找不到 → 错误。
pub fn resolve_workspace_root(records: &[WorkspaceRecord], workspace_id: Option<&str>) -> Result<String, String> {
    let id = match workspace_id {
        Some(value) if !value.trim().is_empty() => value.trim(),
        _ => DEFAULT_WORKSPACE_ID,
    };
    if id == DEFAULT_WORKSPACE_ID {
        return records
            .iter()
            .find(|record| record.id == DEFAULT_WORKSPACE_ID)
            .map(|record| record.root_path.clone())
            .ok_or_else(|| "默认 workspace 未初始化".into());
    }
    records
        .iter()
        .find(|record| record.id == id)
        .map(|record| record.root_path.clone())
        .ok_or_else(|| format!("workspace 不存在：{id}"))
}

/// PA-114 workspace-cwd 第二轮（W1/R1-2）：会话归属 workspace id 归一为单一真相源。
/// `id == DEFAULT_WORKSPACE_ID` 或已注册 → 原样返回；否则（死 id/空白/超长/控制字符）→
/// `DEFAULT_WORKSPACE_ID`。保持 stamp 既有日志口径：控制字符压平 + 80 字符截断。
/// 本函数不触碰 PA-080 的显式调用边界——工具链 `resolved_workspace_root` 对调用方显式传入
/// 的未注册 id 依旧 fail-closed `invalid_workspace`；归一只统一**会话归属**链。
pub fn normalize_session_workspace_id(workspaces: &[WorkspaceRecord], id: &str) -> String {
    if id == DEFAULT_WORKSPACE_ID || workspaces.iter().any(|record| record.id == id) {
        return id.to_string();
    }
    // L-C（ADR 0015）：id 来自外部输入——压平换行并截断，防日志伪造。
    let safe_echo: String = id
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .take(80)
        .collect();
    eprintln!(
        "[pony-agent] normalize_session_workspace_id: 未注册的 workspace '{safe_echo}'，归一为 default"
    );
    DEFAULT_WORKSPACE_ID.to_string()
}

/// PA-114 workspace-cwd 第二轮（W2/R2-4）：默认工作区 root 兜底 seam——`computed` 可解析则
/// 原样返回；为 `None` 时输出与 control_plane 对齐的瞬时兜底告警并返回 `cwd()`（进程 cwd，
/// 不持久化为默认工作区）。`cwd` 以闭包传入使告警分支不会急切求值。
pub fn default_root_or_cwd_with_warning(
    computed: Option<PathBuf>,
    cwd: impl FnOnce() -> PathBuf,
) -> PathBuf {
    if let Some(root) = computed {
        return root;
    }
    eprintln!(
        "[pony-agent] 默认工作区无法解析，使用进程 cwd 作为瞬时兜底（不持久化为默认工作区）"
    );
    cwd()
}
