use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::SystemTime;

/// 指令文档（AGENTS.md / CLAUDE.md）的候选文件名，按优先级从高到低。
/// 项目级同时兼容 Claude Code 的 `.claude/CLAUDE.md` 布局。
const CONSTITUTION_CANDIDATES: [&str; 3] = ["AGENTS.md", "CLAUDE.md", ".claude/CLAUDE.md"];

/// 用户级（全局）根目录的环境变量覆盖入口。
pub const PONY_AGENT_HOME_ENV: &str = "PONY_AGENT_HOME";

/// 指令与技能的作用域（用户级全局 / 项目级）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AgentScope {
    Global,
    Project,
}

impl AgentScope {
    /// 稳定短标签，用于提示词标注与缓存键。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Project => "project",
        }
    }
}

/// 文件元数据与指纹（用于判断是否需要重新读取与编译）
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileFingerprint {
    pub path: PathBuf,
    pub mtime: Option<SystemTime>,
    pub file_size: u64,
    pub content_hash: u64, // FNV-1a 64-bit 快速内容校验
}

impl FileFingerprint {
    pub fn compute<P: AsRef<Path>>(path: P) -> Option<(Self, String)> {
        let path = path.as_ref();
        let metadata = fs::metadata(path).ok()?;
        let mtime = metadata.modified().ok();
        let file_size = metadata.len();
        let content = fs::read_to_string(path).ok()?;
        let content_hash = fnv1a_hash(content.as_bytes());

        Some((
            Self {
                path: path.to_path_buf(),
                mtime,
                file_size,
                content_hash,
            },
            content,
        ))
    }
}

pub fn fnv1a_hash(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for &byte in bytes {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3u64);
    }
    hash
}

/// 热加载解析后的 Skill 摘要（支持 YAML Frontmatter 标准字段）
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HotloadedSkill {
    pub name: String,
    pub description: String,
    pub user_invocable: bool,
    pub allowed_tools: Vec<String>,
    pub path: PathBuf,
    pub fingerprint: FileFingerprint,
    /// 该 Skill 来自用户级全局根目录还是当前工作区。
    pub scope: AgentScope,
    /// 注入排序权重（wave-42 契约字段，数值越小越先注入）。
    pub rank: u16,
}

/// 指令注入预算上限（wave-42 契约桩）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstructionBudget {
    pub max_bytes: usize,
    pub max_source_bytes: usize,
}

impl Default for InstructionBudget {
    fn default() -> Self {
        Self {
            max_bytes: 65_536,
            max_source_bytes: 1_048_576,
        }
    }
}

/// 单层指令文档（用户级或项目级宪法文件）
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConstitutionLayer {
    pub scope: AgentScope,
    pub source_path: PathBuf,
    pub content: String,
    pub fingerprint: FileFingerprint,
}

/// 宪法文件内容缓存（可能由多个层级组成：全局层在前，项目层在后）
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConstitutionCache {
    pub layers: Vec<ConstitutionLayer>,
}

/// 会话级已固化的静态基线（用于保障历史 KV Cache 不被截断冲刷）
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionBaselineSnapshot {
    pub constitution_hash: u64,
    pub constitution_content: String,
    pub skills_hash: u64,
    pub skills: Vec<HotloadedSkill>,
    /// 固化基线时使用的注入预算（wave-42 契约字段）。
    pub budget: InstructionBudget,
}

/// 针对会话的增量规则变更记录（DSH 风格的 Append-Only 变动事件）
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionDeltaUpdate {
    pub constitution_changed: bool,
    pub updated_constitution_summary: Option<String>,
    pub added_or_updated_skills: Vec<HotloadedSkill>,
    pub removed_skill_names: Vec<String>,
}

/// 统一热加载与 KV-Cache 保护管理器
#[derive(Debug, Default)]
pub struct WorkspaceHotloader {
    constitution: RwLock<Option<ConstitutionCache>>,
    /// 逐文件 Skill 缓存，键为 `{scope}:{目录名}`，避免重复读盘。
    skill_files: RwLock<BTreeMap<String, HotloadedSkill>>,
    /// 上次扫描的合并结果（按目录名去重，项目级覆盖全局级）。
    skills: RwLock<Vec<HotloadedSkill>>,
    /// 会话基线注册表：session_id -> SessionBaselineSnapshot
    session_baselines: RwLock<HashMap<String, SessionBaselineSnapshot>>,
}

