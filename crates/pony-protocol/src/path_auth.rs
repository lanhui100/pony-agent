//! 路径权限判定唯一真相源（PA-080）。
//!
//! 所有触碰文件的工具统一经本模块做路径级权限判定，取代各工具内联的
//! `resolve_inside_workspace` / `prepare_workspace_file_path` 逻辑：
//!
//! - 权限分区：`WorkspaceRoot`（写+读）/ `ControlledTmp`（写+读）/ `TrustedSkillRoot`（只读）/
//!   `AuthorizedExternal`（只读）/ `Denied`。
//! - 判定核心：canonicalize（写新文件复用"最近存在祖先 + 后缀组件校验"语义）→ **组件级前缀
//!   比较**（Windows 上逐组件 ASCII 小写折叠，杜绝 `/ws-1` vs `/ws-10` 字符串前缀混淆）→
//!   未命中且 Read 时查授权清单（祖先链，止于卷根不越过）。
//! - 写权限默认仅 workspace 根（递归）+ 受控 tmp；workspace 外读取需显式授权
//!   （`requires_authorization` 审批语义入口）；workspace 外写一律拒绝
//!   （`outside_workspace_write_denied`）。
//! - canonicalizer 可注入（`PathPermissionChecker::new`），安全测试用 hermetic fake
//!   确定性覆盖符号链接逃逸等场景，无需真实创建符号链接。
//!
//! 授权清单持久化由调用方（SessionStore / control plane）经 `store_metadata`
//! key=`path_authorizations.v1` 完成；本模块保持纯数据 + 判定逻辑。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, RwLock};



/// 判定后的权限分区。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionZone {
    /// workspace 根内（写 + 读放行）
    WorkspaceRoot,
    /// 受控 tmp 目录内（写 + 读放行）
    ControlledTmp,
    /// 受信技能根内（产品只读资产：读取默认放行，写入仍按 workspace 边界拒绝）
    TrustedSkillRoot,
    /// workspace 外、经显式授权（只读）
    AuthorizedExternal,
    /// 拒绝
    Denied,
}

impl PermissionZone {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::WorkspaceRoot => "workspace_root",
            Self::ControlledTmp => "controlled_tmp",
            Self::TrustedSkillRoot => "trusted_skill_root",
            Self::AuthorizedExternal => "authorized_external",
            Self::Denied => "denied",
        }
    }
}

/// 路径用途：读或写。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathPurpose {
    Read,
    Write,
}

/// 结构化权限错误码（前端审批 UI 可直接消费的共享信封）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionErrorCode {
    /// 通用拒绝（穿越 / 符号链接逃逸 / 卷边界等）
    PermissionDenied,
    /// 写目标在 workspace 根与受控 tmp 之外
    OutsideWorkspaceWriteDenied,
    /// 读目标在 workspace 外且无授权
    RequiresAuthorization,
}

impl PermissionErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PermissionDenied => "permission_denied",
            Self::OutsideWorkspaceWriteDenied => "outside_workspace_write_denied",
            Self::RequiresAuthorization => "requires_authorization",
        }
    }
}

/// 权限错误：`{ code, message }` 结构化信封，message 为中文。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionError {
    pub code: PermissionErrorCode,
    pub message: String,
}

impl PermissionError {
    pub fn denied(message: impl Into<String>) -> Self {
        Self {
            code: PermissionErrorCode::PermissionDenied,
            message: message.into(),
        }
    }

    /// B204：workspace 外写拒绝（无 workspace root 上下文时的旧入口）。
    ///
    /// 旧行为会追加 `[sandbox: escalation available …]` 承诺，但运行时**从不**产生任何可批准的
    /// 升级请求（`pending_asks` 恒为空），模型只能原样重试再被拒 —— 文案承诺了不存在的能力。
    /// 现改为可执行指引：告知改写到工作区内的受控可写目录后重试。
    pub fn outside_workspace_write_denied(message: impl Into<String>) -> Self {
        Self {
            code: PermissionErrorCode::OutsideWorkspaceWriteDenied,
            message: with_writable_guidance(message.into()),
        }
    }

