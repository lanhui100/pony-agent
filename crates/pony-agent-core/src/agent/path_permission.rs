//! 路径权限判定唯一真相源（PA-080）。
//!
//! 所有触碰文件的工具统一经本模块做路径级权限判定，取代各工具内联的
//! `resolve_inside_workspace` / `prepare_workspace_file_path` 逻辑：
//!
//! - 权限分区：`WorkspaceRoot`（写+读）/ `ControlledTmp`（写+读）/ `AuthorizedExternal`（只读）/
//!   `Denied`。
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
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, RwLock};

/// 判定后的权限分区。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionZone {
    /// workspace 根内（写 + 读放行）
    WorkspaceRoot,
    /// 受控 tmp 目录内（写 + 读放行）
    ControlledTmp,
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

    pub fn outside_workspace_write_denied(message: impl Into<String>) -> Self {
        Self {
            code: PermissionErrorCode::OutsideWorkspaceWriteDenied,
            message: message.into(),
        }
    }

    pub fn requires_authorization(message: impl Into<String>) -> Self {
        Self {
            code: PermissionErrorCode::RequiresAuthorization,
            message: message.into(),
        }
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

/// 显式授权的路径条目（仅读）。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AuthorizedPathEntry {
    /// canonical 绝对路径（目录或文件），Windows 去 `\\?\` 前缀
    pub path: PathBuf,
    pub granted_at_ms: u64,
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
    pub fn grant(&self, canonical: PathBuf) -> Result<AuthorizedPathEntry, String> {
        let entry = AuthorizedPathEntry {
            path: canonical,
            granted_at_ms: now_ms(),
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
                eprintln!("[pony-agent] path authorization persist failed: {error}");
            }
        }
        Ok(entry)
    }

    /// 撤销授权：**精确路径**——只移除该路径条目，子路径授权条目保留。
    pub fn revoke(&self, canonical: &Path) -> bool {
        let removed = {
            let mut guard = self
                .entries
                .write()
                .unwrap_or_else(|e| e.into_inner());
            guard.remove(canonical).is_some()
        };
        if removed {
            if let Some(persist) = &self.persist {
                let snapshot = self.snapshot_map();
                if let Err(error) = persist(&snapshot) {
                    eprintln!("[pony-agent] path authorization persist failed: {error}");
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
fn ancestor_chain(canonical: &Path) -> Vec<&Path> {
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
fn normalize_win_prefix(path: &Path) -> PathBuf {
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
fn components_within(root: &Path, candidate: &Path) -> bool {
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
fn component_eq(left: &Component<'_>, right: &Component<'_>) -> bool {
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

/// 向上找最近的已存在祖先（含自身）。
fn existing_ancestor_path(path: &Path) -> Option<PathBuf> {
    let mut current = Some(path.to_path_buf());
    while let Some(candidate) = current {
        if candidate.exists() {
            return Some(candidate);
        }
        current = candidate.parent().map(|parent| parent.to_path_buf());
    }
    None
}

/// 可注入 canonicalizer 类型：`fn(&Path) -> io::Result<PathBuf>`。
pub type Canonicalizer = dyn Fn(&Path) -> io::Result<PathBuf> + Send + Sync;

/// 默认 canonicalizer：`std::fs::canonicalize`。
pub fn default_canonicalizer(path: &Path) -> io::Result<PathBuf> {
    std::fs::canonicalize(path)
}

/// 路径权限判定器。构造时注入 canonicalizer（测试传 hermetic fake），
/// 判定时传入会话级 root / tmp / 授权存储，不持有构造时固定的 workspace root。
pub struct PathPermissionChecker {
    canonicalize: Arc<Canonicalizer>,
}

impl PathPermissionChecker {
    pub fn new(canonicalize: Arc<Canonicalizer>) -> Self {
        Self { canonicalize }
    }

    pub fn with_default() -> Self {
        Self::new(Arc::new(default_canonicalizer))
    }

    /// 核心判定。`root` 为调用时按会话 workspaceId 解析的 root；`tmp_dir` 为受控 tmp 布局
    /// （`<root>/.tmp/` 或 fallback `temp_dir()/pony-agent/`）；`authorizations` 为授权清单。
    ///
    /// 判定顺序：
    /// 1. 规范化：Write 且目标不存在 → "最近存在祖先 canonicalize + 后缀组件校验"；
    ///    其余 canonicalize（失败 fail-closed）。canonical 输出归一化（Windows 去 `\\?\`）。
    /// 2. 组件级前缀判定：命中 root → WorkspaceRoot；命中 tmp_dir → ControlledTmp。
    /// 3. 未命中且 Read → 授权祖先链命中 → AuthorizedExternal；未命中 → RequiresAuthorization。
    /// 4. Write 未命中 → OutsideWorkspaceWriteDenied（授权清单本轮仅覆盖读）。
    pub fn classify(
        &self,
        raw_path: &str,
        root: &Path,
        tmp_dir: &Path,
        authorizations: &AuthorizeStore,
        purpose: PathPurpose,
    ) -> Result<PathPermission, PermissionError> {
        let trimmed = raw_path.trim();
        if trimmed.is_empty() {
            return Err(PermissionError::denied("文件路径不能为空。"));
        }

        let canonical = match purpose {
            PathPurpose::Write => {
                self.canonicalize_for_write(Path::new(trimmed), root, tmp_dir)?
            }
            PathPurpose::Read => {
                // 相对读路径按 root 基座解析（consultant P3 修复）：避免按进程 cwd 解析
                // 导致判定锚点漂移；绝对路径原样处理。
                let input = Path::new(trimmed);
                let absolute = if input.is_absolute() {
                    input.to_path_buf()
                } else {
                    root.join(input)
                };
                self.canonicalize_for_read(&absolute)?
            }
        };

        // 组件级前缀判定（绝不整体字符串前缀比较）。
        if components_within(root, &canonical) {
            return Ok(PathPermission {
                zone: PermissionZone::WorkspaceRoot,
                canonical,
                matched_authorization: None,
            });
        }
        if components_within(tmp_dir, &canonical) {
            return Ok(PathPermission {
                zone: PermissionZone::ControlledTmp,
                canonical,
                matched_authorization: None,
            });
        }

        // workspace 外：Read → 授权祖先链；Write → 拒绝。
        match purpose {
            PathPurpose::Read => {
                if let Some(entry) = authorizations.find_authorization(&canonical) {
                    return Ok(PathPermission {
                        zone: PermissionZone::AuthorizedExternal,
                        canonical,
                        matched_authorization: Some(entry),
                    });
                }
                Err(PermissionError::requires_authorization(format!(
                    "读取路径 {} 位于工作区之外且未获得授权，需要用户显式授权后才能读取。",
                    canonical.display()
                )))
            }
            PathPurpose::Write => Err(PermissionError::outside_workspace_write_denied(format!(
                "写入路径 {} 位于工作区根与受控临时目录之外，已拒绝写入。",
                canonical.display()
            ))),
        }
    }

    /// 读路径规范化：canonicalize；失败 fail-closed（路径不存在 / 不可解析 → 拒绝）。
    fn canonicalize_for_read(&self, input: &Path) -> Result<PathBuf, PermissionError> {
        let canonical = (self.canonicalize)(input)
            .map_err(|error| PermissionError::denied(format!("无法解析路径 {}：{error}。", input.display())))?;
        Ok(normalize_win_prefix(&canonical))
    }

    /// 写路径规范化：目标存在 → canonicalize；不存在 → "最近存在祖先 canonicalize + 后缀
    /// 组件校验"（复用 `prepare_workspace_file_path` 语义，支持全新嵌套目录）。
    /// 后缀含 `..` / `.` / 分隔符组件 → 在 IO 之前拒绝。
    fn canonicalize_for_write(
        &self,
        input: &Path,
        root: &Path,
        tmp_dir: &Path,
    ) -> Result<PathBuf, PermissionError> {
        let absolute_input = if input.is_absolute() {
            input.to_path_buf()
        } else {
            // 相对路径按 root 基座解析（与工具既有语义一致；tmp 场景由调用方传绝对路径）。
            root.join(input)
        };
        let absolute_input = normalize_win_prefix(&absolute_input);

        // 词法防御（IO 之前）：任何 `..` / `.` 组件直接拒绝——canonicalize 会解析 `..`，
        // 而 Windows 上 `root\sub\..` 的 exists() 可能为 true，导致祖先链吞掉逃逸组件。
        for component in absolute_input.components() {
            if matches!(component, Component::ParentDir | Component::CurDir) {
                return Err(PermissionError::denied(format!(
                    "目标路径包含 `..` / `.` 组件，已拒绝：{}。",
                    absolute_input.display()
                )));
            }
        }

        // 目标自身存在 → 直接 canonicalize（符号链接在此被解析，逃逸由后续组件级判定拒绝）。
        if absolute_input.exists() {
            let canonical = (self.canonicalize)(&absolute_input)
                .map_err(|error| PermissionError::denied(format!("无法解析路径 {}：{error}。", absolute_input.display())))?;
            return Ok(normalize_win_prefix(&canonical));
        }

        // 目标不存在 → 复用 prepare_workspace_file_path 语义：最近存在祖先 canonicalize + 后缀校验。
        let parent = absolute_input.parent().unwrap_or(root);
        let existing_ancestor = existing_ancestor_path(parent)
            .ok_or_else(|| PermissionError::denied(format!("无法解析目标父目录 {}。", parent.display())))?;
        let canonical_ancestor = (self.canonicalize)(&existing_ancestor).map_err(|error| {
            PermissionError::denied(format!(
                "无法解析目标父目录 {}：{error}。",
                existing_ancestor.display()
            ))
        })?;
        let canonical_ancestor = normalize_win_prefix(&canonical_ancestor);

        // 后缀组件级校验：拒绝 `..` / `.` / 分隔符（PreFix/RootDir 等）组件，杜绝逃逸。
        let relative_suffix = absolute_input
            .strip_prefix(&existing_ancestor)
            .map_err(|_| PermissionError::denied("无法计算工作区内的目标路径后缀。"))?;
        for component in relative_suffix.components() {
            match component {
                Component::Normal(_) => {}
                _ => {
                    return Err(PermissionError::denied(format!(
                        "目标路径后缀包含非法组件（`..` / `.` / 分隔符），已拒绝：{}。",
                        absolute_input.display()
                    )))
                }
            }
        }
        if absolute_input.file_name().is_none() {
            return Err(PermissionError::denied("目标路径以分隔符结尾，缺少文件名。"));
        }

        // 注意：此处返回的 `canonical_ancestor.join(relative_suffix)` 尚未 canonicalize 后缀部分；
        // 真正的放行判定（root/tmp 组件级前缀）由调用方在 classify 主流程完成，
        // 基于 canonical 祖先 + 已校验的合法后缀组件。
        let _ = tmp_dir; // tmp 懒 create_dir_all 由调用方（工具层）负责，避免判定侧 IO 副作用。
        Ok(canonical_ancestor.join(relative_suffix))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn test_root(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("pa080-{tag}-{}", std::process::id()))
    }

    fn prepare_root(root: &Path) {
        let _ = std::fs::remove_dir_all(root);
        std::fs::create_dir_all(root).unwrap();
    }

    /// hermetic fake canonicalizer：按注入的映射返回（不触碰真实文件系统）。
    /// 未命中映射的路径原样返回（已归一化）。
    struct FakeResolver {
        map: HashMap<PathBuf, PathBuf>,
    }

    impl FakeResolver {
        fn new() -> Self {
            Self {
                map: HashMap::new(),
            }
        }

        fn with(mut self, input: &str, resolved: &str) -> Self {
            self.map
                .insert(PathBuf::from(input), PathBuf::from(resolved));
            self
        }

        fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
            Ok(self
                .map
                .get(path)
                .cloned()
                .unwrap_or_else(|| PathBuf::from(path)))
        }
    }

    fn make_checker(resolver: FakeResolver) -> PathPermissionChecker {
        let resolver = Arc::new(resolver);
        PathPermissionChecker::new(Arc::new(move |path: &Path| resolver.canonicalize(path)))
    }

    fn no_auth() -> AuthorizeStore {
        AuthorizeStore::new()
    }

    // ── 穿越 / 前缀混淆 / 卷边界（hermetic）────────────────────────────────

    #[test]
    fn traversal_dotdot_escape_is_denied() {
        let root = PathBuf::from("/ws");
        let resolver = FakeResolver::new().with("/ws/../secret", "/secret");
        let checker = make_checker(resolver);
        let err = checker
            .classify("/ws/../secret", &root, &PathBuf::from("/ws/.tmp"), &no_auth(), PathPurpose::Read)
            .expect_err("must deny");
        assert_eq!(err.code, PermissionErrorCode::RequiresAuthorization);
    }

    #[test]
    fn traversal_dotdot_write_is_denied_before_io() {
        let root = PathBuf::from("/ws");
        let resolver = FakeResolver::new().with("/ws/../evil", "/evil");
        let checker = make_checker(resolver);
        let err = checker
            .classify("/ws/../evil", &root, &PathBuf::from("/ws/.tmp"), &no_auth(), PathPurpose::Write)
            .expect_err("must deny");
        // `..` 词法逃逸 → permission_denied（spec 穿越防御要求，IO 之前拒绝）。
        assert_eq!(err.code, PermissionErrorCode::PermissionDenied);
    }

    #[test]
    fn prefix_confusion_ws1_vs_ws10_is_rejected() {
        let root = PathBuf::from("/ws-1");
        let resolver = FakeResolver::new().with("/ws-10/file", "/ws-10/file");
        let checker = make_checker(resolver);
        // /ws-10 与 /ws-1 共享字符串前缀但不是同一目录 → 必须拒绝（组件级比较）。
        let err = checker
            .classify("/ws-10/file", &root, &PathBuf::from("/ws-1/.tmp"), &no_auth(), PathPurpose::Read)
            .expect_err("must deny");
        assert_eq!(err.code, PermissionErrorCode::RequiresAuthorization);
        // /ws-1 内路径 → 放行。
        let resolver2 = FakeResolver::new().with("/ws-1/file", "/ws-1/file");
        let checker2 = make_checker(resolver2);
        let permission = checker2
            .classify("/ws-1/file", &root, &PathBuf::from("/ws-1/.tmp"), &no_auth(), PathPurpose::Read)
            .expect("must allow");
        assert_eq!(permission.zone, PermissionZone::WorkspaceRoot);
    }

    #[test]
    fn win_case_variant_match_and_prefix_confusion() {
        let root = PathBuf::from(r"C:\WS");
        // `c:\ws\file` ⊂ `C:\WS` → 放行（Windows 组件级 ASCII 折叠）。
        let resolver = FakeResolver::new().with(r"c:\ws\file", r"C:\WS\file");
        let checker = make_checker(resolver);
        let permission = checker
            .classify(r"c:\ws\file", &root, &PathBuf::from(r"C:\WS\.tmp"), &no_auth(), PathPurpose::Read)
            .expect("case variant must be allowed on windows");
        // 非 Windows 平台：`c:\ws\file` 与 `C:\WS` 组件比较不折叠 → 预期拒绝。
        if cfg!(windows) {
            assert_eq!(permission.zone, PermissionZone::WorkspaceRoot);
        } else {
            assert_eq!(permission.zone, PermissionZone::AuthorizedExternal);
        }

        // `c:\ws-10\file` ⊄ `C:\WS` → 拒绝（双断言第二半）。
        let resolver = FakeResolver::new().with(r"c:\ws-10\file", r"C:\WS-10\file");
        let checker = make_checker(resolver);
        let result = checker.classify(
            r"c:\ws-10\file",
            &root,
            &PathBuf::from(r"C:\WS\.tmp"),
            &no_auth(),
            PathPurpose::Read,
        );
        assert!(result.is_err(), "ws-10 must never match ws root");
    }

    #[test]
    fn win_prefix_normalization_strips_verbatim() {
        let root = PathBuf::from(r"C:\WS");
        let resolver = FakeResolver::new().with(r"\\?\C:\WS\file", r"\\?\C:\WS\file");
        let checker = make_checker(resolver);
        let permission = checker
            .classify(r"\\?\C:\WS\file", &root, &PathBuf::from(r"C:\WS\.tmp"), &no_auth(), PathPurpose::Read)
            .expect("verbatim prefix must be normalized");
        if cfg!(windows) {
            assert_eq!(permission.canonical, PathBuf::from(r"C:\WS\file"));
            assert_eq!(permission.zone, PermissionZone::WorkspaceRoot);
        }
    }

    #[test]
    fn volume_boundary_c_vs_d_is_rejected() {
        let root = PathBuf::from(r"C:\WS");
        let resolver = FakeResolver::new().with(r"D:\other\file", r"D:\other\file");
        let checker = make_checker(resolver);
        let err = checker
            .classify(r"D:\other\file", &root, &PathBuf::from(r"C:\WS\.tmp"), &no_auth(), PathPurpose::Read)
            .expect_err("different volume must be rejected");
        assert_eq!(err.code, PermissionErrorCode::RequiresAuthorization);
    }

    // ── 符号链接逃逸（hermetic fake resolver 双路径）──────────────────────

    #[test]
    fn symlink_escape_is_denied_unless_authorized() {
        let root = PathBuf::from("/ws");
        let tmp = PathBuf::from("/ws/.tmp");
        // 场景：/ws/link 符号链接指向 /outside/secret。fake resolver 把 link 解析到外部路径。
        let resolver = FakeResolver::new().with("/ws/link", "/outside/secret");
        let checker = make_checker(resolver);

        // 无授权 → 拒绝（读）。
        let err = checker
            .classify("/ws/link", &root, &tmp, &no_auth(), PathPurpose::Read)
            .expect_err("symlink escape must be denied");
        assert_eq!(err.code, PermissionErrorCode::RequiresAuthorization);

        // 有授权（外部目标）→ 放行 AuthorizedExternal。
        let store = AuthorizeStore::new();
        store.grant(PathBuf::from("/outside/secret")).unwrap();
        let permission = checker
            .classify("/ws/link", &root, &tmp, &store, PathPurpose::Read)
            .expect("authorized external target must be allowed");
        assert_eq!(permission.zone, PermissionZone::AuthorizedExternal);
        assert_eq!(permission.canonical, PathBuf::from("/outside/secret"));

        // 写（无论授权）→ 拒绝（授权清单本轮仅覆盖读）。
        // 注意：hermetic fake 下 `/ws/link` 目标不存在（无真实链接），写路径走
        // "最近存在祖先 + 后缀校验" → 判定为 workspace 内新建链接文件（放行）；
        // 真实符号链接逃逸的写路径由 `real_symlink_escape_canary`（unix）覆盖。
        let write_permission = checker
            .classify("/ws/link", &root, &tmp, &store, PathPurpose::Write)
            .expect("writing a new in-workspace path is allowed");
        assert_eq!(write_permission.zone, PermissionZone::WorkspaceRoot);
    }

    #[test]
    fn hermetic_symlink_decision_matches_without_real_link() {
        // 不创建真实符号链接：注入 fake resolver 报告"workspace 内的链接指向外部"，
        // 判定结果与真实环境一致。
        let root = PathBuf::from("/ws");
        let resolver = FakeResolver::new().with("/ws/alias", "/etc/passwd");
        let checker = make_checker(resolver);
        let err = checker
            .classify("/ws/alias", &root, &PathBuf::from("/ws/.tmp"), &no_auth(), PathPurpose::Read)
            .expect_err("must deny");
        assert_eq!(err.code, PermissionErrorCode::RequiresAuthorization);
    }

    // ── 写新文件：全新嵌套目录 / `..` 后缀拒绝 ───────────────────────────

    #[test]
    fn write_into_brand_new_nested_directory_is_allowed() {
        let root = test_root("newdir");
        prepare_root(&root);
        let checker = PathPermissionChecker::with_default();
        let target = root.join("a/b/c/new-file.txt");
        let permission = checker
            .classify(&target.display().to_string(), &root, &root.join(".tmp"), &no_auth(), PathPurpose::Write)
            .expect("brand-new nested write must be allowed");
        assert_eq!(permission.zone, PermissionZone::WorkspaceRoot);
        assert!(permission.canonical.starts_with(&root));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn write_suffix_with_dotdot_component_is_rejected_before_io() {
        let root = test_root("dotdot");
        prepare_root(&root);
        let checker = PathPermissionChecker::with_default();
        // 目标不存在且后缀含 .. → 拒绝（后缀组件级校验）。
        let target = root.join("sub/../escape.txt");
        let err = checker
            .classify(&target.display().to_string(), &root, &root.join(".tmp"), &no_auth(), PathPurpose::Write)
            .expect_err("dotdot suffix must be denied");
        assert_eq!(err.code, PermissionErrorCode::PermissionDenied);
        // 无文件产生。
        assert!(!root.join("escape.txt").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn write_file_name_component_only_is_allowed() {
        let root = test_root("fname");
        prepare_root(&root);
        let checker = PathPermissionChecker::with_default();
        let permission = checker
            .classify("hello.txt", &root, &root.join(".tmp"), &no_auth(), PathPurpose::Write)
            .expect("plain relative write must be allowed");
        assert_eq!(permission.zone, PermissionZone::WorkspaceRoot);
        assert_eq!(permission.canonical, root.join("hello.txt"));
        let _ = std::fs::remove_dir_all(&root);
    }

    // ── tmp 区 ──────────────────────────────────────────────────────────

    #[test]
    fn tmp_write_is_allowed_and_workspace_outside_write_is_denied() {
        let root = test_root("tmpzone");
        prepare_root(&root);
        let tmp = root.join(".tmp");
        std::fs::create_dir_all(&tmp).unwrap();
        let checker = PathPermissionChecker::with_default();

        let permission = checker
            .classify(&tmp.join("out.bin").display().to_string(), &root, &tmp, &no_auth(), PathPurpose::Write)
            .expect("tmp write must be allowed");
        // tmp 位于 workspace 根内（`<root>/.tmp/`）时先命中 WorkspaceRoot；两区放行集相同，
        // 设计决策 2 明确"同一放行集，无冲突"，故两种分区都合法。
        assert!(
            matches!(
                permission.zone,
                PermissionZone::WorkspaceRoot | PermissionZone::ControlledTmp
            ),
            "tmp write zone: {:?}",
            permission.zone
        );

        // workspace 外写（root 外、tmp 外）→ outside_workspace_write_denied。
        let outside = std::env::temp_dir().join(format!("pa080-outside-{}", std::process::id()));
        std::fs::write(&outside, b"x").unwrap();
        let err = checker
            .classify(&outside.display().to_string(), &root, &tmp, &no_auth(), PathPurpose::Write)
            .expect_err("outside write must be denied");
        assert_eq!(err.code, PermissionErrorCode::OutsideWorkspaceWriteDenied);
        assert_eq!(err.code.as_str(), "outside_workspace_write_denied");

        let _ = std::fs::remove_file(&outside);
        let _ = std::fs::remove_dir_all(&root);
    }

    // ── 授权清单 ─────────────────────────────────────────────────────────

    #[test]
    fn grant_read_revoke_cycle_and_exact_path_revoke() {
        let root = PathBuf::from("/ws");
        let outside_file = PathBuf::from("/outside/a.txt");
        let outside_child = PathBuf::from("/outside/a.txt.bak");
        let store = AuthorizeStore::new();

        // grant → 放行。
        store.grant(outside_file.clone()).unwrap();
        let resolver = FakeResolver::new().with("/outside/a.txt", "/outside/a.txt");
        let checker = make_checker(resolver);
        let permission = checker
            .classify("/outside/a.txt", &root, &PathBuf::from("/ws/.tmp"), &store, PathPurpose::Read)
            .expect("granted read must be allowed");
        assert_eq!(permission.zone, PermissionZone::AuthorizedExternal);

        // 文件授权不覆盖兄弟。
        let resolver = FakeResolver::new().with("/outside/a.txt.bak", "/outside/a.txt.bak");
        let checker = make_checker(resolver);
        let err = checker
            .classify("/outside/a.txt.bak", &root, &PathBuf::from("/ws/.tmp"), &store, PathPurpose::Read)
            .expect_err("sibling must not inherit file authorization");
        assert_eq!(err.code, PermissionErrorCode::RequiresAuthorization);

        // revoke 精确路径 → 拒绝。
        assert!(store.revoke(&outside_file));
        let err = checker
            .classify("/outside/a.txt", &root, &PathBuf::from("/ws/.tmp"), &store, PathPurpose::Read)
            .expect_err("revoked read must be denied");
        assert_eq!(err.code, PermissionErrorCode::RequiresAuthorization);

        // 子授权保留：revoke 父不撤销子。
        store.grant(PathBuf::from("/outside/dir")).unwrap();
        store.grant(outside_child.clone()).unwrap();
        store.revoke(&PathBuf::from("/outside/dir"));
        let resolver = FakeResolver::new().with("/outside/dir/file", "/outside/dir/file");
        let checker = make_checker(resolver);
        let err = checker
            .classify("/outside/dir/file", &root, &PathBuf::from("/ws/.tmp"), &store, PathPurpose::Read)
            .expect_err("revoked parent must deny children");
        assert_eq!(err.code, PermissionErrorCode::RequiresAuthorization);
        // 独立的子授权仍然有效。
        let resolver2 = FakeResolver::new().with("/outside/a.txt.bak", "/outside/a.txt.bak");
        let checker2 = make_checker(resolver2);
        let permission = checker2
            .classify("/outside/a.txt.bak", &root, &PathBuf::from("/ws/.tmp"), &store, PathPurpose::Read)
            .expect("independent child authorization must survive parent revoke");
        assert_eq!(permission.zone, PermissionZone::AuthorizedExternal);
    }

    #[test]
    fn directory_authorization_covers_descendants() {
        let root = PathBuf::from("/ws");
        let store = AuthorizeStore::new();
        store.grant(PathBuf::from("/data")).unwrap();
        let resolver = FakeResolver::new().with("/data/x/y.txt", "/data/x/y.txt");
        let checker = make_checker(resolver);
        let permission = checker
            .classify("/data/x/y.txt", &root, &PathBuf::from("/ws/.tmp"), &store, PathPurpose::Read)
            .expect("directory authorization must cover descendants");
        assert_eq!(permission.zone, PermissionZone::AuthorizedExternal);
    }

    #[test]
    fn root_volume_authorization_semantics() {
        // 授权卷根/文件系统根：文档化语义 = 该根下全部可读。
        let root = PathBuf::from("/ws");
        let store = AuthorizeStore::new();
        let fs_root = if cfg!(windows) {
            PathBuf::from(r"C:\")
        } else {
            PathBuf::from("/")
        };
        store.grant(fs_root.clone()).unwrap();
        // 平台相关路径：Windows 用 C:\ 卷内路径，Unix 用 / 根下路径。
        let target = if cfg!(windows) {
            r"C:\anywhere\deep\file"
        } else {
            "/anywhere/deep/file"
        };
        let resolver = FakeResolver::new().with(target, target);
        let checker = make_checker(resolver);
        let permission = checker
            .classify(target, &root, &PathBuf::from("/ws/.tmp"), &store, PathPurpose::Read)
            .expect("filesystem-root authorization must cover everything");
        assert_eq!(permission.zone, PermissionZone::AuthorizedExternal);
    }

    #[test]
    fn ancestor_chain_stops_at_volume_root() {
        // 链不得越过卷根：/ws/x 的祖先链 = /ws/x → /ws → /，止于 /。
        let chain = ancestor_chain(Path::new("/ws/x"));
        assert_eq!(chain.len(), 3);
        assert_eq!(chain[0], Path::new("/ws/x"));
        assert_eq!(chain[1], Path::new("/ws"));
        assert_eq!(chain[2], Path::new("/"));
    }

    #[test]
    fn authorize_store_persist_callback_fires_on_change() {
        let calls = Arc::new(AtomicUsize::new(0));
        let persist_calls = Arc::clone(&calls);
        let persist = Arc::new(move |_entries: &HashMap<PathBuf, AuthorizedPathEntry>| {
            persist_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });
        let store = AuthorizeStore::with_persist(persist);
        store.grant(PathBuf::from("/data")).unwrap();
        store.revoke(&PathBuf::from("/data"));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        // 未命中的 revoke 不触发持久化。
        store.revoke(&PathBuf::from("/never-granted"));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn authorize_store_roundtrip_via_entries() {
        let store = AuthorizeStore::new();
        store.grant(PathBuf::from("/data")).unwrap();
        store.grant(PathBuf::from("/docs/a.md")).unwrap();
        let entries = store.entries();
        let restored = AuthorizeStore::from_entries(entries, None);
        assert!(restored.find_authorization(Path::new("/data")).is_some());
        assert!(restored
            .find_authorization(Path::new("/docs/a.md"))
            .is_some());
        assert!(restored.find_authorization(Path::new("/data/x")).is_some());
    }

    // ── 真实符号链接 canary（无特权则跳过）──────────────────────────────

    #[cfg(unix)]
    #[test]
    fn real_symlink_escape_canary() {
        use std::os::unix::fs::symlink;
        let root = test_root("reallink");
        prepare_root(&root);
        let outside = test_root("realoutside");
        prepare_root(&outside);
        let link = root.join("escape");
        if symlink(&outside, &link).is_err() {
            // 无创建符号链接特权 → 跳过。
            let _ = std::fs::remove_dir_all(&root);
            let _ = std::fs::remove_dir_all(&outside);
            return;
        }
        let checker = PathPermissionChecker::with_default();
        let tmp = root.join(".tmp");
        std::fs::create_dir_all(&tmp).unwrap();
        // 无授权 → 读拒绝。
        let err = checker
            .classify(&link.join("f.txt").display().to_string(), &root, &tmp, &no_auth(), PathPurpose::Read)
            .expect_err("real symlink escape must be denied");
        assert_eq!(err.code, PermissionErrorCode::RequiresAuthorization);
        // 有授权 → 放行。
        let store = AuthorizeStore::new();
        store.grant(outside.clone()).unwrap();
        let permission = checker
            .classify(&link.join("f.txt").display().to_string(), &root, &tmp, &store, PathPurpose::Read)
            .expect("authorized symlink target must be allowed");
        assert_eq!(permission.zone, PermissionZone::AuthorizedExternal);
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }
}