static GLOBAL_HOTLOADER: std::sync::OnceLock<WorkspaceHotloader> = std::sync::OnceLock::new();

/// 获取进程级全局持久化热加载管理器，确保跨轮次持有基线
pub fn get_global_workspace_hotloader() -> &'static WorkspaceHotloader {
    GLOBAL_HOTLOADER.get_or_init(WorkspaceHotloader::new)
}

/// 纯逻辑：解析用户级（全局）根目录候选，按优先级从低到高返回。
///
/// 顺序（后者优先级更高，项目级永远在其之上）：
/// 1. `~/.agents`：跨 agent 共享根（与项目级 `.agents` 约定对称）
/// 2. `~/.claude`：Claude Code 兼容根（与项目级 `.claude` 约定对称）
/// 3. `config_dir()/pony-agent`：产品自有全局根
/// 4. `$PONY_AGENT_HOME`：显式覆盖（最高）
///
/// 结果去重且保留优先级顺序；任何输入缺失只跳过该来源，不产生兜底目录。
pub fn resolve_global_agent_homes_from(
    env_home: Option<&Path>,
    config_dir: Option<&Path>,
    home_dir: Option<&Path>,
) -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(home) = home_dir {
        candidates.push(home.join(".agents"));
        candidates.push(home.join(".claude"));
    }
    if let Some(config) = config_dir {
        candidates.push(config.join("pony-agent"));
    }
    if let Some(env) = env_home {
        candidates.push(env.to_path_buf());
    }

    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut resolved: Vec<PathBuf> = Vec::new();
    for candidate in candidates {
        if candidate.as_os_str().is_empty() {
            continue;
        }
        if seen.insert(candidate.clone()) {
            resolved.push(candidate);
        }
    }
    resolved
}

/// 解析当前进程的用户级（全局）根目录候选（读环境变量与系统目录）。
pub fn resolve_global_agent_homes() -> Vec<PathBuf> {
    let env_home = std::env::var_os(PONY_AGENT_HOME_ENV)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty());
    resolve_global_agent_homes_from(
        env_home.as_deref(),
        dirs::config_dir().as_deref(),
        dirs::home_dir().as_deref(),
    )
}

impl WorkspaceHotloader {
    pub fn new() -> Self {
        Self::default()
    }

    /// 热重载/缓存命中感知：加载工作区宪法文档（遵循 AGENTS.md / CLAUDE.md 标准）
    /// 返回值：(内容, 是否命中全局文件内存缓存)
    pub fn resolve_constitution<P: AsRef<Path>>(
        &self,
        workspace_root: P,
    ) -> Option<(String, bool)> {
        let global_homes = resolve_global_agent_homes();
        self.resolve_constitution_with_globals(workspace_root, &global_homes)
    }

    /// 可注入全局根目录的宪法解析变体（测试与宿主显式装配用）。
    ///
    /// 层级顺序：全局层（按 `global_homes` 顺序）在前，项目层在最后（最高优先级）。
    /// 每层在其根目录内取首个存在的候选文件（`AGENTS.md` → `CLAUDE.md` → `.claude/CLAUDE.md`）。
    /// 仅存在单一项目层时返回原始字节，保持既有提示词形态不变。
    pub fn resolve_constitution_with_globals<P: AsRef<Path>>(
        &self,
        workspace_root: P,
        global_homes: &[PathBuf],
    ) -> Option<(String, bool)> {
        let candidates = discover_constitution_sources(workspace_root.as_ref(), global_homes);
        if candidates.is_empty() {
            return None;
        }

        // 一级缓存判定：路径、作用域与文件元数据全部一致时无需重新读盘。
        {
            let read_guard = self.constitution.read().unwrap();
            if let Some(cached) = read_guard.as_ref() {
                let unchanged =
                    cached.layers.len() == candidates.len()
                        && cached.layers.iter().zip(candidates.iter()).all(
                            |(layer, (scope, path))| {
                                layer.scope == *scope
                                    && layer.source_path == *path
                                    && metadata_matches(&layer.fingerprint, path)
                            },
                        );
                if unchanged {
                    return Some((render_constitution(&cached.layers), true));
                }
            }
        }

        let mut layers: Vec<ConstitutionLayer> = Vec::with_capacity(candidates.len());
        for (scope, source_path) in candidates {
            // 文件可能在探测后被删除或变得不可读，跳过该层而不是丢弃整条链路。
            let Some((fingerprint, content)) = FileFingerprint::compute(&source_path) else {
                continue;
            };
            layers.push(ConstitutionLayer {
                scope,
                source_path,
                content,
                fingerprint,
            });
        }
        if layers.is_empty() {
            return None;
        }

        let rendered = render_constitution(&layers);
        *self.constitution.write().unwrap() = Some(ConstitutionCache { layers });
        Some((rendered, false))
    }

