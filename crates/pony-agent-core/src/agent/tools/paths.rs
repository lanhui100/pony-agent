//! workspace 路径解析与判定：workspace 相对路径、root 解析、路径分类（读/写/授权）、
//! 受控 tmp 目录、别名额与路径修复。供文件工具面与 Run cwd 解析共用。
use super::*;

pub(super) const MAX_PATH_REPAIR_SEARCH_FILES: usize = 2_000;

impl ToolRouter {
    pub(crate) fn extract_call_workspace_id<'a>(
        &self,
        call: &'a ToolCall,
        context: &'a ToolExecutionContext,
    ) -> Option<&'a str> {
        call.arguments
            .get("workspaceId")
            .or_else(|| call.arguments.get("workspace_id"))
            .and_then(Value::as_str)
            .or(context.workspace_id.as_deref())
    }

    pub(super) fn resolve_workspace_path(
        &self,
        raw_path: &str,
        context: &ToolExecutionContext,
        workspace_id: Option<&str>,
    ) -> Result<PathBuf, (String, String)> {
        let ws_id = workspace_id.or(context.workspace_id.as_deref());
        let trimmed = raw_path.trim();
        if trimmed.is_empty() {
            return Err(("invalid_path".to_string(), "文件路径不能为空。".to_string()));
        }

        let canonical = match self.canonicalize_workspace_target(trimmed, context, ws_id) {
            Ok(canonical) => canonical,
            Err(primary_error) => match self.try_repair_file_path(trimmed, context, ws_id) {
                Ok(Some(repaired)) => repaired,
                Ok(None) => return Err(primary_error),
                Err(repair_error) => return Err(repair_error),
            },
        };
        if !canonical.is_file() {
            return Err((
                "invalid_path".to_string(),
                format!(
                    "目标不是文件：{}。",
                    self.display_resolved_workspace_relative(&canonical, context, ws_id)
                ),
            ));
        }

        Ok(canonical)
    }
}

impl ToolRouter {
    /// 会话级 root 解析（PA-080 consultant 方案 A / Phase 1 加固）：优先取调用上下文的会话 workspace root
    /// （`execute_with_workspace_root` / `execute_with_context` 注入，显式不可变）；
    /// 其次按 `workspace_id` 经注入的 resolver 解析；
    /// 若提供了未注册或不存在的 `workspace_id`，严格 Fail-closed 拒绝并返回错误，绝不静默回退默认 cwd；
    /// 仅在未提供上下文且未指定 `workspace_id` 时，回退构造时的默认 root。
    pub(super) fn resolved_workspace_root(
        &self,
        context: &ToolExecutionContext,
        workspace_id: Option<&str>,
    ) -> Result<PathBuf, (String, String)> {
        if let Some(session_root) = &context.workspace_root {
            return Ok(session_root.clone());
        }
        let ws_id = workspace_id.or(context.workspace_id.as_deref());
        if let Some(ws_id) = ws_id.filter(|value| !value.trim().is_empty()) {
            if let Some(resolver) = &self.root_resolver {
                if let Some(root) = resolver(ws_id) {
                    return Ok(root);
                }
                return Err((
                    "invalid_workspace".to_string(),
                    format!("工作区 `{ws_id}` 未注册或不存在。"),
                ));
            } else {
                return Err((
                    "invalid_workspace".to_string(),
                    format!("工作区 `{ws_id}` 未注册。"),
                ));
            }
        }
        Ok(self.workspace_root.clone())
    }
}

impl ToolRouter {
    /// 受控 tmp 目录（PA-080）：`<root>/.tmp/`，与 PA-078 导入目录布局一致。
    pub(super) fn controlled_tmp_dir(&self, root: &Path) -> PathBuf {
        root.join(".tmp")
    }
}