    /// B223 + B204：workspace 外写拒绝 + **可执行替代**（受控可写目录 `<root>/.tmp`）。
    ///
    /// 现场：旧文案只说「已拒绝写入」+ escalation，用户不知道往哪写、下一步做什么；
    /// 而 escalation 承诺在运行时**没有落点**（拒绝路径不产生任何审批请求）。
    /// 本构造器只给可执行纠正（改写到该可写目录后重试），不再追加 escalation 提示；
    /// 结构化错误码**不换码**（前端审批 UI 依赖 `outside_workspace_write_denied`）。
    ///
    /// 幂等：已含受控可写目录提示 ⇒ 不叠加。判定按 `trim_end` 后的文本，
    /// 避免尾随空白把「已带提示」误判成「未带提示」。
    pub fn outside_workspace_write_denied_with_workspace(
        message: impl Into<String>,
        workspace_root: &Path,
    ) -> Self {
        let base = message.into();
        let base = base.trim_end();
        let message = if base.contains(WRITABLE_HINT_MARKER) {
            base.to_string()
        } else {
            // 受控可写目录真源：`<workspace_root>/.tmp`（见 `agent::tools::paths`
            // 的 `controlled_tmp_dir`，PA-080 约定）。此处只呈现路径文本，不做 IO。
            let tmp_dir = workspace_root.join(CONTROLLED_TMP_DIR_NAME);
            let hint = format!(
                "{WRITABLE_HINT_MARKER}：{}（工作区内的可写替代目录）。请改写到该目录后重试。",
                tmp_dir.display()
            );
            if base.is_empty() {
                hint
            } else {
                format!("{base} {hint}")
            }
        };
        Self {
            code: PermissionErrorCode::OutsideWorkspaceWriteDenied,
            message,
        }
    }

    pub fn requires_authorization(message: impl Into<String>) -> Self {
        Self {
            code: PermissionErrorCode::RequiresAuthorization,
            message: message.into(),
        }
    }
}

/// 既有 escalation 提示（逐字保持：旧构造器与 acp 侧同码文案共用这段措辞）。
/// B204：无 workspace root 上下文时的通用可执行指引。
///
/// 刻意**不再**包含 `escalation available` 承诺：运行时不产生任何可批准的升级请求
/// （拒绝路径只有 tool_result，不落 `pending_asks`），承诺即误导。
const WRITABLE_GUIDANCE: &str =
    "请改用工作区内的路径（工作区根下的 .tmp 受控可写目录）后重试。";

/// B223 可写替代提示的去重标记（同一 message 二次经过构造器不得叠加）。
const WRITABLE_HINT_MARKER: &str = "受控可写目录";

/// 受控 tmp 目录名（真源 `agent::tools::paths::controlled_tmp_dir`：`<root>/.tmp`）。
const CONTROLLED_TMP_DIR_NAME: &str = ".tmp";

/// 追加通用可写指引（幂等；已带受控可写目录提示则原样返回）。
fn with_writable_guidance(message: String) -> String {
    if message.contains(WRITABLE_HINT_MARKER) {
        return message;
    }
    if message.trim().is_empty() {
        WRITABLE_GUIDANCE.to_string()
    } else {
        format!("{} {}", message.trim_end(), WRITABLE_GUIDANCE)
    }
}

/// 判定结果：分区 + canonical 路径 + 命中的授权条目（如有）。
#[derive(Clone, Debug)]
pub struct PathPermission {
    pub zone: PermissionZone,
    /// 判定后的路径（canonical 归一化；Windows 去 `\\?\` 前缀）
    pub canonical: PathBuf,
    pub matched_authorization: Option<AuthorizedPathEntry>,
}

/// 授权作用域（ADR 0029）：`read` 仅解除外部读；`read-write` 额外允许外部写。
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PathAuthorizationScope {
    /// 只读授权（默认；历史持久化条目缺省即此值）
    #[default]
    Read,
    /// 读写授权（用户显式批准的受控外部写）
    ReadWrite,
}

impl PathAuthorizationScope {
    /// 解析 host `authorize_path` 传入的 scope 字符串（兼容 `write` 别名）。
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim() {
            "read" => Ok(Self::Read),
            "read-write" | "write" => Ok(Self::ReadWrite),
            other => Err(format!(
                "authorize_path 暂不支持 scope `{other}`：仅支持 `read` 或 `read-write`。"
            )),
        }
    }
}