    /// 热重载/缓存命中感知：多层级扫描并解析 Skills
    /// （用户级全局根 + 项目内 `.agents/skills`、`.claude/skills`）
    /// 返回值：(Skills 列表有序且稳定输出, 是否完全命中缓存未变动)
    pub fn scan_skills<P: AsRef<Path>>(&self, workspace_root: P) -> (Vec<HotloadedSkill>, bool) {
        let global_homes = resolve_global_agent_homes();
        self.scan_skills_with_globals(workspace_root, &global_homes)
    }

    /// 可注入全局根目录的 Skills 扫描变体。
    ///
    /// 扫描顺序即优先级（后者覆盖前者）：全局根按 `global_homes` 顺序，随后项目级
    /// `.agents/skills` 与 `.claude/skills`。同名（同目录名）时项目级覆盖全局级。
    pub fn scan_skills_with_globals<P: AsRef<Path>>(
        &self,
        workspace_root: P,
        global_homes: &[PathBuf],
    ) -> (Vec<HotloadedSkill>, bool) {
        let skill_dirs = discover_skill_dirs(workspace_root.as_ref(), global_homes);
        let mut discovered: BTreeMap<String, HotloadedSkill> = BTreeMap::new();
        let mut file_cache: BTreeMap<String, HotloadedSkill> = BTreeMap::new();
        let mut any_changed = false;

        for (scope, base_dir) in skill_dirs {
            let entries = match fs::read_dir(&base_dir) {
                Ok(entries) => entries,
                Err(_) => continue,
            };

            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let folder_name = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    let skill_file = path.join("SKILL.md");
                    if !skill_file.is_file() {
                        continue;
                    }

                    let metadata = match fs::metadata(&skill_file) {
                        Ok(m) => m,
                        Err(_) => continue,
                    };
                    let mtime = metadata.modified().ok();
                    let file_size = metadata.len();
                    let cache_key = format!("{}:{}", scope.as_str(), folder_name);

                    let cached = {
                        let read_guard = self.skill_files.read().unwrap();
                        read_guard.get(&cache_key).cloned()
                    };

                    let skill = match cached {
                        Some(existing)
                            if existing.scope == scope
                                && existing.fingerprint.mtime == mtime
                                && existing.fingerprint.file_size == file_size =>
                        {
                            existing
                        }
                        _ => {
                            any_changed = true;
                            match FileFingerprint::compute(&skill_file) {
                                Some((fingerprint, content)) => {
                                    let (parsed_name, description, user_invocable, allowed_tools) =
                                        parse_skill_metadata(&content, &folder_name);
                                    HotloadedSkill {
                                        name: parsed_name,
                                        description,
                                        user_invocable,
                                        allowed_tools,
                                        path: skill_file,
                                        fingerprint,
                                        scope,
                                        rank: 0,
                                    }
                                }
                                None => continue,
                            }
                        }
                    };

                    file_cache.insert(cache_key, skill.clone());
                    // 后写入者优先级更高：项目级同名技能覆盖全局级同名技能。
                    discovered.insert(folder_name, skill);
                }
            }
        }

        let merged: Vec<HotloadedSkill> = discovered.into_values().collect();
        {
            let read_guard = self.skills.read().unwrap();
            if !any_changed && *read_guard == merged {
                return (read_guard.clone(), true);
            }
        }

        *self.skill_files.write().unwrap() = file_cache;
        *self.skills.write().unwrap() = merged.clone();
        (merged, false)
    }

    /// 会话 KV-Cache 保护机制（Append-Only Delta Update）：
    /// 1. 若会话为首次轮次（turn_count <= 1 或无基线）：将当前宪法与 Skills 固化为该会话的永久静态前缀基线；
    /// 2. 若会话已有基线且文件发生变动：**绝对不篡改头部静态基线**（防止全量历史 KV Cache 失效），而是计算出增量 Delta，生成一条追加在队尾的 Notice 提示。
    pub fn resolve_session_prompt_context<P: AsRef<Path>>(
        &self,
        session_id: &str,
        turn_count: usize,
        workspace_root: P,
    ) -> (String, Option<String>) {
        let global_homes = resolve_global_agent_homes();
        self.resolve_session_prompt_context_with_globals(
            session_id,
            turn_count,
            workspace_root,
            &global_homes,
        )
    }

    /// 可注入全局根目录的会话前缀变体（测试与宿主显式装配用）。
    pub fn resolve_session_prompt_context_with_globals<P: AsRef<Path>>(
        &self,
        session_id: &str,
        turn_count: usize,
        workspace_root: P,
        global_homes: &[PathBuf],
    ) -> (String, Option<String>) {
        let (current_constitution, _) = self
            .resolve_constitution_with_globals(&workspace_root, global_homes)
            .unwrap_or_else(|| (String::new(), true));
        let (current_skills, _) = self.scan_skills_with_globals(&workspace_root, global_homes);

        let const_hash = fnv1a_hash(current_constitution.as_bytes());
        let mut skill_hash_input = Vec::new();
        for s in &current_skills {
            skill_hash_input.extend_from_slice(s.name.as_bytes());
            skill_hash_input.extend_from_slice(s.scope.as_str().as_bytes());
            skill_hash_input.extend_from_slice(s.description.as_bytes());
            skill_hash_input.extend_from_slice(&s.fingerprint.content_hash.to_le_bytes());
        }
        let skills_hash = fnv1a_hash(&skill_hash_input);

        let mut baselines = self.session_baselines.write().unwrap();

        if turn_count <= 1 || !baselines.contains_key(session_id) {
            // 首轮：固化静态 Baseline
            let baseline = SessionBaselineSnapshot {
                constitution_hash: const_hash,
                constitution_content: current_constitution.clone(),
                skills_hash,
                skills: current_skills.clone(),
                budget: InstructionBudget::default(),
            };
            let static_prefix =
                self.build_cached_prompt_prefix(Some(&current_constitution), &current_skills);
            baselines.insert(session_id.to_string(), baseline);
            return (static_prefix, None);
        }

        // 非首轮：取出会话固化的基线，保持最头部静态 Prompt 前缀字节级完全相同（KV-Cache Hit 100%）
        let baseline = baselines.get(session_id).unwrap().clone();
        let stable_prefix =
            self.build_cached_prompt_prefix(Some(&baseline.constitution_content), &baseline.skills);

        // 检测是否存在中途变更
        let const_changed = baseline.constitution_hash != const_hash;
        let skills_changed = baseline.skills_hash != skills_hash;

        if !const_changed && !skills_changed {
            return (stable_prefix, None);
        }

        // 计算增量并生成追加消息（Append-Only Notification）
        let mut delta_notice =
            String::from("Notice: Workspace configuration was updated mid-session:\n");
        if const_changed {
            delta_notice.push_str(
                "- Workspace constitution/rules updated. Latest guidelines are active.\n",
            );
        }
        if skills_changed {
            let old_names: Vec<_> = baseline.skills.iter().map(|s| &s.name).collect();
            let new_names: Vec<_> = current_skills.iter().map(|s| &s.name).collect();
            let added: Vec<_> = new_names
                .iter()
                .filter(|n| !old_names.contains(n))
                .collect();
            let removed: Vec<_> = old_names
                .iter()
                .filter(|n| !new_names.contains(n))
                .collect();
            if !added.is_empty() {
                delta_notice.push_str(&format!("- Newly installed skills: {:?}\n", added));
            }
            if !removed.is_empty() {
                delta_notice.push_str(&format!("- Removed skills: {:?}\n", removed));
            }
        }

        (stable_prefix, Some(delta_notice))
    }

    /// 构造静态保真的 Prompt 前缀块，确保 Prompt Cache 最佳命中
    pub fn build_cached_prompt_prefix(
        &self,
        constitution_content: Option<&str>,
        skills: &[HotloadedSkill],
    ) -> String {
        let mut prefix = String::with_capacity(4096);

        if let Some(const_text) = constitution_content {
            if !const_text.trim().is_empty() {
                prefix.push_str("=== WORKSPACE CONSTITUTION & RULES ===\n");
                prefix.push_str(const_text.trim());
                prefix.push_str("\n\n");
            }
        }

        if !skills.is_empty() {
            prefix.push_str("=== AVAILABLE REUSABLE SKILLS ===\n");
            prefix.push_str(
                "The following project-specific skills are available and loadable on demand:\n",
            );
            for skill in skills {
                prefix.push_str(&format!(
                    "- `{}` [{}]: {}\n",
                    skill.name,
                    skill.scope.as_str(),
                    skill.description
                ));
            }
            prefix.push_str("\n");
        }

        prefix
    }
}