impl ToolRouter {
    /// 受控 tmp 判定目录列表（PA-080 consultant 修复 / Phase 1 多工作区物理隔离）：除 `<root>/.tmp/` 外，
    /// 覆盖 PA-078 附件导入的 fallback 布局 `<temp_dir>/pony-agent/<workspace_id>/.tmp/imports/`，
    /// 保证多工作区并发任务的 fallback 临时目录物理隔离。
    pub(super) fn controlled_tmp_dirs(&self, root: &Path, workspace_id: Option<&str>) -> Vec<PathBuf> {
        let ws_subdir = workspace_id.unwrap_or("default");
        vec![
            self.controlled_tmp_dir(root),
            std::env::temp_dir()
                .join(crate::agent::attachment_import::FALLBACK_TEMP_SUBDIR)
                .join(ws_subdir)
                .join(crate::agent::attachment_import::IMPORT_RELATIVE_DIR),
        ]
    }
}

impl ToolRouter {
    /// 统一路径权限判定（PA-080）：读/写工具一律经 `classify_path`。
    /// 返回 `(错误码, 消息)` 结构化信封：权限错误码（`requires_authorization` /
    /// `outside_workspace_write_denied` / `permission_denied`）作为顶层错误码透出，
    /// 前端授权 UI 可直接消费（consultant P1 修复）。
    pub(super) fn classify_workspace_path(
        &self,
        raw_path: &str,
        purpose: crate::agent::path_permission::PathPurpose,
        context: &ToolExecutionContext,
        workspace_id: Option<&str>,
    ) -> Result<PathBuf, (String, String)> {
        use crate::agent::path_permission::{PermissionErrorCode, PermissionZone};
        let ws_id = workspace_id.or(context.workspace_id.as_deref());
        let root = self.resolved_workspace_root(context, ws_id)?;
        let tmp_dirs = self.controlled_tmp_dirs(&root, ws_id);
        // 依次对 root + 各受控 tmp 区域判定：命中任一即放行（组件级比较，无 IO 副作用）。
        // 全部未命中时，最后一次判定的错误码（requires_authorization / outside_workspace_write_denied）透出。
        let mut last_error: Option<(String, String)> = None;
        let mut zones = vec![root.clone()];
        zones.extend(tmp_dirs.iter().cloned());
        for zone_root in &zones {
            let tmp = zone_root.join(".tmp");
            match self
                .path_checker
                .classify(raw_path, &root, &tmp, &self.authorize_store, purpose)
            {
                Ok(permission) => {
                    if matches!(
                        permission.zone,
                        PermissionZone::WorkspaceRoot | PermissionZone::ControlledTmp
                    ) {
                        return Ok(permission.canonical);
                    }
                    // AuthorizedExternal 放行。注意：写边界由 `path_permission::classify` 按
                    // ADR 0029 的授权作用域把关——只有 read-write 授权才会在 Write 判定下
                    // 返回 AuthorizedExternal；只读授权在 classify 内即被拒绝。
                    if permission.zone == PermissionZone::AuthorizedExternal {
                        return Ok(permission.canonical);
                    }
                }
                Err(error) => {
                    let code = match error.code {
                        PermissionErrorCode::RequiresAuthorization => "requires_authorization",
                        PermissionErrorCode::OutsideWorkspaceWriteDenied => {
                            "outside_workspace_write_denied"
                        }
                        PermissionErrorCode::PermissionDenied => "permission_denied",
                    };
                    last_error = Some((code.to_string(), error.message));
                }
            }
        }
        Err(last_error.unwrap_or_else(|| {
            (
                "permission_denied".to_string(),
                "路径权限判定失败。".to_string(),
            )
        }))
    }

    pub(super) fn prepare_workspace_file_path(
        &self,
        raw_path: &str,
        context: &ToolExecutionContext,
        workspace_id: Option<&str>,
    ) -> Result<PathBuf, (String, String)> {
        use crate::agent::path_permission::PathPurpose;
        let ws_id = workspace_id.or(context.workspace_id.as_deref());
        let trimmed = raw_path.trim();
        if trimmed.is_empty() {
            return Err(("invalid_path".to_string(), "文件路径不能为空。".to_string()));
        }

        // PA-080：写路径统一经 classify_path(Write)，workspace 外写返回
        // `outside_workspace_write_denied`；写新文件复用"最近存在祖先 + 后缀组件校验"语义。
        self.classify_workspace_path(trimmed, PathPurpose::Write, context, ws_id)
    }