/// 显式授权的路径条目。`scope` 缺省为只读（向后兼容既有持久化数据）。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AuthorizedPathEntry {
    /// canonical 绝对路径（目录或文件），Windows 去 `\\?\` 前缀
    pub path: PathBuf,
    pub granted_at_ms: u64,
    /// ADR 0029 作用域；旧数据无该字段 → Read（保守，不放行外部写）
    #[serde(default)]
    pub scope: PathAuthorizationScope,
}

/// 授权存储：内存 `RwLock<HashMap>`，祖先链查找 O(depth)。
/// 持久化由调用方注入的 `persist` 回调完成（每次变更即写）。
pub struct AuthorizeStore {
    entries: RwLock<HashMap<PathBuf, AuthorizedPathEntry>>,
    persist: Option<Arc<dyn Fn(&HashMap<PathBuf, AuthorizedPathEntry>) -> Result<(), String> + Send + Sync>>,
}

impl std::fmt::Debug for AuthorizeStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AuthorizeStore")
            .field("entries", &self.entries())
            .finish()
    }
}

impl AuthorizeStore {
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
            persist: None,
        }
    }

    /// 带持久化回调：每次 grant/revoke 后调用（写失败返回错误，条目已生效但调用方可告警）。
    pub fn with_persist(
        persist: Arc<dyn Fn(&HashMap<PathBuf, AuthorizedPathEntry>) -> Result<(), String> + Send + Sync>,
    ) -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
            persist: Some(persist),
        }
    }

    /// 从既有条目恢复（重启加载）。
    pub fn from_entries(
        entries: Vec<AuthorizedPathEntry>,
        persist: Option<Arc<dyn Fn(&HashMap<PathBuf, AuthorizedPathEntry>) -> Result<(), String> + Send + Sync>>,
    ) -> Self {
        Self {
            entries: RwLock::new(
                entries
                    .into_iter()
                    .map(|entry| (entry.path.clone(), entry))
                    .collect(),
            ),
            persist,
        }
    }

    pub fn entries(&self) -> Vec<AuthorizedPathEntry> {
        let guard = self
            .entries
            .read()
            .unwrap_or_else(|e| e.into_inner());
        let mut values = guard.values().cloned().collect::<Vec<_>>();
        values.sort_by(|a, b| a.path.cmp(&b.path));
        values
    }

    /// 供持久化回调使用的快照（HashMap 全量）。
    fn snapshot_map(&self) -> HashMap<PathBuf, AuthorizedPathEntry> {
        self.entries
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// 显式授权一个路径（目录或文件）。目标必须存在（canonicalize 成功）才可授权。
    /// 不存在、不可解析 → 拒绝授权（不产生"看似授权实际无效"的条目）。
    /// 显式授权一个路径（**只读**语义，保持既有调用方行为不变）。
    pub fn grant(&self, canonical: PathBuf) -> Result<AuthorizedPathEntry, String> {
        self.grant_with_scope(canonical, PathAuthorizationScope::Read)
    }

    /// 显式授权一个路径并指定作用域（ADR 0029：`read` / `read-write`）。
    pub fn grant_with_scope(
        &self,
        mut canonical: PathBuf,
        scope: PathAuthorizationScope,
    ) -> Result<AuthorizedPathEntry, String> {
        // B230：调用方可能传 `std::fs::canonicalize` 的 verbatim 形态（Windows `\\?\C:\...`），
        // 而 `classify_path` 的 canonical 已去前缀 ⇒ 不归一会让「已授权」与「命中」永远错位。
        // 授权条目与查找必须同一形式。
        canonical = normalize_canonical(&canonical);
        let entry = AuthorizedPathEntry {
            path: canonical,
            granted_at_ms: now_ms(),
            scope,
        };
        {
            let mut guard = self
                .entries
                .write()
                .unwrap_or_else(|e| e.into_inner());
            guard.insert(entry.path.clone(), entry.clone());
        }
        if let Some(persist) = &self.persist {
            let snapshot = self.snapshot_map();
            if let Err(error) = persist(&snapshot) {
                tracing::error!(target: "pony_core", error = %error, "[pony-agent] path authorization persist failed");
            }
        }
        Ok(entry)
    }

    /// 撤销授权：**精确路径**——只移除该路径条目，子路径授权条目保留。
    pub fn revoke(&self, canonical: &Path) -> bool {
        // 与 grant/find 同口径：verbatim 与可读绝对路径视为同一条目（B230）。
        let canonical = normalize_canonical(canonical);
        let removed = {
            let mut guard = self
                .entries
                .write()
                .unwrap_or_else(|e| e.into_inner());
            guard.remove(&canonical).is_some()
        };
        if removed {
            if let Some(persist) = &self.persist {
                let snapshot = self.snapshot_map();
                if let Err(error) = persist(&snapshot) {
                    tracing::error!(target: "pony_core", error = %error, "[pony-agent] path authorization persist failed");
                }
            }
        }
        removed
    }

    /// 祖先链查找：目标自身逐级向上找父目录（止于卷根，不越过），任一命中即放行。
    pub fn find_authorization(&self, canonical: &Path) -> Option<AuthorizedPathEntry> {
        let guard = self
            .entries
            .read()
            .unwrap_or_else(|e| e.into_inner());
        for ancestor in ancestor_chain(canonical) {
            if let Some(entry) = guard.get(ancestor) {
                return Some(entry.clone());
            }
        }
        None
    }
}

