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

use crate::workspace_hotloader::{AgentScope, SkillRootSpec};
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

pub use pony_protocol::path_auth::{
    ancestor_chain, normalize_canonical, AuthorizeStore, AuthorizedPathEntry,
    PathAuthorizationScope, PathPermission, PathPurpose, PermissionError, PermissionErrorCode,
    PermissionZone, component_eq, components_within, normalize_win_prefix,
};
#[cfg(test)]
use std::collections::HashMap;
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
    /// 3. 未命中且 Read → 授权祖先链命中 → AuthorizedExternal；受信技能根内 →
    ///    TrustedSkillRoot；均未命中 → RequiresAuthorization。
    /// 4. Write 未命中 → OutsideWorkspaceWriteDenied（授权清单本轮仅覆盖读）。
    pub fn classify(
        &self,
        raw_path: &str,
        root: &Path,
        tmp_dir: &Path,
        authorizations: &AuthorizeStore,
        purpose: PathPurpose,
    ) -> Result<PathPermission, PermissionError> {
        self.classify_with_options(raw_path, root, tmp_dir, authorizations, purpose, false)
    }

    /// 扩展判定入口：`allow_external_read == true` 时（例如 `full_access` 模式或本会话已授权工作区外读取），
    /// 仅放行工作区外 `PathPurpose::Read`（2A：外部写仍保持现有写边界拒绝）。
    pub fn classify_with_options(
        &self,
        raw_path: &str,
        root: &Path,
        tmp_dir: &Path,
        authorizations: &AuthorizeStore,
        purpose: PathPurpose,
        allow_external_read: bool,
    ) -> Result<PathPermission, PermissionError> {
        let trimmed = raw_path.trim();
        if trimmed.is_empty() {
            return Err(PermissionError::denied("文件路径不能为空。"));
        }

        // 读用途保留**词法绝对输入**：技能根信任按分域策略判定 —— 全局根以词法归属为准
        // （用户自有域允许符号链接解析到根外，Wave-58 D1），不能只看 canonical 结果。
        let mut read_lexical: Option<PathBuf> = None;
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
                read_lexical = Some(normalize_win_prefix(&absolute));
                self.canonicalize_for_read(&absolute)?
            }
        };

        // 归一化 root / tmp_dir：先尝试 canonical 化（解析 junction/symlink/8.3 短名），
        // 失败则仅去 `\\?\` 前缀。确保与 canonical（canonicalize 输出）的形式一致，
        // 否则 prefix 组件不匹配（RUNNER~1 vs runneradmin、\\?\ 前缀差异）导致误判。
        let root_normalized = (self.canonicalize)(root)
            .map(|p| normalize_win_prefix(&p))
            .unwrap_or_else(|_| normalize_win_prefix(root));
        let tmp_dir_normalized = (self.canonicalize)(tmp_dir)
            .map(|p| normalize_win_prefix(&p))
            .unwrap_or_else(|_| normalize_win_prefix(tmp_dir));

        // 组件级前缀判定（绝不整体字符串前缀比较）。
        if components_within(&root_normalized, &canonical) {
            return Ok(PathPermission {
                zone: PermissionZone::WorkspaceRoot,
                canonical,
                matched_authorization: None,
            });
        }
        if components_within(&tmp_dir_normalized, &canonical) {
            return Ok(PathPermission {
                zone: PermissionZone::ControlledTmp,
                canonical,
                matched_authorization: None,
            });
        }

        // workspace 外：Read → 显式授权 / 受信技能根 / 会话或完全访问放行；Write → 授权祖先链或拒绝。
        match purpose {
            PathPurpose::Read => {
                if let Some(entry) = authorizations.find_authorization(&canonical) {
                    return Ok(PathPermission {
                        zone: PermissionZone::AuthorizedExternal,
                        canonical,
                        matched_authorization: Some(entry),
                    });
                }
                if let Some(lexical) = read_lexical.as_deref() {
                    if is_read_within_trusted_skill_roots(lexical, &canonical, Some(root)) {
                        return Ok(PathPermission {
                            zone: PermissionZone::TrustedSkillRoot,
                            canonical,
                            matched_authorization: None,
                        });
                    }
                }
                if allow_external_read {
                    return Ok(PathPermission {
                        zone: PermissionZone::AuthorizedExternal,
                        canonical,
                        matched_authorization: None,
                    });
                }
                Err(PermissionError::requires_authorization(format!(
                    "读取路径 {} 位于工作区之外且未获得授权，需要用户显式授权后才能读取。",
                    canonical.display()
                )))
            }
            PathPurpose::Write => {
                // ADR 0029：只有 **read-write** 作用域的授权才解除外部写阻断；
                // 只读授权（含历史无 scope 条目）不得升级为写，保持 Fail-Closed。
                if let Some(entry) = authorizations.find_authorization(&canonical) {
                    if entry.scope == PathAuthorizationScope::ReadWrite {
                        return Ok(PathPermission {
                            zone: PermissionZone::AuthorizedExternal,
                            canonical,
                            matched_authorization: Some(entry),
                        });
                    }
                }
                // B223：拒绝文案必须给出**可执行替代**——受控可写目录 `<root>/.tmp`。
                // 传真实 workspace root（取归一化后的根，路径可直接照着用）。
                Err(
                    PermissionError::outside_workspace_write_denied_with_workspace(
                        format!(
                            "写入路径 {} 位于工作区根与受控临时目录之外，已拒绝写入。",
                            canonical.display()
                        ),
                        &root_normalized,
                    ),
                )
            }
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

/// 技能正文文件名（白名单只放行技能正文本身）。
const SKILL_BODY_FILE_NAME: &str = "SKILL.md";

/// 受信只读根（PA-149）：用户级全局根下**实际存在**的 `skills` 目录（已 canonical 化）。
pub fn trusted_readonly_roots() -> Vec<PathBuf> {
    trusted_skill_roots(None)
}

/// 受信技能只读根全集（canonical 形态）：全局根 + 仓库根 / 会话根内三处技能目录。
///
/// 根清单与技能扫描共用 [`crate::workspace_hotloader::enumerate_skill_roots`] 的
/// **单一枚举**（PA-173）：可见性与可加载性各自维护根表，正是 P1-6「列得出、载不进」的成因。
///
/// L3 r3 A6：项目根自身仍做 canonical **包含**校验 —— `<ws>/.agents/skills` 若是指向
/// 项目域外的符号链接，canonical 后不在所属域（仓库根 / 会话根）内，该根整根不采纳
/// （否则 symlink 重定向会让外部目录变成受信根）。全局根属用户自有域，允许解析到根外（D1）。
pub fn trusted_skill_roots(workspace_root: Option<&Path>) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    for spec in skill_root_specs(workspace_root) {
        match spec.scope {
            AgentScope::Global => push_canonical_root(&mut roots, &spec.dir),
            AgentScope::Project => {
                let base = spec
                    .containment_base
                    .clone()
                    .unwrap_or_else(|| spec.dir.clone());
                push_contained_canonical_root(&mut roots, &spec.dir, &base);
            }
        }
    }
    roots
}