/// 探测单个根目录内的宪法文件：按候选顺序取首个存在者。
fn first_existing_constitution(root: &Path) -> Option<PathBuf> {
    for candidate in CONSTITUTION_CANDIDATES {
        let path = root.join(candidate);
        if path.exists() {
            return Some(path);
        }
    }
    None
}

/// 汇总宪法来源：全局根按顺序在前，项目根最后（最高优先级）。
fn discover_constitution_sources(
    workspace_root: &Path,
    global_homes: &[PathBuf],
) -> Vec<(AgentScope, PathBuf)> {
    let mut sources: Vec<(AgentScope, PathBuf)> = Vec::new();
    for home in global_homes {
        if let Some(path) = first_existing_constitution(home) {
            sources.push((AgentScope::Global, path));
        }
    }
    if let Some(path) = first_existing_constitution(workspace_root) {
        sources.push((AgentScope::Project, path));
    }
    sources
}

/// 汇总 Skills 根目录：全局根按顺序在前，项目级目录最后（最高优先级）。
fn discover_skill_dirs(
    workspace_root: &Path,
    global_homes: &[PathBuf],
) -> Vec<(AgentScope, PathBuf)> {
    let mut dirs: Vec<(AgentScope, PathBuf)> = Vec::new();
    for home in global_homes {
        push_skill_dir(&mut dirs, AgentScope::Global, home.join("skills"));
    }
    push_skill_dir(
        &mut dirs,
        AgentScope::Project,
        workspace_root.join(".agents").join("skills"),
    );
    push_skill_dir(
        &mut dirs,
        AgentScope::Project,
        workspace_root.join(".claude").join("skills"),
    );
    dirs
}