    pub(super) fn resolve_workspace_entry(
        &self,
        raw_path: &str,
        context: &ToolExecutionContext,
        workspace_id: Option<&str>,
    ) -> Result<PathBuf, (String, String)> {
        let ws_id = workspace_id.or(context.workspace_id.as_deref());
        let trimmed = if raw_path.trim().is_empty() {
            "."
        } else {
            raw_path.trim()
        };
        match self.canonicalize_workspace_target(trimmed, context, ws_id) {
            Ok(canonical) => Ok(canonical),
            Err(primary_error) => match self.try_repair_file_path(trimmed, context, ws_id) {
                Ok(Some(repaired)) => Ok(repaired),
                Ok(None) => Err(primary_error),
                Err(repair_error) => Err(repair_error),
            },
        }
    }

    pub(super) fn resolve_workspace_dir(
        &self,
        raw_path: &str,
        context: &ToolExecutionContext,
        workspace_id: Option<&str>,
    ) -> Result<PathBuf, (String, String)> {
        let ws_id = workspace_id.or(context.workspace_id.as_deref());
        let trimmed = if raw_path.trim().is_empty() {
            "."
        } else {
            raw_path.trim()
        };
        let canonical = self.canonicalize_workspace_target(trimmed, context, ws_id)?;
        if !canonical.is_dir() {
            return Err((
                "invalid_path".to_string(),
                format!(
                    "目标不是目录：{}。",
                    self.display_resolved_workspace_relative(&canonical, context, ws_id)
                ),
            ));
        }

        Ok(canonical)
    }
}

impl ToolRouter {
    /// Run 命令 cwd 专用解析（PA-080 修复，code-review P1-1）：cwd 必须是 workspace 根内或受控
    /// tmp 内，授权的外部目录不得作为命令工作目录（spec：`cwd` SHALL be inside the workspace
    /// root or the controlled tmp）。与 `resolve_workspace_dir` 的差异：不做 Read 授权回退，
    /// 外部目录一律拒绝（`outside_workspace_write_denied` 语义）。
    pub(super) fn resolve_workspace_dir_for_execution(
        &self,
        raw_path: &str,
        context: &ToolExecutionContext,
        workspace_id: Option<&str>,
    ) -> Result<PathBuf, (String, String)> {
        use crate::agent::path_permission::PathPurpose;
        let ws_id = workspace_id.or(context.workspace_id.as_deref());
        let trimmed = if raw_path.trim().is_empty() {
            "."
        } else {
            raw_path.trim()
        };
        let canonical =
            self.classify_workspace_path(trimmed, PathPurpose::Write, context, ws_id)?;
        if !canonical.is_dir() {
            return Err((
                "invalid_cwd".to_string(),
                format!(
                    "目标不是目录：{}。",
                    self.display_resolved_workspace_relative(&canonical, context, ws_id)
                ),
            ));
        }
        Ok(canonical)
    }