/// 当前进程可见的技能根枚举（默认入口：全局根 + 自定义根 + 项目根）。
///
/// 只做枚举，不做信任判定；信任策略由调用方按 `scope` 决定（全局允许符号链接逃逸、
/// 项目要求 canonical 包含）。
fn skill_root_specs(workspace_root: Option<&Path>) -> Vec<SkillRootSpec> {
    let global_homes = crate::workspace_hotloader::resolve_global_agent_homes();
    let custom_skill_dirs = crate::workspace_hotloader::resolve_custom_skill_dirs();
    let mut specs = crate::workspace_hotloader::enumerate_global_skill_roots(
        &global_homes,
        &custom_skill_dirs,
    );
    if let Some(root) = workspace_root {
        specs.extend(crate::workspace_hotloader::enumerate_project_skill_roots(root));
    }
    specs
}

/// 收录「存在且可解析」的技能根（全局根：允许符号链接解析到根外）。
fn push_canonical_root(roots: &mut Vec<PathBuf>, candidate: &Path) {
    if !candidate.is_dir() {
        return;
    }
    let Ok(canonical) = candidate.canonicalize() else {
        return;
    };
    if !roots.iter().any(|existing| existing == &canonical) {
        roots.push(canonical);
    }
}

/// 收录「canonical 后仍在 `base` 内」的技能根；符号链接重定向 / 不存在 / 不可解析一律不采纳。
fn push_contained_canonical_root(roots: &mut Vec<PathBuf>, candidate: &Path, base: &Path) {
    if !candidate.is_dir() {
        return;
    }
    let (Ok(canonical_base), Ok(canonical_candidate)) = (base.canonicalize(), candidate.canonicalize())
    else {
        return;
    };
    if !canonical_candidate.starts_with(&canonical_base) {
        return;
    }
    if !roots.iter().any(|existing| existing == &canonical_candidate) {
        roots.push(canonical_candidate);
    }
}

