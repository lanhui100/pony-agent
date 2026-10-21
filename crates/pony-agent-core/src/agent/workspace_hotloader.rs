use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::SystemTime;

/// 指令文档（AGENTS.md / CLAUDE.md）的候选文件名，按优先级从高到低。
/// 项目级同时兼容 Claude Code 的 `.claude/CLAUDE.md` 布局。
const CONSTITUTION_CANDIDATES: [&str; 3] = ["AGENTS.md", "CLAUDE.md", ".claude/CLAUDE.md"];

/// 同目录个人覆盖层（local overlay）候选文件名，按优先级从高到低。
/// 排在基础候选之后加载，因此在同一目录内具有更高优先级。
const LOCAL_OVERLAY_CANDIDATES: [&str; 2] = ["AGENTS.local.md", "CLAUDE.local.md"];

/// 技能元数据解析所需的正文前缀上限：YAML frontmatter 足够，禁止整文件入内存。
const SKILL_FRONTMATTER_READ_LIMIT: usize = 65_536;

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
    /// 完整读取（既有语义；仅用于已知小文件的调用方）。
    pub fn compute<P: AsRef<Path>>(path: P) -> Option<(Self, String)> {
        Self::compute_bounded(path, usize::MAX)
    }

    /// **有界读取**：最多读取 `limit` 字节，避免超大文件整份进入内存。
    ///
    /// 指纹里的 `file_size` 仍是文件真实大小，`content_hash` 只覆盖有界前缀；
    /// 因此超出前缀的变更由 `mtime + file_size` 兜底检测（与既有缓存语义一致）。
    pub fn compute_bounded<P: AsRef<Path>>(path: P, limit: usize) -> Option<(Self, String)> {
        let path = path.as_ref();
        let metadata = fs::metadata(path).ok()?;
        let mtime = metadata.modified().ok();
        let file_size = metadata.len();
        let file = fs::File::open(path).ok()?;
        let mut bytes: Vec<u8> = Vec::new();
        file.take(limit as u64).read_to_end(&mut bytes).ok()?;
        if (file_size as usize) > limit {
            let keep = utf8_safe_prefix_len(&bytes, limit.min(bytes.len()));
            bytes.truncate(keep);
        }
        let content = String::from_utf8(bytes).ok()?;
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
    /// 有界读取得到的层（去重前、读取顺序）。渲染阶段的去重/预算决策不写回缓存。
    pub layers: Vec<ConstitutionLayer>,
    /// **去重前全部来源**的读取状态快照。命中校验必须覆盖每个来源，
    /// 否则被去重丢弃的同文来源、以及读取失败后恢复的来源都将永远不可见。
    sources: Vec<ConstitutionSourceSnapshot>,
    /// 生成缓存时的单文件读取上限；上限变化意味着已读正文可能被提前截短，必须重读。
    max_source_bytes: usize,
}

/// 单个宪法来源的读取状态快照（缓存身份 + 预算判定输入）。
///
/// 命中语义（诚实假设）：成功读取且 `mtime + file_size` 均未变化的来源视为未变更；
/// 「同尺寸 + 同 mtime 的原地成功改写」不在本波检测范围（需逐次内容哈希比对）。
/// 反之，任一来源**读取失败**都会把 `read_ok` 记为 false，并使整份缓存 miss：
/// 即使权限/文件恢复后 mtime 与 size 完全不变，下次调用也必然重试读盘。
#[derive(Clone, Debug, PartialEq, Eq)]
struct ConstitutionSourceSnapshot {
    scope: AgentScope,
    path: PathBuf,
    mtime: Option<SystemTime>,
    file_size: u64,
    /// 是否成功读取并解码正文（false → 缓存永不命中，强制重试）。
    read_ok: bool,
    /// 单文件是否超过 `max_source_bytes`（正文只保留首部，最高优先级层截断保留）。
    oversized: bool,
    /// 有界读取到的正文（oversized 时为文件首部，按 UTF-8 边界安全截断）。
    content: String,
    content_hash: u64,
}