    pub(super) fn canonicalize_workspace_target(
        &self,
        raw_path: &str,
        context: &ToolExecutionContext,
        workspace_id: Option<&str>,
    ) -> Result<PathBuf, (String, String)> {
        use crate::agent::path_permission::PathPurpose;
        let ws_id = workspace_id.or(context.workspace_id.as_deref());
        let root = self.canonical_resolved_root(context, ws_id)?;
        let input = PathBuf::from(raw_path);
        let candidate = if input.is_absolute() {
            input
        } else {
            let base_root = self.resolved_workspace_root(context, ws_id)?;
            base_root.join(raw_path)
        };
        let canonical = match candidate.canonicalize() {
            Ok(value) => value,
            Err(error) => {
                // PA-080（Windows B 类修复）：`..` 越过 root 的逃逸路径在 Windows 上
                // canonicalize 直接失败（越过卷根或逃逸目标不存在），使下方逃逸判定分支
                // 不可达而误落 invalid_path。先做词法逃逸判定：词法折叠后已不在 root 内 →
                // 按 PA-080 意图返回 requires_authorization（宿主审批语义），不吞成 invalid_path。
                if !lexically_within_root(&root, &candidate) {
                    let display = candidate.display().to_string();
                    return Err((
                        "requires_authorization".to_string(),
                        format!(
                            "读取路径 {} 位于工作区之外且未获得授权，需要用户显式授权后才能读取。",
                            display
                        ),
                    ));
                }
                return Err((
                    "invalid_path".to_string(),
                    format!("无法解析路径 {}：{}", raw_path, error),
                ));
            }
        };

        // 同一 Windows 前缀问题（安全面，但与既有意图一致）：`root` 来自
        // `canonical_resolved_root`（已归一为 plain），而 `canonical` 是 `canonicalize()` 的原始
        // 输出（Windows 带 `\\?\`）。不归一的话工作区内路径会被误判为越界，白白落到下面的
        // 授权分支（`classify_workspace_path` 内部会再归一，所以历史上"看起来还能放行"，
        // 但语义上不该走到这里）。两侧统一形态后再判定；Linux 无前缀 ⇒ 恒等。
        let root_for_within = normalize_if_verbatim(&root);
        let canonical_for_within = normalize_if_verbatim(&canonical);
        if !is_within_root(&root_for_within, &canonical_for_within) {
            // PA-080：workspace 外读 → 授权清单命中放行（classify_path(Read) 判定），
            // 未命中返回 `requires_authorization` 结构化错误。
            let display = canonical.display().to_string();
            return self
                .classify_workspace_path(&display, PathPurpose::Read, context, ws_id)
                .map_err(|(code, message)| {
                    if code == "requires_authorization" {
                        (code, message)
                    } else {
                        (code, format!("只允许访问当前工作区内的相对路径。{message}"))
                    }
                });
        }

        // **返回形态必须与改动前一致（plain 归一形态）**：改动前这类"工作区内路径"走的是上方
        // `classify_workspace_path` 分支，返回的是 `permission.canonical`（已归一为 plain）。
        // 若这里直接返回 `canonicalize()` 的原始 verbatim 形态，下游的相对化
        // （`display_resolved_workspace_relative` 的 `path.strip_prefix(plain_root)`）会失配，
        // 退化成绝对路径、再经 `replace('\\', "/")` 变成 `//?/C:/...` —— Windows 上
        // `search_text` 的路径契约（workspace 相对）就是这样被打破的。
        // 结论：**边界判定用归一形态（保留 Windows 修复），返回值用归一形态（保持对外语义）**。
        Ok(normalize_if_verbatim(&canonical))
    }

    pub(super) fn canonical_resolved_root(
        &self,
        context: &ToolExecutionContext,
        workspace_id: Option<&str>,
    ) -> Result<PathBuf, (String, String)> {
        let ws_id = workspace_id.or(context.workspace_id.as_deref());
        let raw_root = self.resolved_workspace_root(context, ws_id)?;
        let canonical = raw_root.canonicalize().unwrap_or_else(|_| raw_root);
        Ok(crate::agent::path_permission::normalize_canonical(
            &canonical,
        ))
    }

    pub(super) fn canonical_workspace_root(&self) -> PathBuf {
        self.canonical_resolved_root(&ToolExecutionContext::default(), None)
            .unwrap_or_else(|_| self.workspace_root.clone())
    }

    pub(super) fn display_workspace_relative(&self, path: &Path) -> String {
        self.display_resolved_workspace_relative(path, &ToolExecutionContext::default(), None)
    }

    pub(super) fn display_resolved_workspace_relative(
        &self,
        path: &Path,
        context: &ToolExecutionContext,
        workspace_id: Option<&str>,
    ) -> String {
        let ws_id = workspace_id.or(context.workspace_id.as_deref());
        let root = self
            .canonical_resolved_root(context, ws_id)
            .unwrap_or_else(|_| self.canonical_workspace_root());
        // 兜底：调用方若传入 verbatim 形态（Windows `\\\\?\\C:\\...`），先归一为 plain 再相对化，
        // 否则 `strip_prefix(plain_root)` 必然失配 → 输出绝对路径且前缀被转成 `//?/` 的怪形态。
        // Linux 无前缀 ⇒ 归一返回入参的 owned 副本（形态不变），行为零变化。
        let normalized_path = normalize_if_verbatim(path);
        let path = normalized_path.as_path();
        path.strip_prefix(&root)
            .ok()
            .map(|value| {
                let display = value.display().to_string().replace('\\', "/");
                if display.is_empty() {
                    ".".to_string()
                } else {
                    display
                }
            })
            .unwrap_or_else(|| path.display().to_string().replace('\\', "/"))
    }