fn push_skill_dir(dirs: &mut Vec<(AgentScope, PathBuf)>, scope: AgentScope, dir: PathBuf) {
    if dir.is_dir() && !dirs.iter().any(|(_, existing)| existing == &dir) {
        dirs.push((scope, dir));
    }
}

/// 元数据是否与缓存指纹一致（不读文件内容）。
fn metadata_matches(fingerprint: &FileFingerprint, path: &Path) -> bool {
    match fs::metadata(path) {
        Ok(metadata) => {
            fingerprint.file_size == metadata.len() && fingerprint.mtime == metadata.modified().ok()
        }
        Err(_) => false,
    }
}

/// 渲染多层宪法内容。
///
/// 只有一个项目层时返回原始字节（与既有单文件行为完全一致）；多层级时按
/// 全局→项目的顺序拼接，并为每层标注作用域与来源路径。
fn render_constitution(layers: &[ConstitutionLayer]) -> String {
    if layers.len() == 1 && layers[0].scope == AgentScope::Project {
        return layers[0].content.clone();
    }

    let mut rendered = String::new();
    for layer in layers {
        if !rendered.is_empty() {
            rendered.push_str("\n\n");
        }
        let heading = match layer.scope {
            AgentScope::Global => "Global user-level instructions",
            AgentScope::Project => "Project instructions",
        };
        rendered.push_str(&format!(
            "## {} ({})\n",
            heading,
            layer.source_path.display()
        ));
        rendered.push_str(layer.content.trim_end());
    }
    rendered
}

/// 解析 SKILL.md 的标准 YAML Frontmatter
fn parse_skill_metadata(content: &str, fallback_name: &str) -> (String, String, bool, Vec<String>) {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return (
            fallback_name.to_string(),
            extract_fallback_description(content),
            true,
            Vec::new(),
        );
    }

    let mut name = fallback_name.to_string();
    let mut description = String::new();
    let mut user_invocable = true;
    let mut allowed_tools = Vec::new();

    let rest = &trimmed[3..];
    if let Some(end_idx) = rest.find("\n---") {
        let frontmatter = &rest[..end_idx];
        for line in frontmatter.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(colon_idx) = line.find(':') {
                let key = line[..colon_idx].trim();
                let val = line[colon_idx + 1..]
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'');
                match key {
                    "name" => {
                        if !val.is_empty() {
                            name = val.to_string();
                        }
                    }
                    "description" => {
                        description = val.to_string();
                    }
                    "user-invocable" => {
                        user_invocable = val.parse().unwrap_or(true);
                    }
                    "allowed-tools" => {
                        allowed_tools = val
                            .split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect();
                    }
                    _ => {}
                }
            }
        }
    }

    if description.is_empty() {
        description = extract_fallback_description(content);
    }

    (name, description, user_invocable, allowed_tools)
}