impl Default for AuthorizeStore {
    fn default() -> Self {
        Self::new()
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 目标自身 + 逐级祖先链，止于卷根（不越过根组件）。
/// 授权 `/` 或卷根（如 `C:\`）时，该链包含根组件 → 语义 = 全盘/全卷授权。
pub fn ancestor_chain(canonical: &Path) -> Vec<&Path> {
    let mut chain = Vec::new();
    let mut current: Option<&Path> = Some(canonical);
    while let Some(path) = current {
        chain.push(path);
        let parent = path.parent();
        match parent {
            Some(parent) if !parent.as_os_str().is_empty() && parent != path => current = Some(parent),
            _ => current = None,
        }
    }
    chain
}

/// Windows canonicalize 输出归一化：去 `\\?\` / `\\?\UNC\` 前缀。
pub fn normalize_win_prefix(path: &Path) -> PathBuf {
    let display = path.display().to_string();
    let normalized = if let Some(rest) = display.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = display.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        display
    };
    PathBuf::from(normalized)
}

/// 公共归一化入口：与 `classify_path` 的 canonical 输出保持同一形式
/// （Windows 去 `\\?\` 前缀），供 tools.rs 的 `canonical_workspace_root` 等调用方复用。
pub fn normalize_canonical(path: &Path) -> PathBuf {
    normalize_win_prefix(path)
}

/// 逐组件比较（Windows 上每组件 ASCII 小写折叠）。`candidate` 必须与 `root` 逐组件相等或更深。
/// 防御：candidate 中出现 `..` / `.` 组件（canonical 路径不应包含）→ 直接判不命中，
/// 杜绝 `root/sub/../escape` 这类词法逃逸被前缀比较放行。
pub fn components_within(root: &Path, candidate: &Path) -> bool {
    let root_components = root.components().collect::<Vec<_>>();
    let candidate_components = candidate.components().collect::<Vec<_>>();
    if candidate_components.len() < root_components.len() {
        return false;
    }
    for component in &candidate_components {
        if matches!(component, Component::ParentDir | Component::CurDir) {
            return false;
        }
    }
    for (index, root_component) in root_components.iter().enumerate() {
        if !component_eq(root_component, &candidate_components[index]) {
            return false;
        }
    }
    true
}

/// 组件相等：`Normal` 逐文本比较（Windows ASCII 小写折叠）；`Prefix` 比较其原始串（同样折叠）；
/// 其余组件（RootDir / ParentDir / CurDir）按枚举相等。
pub fn component_eq(left: &Component<'_>, right: &Component<'_>) -> bool {
    match (left, right) {
        (Component::Normal(left), Component::Normal(right)) => {
            let left = left.to_string_lossy();
            let right = right.to_string_lossy();
            if cfg!(windows) {
                left.eq_ignore_ascii_case(&right)
            } else {
                left == right
            }
        }
        (Component::Prefix(left), Component::Prefix(right)) => {
            let left = left.as_os_str().to_string_lossy();
            let right = right.as_os_str().to_string_lossy();
            if cfg!(windows) {
                left.eq_ignore_ascii_case(&right)
            } else {
                left == right
            }
        }
        (left, right) => left == right,
    }
}