impl ConstitutionSourceSnapshot {
    /// 有界读取来源：最多读 `max_source_bytes + 1` 字节以判定是否超限，
    /// 避免超大文件整份进入内存。
    fn capture(scope: AgentScope, path: &Path, max_source_bytes: usize) -> Self {
        let metadata = fs::metadata(path).ok();
        let mtime = metadata
            .as_ref()
            .and_then(|metadata| metadata.modified().ok());
        let file_size = metadata.map(|metadata| metadata.len()).unwrap_or(0);

        let mut read_ok = false;
        let mut oversized = false;
        let mut content = String::new();
        if let Ok(file) = fs::File::open(path) {
            let mut bytes: Vec<u8> = Vec::new();
            let limit = (max_source_bytes as u64).saturating_add(1);
            if file.take(limit).read_to_end(&mut bytes).is_ok() {
                oversized = bytes.len() > max_source_bytes;
                if oversized {
                    let keep = utf8_safe_prefix_len(&bytes, max_source_bytes);
                    bytes.truncate(keep);
                }
                if let Ok(decoded) = String::from_utf8(bytes) {
                    read_ok = true;
                    content = decoded;
                }
            }
        }

        Self {
            scope,
            path: path.to_path_buf(),
            mtime,
            file_size,
            read_ok,
            oversized,
            content_hash: fnv1a_hash(content.as_bytes()),
            content,
        }
    }

    fn to_layer(&self) -> ConstitutionLayer {
        ConstitutionLayer {
            scope: self.scope,
            source_path: self.path.clone(),
            content: self.content.clone(),
            fingerprint: FileFingerprint {
                path: self.path.clone(),
                mtime: self.mtime,
                file_size: self.file_size,
                content_hash: self.content_hash,
            },
        }
    }

    /// 当前磁盘元数据是否与快照一致（来源被删除也视为不一致）。
    fn metadata_matches(&self) -> bool {
        match fs::metadata(&self.path) {
            Ok(metadata) => {
                self.file_size == metadata.len() && self.mtime == metadata.modified().ok()
            }
            Err(_) => false,
        }
    }
}