fn extract_fallback_description(content: &str) -> String {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("---") {
            continue;
        }
        return trimmed.chars().take(200).collect();
    }
    "No description provided.".to_string()
}

impl WorkspaceHotloader {
    /// 带预算的宪法解析：超出预算时按 `max_source_bytes` 截断并标记（wave-42 契约桩）。
    #[allow(unused_variables)]
    pub fn resolve_constitution_with_budget<P: AsRef<Path>>(
        &self,
        workspace_root: P,
        global_homes: &[PathBuf],
        budget: InstructionBudget,
    ) -> Option<(String, bool)> {
        unimplemented!("wave42 stub: resolve_constitution_with_budget")
    }

    /// 带预算的 Skills 扫描：总注入字节受 `max_bytes` 约束（wave-42 契约桩）。
    #[allow(unused_variables)]
    pub fn scan_skills_with_budget<P: AsRef<Path>>(
        &self,
        workspace_root: P,
        global_homes: &[PathBuf],
        budget: InstructionBudget,
    ) -> (Vec<HotloadedSkill>, bool) {
        unimplemented!("wave42 stub: scan_skills_with_budget")
    }
}

/// 由 cwd 向上探测 `marker` 定位项目根，`bound` 为不可越过的上界（wave-42 契约桩）。
#[allow(unused_variables)]
pub fn find_project_root(cwd: &Path, marker: &str, bound: &Path) -> PathBuf {
    unimplemented!("wave42 stub: find_project_root")
}