    pub(super) fn try_repair_file_path(
        &self,
        raw_path: &str,
        context: &ToolExecutionContext,
        workspace_id: Option<&str>,
    ) -> Result<Option<PathBuf>, (String, String)> {
        let ws_id = workspace_id.or(context.workspace_id.as_deref());
        let normalized_raw_path = raw_path
            .trim()
            .trim_end_matches(['/', '\\'])
            .replace('\\', "/");
        let raw_path_object = Path::new(raw_path);
        let file_name = raw_path_object
            .file_name()
            .and_then(|value| value.to_str())
            .map(str::trim)
            .filter(|value| !value.is_empty() && *value != "." && *value != "..");

        let Some(file_name) = file_name else {
            return Ok(None);
        };

        let root = self.canonical_resolved_root(context, ws_id)?;
        let mut files = Vec::new();
        if collect_files_recursively(&root, &mut files, MAX_PATH_REPAIR_SEARCH_FILES).is_err() {
            return Ok(None);
        }

        let exact_name_matches = files
            .iter()
            .filter(|path| {
                path.file_name()
                    .and_then(|value| value.to_str())
                    .map(|value| value.eq_ignore_ascii_case(file_name))
                    .unwrap_or(false)
            })
            .cloned()
            .collect::<Vec<_>>();

        match exact_name_matches.len() {
            0 => {}
            1 => return Ok(exact_name_matches.into_iter().next()),
            _ => {
                let candidates = exact_name_matches
                    .iter()
                    .take(5)
                    .map(|path| self.display_resolved_workspace_relative(path, context, ws_id))
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err((
                    "invalid_path".to_string(),
                    format!(
                        "无法解析路径 {}：工作区内发现多个同名文件 {}，请提供更精确的相对路径。",
                        raw_path, candidates
                    ),
                ));
            }
        }

        let stem_matches = files
            .iter()
            .filter(|path| {
                let relative = match path.strip_prefix(&root) {
                    Ok(value) => value,
                    Err(_) => return false,
                };
                let relative_without_extension = relative
                    .parent()
                    .map(|parent| parent.join(relative.file_stem().unwrap_or_default()))
                    .unwrap_or_else(|| PathBuf::from(relative.file_stem().unwrap_or_default()));

                relative_without_extension
                    .display()
                    .to_string()
                    .replace('\\', "/")
                    .eq_ignore_ascii_case(&normalized_raw_path)
                    || path
                        .file_stem()
                        .and_then(|value| value.to_str())
                        .map(|value| value.eq_ignore_ascii_case(file_name))
                        .unwrap_or(false)
            })
            .cloned()
            .collect::<Vec<_>>();

        match stem_matches.len() {
            0 => Ok(None),
            1 => Ok(stem_matches.into_iter().next()),
            _ => {
                let candidates = stem_matches
                    .iter()
                    .take(5)
                    .map(|path| self.display_resolved_workspace_relative(path, context, ws_id))
                    .collect::<Vec<_>>()
                    .join(", ");
                Err((
                    "invalid_path".to_string(),
                    format!(
                        "无法解析路径 {}：工作区内发现多个缺扩展名候选文件 {}，请提供更精确的相对路径。",
                        raw_path, candidates
                    ),
                ))
            }
        }
    }
}

pub(super) fn truncate_preview(content: &str, max_chars: usize) -> String {
    let preview = content.chars().take(max_chars).collect::<String>();
    if content.chars().count() > max_chars {
        format!(
            "{}\n\n[内容已截断，当前仅展示前 {} 个字符]",
            preview, max_chars
        )
    } else {
        preview
    }
}