/// 取字节序列中不超过 `max_bytes` 的 UTF-8 安全前缀长度（不切裂多字节字符）。
fn utf8_safe_prefix_len(bytes: &[u8], max_bytes: usize) -> usize {
    if bytes.len() <= max_bytes {
        return bytes.len();
    }
    let mut end = max_bytes;
    while end > 0 && (bytes[end] & 0xC0) == 0x80 {
        end -= 1;
    }
    end
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
        self.resolve_constitution_with_budget(
            workspace_root,
            global_homes,
            InstructionBudget::default(),
        )
    }

    /// 加载（并缓存）给定来源清单的宪法层。
    /// 返回 (来源读取快照, 有界层, 是否命中内存缓存)。
    ///
    /// 缓存以「来源清单 + 单文件读取上限 + **去重前每个来源**的读取状态」为命中条件；
    /// 锁纪律：读锁内只克隆快照，stat / 读盘一律在锁外执行。
    fn load_constitution_layers(
        &self,
        sources: &[ConstitutionSource],
        max_source_bytes: usize,
    ) -> (
        Vec<ConstitutionSourceSnapshot>,
        Vec<ConstitutionLayer>,
        bool,
    ) {
        let cached_snapshot = {
            let guard = self
                .constitution
                .read()
                .unwrap_or_else(|poison| poison.into_inner());
            guard
                .as_ref()
                .filter(|cached| {
                    cached.max_source_bytes == max_source_bytes
                        && cached.sources.len() == sources.len()
                        && cached
                            .sources
                            .iter()
                            .zip(sources.iter())
                            .all(|(cached, source)| {
                                cached.scope == source.scope && cached.path == source.path
                            })
                })
                .cloned()
        };

        if let Some(cached) = cached_snapshot {
            // 任一来源上次读取失败 → 强制 miss 重试（权限恢复时 mtime/size 可能不变）。
            let unchanged = cached
                .sources
                .iter()
                .all(|source| source.read_ok && source.metadata_matches());
            if unchanged {
                return (cached.sources, cached.layers, true);
            }
        }

        let snapshots: Vec<ConstitutionSourceSnapshot> = sources
            .iter()
            .map(|source| {
                ConstitutionSourceSnapshot::capture(source.scope, &source.path, max_source_bytes)
            })
            .collect();
        let layers: Vec<ConstitutionLayer> = snapshots
            .iter()
            .filter(|snapshot| snapshot.read_ok)
            .map(|snapshot| snapshot.to_layer())
            .collect();
        if layers.is_empty() {
            return (snapshots, layers, false);
        }

        let cache = ConstitutionCache {
            layers: layers.clone(),
            sources: snapshots.clone(),
            max_source_bytes,
        };
        *self
            .constitution
            .write()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(cache);
        (snapshots, layers, false)
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

        for (scope, base_dir, rank) in skill_dirs {
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
                    // B7：超过技能正文上限的文件按 TooLarge 语义整条跳过，
                    // 不读取任何正文（否则 768MiB 级文件会被整份读入）。
                    if file_size > crate::agent::skill_body::MAX_SKILL_BODY_BYTES as u64 {
                        continue;
                    }
                    // 同一 scope 下可能存在多个技能根（.dsh/.agents/.claude），
                    // 键中带上 rank 以免不同根的缓存互相覆盖。
                    let cache_key = format!("{}:{}:{}", scope.as_str(), rank, folder_name);

                    let cached = {
                        let read_guard = self
                            .skill_files
                            .read()
                            .unwrap_or_else(|poison| poison.into_inner());
                        read_guard.get(&cache_key).cloned()
                    };

                    let skill = match cached {
                        Some(existing)
                            if existing.scope == scope
                                && existing.rank == rank
                                && existing.fingerprint.mtime == mtime
                                && existing.fingerprint.file_size == file_size =>
                        {
                            existing
                        }
                        _ => {
                            any_changed = true;
                            // B7：只为解析 frontmatter 读取有界前缀。
                            match FileFingerprint::compute_bounded(
                                &skill_file,
                                SKILL_FRONTMATTER_READ_LIMIT,
                            ) {
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
                                        rank,
                                    }
                                }
                                None => continue,
                            }
                        }
                    };

                    file_cache.insert(cache_key, skill.clone());
                    // 同名技能裁决：rank 小者胜；rank 相同时项目层胜（不依赖遍历顺序）。
                    match discovered.entry(folder_name) {
                        std::collections::btree_map::Entry::Occupied(mut entry) => {
                            let existing_rank = entry.get().rank;
                            let challenger_wins = skill.rank < existing_rank
                                || (skill.rank == existing_rank
                                    && skill.scope == AgentScope::Project);
                            if challenger_wins {
                                entry.insert(skill);
                            }
                        }
                        std::collections::btree_map::Entry::Vacant(entry) => {
                            entry.insert(skill);
                        }
                    }
                }
            }
        }

        let merged: Vec<HotloadedSkill> = discovered.into_values().collect();
        {
            let read_guard = self
                .skills
                .read()
                .unwrap_or_else(|poison| poison.into_inner());
            if !any_changed && *read_guard == merged {
                return (read_guard.clone(), true);
            }
        }

        *self
            .skill_files
            .write()
            .unwrap_or_else(|poison| poison.into_inner()) = file_cache;
        *self
            .skills
            .write()
            .unwrap_or_else(|poison| poison.into_inner()) = merged.clone();
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
        self.resolve_session_prompt_context_with_budget(
            session_id,
            turn_count,
            workspace_root,
            global_homes,
            InstructionBudget::default(),
        )
    }

    /// 可显式指定注入预算的会话前缀变体。
    ///
    /// 静态前缀一旦固化即永不重写（KV-Cache 头保护）；预算变化与宪法/技能变化一样，
    /// 只以 Append-Only notice 表达。
    pub fn resolve_session_prompt_context_with_budget<P: AsRef<Path>>(
        &self,
        session_id: &str,
        turn_count: usize,
        workspace_root: P,
        global_homes: &[PathBuf],
        budget: InstructionBudget,
    ) -> (String, Option<String>) {
        let (current_constitution, _) = self
            .resolve_constitution_with_budget(&workspace_root, global_homes, budget)
            .unwrap_or_else(|| (String::new(), true));
        let (current_skills, _) =
            self.scan_skills_with_budget(&workspace_root, global_homes, budget);

        let const_hash = fnv1a_hash(current_constitution.as_bytes());
        let mut skill_hash_input = Vec::new();
        for s in &current_skills {
            skill_hash_input.extend_from_slice(s.name.as_bytes());
            skill_hash_input.extend_from_slice(s.scope.as_str().as_bytes());
            skill_hash_input.extend_from_slice(s.description.as_bytes());
            skill_hash_input.extend_from_slice(&s.fingerprint.content_hash.to_le_bytes());
        }
        let skills_hash = fnv1a_hash(&skill_hash_input);

        let mut baselines = self
            .session_baselines
            .write()
            .unwrap_or_else(|poison| poison.into_inner());

        if turn_count <= 1 || !baselines.contains_key(session_id) {
            // 首轮：固化静态 Baseline
            let baseline = SessionBaselineSnapshot {
                constitution_hash: const_hash,
                constitution_content: current_constitution.clone(),
                skills_hash,
                skills: current_skills.clone(),
                budget,
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

        // 检测是否存在中途变更（预算变化同样视为配置变化：已固化前缀不得重算）
        let const_changed = baseline.constitution_hash != const_hash;
        let skills_changed = baseline.skills_hash != skills_hash;
        let budget_changed = baseline.budget != budget;

        if !const_changed && !skills_changed && !budget_changed {
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
        if budget_changed {
            delta_notice
                .push_str("- Instruction injection budget changed. Baseline prefix is retained.\n");
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

/// 宪法来源候选：作用域 + 被采纳的文件 + 其作用域根（canonical 包含校验基准）。
#[derive(Clone, Debug, PartialEq, Eq)]
struct ConstitutionSource {
    scope: AgentScope,
    path: PathBuf,
    root: PathBuf,
}

/// 采纳判定（fail-closed）：只有「链接本身不是目录/FIFO/设备/套接字」且「canonical 后
/// 是普通文件」且「落在作用域根内」的候选才可被读取。
///
/// 先用 `symlink_metadata` 判型，避免对 FIFO/设备文件发起读取而阻塞或注入非文件内容；
/// 符号链接一律解析后再校验归属，`ln -s /tmp/out/secret ws/AGENTS.md` 这类逃逸被拒绝。
fn is_adoptable_constitution_file(path: &Path, root: &Path) -> bool {
    let Ok(link_metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    let file_type = link_metadata.file_type();
    if file_type.is_dir() || (!file_type.is_file() && !file_type.is_symlink()) {
        return false;
    }

    let (Ok(canonical), Ok(canonical_root)) = (fs::canonicalize(path), fs::canonicalize(root))
    else {
        return false;
    };
    if !canonical.starts_with(&canonical_root) {
        return false;
    }
    match fs::metadata(&canonical) {
        Ok(metadata) => metadata.is_file(),
        Err(_) => false,
    }
}

/// 在根目录内按候选顺序取首个**可采纳**文件。
fn first_existing_constitution(root: &Path) -> Option<PathBuf> {
    for candidate in CONSTITUTION_CANDIDATES {
        let path = root.join(candidate);
        if is_adoptable_constitution_file(&path, root) {
            return Some(path);
        }
    }
    None
}

/// 探测单个目录内的 local overlay（个人覆盖层）：按候选顺序取首个可采纳文件。
fn first_existing_local_overlay(root: &Path) -> Option<PathBuf> {
    for candidate in LOCAL_OVERLAY_CANDIDATES {
        let path = root.join(candidate);
        if is_adoptable_constitution_file(&path, root) {
            return Some(path);
        }
    }
    None
}

/// 汇总宪法来源：全局根按顺序在前，祖先链（root→cwd，浅→深）在后（最高优先级）。
///
/// 每级链目录先取基础候选首命中，再取 local overlay 首命中（overlay 排在 base 之后，
/// 即在同一目录内具有更高优先级）。
fn discover_constitution_sources(
    workspace_root: &Path,
    global_homes: &[PathBuf],
) -> Vec<ConstitutionSource> {
    let mut sources: Vec<ConstitutionSource> = Vec::new();
    for home in global_homes {
        if let Some(path) = first_existing_constitution(home) {
            sources.push(ConstitutionSource {
                scope: AgentScope::Global,
                path,
                root: home.clone(),
            });
        }
    }
    // 宪法发现不做无界上探：bound 未由调用方给出前 root=cwd=workspace_root，避免载入宿主 home 层指令。
    let chain = ancestor_chain(workspace_root, workspace_root);
    for dir in chain {
        if let Some(path) = first_existing_constitution(&dir) {
            sources.push(ConstitutionSource {
                scope: AgentScope::Project,
                path,
                root: dir.clone(),
            });
        }
        if let Some(path) = first_existing_local_overlay(&dir) {
            sources.push(ConstitutionSource {
                scope: AgentScope::Project,
                path,
                root: dir.clone(),
            });
        }
    }
    sources
}

/// 汇总 Skills 根目录与注入 rank：全局根按顺序在前，项目级目录最后（最高优先级）。
///
/// rank 数值越小越优先，冻结表见 [`GlobalRootIdentity`] 与项目带常量；
/// 项目带 rank 恒小于全局带，确保「项目层永远最高优先级」不变量。
fn discover_skill_dirs(
    workspace_root: &Path,
    global_homes: &[PathBuf],
) -> Vec<(AgentScope, PathBuf, u16)> {
    let mut dirs: Vec<(AgentScope, PathBuf, u16)> = Vec::new();
    // 身份解析（env / home_dir / 已知全局根）在一次扫描内只采样一次，
    // 保证 rank 是本次扫描的纯函数快照，不会在中途因 env 变化而漂移。
    let identity = GlobalRootIdentity::snapshot();
    for (index, home) in global_homes.iter().enumerate() {
        push_skill_dir(
            &mut dirs,
            AgentScope::Global,
            home.join("skills"),
            identity.rank(home, index),
        );
    }
    push_skill_dir(
        &mut dirs,
        AgentScope::Project,
        workspace_root.join(".dsh").join("skills"),
        100,
    );
    push_skill_dir(
        &mut dirs,
        AgentScope::Project,
        workspace_root.join(".agents").join("skills"),
        200,
    );
    push_skill_dir(
        &mut dirs,
        AgentScope::Project,
        workspace_root.join(".claude").join("skills"),
        300,
    );
    dirs
}

/// 全局根身份快照：`$PONY_AGENT_HOME`、`home_dir` 与已知产品全局根**同一扫描内单次采样**；
/// 跨调用允许随环境变化（环境变了 rank 也随之变化，属预期行为）。
///
/// 冻结 rank 表：`$PONY_AGENT_HOME/skills`（及产品自有根）=400、`~/.dsh/skills`=500、
/// `~/.agents/skills`=510、`~/.claude/skills`=520；项目带 `.dsh`=100 / `.agents`=200 /
/// `.claude`=300。无法识别身份的注入根按 `450 + index`（`index` 上限 49）兜底，
/// 恒落在全局带内，不得越过 500/510/520。
struct GlobalRootIdentity {
    env_home: Option<PathBuf>,
    home_dir: Option<PathBuf>,
    known: Vec<PathBuf>,
}

impl GlobalRootIdentity {
    /// 单次采样：环境变量与系统目录只在本函数里读取一次，之后 rank 为纯函数。
    fn snapshot() -> Self {
        let env_home = std::env::var_os(PONY_AGENT_HOME_ENV)
            .map(PathBuf::from)
            .filter(|path| !path.as_os_str().is_empty());
        let home_dir = dirs::home_dir();
        let config_dir = dirs::config_dir();
        let known = resolve_global_agent_homes_from(
            env_home.as_deref(),
            config_dir.as_deref(),
            home_dir.as_deref(),
        );
        Self {
            env_home,
            home_dir,
            known,
        }
    }

    /// 基于本次快照计算单个全局根的注入 rank（越小越优先）；不再读取环境状态。
    fn rank(&self, home: &Path, index: usize) -> u16 {
        if let Some(env) = self.env_home.as_deref() {
            if home == env {
                return 400;
            }
        }
        if let Some(home_dir) = self.home_dir.as_deref() {
            if home == home_dir.join(".dsh") {
                return 500;
            }
            if home == home_dir.join(".agents") {
                return 510;
            }
            if home == home_dir.join(".claude") {
                return 520;
            }
        }
        // 产品自有全局根（config_dir/pony-agent）与 `$PONY_AGENT_HOME` 同属产品带。
        if self.known.iter().any(|candidate| candidate == home) {
            return 400;
        }

        450u16.saturating_add(index.min(49) as u16)
    }
}

fn push_skill_dir(
    dirs: &mut Vec<(AgentScope, PathBuf, u16)>,
    scope: AgentScope,
    dir: PathBuf,
    rank: u16,
) {
    if dir.is_dir() && !dirs.iter().any(|(_, existing, _)| existing == &dir) {
        dirs.push((scope, dir, rank));
    }
}

/// 按 UTF-8 字符边界截断到不超过 `max_bytes`（避免切裂多字节字符）。
///
/// 截断后的文本由调用方追加 `truncated <path>` 标记。
fn truncate_at_char_boundary(content: &str, max_bytes: usize) -> &str {
    if content.len() <= max_bytes {
        return content;
    }
    let mut end = max_bytes;
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    &content[..end]
}

/// 内容去重归一化：整体 trim + 逐行 trim。
///
/// 逐行 trim 用于抹平「同文不同缩进/行尾空白」的形态差异（例如全局模板与项目
/// 副本仅空行/行尾空格不同），使 trim 后相同的层可被识别为同一份。
fn normalize_constitution_content(content: &str) -> String {
    content
        .trim()
        .lines()
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("\n")
}

/// 渲染多层宪法内容。
///
/// 只有一个项目层时返回原始字节（与既有单文件行为完全一致）；多层级时按
/// 全局→项目的顺序拼接，并为每层标注作用域与来源路径。
fn render_constitution(layers: &[&ConstitutionLayer]) -> String {
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

/// 预算渲染候选层（`candidates` 顺序 = 渲染顺序 = 优先级从低到高）。
struct BudgetCandidate {
    source_index: usize,
    layer: ConstitutionLayer,
    oversized: bool,
    kept: bool,
}

/// 跨层去重标记：`true` 表示保留最高优先级的一份。
///
/// 逆序扫描（高优先级在前）保证「后出现者胜」；超限层的正文只是文件首部，
/// 不参与内容判等、也不会被去重丢弃。
fn dedupe_keep_flags(candidates: &[BudgetCandidate]) -> Vec<bool> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut keep = vec![true; candidates.len()];
    for (index, candidate) in candidates.iter().enumerate().rev() {
        if candidate.oversized {
            continue;
        }
        let key = normalize_constitution_content(&candidate.layer.content);
        if !seen.insert(key) {
            keep[index] = false;
        }
    }
    keep
}

/// 按既定格式拼装：保留层走 `render_constitution`，被省略层在末尾按优先级高→低
/// 追加 `omitted <path>`。
fn compose_constitution(candidates: &[BudgetCandidate]) -> String {
    let kept: Vec<&ConstitutionLayer> = candidates
        .iter()
        .filter(|candidate| candidate.kept)
        .map(|candidate| &candidate.layer)
        .collect();
    let mut rendered = render_constitution(&kept);

    let mut omitted: Vec<&BudgetCandidate> = candidates
        .iter()
        .filter(|candidate| !candidate.kept)
        .collect();
    omitted.sort_by(|left, right| right.source_index.cmp(&left.source_index));
    for candidate in omitted {
        if !rendered.is_empty() {
            rendered.push('\n');
        }
        rendered.push_str(&format!(
            "omitted {}",
            candidate.layer.source_path.display()
        ));
    }
    rendered
}

/// 预算内渲染：先按渲染后字节定保留集合（必要时截断最高优先级层），再输出。
fn render_constitution_with_budget(
    snapshots: &[ConstitutionSourceSnapshot],
    layers: Vec<ConstitutionLayer>,
    max_bytes: usize,
) -> String {
    let source_index: HashMap<&Path, usize> = snapshots
        .iter()
        .enumerate()
        .map(|(index, snapshot)| (snapshot.path.as_path(), index))
        .collect();

    let mut candidates: Vec<BudgetCandidate> = layers
        .into_iter()
        .map(|layer| {
            let index = source_index
                .get(layer.source_path.as_path())
                .copied()
                .unwrap_or(0);
            let oversized = snapshots
                .get(index)
                .map(|snapshot| snapshot.oversized)
                .unwrap_or(false);
            BudgetCandidate {
                source_index: index,
                layer,
                oversized,
                kept: true,
            }
        })
        .collect();

    // 1) 跨层去重：trim 后同文只保留最高优先级一份，被丢弃者不产生 omitted 标记。
    let keep_flags = dedupe_keep_flags(&candidates);
    let mut cursor = 0usize;
    candidates.retain(|_| {
        let keep = keep_flags[cursor];
        cursor += 1;
        keep
    });

    // 2) 低优先级超 `max_source_bytes` 的层整层省略；最高优先级层保留首部待预算裁决。
    let top_source_index = candidates
        .iter()
        .filter(|candidate| candidate.kept)
        .map(|candidate| candidate.source_index)
        .max()
        .unwrap_or(usize::MAX);
    for candidate in candidates.iter_mut() {
        if candidate.oversized && candidate.source_index != top_source_index {
            candidate.kept = false;
        }
    }

    // 3) 从最低优先级保留层开始逐层省略，直到渲染后字节落入预算。
    let mut rendered = compose_constitution(&candidates);
    while rendered.len() > max_bytes {
        // 最低优先级 = 候选列表最靠前（source_index 最小）的保留层。
        let victim = candidates
            .iter()
            .position(|candidate| candidate.kept && candidate.source_index != top_source_index);
        match victim {
            Some(index) => {
                candidates[index].kept = false;
                rendered = compose_constitution(&candidates);
            }
            None => break,
        }
    }

    // 4) 最高优先级层永不整层丢弃：UTF-8 边界截断 + `truncated <path>` 标记。
    if rendered.len() > max_bytes {
        if let Some(top) = candidates.iter().rposition(|candidate| candidate.kept) {
            let original = candidates[top].layer.content.clone();
            candidates[top].layer.content.clear();
            let envelope_len = compose_constitution(&candidates).len();
            let marker = format!("truncated {}", candidates[top].layer.source_path.display());
            let available = max_bytes
                .saturating_sub(envelope_len)
                .saturating_sub(marker.len() + 1);
            let mut content =
                String::with_capacity(available.min(original.len()) + marker.len() + 1);
            content.push_str(truncate_at_char_boundary(&original, available));
            content.push('\n');
            content.push_str(&marker);
            candidates[top].layer.content = content;
            rendered = compose_constitution(&candidates);
        }
    }

    // 5) 极限预算（连标题/标记都放不下）：退化为不超过 max_bytes 的截断摘要。
    if rendered.len() > max_bytes {
        rendered = clamp_to_budget(&rendered, max_bytes);
    }
    rendered
}

/// 硬裁剪到 `max_bytes` 以内的截断摘要（UTF-8 安全，尾部以 `...` 标记）。
fn clamp_to_budget(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    if max_bytes <= 3 {
        return ".".repeat(max_bytes);
    }
    let mut clamped = truncate_at_char_boundary(text, max_bytes - 3).to_string();
    clamped.push_str("...");
    clamped
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
    /// 带预算的宪法解析（wave-42 / PA-150）。
    ///
    /// 契约：
    /// - 单层文件超过 `max_source_bytes` 时**有界读取**（不整份入内存）：最高优先级层
    ///   截断保留首部，低优先级层整层省略并计入 `omitted`；
    /// - 预算按**渲染后字节**计（含 `## ... (<path>)` 标题、层间空行、`omitted` 行），
    ///   两遍法先定保留集合再渲染，保证 `rendered.len() <= max_bytes` 严格成立；
    /// - **最高优先级层永不整层丢弃**（无论 Project 还是 Global）：放不下时按 UTF-8
    ///   边界截断正文并追加 `truncated <path>` 标记；预算极小时退化为不超过上限的摘要；
    /// - 低优先级层从低到高逐层省略，每个省略层在末尾按优先级高→低追加 `omitted <path>`；
    /// - 未被省略/截断的层沿用 `render_constitution` 的既有格式（默认预算下与
    ///   `resolve_constitution_with_globals` 字节完全一致）。
    pub fn resolve_constitution_with_budget<P: AsRef<Path>>(
        &self,
        workspace_root: P,
        global_homes: &[PathBuf],
        budget: InstructionBudget,
    ) -> Option<(String, bool)> {
        let sources = discover_constitution_sources(workspace_root.as_ref(), global_homes);
        if sources.is_empty() {
            return None;
        }

        let (snapshots, layers, hit) =
            self.load_constitution_layers(&sources, budget.max_source_bytes);
        if layers.is_empty() {
            // 全部来源读取失败：与既有行为一致，视为无可注入内容（不伪造 omitted）。
            return None;
        }

        let rendered = render_constitution_with_budget(&snapshots, layers, budget.max_bytes);
        Some((rendered, hit))
    }

    /// 带预算的 Skills 扫描（wave-42 / PA-150）：注入字节总量受 `max_bytes` 约束，
    /// 超限时从**最低优先级**技能开始丢弃（rank 大者先出局，同 rank 全局层先出局）。
    pub fn scan_skills_with_budget<P: AsRef<Path>>(
        &self,
        workspace_root: P,
        global_homes: &[PathBuf],
        budget: InstructionBudget,
    ) -> (Vec<HotloadedSkill>, bool) {
        let (skills, hit) = self.scan_skills_with_globals(workspace_root, global_homes);
        let costs: Vec<usize> = skills.iter().map(skill_injection_cost).collect();
        let mut total: usize = costs.iter().sum();
        if total <= budget.max_bytes {
            return (skills, hit);
        }

        // 优先级从高到低：rank 小者优先；同 rank 时项目层优先；再按既定列表顺序。
        let mut by_priority: Vec<usize> = (0..skills.len()).collect();
        by_priority.sort_by(|&left, &right| {
            skills[left]
                .rank
                .cmp(&skills[right].rank)
                .then(skills[right].scope.cmp(&skills[left].scope))
                .then(left.cmp(&right))
        });

        let mut dropped: HashSet<usize> = HashSet::new();
        for index in by_priority.into_iter().rev() {
            if total <= budget.max_bytes {
                break;
            }
            total -= costs[index];
            dropped.insert(index);
        }

        let kept = skills
            .into_iter()
            .enumerate()
            .filter(|(index, _)| !dropped.contains(index))
            .map(|(_, skill)| skill)
            .collect();
        (kept, hit)
    }
}

/// 单个技能在静态前缀里占用的注入字节数（`- `name` [scope]: description`）。
fn skill_injection_cost(skill: &HotloadedSkill) -> usize {
    skill.name.len() + skill.scope.as_str().len() + skill.description.len() + 8
}

/// 由 cwd 向上探测 `marker` 定位项目根，`bound` 为不可越过的上界。
///
/// - `marker` 可以是文件或目录（`.git` 的 worktree/submodule 形态即为文件）；
/// - 探测包含 `bound` 自身，但绝不越过它（bound 之上不再探测）；
/// - 未命中返回 `bound`。
pub fn find_project_root(cwd: &Path, marker: &str, bound: &Path) -> PathBuf {
    let mut current = cwd.to_path_buf();
    loop {
        if current.join(marker).exists() {
            return current;
        }
        if current == bound {
            return bound.to_path_buf();
        }
        // 只在 bound 之内（含 bound）继续上爬；越界即停在 bound。
        match current.parent() {
            Some(parent) if parent != current && parent.starts_with(bound) => {
                current = parent.to_path_buf();
            }
            _ => return bound.to_path_buf(),
        }
    }
}

/// 生成 root→cwd 的祖先目录链（含两端，浅→深）。
///
/// `root == cwd` 时返回单元素；若 `cwd` 不在 `root` 之下，仍保证链首为 `root`、链尾为 `cwd`。
pub fn ancestor_chain(root: &Path, cwd: &Path) -> Vec<PathBuf> {
    let mut chain: Vec<PathBuf> = vec![cwd.to_path_buf()];
    let mut current = cwd.to_path_buf();
    while current != root {
        match current.parent() {
            Some(parent) if parent != current && parent.starts_with(root) => {
                chain.push(parent.to_path_buf());
                current = parent.to_path_buf();
            }
            _ => break,
        }
    }
    chain.reverse();
    if chain.first().map(PathBuf::as_path) != Some(root) {
        chain.insert(0, root.to_path_buf());
    }
    chain
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