/// 生成 root→cwd 的祖先目录链（含两端，浅→深）（wave-42 契约桩）。
#[allow(unused_variables)]
pub fn ancestor_chain(root: &Path, cwd: &Path) -> Vec<PathBuf> {
    unimplemented!("wave42 stub: ancestor_chain")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_dir(prefix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "{prefix}_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_skill(root: &Path, folder: &str, name: &str, description: &str) {
        let dir = root.join("skills").join(folder);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {description}\n---\n\n# Body\n"),
        )
        .unwrap();
    }

    #[test]
    fn test_hotloader_cache_hit_and_reload() {
        let temp_dir = unique_dir("pony_test");

        let agents_md = temp_dir.join("AGENTS.md");
        fs::write(&agents_md, "# Test Rules v1\nDo not break.").unwrap();

        let hotloader = WorkspaceHotloader::new();
        // 显式注入空全局层，避免宿主机用户级指令影响断言。
        let (content, hit) = hotloader
            .resolve_constitution_with_globals(&temp_dir, &[])
            .unwrap();
        assert_eq!(content, "# Test Rules v1\nDo not break.");
        assert!(!hit);

        let (content2, hit2) = hotloader
            .resolve_constitution_with_globals(&temp_dir, &[])
            .unwrap();
        assert_eq!(content2, content);
        assert!(hit2);

        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&agents_md, "# Test Rules v2\nUpdated.").unwrap();
        let (content3, hit3) = hotloader
            .resolve_constitution_with_globals(&temp_dir, &[])
            .unwrap();
        assert_eq!(content3, "# Test Rules v2\nUpdated.");
        assert!(!hit3);

        fs::remove_dir_all(&temp_dir).unwrap();
    }

    #[test]
    fn test_global_homes_order_and_dedupe() {
        let home = PathBuf::from("/home/tester");
        let config = PathBuf::from("/home/tester/.config");
        let explicit = PathBuf::from("/home/tester/.config/pony-agent");

        let homes = resolve_global_agent_homes_from(Some(&explicit), Some(&config), Some(&home));
        assert_eq!(
            homes,
            vec![
                home.join(".agents"),
                home.join(".claude"),
                config.join("pony-agent"),
            ],
            "显式覆盖目录与产品根重合时只保留一次"
        );

        let from_home_only = resolve_global_agent_homes_from(None, None, Some(&home));
        assert_eq!(from_home_only.len(), 2);
        assert!(from_home_only[0].ends_with(".agents"));
        assert!(from_home_only[1].ends_with(".claude"));

        assert!(resolve_global_agent_homes_from(None, None, None).is_empty());
        assert!(
            resolve_global_agent_homes_from(Some(Path::new("")), None, None).is_empty(),
            "空环境变量不得产生兜底目录"
        );
    }

    #[test]
    fn test_global_instructions_precede_project_instructions() {
        let workspace = unique_dir("pony_global_ws");
        let global_home = unique_dir("pony_global_home");
        fs::write(global_home.join("AGENTS.md"), "# Global Rules\nBe terse.").unwrap();
        fs::write(workspace.join("AGENTS.md"), "# Project Rules\nUse Rust.").unwrap();

        let hotloader = WorkspaceHotloader::new();
        let (content, hit) = hotloader
            .resolve_constitution_with_globals(&workspace, &[global_home.clone()])
            .unwrap();

        assert!(!hit);
        assert!(
            content.contains("# Global Rules\nBe terse."),
            "全局层内容必须出现"
        );
        assert!(
            content.contains("# Project Rules\nUse Rust."),
            "项目层内容必须出现"
        );
        assert!(content.contains("Global user-level instructions"));
        assert!(content.contains("Project instructions"));
        assert!(
            content.find("Global user-level instructions").unwrap()
                < content.find("Project instructions").unwrap(),
            "全局层必须排在项目层之前（优先级更低）"
        );
        assert!(
            content.find("# Global Rules").unwrap() < content.find("# Project Rules").unwrap(),
            "项目层内容必须位于全局层内容之后"
        );

        let (cached, cached_hit) = hotloader
            .resolve_constitution_with_globals(&workspace, &[global_home.clone()])
            .unwrap();
        assert!(cached_hit);
        assert_eq!(cached, content);

        fs::remove_dir_all(&workspace).ok();
        fs::remove_dir_all(&global_home).ok();
    }

    #[test]
    fn test_global_only_instructions_and_global_cache_invalidation() {
        let workspace = unique_dir("pony_global_only_ws");
        let global_home = unique_dir("pony_global_only_home");
        let global_md = global_home.join("AGENTS.md");
        fs::write(&global_md, "# User Rules v1").unwrap();

        let hotloader = WorkspaceHotloader::new();
        let (content, hit) = hotloader
            .resolve_constitution_with_globals(&workspace, &[global_home.clone()])
            .unwrap();
        assert!(!hit);
        assert!(content.contains("# User Rules v1"));
        assert!(content.contains("Global user-level instructions"));

        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&global_md, "# User Rules v2").unwrap();
        let (content2, hit2) = hotloader
            .resolve_constitution_with_globals(&workspace, &[global_home.clone()])
            .unwrap();
        assert!(!hit2, "全局层文件变更必须使缓存失效");
        assert!(content2.contains("# User Rules v2"));
        assert!(!content2.contains("# User Rules v1"));

        fs::remove_dir_all(&workspace).ok();
        fs::remove_dir_all(&global_home).ok();
    }

    #[test]
    fn test_claude_compat_instruction_candidate_is_honored() {
        let workspace = unique_dir("pony_claude_ws");
        let global_home = unique_dir("pony_claude_home");
        fs::create_dir_all(global_home.join(".claude")).unwrap();
        fs::write(global_home.join(".claude/CLAUDE.md"), "# Claude Global").unwrap();

        let hotloader = WorkspaceHotloader::new();
        let (content, _) = hotloader
            .resolve_constitution_with_globals(&workspace, &[global_home.clone()])
            .unwrap();
        assert!(content.contains("# Claude Global"));
        assert!(content.contains(".claude/CLAUDE.md"));

        fs::remove_dir_all(&workspace).ok();
        fs::remove_dir_all(&global_home).ok();
    }

    #[test]
    fn test_scan_skills_merges_global_and_project_layers() {
        let workspace = unique_dir("pony_skill_ws");
        let global_home = unique_dir("pony_skill_home");
        write_skill(&global_home, "global-one", "global-one", "A global skill");
        let project_skill_dir = workspace.join(".agents").join("skills");
        fs::create_dir_all(project_skill_dir.join("project-one")).unwrap();
        fs::write(
            project_skill_dir.join("project-one").join("SKILL.md"),
            "---\nname: project-one\ndescription: A project skill\n---\n\n# Body\n",
        )
        .unwrap();

        let hotloader = WorkspaceHotloader::new();
        let (skills, hit) = hotloader.scan_skills_with_globals(&workspace, &[global_home.clone()]);

        assert!(!hit);
        assert_eq!(skills.len(), 2);
        let by_name = |target: &str| {
            skills
                .iter()
                .find(|skill| skill.name == target)
                .unwrap_or_else(|| panic!("missing skill {target}"))
        };
        assert_eq!(by_name("global-one").scope, AgentScope::Global);
        assert_eq!(by_name("project-one").scope, AgentScope::Project);

        let (again, hit_again) =
            hotloader.scan_skills_with_globals(&workspace, &[global_home.clone()]);
        assert!(hit_again, "未变更时第二次扫描必须命中缓存");
        assert_eq!(again, skills);

        fs::remove_dir_all(&workspace).ok();
        fs::remove_dir_all(&global_home).ok();
    }

    #[test]
    fn test_project_skill_overrides_global_skill_with_same_folder_name() {
        let workspace = unique_dir("pony_override_ws");
        let global_home = unique_dir("pony_override_home");
        write_skill(&global_home, "review", "review", "Global review skill");
        let project_skill_dir = workspace.join(".agents").join("skills").join("review");
        fs::create_dir_all(&project_skill_dir).unwrap();
        fs::write(
            project_skill_dir.join("SKILL.md"),
            "---\nname: review\ndescription: Project review skill\n---\n\n# Body\n",
        )
        .unwrap();

        let hotloader = WorkspaceHotloader::new();
        let (skills, _) = hotloader.scan_skills_with_globals(&workspace, &[global_home.clone()]);

        assert_eq!(skills.len(), 1, "同名技能只保留一个");
        assert_eq!(skills[0].description, "Project review skill");
        assert_eq!(skills[0].scope, AgentScope::Project);

        fs::remove_dir_all(&workspace).ok();
        fs::remove_dir_all(&global_home).ok();
    }

    #[test]
    fn test_prompt_prefix_labels_skill_scope() {
        let hotloader = WorkspaceHotloader::new();
        let prefix = hotloader.build_cached_prompt_prefix(
            Some("rules"),
            &[HotloadedSkill {
                name: "audit".to_string(),
                description: "audit things".to_string(),
                user_invocable: true,
                allowed_tools: Vec::new(),
                path: PathBuf::from("/tmp/skills/audit/SKILL.md"),
                fingerprint: FileFingerprint {
                    path: PathBuf::from("/tmp/skills/audit/SKILL.md"),
                    mtime: None,
                    file_size: 10,
                    content_hash: 7,
                },
                scope: AgentScope::Global,
                rank: 0,
            }],
        );
        assert!(prefix.contains("- `audit` [global]: audit things"));
    }

    #[test]
    fn test_yaml_frontmatter_parsing() {
        let sample = r#"---
name: awesome-skill
description: Comprehensive auditing tool for code.
user-invocable: false
allowed-tools: Bash, Read
---

# Details
"#;
        let (name, desc, user_invocable, tools) = parse_skill_metadata(sample, "fallback");
        assert_eq!(name, "awesome-skill");
        assert_eq!(desc, "Comprehensive auditing tool for code.");
        assert!(!user_invocable);
        assert_eq!(tools, vec!["Bash", "Read"]);
    }

    #[test]
    fn test_session_kv_cache_preservation_on_mid_turn_edit() {
        let temp_dir = unique_dir("pony_session");

        let agents_md = temp_dir.join("AGENTS.md");
        fs::write(&agents_md, "# Baseline Rules").unwrap();

        let hotloader = WorkspaceHotloader::new();
        let session_id = "session_123";

        // 第 1 轮交互：创建基线
        let (prefix_turn_1, notice_turn_1) =
            hotloader.resolve_session_prompt_context_with_globals(session_id, 1, &temp_dir, &[]);
        assert!(prefix_turn_1.contains("# Baseline Rules"));
        assert!(notice_turn_1.is_none());

        // 用户在第 2 轮中途修改了 AGENTS.md 宪法文件
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&agents_md, "# Modified Mid-Session Rules").unwrap();

        // 第 2 轮交互：静态前缀必须保持不变（KV-Cache 命中保护！），变动以增量 notice 输出
        let (prefix_turn_2, notice_turn_2) =
            hotloader.resolve_session_prompt_context_with_globals(session_id, 2, &temp_dir, &[]);
        assert_eq!(
            prefix_turn_1, prefix_turn_2,
            "头部静态前缀必须严格相等，确保 KV Cache 不被冲刷！"
        );
        assert!(notice_turn_2.is_some());
        assert!(notice_turn_2
            .unwrap()
            .contains("constitution/rules updated"));

        fs::remove_dir_all(&temp_dir).unwrap();
    }
}