/// 技能根的**分域信任策略**（Wave-58 F2-3 / 契约 D1）。
///
/// 与 [`trusted_skill_roots`] 的区别：本结构把「根」与「是否允许逃逸」显式绑定，
/// 供加载侧
/// [`crate::skill_body::is_trusted_skill_file`] 使用。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillTrustRoot {
    /// 词法技能根（`<home>/skills` 或 `<ws>/.agents/skills` 等，不做 canonicalize）。
    pub root: PathBuf,
    /// 该根的 canonical 形态；无法解析时为 `None`。
    pub canonical_root: Option<PathBuf>,
    /// `true` = 用户级全局根：允许技能目录 / `SKILL.md` 为符号链接并解析到根外（D1）。
    /// `false` = 项目级根：保持严格 canonical 包含判定。
    pub allow_symlink_escape: bool,
}

/// 加载侧使用的分域策略技能根清单（Wave-58 F2-3 / 契约 D1）。
///
/// - 全局根（`<home>/skills`，`home` 来自 [`crate::workspace_hotloader::resolve_global_agent_homes`]）：
///   `allow_symlink_escape = true`，**不再要求**根自身 canonical 落在 home 内
///   （用户自有域；项目级才是不可信输入）。
/// - 项目根（`<root>/{.dsh,.agents,.claude}/skills`，`<root>` = 会话根或 `.git` 上探到的仓库根）：
///   `allow_symlink_escape = false`，且根自身 canonical 后必须仍在所属项目域内；逃逸根整根不采纳
///   （`<ws>/.claude/skills -> /tmp/evil/skills` 不得让外部目录变成受信根）。
///
/// 结果按词法路径去重并保持「全局在前、项目在后」的顺序。
pub fn trusted_skill_roots_with_policy(workspace_root: Option<&Path>) -> Vec<SkillTrustRoot> {
    let mut roots: Vec<SkillTrustRoot> = Vec::new();
    for spec in skill_root_specs(workspace_root) {
        match spec.scope {
            // 全局根（含 `~/.dsh/skills` 与 `PONY_AGENT_SKILL_DIRS` 自定义根）：用户自有域。
            AgentScope::Global => push_trust_root(&mut roots, spec.dir, true),
            AgentScope::Project => {
                // 项目级：根自身不得经符号链接逃出所属项目域（否则外部目录会成为受信根）。
                let Some(base) = spec.containment_base.as_deref() else {
                    continue;
                };
                let (Ok(canonical_base), Ok(canonical_root)) =
                    (base.canonicalize(), spec.dir.canonicalize())
                else {
                    continue;
                };
                if !canonical_root.starts_with(&canonical_base) {
                    continue;
                }
                push_trust_root(&mut roots, spec.dir, false);
            }
        }
    }
    roots
}

/// 追加策略根（按词法路径去重；重复出现时保留先入的较窄/较宽策略）。
fn push_trust_root(roots: &mut Vec<SkillTrustRoot>, root: PathBuf, allow_symlink_escape: bool) {
    if roots.iter().any(|existing| existing.root == root) {
        return;
    }
    let canonical_root = root.canonicalize().ok();
    roots.push(SkillTrustRoot {
        root,
        canonical_root,
        allow_symlink_escape,
    });
}

/// 路径是否落在受信只读根内（PA-149，fail-closed；全局根版本）。
pub fn is_within_trusted_readonly_roots(path: &Path) -> bool {
    is_within_trusted_skill_roots(path, None)
}