pub(super) fn is_within_root(root: &Path, path: &Path) -> bool {
    path.starts_with(root)
}

/// 把「Windows verbatim 前缀形态」的路径归一为 plain 形态；**无前缀时原样返回（恒等）**。
///
/// 为什么要收口成一个函数：`Path::starts_with` 是**逐组件**比较，
/// Windows 上 `\\?\C:\a` 的首组件是 `Prefix(VerbatimDisk("C"))`，而 `C:\a` 是
/// `Prefix(Disk("C"))` —— 两者不相等，于是「canonicalize 出来的 verbatim 路径」与
/// 「会话里已归一（`workspace::normalize_workspace_root` 剥掉 `\\?\`）的 plain root」做
/// 前缀判定会**恒为假**。触达指令注入（`with_touched_instructions`）正是栽在这里：
/// 整段附加指令在 Windows 上静默消失且无任何报错。
///
/// 归一实现复用已存在的 [`crate::agent::path_permission::normalize_canonical`]（同一套
/// `\\?\` / `\\?\UNC\` 剥离规则），不新增第二套规则。这里只加了一个**前缀守卫**：
/// 仅当路径字节以 `\\?\` 起时才调用归一（生产侧两个调用点传入的都是绝对路径：Linux 绝对
/// 路径必以 `/` 起，永不命中），其余情况直接返回 `PathBuf`。
/// 这样：
/// 1. Linux/macOS 上绝对路径必以 `/` 起，**永远不会**命中该前缀分支 ⇒ 恒等 ⇒ 行为零变化；
/// 2. 避免对非 UTF-8 路径走 `display()` 造成有损转换（`normalize_canonical` 内部用
///    `display().to_string()`，只在 Windows verbatim 场景才需要）；
/// 3. 判定只看字节前缀、**不调宿主 `Path::is_absolute()`**，因此 Windows 规则可以在 Linux 上
///    被镜像单测（与 `tui_dialog_wave39.rs` 的 `PathForms` 同一教训：宿主相关的判据没法跨平台验证）。
pub(super) fn normalize_if_verbatim(path: &Path) -> PathBuf {
    if path.as_os_str().as_encoded_bytes().starts_with(b"\\\\?\\") {
        crate::agent::path_permission::normalize_canonical(path)
    } else {
        path.to_path_buf()
    }
}

/// 词法逃逸判定（Windows B 类修复）：canonicalize 失败时（如相对路径 `..` 越过卷根或
/// 逃逸目标不存在），Windows 上无法像 Linux 那样解析出 canonical 再做 `is_within_root`，
/// 导致逃逸判定分支不可达、误落 `invalid_path`。这里在 IO 失败分支用词法折叠兜底：
/// 折叠 `..` / `.` 组件（卷根之上钳制：`C:\..` 语义仍是 `C:\`），再做组件级前缀判定。
/// 判定前统一剥 `\\?\` 前缀（与 `canonical_resolved_root` 的 root 形式一致），避免
/// Prefix 组件（`\\?\C:` vs `C:`）不一致造成误判。
pub(super) fn lexically_within_root(root: &Path, candidate: &Path) -> bool {
    use crate::agent::path_permission::normalize_canonical;

    #[derive(Clone, PartialEq, Debug)]
    enum FoldComponent {
        Prefix(String),
        RootDir,
        Normal(String),
    }

    fn fold_lexically(path: &Path) -> Vec<FoldComponent> {
        let mut folded: Vec<FoldComponent> = Vec::new();
        for component in path.components() {
            match component {
                Component::CurDir => {}
                Component::ParentDir => {
                    // 只折叠 Normal 组件；Prefix/RootDir 之上的 `..` 钳制
                    //（卷根之上不可再逃逸，`C:\..` 仍是 `C:\`）。
                    if let Some(FoldComponent::Normal(_)) = folded.last() {
                        folded.pop();
                    }
                }
                Component::Prefix(prefix) => folded.push(FoldComponent::Prefix(
                    prefix.as_os_str().to_string_lossy().to_string(),
                )),
                Component::RootDir => folded.push(FoldComponent::RootDir),
                Component::Normal(part) => folded.push(FoldComponent::Normal(
                    part.to_string_lossy().to_string(),
                )),
            }
        }
        folded
    }

    let root_folded = fold_lexically(&normalize_canonical(root));
    let candidate_folded = fold_lexically(&normalize_canonical(candidate));
    if candidate_folded.len() < root_folded.len() {
        return false;
    }
    for (index, root_part) in root_folded.iter().enumerate() {
        let candidate_part = &candidate_folded[index];
        let equal = match (root_part, candidate_part) {
            (FoldComponent::Prefix(left), FoldComponent::Prefix(right))
            | (FoldComponent::Normal(left), FoldComponent::Normal(right)) => {
                if cfg!(windows) {
                    left.eq_ignore_ascii_case(right)
                } else {
                    left == right
                }
            }
            (left, right) => left == right,
        };
        if !equal {
            return false;
        }
    }
    true
}