/// 路径是否是受信技能根内的技能正文（PA-149 / L3-B F3+F4 / L3 r3 A6+A7，fail-closed）。
///
/// - `..` 组件一律拒绝（不做词法放行）；
/// - **先 canonicalize 再比**：失败即不受信（悬空符号链接/不存在路径不得因字面前缀放行）；
/// - 目标（canonical 后）必须是 `SKILL.md`，且逐组件落在某个 canonical 受信根内；
/// - 无显式 workspace root 时只有全局根受信（项目层必须显式传入 root，杜绝
///   `*/.agents/skills/*/SKILL.md` 任意路径被接受的 fail-open）。
pub fn is_within_trusted_skill_roots(path: &Path, workspace_root: Option<&Path>) -> bool {
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return false;
    }
    // fail-closed（L3-B F3）：canonicalize 失败意味着路径不可解析（悬空符号链接等），
    // 任何字面前缀放行都会让逃逸链接被误判为受信根内，故直接判否。
    let Ok(canonical_path) = path.canonicalize() else {
        return false;
    };
    if canonical_path.file_name().and_then(|name| name.to_str()) != Some(SKILL_BODY_FILE_NAME) {
        return false;
    }
    trusted_skill_roots(workspace_root)
        .iter()
        .any(|root| canonical_path.starts_with(root))
}

/// 读用途：路径是否落在受信技能根内（技能是产品只读资产，读取默认放行）。
///
/// 分域策略与加载侧 [`trusted_skill_roots_with_policy`] **完全一致**：
/// - 全局根（`allow_symlink_escape = true`，用户自有域）：按**词法**归属判定，允许技能目录 /
///   文件为符号链接并解析到根外（`npx skills` 与 Claude Code 的标准安装形态）；
/// - 项目级根（`allow_symlink_escape = false`，仓库内容不可信）：按 **canonical 包含**判定，
///   `<ws>/.claude/skills -> /tmp/evil` 不得把外部目录变成可默认读取的技能根。
///
/// 只服务 `PathPurpose::Read`：写入仍按 workspace 边界拒绝（技能目录不是可写域）。
fn is_read_within_trusted_skill_roots(
    lexical: &Path,
    canonical: &Path,
    workspace_root: Option<&Path>,
) -> bool {
    read_within_skill_roots(
        lexical,
        canonical,
        &trusted_skill_roots_with_policy(workspace_root),
    )
}

/// [`is_read_within_trusted_skill_roots`] 的纯函数本体（根清单注入，便于 hermetic 单测）。
fn read_within_skill_roots(
    lexical: &Path,
    canonical: &Path,
    roots: &[SkillTrustRoot],
) -> bool {
    roots.iter().any(|root| {
        let canonical_hit = root
            .canonical_root
            .as_deref()
            .map(|canonical_root| {
                components_within(&normalize_win_prefix(canonical_root), canonical)
            })
            .unwrap_or(false);
        if root.allow_symlink_escape {
            components_within(&normalize_win_prefix(&root.root), lexical) || canonical_hit
        } else {
            canonical_hit
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn test_root(tag: &str) -> PathBuf {
        let raw = std::env::temp_dir().join(format!("pa080-{tag}-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&raw);
        let root = raw.canonicalize().unwrap_or(raw);
        normalize_win_prefix(&root)
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

    #[cfg(windows)]
    #[test]
    fn win_case_variant_match_and_prefix_confusion() {
        let root = PathBuf::from(r"C:\WS");
        // `c:\ws\file` ⊂ `C:\WS` → 放行（Windows 组件级 ASCII 折叠）。
        let resolver = FakeResolver::new().with(r"c:\ws\file", r"C:\WS\file");
        let checker = make_checker(resolver);
        let permission = checker
            .classify(r"c:\ws\file", &root, &PathBuf::from(r"C:\WS\.tmp"), &no_auth(), PathPurpose::Read)
            .expect("case variant must be allowed on windows");
        assert_eq!(permission.zone, PermissionZone::WorkspaceRoot);

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

    /// POSIX 等价断言:组件比较不折叠 ASCII 大小写(`/WS` ⊄ `/ws`),且反斜杠只是普通
    /// 文件名字符、不得被当作路径分隔符而逃逸 root。
    #[cfg(unix)]
    #[test]
    fn posix_case_variant_and_backslash_do_not_escape_root() {
        let root = PathBuf::from("/ws");
        let tmp = PathBuf::from("/ws/.tmp");
        let resolver = FakeResolver::new().with("/WS/file", "/WS/file");
        let checker = make_checker(resolver);
        let err = checker
            .classify("/WS/file", &root, &tmp, &no_auth(), PathPurpose::Read)
            .expect_err("case variant must not fold into root");
        assert_eq!(err.code, PermissionErrorCode::RequiresAuthorization);

        // `c:\ws\file` 在 POSIX 是单个普通组件(相对路径)→ 按 root 基座解析,仍在 root 内,
        // 绝不因反斜杠被解读为 Windows 绝对路径而逃逸。
        let checker = make_checker(FakeResolver::new());
        let permission = checker
            .classify(r"c:\ws\file", &root, &tmp, &no_auth(), PathPurpose::Read)
            .expect("backslash literal stays a relative in-workspace name");
        assert_eq!(permission.zone, PermissionZone::WorkspaceRoot);
        assert!(permission.canonical.starts_with(&root), "{:?}", permission.canonical);
    }

    /// PA-142：两个平台分支都必须有真实断言。`cfg!` 是编译期常量——旧写法在 POSIX 上只执行
    /// `.expect(..)` 后便零断言报 ok（等价于被忽略的用例却伪装成通过）。
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
        } else {
            // POSIX：`\\?\C:\WS\file` 不是 Windows verbatim 绝对路径，只是普通相对文件名
            // （反斜杠不是分隔符）⇒ 必须以 root 为基座解析、不得被归一化成 `C:\WS\file`，
            // 更不得逃逸 root。
            assert_eq!(permission.zone, PermissionZone::WorkspaceRoot);
            assert_eq!(permission.canonical, root.join(r"\\?\C:\WS\file"));
            assert_ne!(permission.canonical, PathBuf::from(r"C:\WS\file"));
            assert!(
                permission.canonical.starts_with(&root),
                "POSIX verbatim literal must stay inside root: {:?}",
                permission.canonical
            );
        }
    }

    #[cfg(windows)]
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

    /// POSIX 等价断言:卷边界在 POSIX 上表现为"跨根绝对路径"(独立于 root 的另一个根目录)。
    /// 不存在盘符折叠 ⇒ 必须 Err(RequiresAuthorization),且同 root 内的绝对路径仍放行(负向对照)。
    #[cfg(unix)]
    #[test]
    fn posix_cross_root_absolute_path_is_rejected() {
        let root = PathBuf::from("/ws");
        let resolver = FakeResolver::new().with("/other/file", "/other/file");
        let checker = make_checker(resolver);
        let err = checker
            .classify("/other/file", &root, &PathBuf::from("/ws/.tmp"), &no_auth(), PathPurpose::Read)
            .expect_err("cross-root absolute path must be rejected");
        assert_eq!(err.code, PermissionErrorCode::RequiresAuthorization);

        // 负向对照:同 root 内的绝对路径必须放行,避免"一律拒绝"式假绿。
        let checker = make_checker(FakeResolver::new());
        let permission = checker
            .classify("/ws/file", &root, &PathBuf::from("/ws/.tmp"), &no_auth(), PathPurpose::Read)
            .expect("in-root absolute path must be allowed");
        assert_eq!(permission.zone, PermissionZone::WorkspaceRoot);
        assert_eq!(permission.canonical, PathBuf::from("/ws/file"));
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

        // ADR 0029 作用域语义：**只读**授权不得升级为写（Fail-Closed）。
        store.grant(outside_file.clone()).unwrap();
        let readonly_write_err = checker
            .classify("/outside/a.txt", &root, &PathBuf::from("/ws/.tmp"), &store, PathPurpose::Write)
            .expect_err("read-only authorization must NOT permit external write");
        assert_eq!(
            readonly_write_err.code,
            PermissionErrorCode::OutsideWorkspaceWriteDenied
        );
        assert!(store.revoke(&outside_file));

        // read-write 授权才解除外部写阻断；撤销后重新阻断并带升权提示。
        store
            .grant_with_scope(outside_file.clone(), PathAuthorizationScope::ReadWrite)
            .unwrap();
        let write_permission = checker
            .classify("/outside/a.txt", &root, &PathBuf::from("/ws/.tmp"), &store, PathPurpose::Write)
            .expect("read-write authorized write must be allowed");
        assert_eq!(write_permission.zone, PermissionZone::AuthorizedExternal);
        assert_eq!(
            write_permission
                .matched_authorization
                .as_ref()
                .map(|entry| entry.scope),
            Some(PathAuthorizationScope::ReadWrite)
        );

        assert!(store.revoke(&outside_file));
        let write_err = checker
            .classify("/outside/a.txt", &root, &PathBuf::from("/ws/.tmp"), &store, PathPurpose::Write)
            .expect_err("revoked write must be denied");
        assert_eq!(write_err.code, PermissionErrorCode::OutsideWorkspaceWriteDenied);
        // B204：拒绝文案只给可执行替代路径，不得再承诺运行时不存在的「升级审批」。
        assert!(
            write_err.message.contains("受控可写目录"),
            "拒绝文案必须给出可写替代目录，实际 = {:?}",
            write_err.message
        );
        assert!(
            !write_err.message.contains("escalation available"),
            "不得承诺运行时不产生的升级审批，实际 = {:?}",
            write_err.message
        );

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
        // 夹具:被符号链接指向的外部目标文件必须真实存在,否则 canonicalize 得到
        // NotFound ⇒ PermissionDenied,而本用例断言的是 RequiresAuthorization。
        std::fs::write(outside.join("f.txt"), "secret").expect("write outside fixture");
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

    // ── 受信技能根：读取默认放行（B236）────────────────────────────────────

    /// 仓库根技能目录不在会话 workspace 内时：读默认放行（TrustedSkillRoot），写仍拒绝，
    /// 非技能路径照旧要授权 —— 三条断言锁死「只放宽技能只读」的边界。
    #[test]
    fn read_within_project_skill_root_is_trusted_but_write_is_not() {
        let repo = test_root("skillrepo");
        prepare_root(&repo);
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        let ws = repo.join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        let skill_dir = repo.join(".agents/skills/demo");
        std::fs::create_dir_all(&skill_dir).unwrap();
        let skill_file = skill_dir.join("SKILL.md");
        std::fs::write(&skill_file, "# demo").unwrap();
        let other = repo.join("notes.txt");
        std::fs::write(&other, "notes").unwrap();

        let checker = PathPermissionChecker::with_default();
        let tmp = ws.join(".tmp");
        std::fs::create_dir_all(&tmp).unwrap();

        let read = checker
            .classify(
                &skill_file.display().to_string(),
                &ws,
                &tmp,
                &no_auth(),
                PathPurpose::Read,
            )
            .expect("skill file read must be default-allowed");
        assert_eq!(read.zone, PermissionZone::TrustedSkillRoot);

        let write = checker
            .classify(
                &skill_file.display().to_string(),
                &ws,
                &tmp,
                &no_auth(),
                PathPurpose::Write,
            )
            .expect_err("skill roots are read-only");
        assert_eq!(
            write.code,
            PermissionErrorCode::OutsideWorkspaceWriteDenied
        );

        let err = checker
            .classify(
                &other.display().to_string(),
                &ws,
                &tmp,
                &no_auth(),
                PathPurpose::Read,
            )
            .expect_err("non-skill external reads still need authorization");
        assert_eq!(err.code, PermissionErrorCode::RequiresAuthorization);

        let _ = std::fs::remove_dir_all(&repo);
    }

    /// 项目级技能根的符号链接逃逸不得变成可默认读取的外部目录（D1：项目内容不可信）。
    #[cfg(unix)]
    #[test]
    fn project_skill_root_symlink_escape_still_requires_authorization() {
        use std::os::unix::fs::symlink;
        let repo = test_root("skilllinkrepo");
        prepare_root(&repo);
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        let ws = repo.join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        let outside = test_root("skilllinkoutside");
        prepare_root(&outside);
        std::fs::write(outside.join("SKILL.md"), "# evil").unwrap();
        let link = repo.join(".claude/skills");
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        if symlink(&outside, &link).is_err() {
            let _ = std::fs::remove_dir_all(&repo);
            let _ = std::fs::remove_dir_all(&outside);
            return;
        }
        let checker = PathPermissionChecker::with_default();
        let tmp = ws.join(".tmp");
        std::fs::create_dir_all(&tmp).unwrap();
        let err = checker
            .classify(
                &link.join("SKILL.md").display().to_string(),
                &ws,
                &tmp,
                &no_auth(),
                PathPurpose::Read,
            )
            .expect_err("project skill root escape must stay fail-closed");
        assert_eq!(err.code, PermissionErrorCode::RequiresAuthorization);
        let _ = std::fs::remove_dir_all(&repo);
        let _ = std::fs::remove_dir_all(&outside);
    }

    /// B230：授权条目与命中路径必须同形——调用方传 Windows verbatim 形态时，
    /// grant/find/revoke 都按 `normalize_canonical` 归一（与 `classify_path` 同源）。
    #[test]
    fn grant_and_find_normalize_windows_verbatim_prefix() {
        let store = AuthorizeStore::new();
        let verbatim = PathBuf::from(r"\\?\C:\ws\outside.txt");
        let readable = PathBuf::from(r"C:\ws\outside.txt");
        store
            .grant_with_scope(verbatim, PathAuthorizationScope::Read)
            .expect("授权 verbatim 形态");
        assert!(
            store.find_authorization(&readable).is_some(),
            "可读绝对路径必须命中同一授权条目"
        );
        assert!(
            store.revoke(&readable),
            "revoke 同口径：可读路径必须能撤销 verbatim 授权"
        );
        assert!(store.find_authorization(&readable).is_none());
    }

    /// 读放行策略必须与加载侧分域策略逐字一致：全局根允许词法归属（符号链接解析到根外），
    /// 项目根只认 canonical 包含。
    #[test]
    fn skill_root_read_policy_matches_load_side_domains() {
        let global_root = PathBuf::from("/home/u/.claude/skills");
        let lexical = global_root.join("demo/SKILL.md");
        let escaped = PathBuf::from("/opt/shared/demo/SKILL.md");

        let global_policy = vec![SkillTrustRoot {
            root: global_root.clone(),
            canonical_root: Some(global_root.clone()),
            allow_symlink_escape: true,
        }];
        assert!(
            read_within_skill_roots(&lexical, &escaped, &global_policy),
            "全局根（用户自有域）允许符号链接解析到根外"
        );

        let project_policy = vec![SkillTrustRoot {
            root: global_root,
            canonical_root: Some(PathBuf::from("/home/u/.claude/skills")),
            allow_symlink_escape: false,
        }];
        assert!(
            !read_within_skill_roots(&lexical, &escaped, &project_policy),
            "项目根逃逸目标不得默认放行"
        );
        assert!(
            read_within_skill_roots(&lexical, &lexical, &project_policy),
            "canonical 落在根内的技能文件仍应放行"
        );
    }

    #[test]
    fn classify_with_options_allows_external_read_but_still_denies_external_write() {
        let root = test_root("extread-ws");
        prepare_root(&root);
        let outside = test_root("extread-out");
        prepare_root(&outside);
        let ext_file = outside.join("secret.txt");
        std::fs::write(&ext_file, "hello").unwrap();

        let checker = PathPermissionChecker::with_default();
        let tmp = root.join(".tmp");
        std::fs::create_dir_all(&tmp).unwrap();
        let auth = no_auth();

        // 默认不放行外部读
        let err = checker
            .classify_with_options(
                &ext_file.display().to_string(),
                &root,
                &tmp,
                &auth,
                PathPurpose::Read,
                false,
            )
            .expect_err("default external read must require authorization");
        assert_eq!(err.code, PermissionErrorCode::RequiresAuthorization);

        // allow_external_read = true ⇒ 放行外部读
        let perm = checker
            .classify_with_options(
                &ext_file.display().to_string(),
                &root,
                &tmp,
                &auth,
                PathPurpose::Read,
                true,
            )
            .expect("allow_external_read=true must allow external read");
        assert_eq!(perm.zone, PermissionZone::AuthorizedExternal);

        // 2A：allow_external_read = true 时外部写仍必须被拒绝
        let write_err = checker
            .classify_with_options(
                &outside.join("new.txt").display().to_string(),
                &root,
                &tmp,
                &auth,
                PathPurpose::Write,
                true,
            )
            .expect_err("external write must stay denied even when allow_external_read=true");
        assert_eq!(
            write_err.code,
            PermissionErrorCode::OutsideWorkspaceWriteDenied
        );

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }
}