pub(super) fn preview_text(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        text.to_string()
    } else {
        let preview = text.chars().take(max_chars).collect::<String>();
        format!("{}...(+{} chars)", preview, count - max_chars)
    }
}

pub(super) fn collect_files_recursively(
    dir: &Path,
    files: &mut Vec<PathBuf>,
    max_files: usize,
) -> std::io::Result<()> {
    if files.len() >= max_files {
        return Ok(());
    }

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            if should_skip_dir(&path) {
                continue;
            }
            collect_files_recursively(&path, files, max_files)?;
            if files.len() >= max_files {
                break;
            }
        } else if path.is_file() {
            files.push(path);
            if files.len() >= max_files {
                break;
            }
        }
    }

    Ok(())
}

pub(super) fn should_skip_dir(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|value| value.to_str()),
        Some(".git" | "node_modules" | "target" | "dist" | "build")
    )
}

pub(super) fn unique_paths(paths: Vec<String>) -> Vec<String> {
    let mut unique = Vec::new();
    for path in paths {
        if !unique.iter().any(|existing| existing == &path) {
            unique.push(path);
        }
    }
    unique
}

/// Validate a path glob with the same globset settings the `SearchEngine` uses. Malformed
/// patterns fail closed so callers can surface a structured error instead of a panic.
pub(super) fn compile_path_glob(pattern: &str) -> Result<(), String> {
    globset::GlobBuilder::new(pattern)
        .case_insensitive(true)
        .literal_separator(true)
        .backslash_escape(true)
        .build()
        .map(|_| ())
        .map_err(|error| format!("invalid glob `{pattern}`: {error}"))
}

/// Mirrors the `SearchEngine`'s glob semantics (design Decision 9): a pattern matches the
/// workspace-relative path or its basename, so `*.rs` matches files in any subdirectory. Used by
/// the file-root branches of `search_text`/`glob_files` and by the file-root post-filter.
/// 与 search.rs 保持一致：无 glob 元字符的模式按"路径子串过滤"回退（如 `.rs` 匹配
/// `src/tauri_adapter.rs`），避免被 globset 当作字面量点文件名导致 0 命中。
pub(super) fn glob_matches_path_or_basename(relative: &str, pattern: &str) -> bool {
    let matcher = globset::GlobBuilder::new(pattern)
        .case_insensitive(true)
        .literal_separator(true)
        .backslash_escape(true)
        .build()
        .expect("path glob was validated up front")
        .compile_matcher();
    if matcher.is_match(relative) {
        return true;
    }
    let basename = relative.rsplit('/').next().unwrap_or(relative);
    if matcher.is_match(basename) {
        return true;
    }
    if !pattern
        .chars()
        .any(|ch| matches!(ch, '*' | '?' | '[' | ']' | '{' | '}' | '(' | ')' | '!'))
    {
        let lower_pattern = pattern.to_lowercase();
        if relative.to_lowercase().contains(&lower_pattern)
            || basename.to_lowercase().contains(&lower_pattern)
        {
            return true;
        }
    }
    false
}
