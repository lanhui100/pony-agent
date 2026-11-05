//! Wave-58 验收测试（Test Agent 冻结，只读）：上下文根 / 技能清单注入 / 触达去重。
//!
//! 契约依据：`.dev-team/contract-matrix-wave58.md`
//!   - F1-1 根来源（会话工作区根，而非进程 cwd）
//!   - F1-2 技能清单注入 semistable 段（标题 + `- \`name\` [scope]: description`）
//!   - F1-3 前缀稳定（同会话第 2 轮宪法/清单字节不变，中途改动只追加 notice）
//!   - F1-4 渐进披露（`SKILL.md` 正文不得进入请求文本）
//!   - F1-5 子代理继承父会话工作区根
//!   - F1-6 预算门（context window < 8192 tokens 时不注入技能清单，宪法照旧）
//!   - F3-1 根层指令不被触达层重复注入
//!   - F3-2 子目录指令仍注入且会话内只注入一次（回归护栏）
//!   - F3-3 根层/触达层共用同一候选常量集合
//!   - F3-4 `constitution_source_paths` 供触达层种子化
//!
//! 观测面（全部为既有公开导出，未臆造 API）：
//! - `DefaultTurnContextBuilder::build_request`（`TurnContextBuilder` trait）→
//!   `ProviderRequest.observation.{stable_prefix_text, semi_stable_context_text, volatile_input_text}`
//!   是 provider 请求文本的可观测投影；
//! - `WorkspaceHotloader::build_cached_prompt_prefix`（semistable 前缀构造层，F1-2 落点）；
//! - `ToolRouter::execute_with_workspace_root`（`workspace_read_file`）→ 工具结果文本里的
//!   `=== Additional instructions` 段落是触达注入的可观测投影。
//!
//! 隔离纪律（test-expert §4.1）：
//! - 所有 fixture 一律临时目录（`std::env::temp_dir()` + PID/纳秒/序号唯一名），`TempDir`
//!   在 Drop（含 panic 展开）时递归清理；
//! - 需要全局层时只使用**显式注入变体**（`*_with_globals` / `build_cached_prompt_prefix`
//!   的显式 `skills` 形参），不读也不断言宿主 `~/.agents` 真实内容；
//! - 不伪造进程 cwd（Lead 裁决 R1）：F1-1 只断言「semistable 段取自 `env_info.cwd` 指向的
//!   目录」，进程 cwd 的真实取值不作为断言输入 ⇒ 无跨用例 cwd 竞态，无 flaky。
//!
//! Lead 裁决 R2（F1-2 分层）：
//! - 「无技能不得出现清单标题」与「清单行格式」在 **prefix 构造层**验证（`build_cached_prompt_prefix`）；
//! - provider 层只验证 F1-4「`SKILL.md` 正文不得进入请求文本」；
//! - F1-1 / F1-6 仍走 provider 层（根来源与预算门只能在 semistable 段观测）。
//!
//! 已知不可从外部观测的契约点（如实上报，不臆造 API）：
//! - **F1-5** 的实质语义是「`HostTurnEngine::run_turn` 用父根构建子代理 runtime」。该路径
//!   内部自建 `ProviderRegistryStore` 且无 provider 注入接缝，外部无法在不真实联网的前提下
//!   观测「建栈用的 root」。本文件因此只断言可观测的装配面（`with_workspace_root` 的 root
//!   传递与 `None` 清空），实质语义需由 in-crate 单测或人工审查裁决（见用例内注释）。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use pony_agent_core::agent::config::{
    ProviderModelCapabilities, ResolvedProviderSelection, ThinkingParamPattern,
};
use pony_agent_core::agent::context::{
    DefaultTurnContextBuilder, LongTermMemory, RetrievedContextState, RunState, SessionContext,
    TranscriptContext, TurnContext, TurnContextBuilder,
};
use pony_agent_core::agent::provider::{
    ProviderAuthType, ProviderManager, ProviderProtocol, ProviderRequest,
};
use pony_agent_core::agent::session::EnvironmentInfo;
use pony_agent_core::agent::subagent_runtime::HostTurnEngine;
use pony_agent_core::agent::tools::{ToolCall, ToolExecutionContext, ToolRouter};
use pony_agent_core::agent::workspace_hotloader::{
    constitution_source_paths, AgentScope, FileFingerprint, HotloadedSkill, WorkspaceHotloader,
    INSTRUCTION_CANDIDATES,
};
use serde_json::json;

/// semistable 段里技能清单的固定标题（契约 F1-2，字面量由契约矩阵冻结）。
const SKILL_MANIFEST_TITLE: &str = "=== AVAILABLE REUSABLE SKILLS ===";

/// 触达注入段落的固定前缀（既有契约 `wave42` C4-5 冻结字面量）。
const TOUCH_PRELUDE_PREFIX: &str = "=== Additional instructions";

// ============================================================
// 夹具：唯一临时目录 / 环境变量守卫 / cwd 守卫
// ============================================================

/// 唯一临时目录，Drop（含 panic 展开）时递归清理。
struct TempDir {
    root: PathBuf,
}

impl TempDir {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let sequence = COUNTER.fetch_add(1, Ordering::SeqCst);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        let root = std::env::temp_dir().join(format!(
            "wave58-f1f3-{tag}-{}-{nanos}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("创建 wave-58 F1/F3 测试临时目录失败");
        Self { root }
    }

    fn path(&self) -> &Path {
        &self.root
    }

    /// canonical 形态（触达层与加载层都以 canonical 路径为判定基准）。
    fn canonical(&self) -> PathBuf {
        self.root
            .canonicalize()
            .expect("临时目录 canonicalize 失败")
    }

    fn write(&self, relative: &str, content: &str) -> PathBuf {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("创建临时子目录失败");
        }
        fs::write(&path, content).expect("写入临时文件失败");
        path
    }

    fn write_skill(&self, relative_dir: &str, name: &str, description: &str) -> PathBuf {
        let body = "---\n".to_string()
            + &format!("name: {name}\n")
            + &format!("description: {description}\n")
            + "---\n\n# Skill body\n";
        self.write(&format!("{relative_dir}/SKILL.md"), &body)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

// ============================================================
// 请求构造辅助（AAA 的 Arrange 部分）
// ============================================================

fn provider_manager(context_window_tokens: Option<u32>) -> ProviderManager {
    ProviderManager::new(ResolvedProviderSelection {
        requested_name: "w58-test-provider".to_string(),
        provider_name: "w58-test-provider".to_string(),
        protocol: ProviderProtocol::OpenAiCompletions,
        base_url: "https://example.invalid/v1".to_string(),
        auth_type: ProviderAuthType::Auto,
        api_key_env_var: "W58_TEST_PROVIDER_API_KEY".to_string(),
        // 仅为构造：build_request 不发网络请求，断言全部落在离线可观测投影上。
        api_key: Some("w58-dummy-key".to_string()),
        model: "w58-test-model".to_string(),
        temperature: 0.0,
        max_output_tokens: 1024,
        reasoning_effort: None,
        reasoning_budget_tokens: None,
        capabilities: ProviderModelCapabilities {
            context_window_tokens,
            supports_reasoning: false,
            ..Default::default()
        },
        thinking_param_pattern: ThinkingParamPattern::None,
    })
}

fn env_info(cwd: &Path) -> EnvironmentInfo {
    EnvironmentInfo {
        cwd: cwd.display().to_string(),
        platform: std::env::consts::OS.to_string(),
        shell: None,
        os_version: None,
        current_date: "2026-11-01".to_string(),
        timezone: Some("UTC".to_string()),
        is_git_repo: false,
        git_branch: None,
    }
}

fn retrieved_state(cwd: &Path, conversation_id: &str, turn_count: usize) -> RetrievedContextState {
    RetrievedContextState {
        turn_context: TurnContext {
            user_message: "wave58 acceptance".to_string(),
            images: Vec::new(),
            references_image: false,
            workspace_mode: None,
        },
        session_context: SessionContext {
            conversation_id: conversation_id.to_string(),
            title: "wave58".to_string(),
            summary: String::new(),
            recent_history: Vec::new(),
            recent_attachment_assets: Vec::new(),
            turn_count,
            last_referenced_file: None,
            env_info: Some(env_info(cwd)),
        },
        run_state: RunState::default(),
        long_term_memory: LongTermMemory::default(),
        transcript: TranscriptContext::default(),
    }
}

fn build_request(
    provider: &ProviderManager,
    retrieved: &RetrievedContextState,
) -> ProviderRequest {
    DefaultTurnContextBuilder.build_request("wave58-graph", provider, retrieved, &[])
}

/// semistable 段（宪法 + 技能清单的落点）文本投影。
fn semistable_text(request: &ProviderRequest) -> String {
    request.observation.semi_stable_context_text.clone()
}

/// 整个请求文本投影（稳定前缀 + semistable + volatile 输入），用于「正文不得泄漏」类断言。
fn request_text(request: &ProviderRequest) -> String {
    format!(
        "{}\n{}\n{}",
        request.observation.stable_prefix_text,
        request.observation.semi_stable_context_text,
        request.observation.volatile_input_text
    )
}

fn unique_conversation_id(tag: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0);
    format!("wave58-{tag}-{}-{nanos}", std::process::id())
}

/// 构造一个 `HotloadedSkill` fixture（字段全部是公开导出，未臆造 API）。
fn skill_fixture(
    name: &str,
    description: &str,
    scope: AgentScope,
    rank: u16,
    path: &Path,
) -> HotloadedSkill {
    HotloadedSkill {
        name: name.to_string(),
        description: description.to_string(),
        user_invocable: true,
        allowed_tools: Vec::new(),
        path: path.to_path_buf(),
        fingerprint: FileFingerprint {
            path: path.to_path_buf(),
            mtime: None,
            file_size: 0,
            content_hash: 0,
        },
        scope,
        rank,
    }
}

// ============================================================
// F1-1：根来源 = 会话工作区根（禁止再读进程 cwd）
// ============================================================

#[test]
fn f1_1_semistable_follows_session_workspace_root() {
    // Arrange：会话工作区根（`env_info.cwd`）指向唯一临时目录，内含可辨识标记的 AGENTS.md。
    // 不伪造进程 cwd（Lead 裁决 R1）：断言只依赖「标记必须来自 env_info.cwd 指向的目录」。
    let workspace = TempDir::new("f1-1-workspace");
    workspace.write(
        "AGENTS.md",
        "# W58-SESSION-ROOT-MARKER\n仅在会话工作区内生效的规则。\n",
    );
    // 同级另一工作区（不是会话根）：其标记绝不应进入本会话提示词。
    let other_workspace = TempDir::new("f1-1-other-workspace");
    other_workspace.write(
        "AGENTS.md",
        "# W58-OTHER-WORKSPACE-MARKER\n别的会话工作区规则。\n",
    );
    let provider = provider_manager(Some(128_000));
    let retrieved = retrieved_state(workspace.path(), &unique_conversation_id("f1-1"), 1);

    // Act
    let request = build_request(&provider, &retrieved);

    // Assert
    let semistable = semistable_text(&request);
    assert!(
        semistable.contains("W58-SESSION-ROOT-MARKER"),
        "F1-1：semistable 段必须以会话工作区根（`env_info.cwd`）为指令发现根，\
         必须包含该根下 AGENTS.md 的正文；实际 semistable 段：{semistable}"
    );
    assert!(
        !semistable.contains("W58-OTHER-WORKSPACE-MARKER"),
        "F1-1：非会话根目录的指令不得进入提示词（发现根必须唯一锚定在 `env_info.cwd`）；\
         实际 semistable 段：{semistable}"
    );
}

#[test]
fn f1_1_missing_env_info_falls_back_without_panic() {
    // Arrange：会话缺少 env_info（契约 F1-1 的回落分支：否则进程 cwd，且不得 panic）
    let provider = provider_manager(Some(128_000));
    let mut retrieved = retrieved_state(
        Path::new("/wave58-nonexistent-root"),
        &unique_conversation_id("f1-1-fallback"),
        1,
    );
    retrieved.session_context.env_info = None;

    // Act
    let request = build_request(&provider, &retrieved);

    // Assert：不得 panic（能走到这里即未 panic），且 semistable 段仍产出会话标签
    let semistable = semistable_text(&request);
    assert!(
        semistable.contains("Session label: graph=wave58-graph"),
        "F1-1：env_info 缺失时必须回落进程 cwd 且不得 panic，semistable 段仍需含会话标签；\
         实际 semistable 段：{semistable}"
    );
}

// ============================================================
// F1-2：技能清单注入 semistable 段
// ============================================================

#[test]
fn f1_2_manifest_title_and_line_format_in_prefix_layer() {
    // Arrange：prefix 构造层显式注入两个技能（项目级 + 用户级），不依赖宿主 ~/.agents
    let workspace = TempDir::new("f1-2-prefix");
    let project_path = workspace.write(
        ".agents/skills/w58-prefix-demo/SKILL.md",
        "---\nname: w58-prefix-demo\ndescription: fixture\n---\n\n# body\n",
    );
    let global_path = workspace.write(
        "global-home/skills/w58-prefix-global/SKILL.md",
        "---\nname: w58-prefix-global\ndescription: fixture\n---\n\n# body\n",
    );
    let project_description = "W58 prefix project description";
    let global_description = "W58 prefix global description";
    let skills = vec![
        skill_fixture(
            "w58-prefix-demo",
            project_description,
            AgentScope::Project,
            200,
            &project_path,
        ),
        skill_fixture(
            "w58-prefix-global",
            global_description,
            AgentScope::Global,
            400,
            &global_path,
        ),
    ];
    let hotloader = WorkspaceHotloader::new();

    // Act
    let prefix = hotloader.build_cached_prompt_prefix(Some("# W58-PREFIX-CONSTITUTION\n"), &skills);

    // Assert：标题 + 逐行格式 `- `name` [global|project]: description`
    assert!(
        prefix.contains(SKILL_MANIFEST_TITLE),
        "F1-2：存在技能时 prefix 必须包含标题 `{SKILL_MANIFEST_TITLE}`；实际前缀：{prefix}"
    );
    assert!(
        prefix.contains(&format!(
            "- `w58-prefix-demo` [project]: {project_description}"
        )),
        "F1-2：项目级技能行必须形如 `- `name` [project]: description`；实际前缀：{prefix}"
    );
    assert!(
        prefix.contains(&format!(
            "- `w58-prefix-global` [global]: {global_description}"
        )),
        "F1-2：用户级技能行必须形如 `- `name` [global]: description`；实际前缀：{prefix}"
    );

    // Assert：逐行格式不变量（任何技能行都必须带作用域标签与非空描述）
    let manifest_region = &prefix[prefix
        .find(SKILL_MANIFEST_TITLE)
        .expect("F1-2 前置：标题已断言存在")..];
    let mut listed = 0usize;
    for line in manifest_region.lines() {
        let line = line.trim_end();
        if !line.starts_with("- `") {
            continue;
        }
        listed += 1;
        let rest = &line[3..];
        let close = rest
            .find('`')
            .unwrap_or_else(|| panic!("F1-2：技能清单行缺少反引号闭合：{line:?}"));
        let (name, after_name) = (&rest[..close], rest[close + 1..].trim_start());
        assert!(!name.trim().is_empty(), "F1-2：技能名不得为空：{line:?}");
        let (scope_prefix, description) = if let Some(rest) = after_name.strip_prefix("[project]: ")
        {
            ("project", rest)
        } else if let Some(rest) = after_name.strip_prefix("[global]: ") {
            ("global", rest)
        } else {
            panic!(
                "F1-2：技能清单行必须紧跟 ` [project]: ` 或 ` [global]: `，实际行：{line:?}"
            )
        };
        assert!(
            !scope_prefix.is_empty() && !description.trim().is_empty(),
            "F1-2：技能清单行必须带非空 description，实际行：{line:?}"
        );
    }
    assert_eq!(
        listed, 2,
        "F1-2：注入 2 个技能就必须渲染 2 行清单，实际 {listed} 行：{manifest_region}"
    );
}

#[test]
fn f1_2_manifest_title_absent_without_skills() {
    // Arrange：不注入任何技能（显式空集合，不读宿主 ~/.agents）
    let hotloader = WorkspaceHotloader::new();

    // Act
    let no_constitution = hotloader.build_cached_prompt_prefix(None, &[]);
    let with_constitution =
        hotloader.build_cached_prompt_prefix(Some("# W58-EMPTY-SKILLS-CONSTITUTION\n"), &[]);

    // Assert：无技能 ⇒ 不得出现清单标题（两个分支都必须成立）
    assert!(
        !no_constitution.contains(SKILL_MANIFEST_TITLE),
        "F1-2：无技能时不得出现清单标题；实际前缀：{no_constitution}"
    );
    assert!(
        no_constitution.is_empty(),
        "F1-2：既无宪法又无技能时 prefix 必须为空，实际：{no_constitution:?}"
    );
    assert!(
        !with_constitution.contains(SKILL_MANIFEST_TITLE),
        "F1-2：无技能时不得出现清单标题；实际前缀：{with_constitution}"
    );
    assert!(
        with_constitution.contains("W58-EMPTY-SKILLS-CONSTITUTION"),
        "F1-2：宪法段必须照旧注入（不得为去掉清单而丢宪法）；实际前缀：{with_constitution}"
    );
}

// ============================================================
// F1-3：同会话前缀稳定 + 中途改动只追加 notice
// ============================================================

#[test]
fn f1_3_semistable_stable_across_turns_and_mid_session_edit_appends_notice() {
    // Arrange：第 1 轮基线
    let workspace = TempDir::new("f1-3-workspace");
    workspace.write(
        "AGENTS.md",
        "# W58-BASELINE-V1\n第一轮固化的规则正文。\n",
    );
    workspace.write_skill(
        ".agents/skills/w58-stable-skill",
        "w58-stable-skill",
        "W58 stable skill description",
    );
    let provider = provider_manager(Some(128_000));
    let conversation_id = unique_conversation_id("f1-3");

    // Act 1：第 1 轮建基线
    let turn1 = build_request(&provider, &retrieved_state(workspace.path(), &conversation_id, 1));
    let turn1_text = semistable_text(&turn1);

    // 断点 1：基线建立
    assert!(
        turn1_text.contains("W58-BASELINE-V1"),
        "F1-3 前置：第 1 轮 semistable 段必须包含当前宪法正文；实际：{turn1_text}"
    );
    assert!(
        turn1_text.contains(SKILL_MANIFEST_TITLE),
        "F1-3 前置：第 1 轮 semistable 段必须包含技能清单；实际：{turn1_text}"
    );

    // Arrange 2：会话中途修改 AGENTS.md（改出新标记）
    workspace.write(
        "AGENTS.md",
        "# W58-BASELINE-V2-REWRITE\n第二轮才出现的规则正文。\n",
    );

    // Act 2：同会话第 2 轮
    let turn2 = build_request(&provider, &retrieved_state(workspace.path(), &conversation_id, 2));
    let turn2_text = semistable_text(&turn2);

    // Assert：头部静态段字节不变（仍是 V1 内容，V2 不得改写前缀）
    assert!(
        turn2_text.contains("W58-BASELINE-V1"),
        "F1-3：同会话第 2 轮起宪法段必须字节级不变（仍含第 1 轮基线正文）；实际：{turn2_text}"
    );
    assert!(
        !turn2_text.contains("W58-BASELINE-V2-REWRITE"),
        "F1-3：中途修改 AGENTS.md 不得改写 semistable 段，新正文只能以 notice 形式出现；实际：{turn2_text}"
    );
    assert!(
        turn2_text.contains(SKILL_MANIFEST_TITLE),
        "F1-3：第 2 轮技能清单段必须仍在（不得因轮次变化而消失）；实际：{turn2_text}"
    );

    // Assert：变更以追加 notice 表达，且位于基线内容之后（append-only 语义）
    let notice_at = turn2_text
        .to_lowercase()
        .find("notice")
        .unwrap_or_else(|| {
            panic!("F1-3：中途修改必须产生追加 notice（volatile 段），实际 semistable 文本：{turn2_text}")
        });
    let baseline_at = turn2_text
        .find("W58-BASELINE-V1")
        .expect("F1-3 前置：第 2 轮基线正文已断言存在");
    assert!(
        notice_at > baseline_at,
        "F1-3：notice 必须追加在基线内容之后（禁止改写头部），baseline@{baseline_at} notice@{notice_at}；\
         实际：{turn2_text}"
    );
}

// ============================================================
// F1-4：渐进披露（正文不进提示词，只有清单里的 description）
// ============================================================

#[test]
fn f1_4_skill_body_not_injected_into_provider_request() {
    // Arrange：技能正文带唯一标记（正文只在 skill_load 工具结果里出现）
    let workspace = TempDir::new("f1-4-workspace");
    let body_marker = "W58-SKILL-BODY-SECRET-MARKER";
    workspace.write(
        ".agents/skills/w58-body-skill/SKILL.md",
        &format!(
            "---\nname: w58-body-skill\ndescription: W58 progressive disclosure description\n---\n\n# {body_marker}\n正文不得进入提示词。\n"
        ),
    );
    let provider = provider_manager(Some(128_000));
    let retrieved = retrieved_state(workspace.path(), &unique_conversation_id("f1-4"), 1);

    // Act
    let request = build_request(&provider, &retrieved);
    let text = request_text(&request);

    // Assert 前置：这是一个真实构造出来的请求（否则下面的「不包含」就是空断言）
    assert!(
        !request.input.is_empty(),
        "F1-4 前置：provider 请求必须已构造出消息层，实际 input 为空"
    );
    assert!(
        text.contains("wave58 acceptance"),
        "F1-4 前置：请求文本必须包含本轮用户输入，实际：{text}"
    );

    // Assert：正文绝不得出现在请求文本的任何投影中
    assert!(
        !text.contains(body_marker),
        "F1-4：`SKILL.md` 正文绝不得出现在 provider 请求文本中（渐进披露：只能经 skill_load 返回）；\
         实际：{text}"
    );
    assert!(
        !text.contains("正文不得进入提示词"),
        "F1-4：`SKILL.md` 正文（含其后续行）不得进入请求文本；实际：{text}"
    );
}

// ============================================================
// F1-5：子代理继承父会话工作区根（可观测面有限，见文件头说明）
// ============================================================

#[test]
fn f1_5_host_engine_carries_parent_workspace_root() {
    // Arrange：父会话工作区
    let parent_workspace = TempDir::new("f1-5-parent");

    // Act：注入父根
    let engine = HostTurnEngine {
        workspace_root: None,
    }
    .with_workspace_root(Some(parent_workspace.path().to_path_buf()));

    // Assert：父根被原样承接（子代理建栈必须以此根构建 runtime，不得固定默认根）
    assert_eq!(
        engine.workspace_root.as_deref(),
        Some(parent_workspace.path()),
        "F1-5：HostTurnEngine 必须承接父会话工作区根（F1-5 实质语义「run_turn 以父根建栈」\
         无外部 provider 注入接缝，本用例只锁定装配面；实质语义需 in-crate 单测/人工审查裁决）"
    );

    // Assert：无父根时才回落默认（清空即为回落路径的输入形态）
    let engine_without_parent = engine.with_workspace_root(None);
    assert!(
        engine_without_parent.workspace_root.is_none(),
        "F1-5：无父根时必须以 None 表达回落默认根，不得残留上一次注入的根"
    );
}

// ============================================================
// F1-6：小上下文窗口不注入技能清单（宪法照旧）
// ============================================================

#[test]
fn f1_6_small_context_window_skips_manifest_but_keeps_constitution() {
    // Arrange
    let workspace = TempDir::new("f1-6-workspace");
    workspace.write("AGENTS.md", "# W58-SMALL-WINDOW-CONSTITUTION\n");
    workspace.write_skill(
        ".agents/skills/w58-window-skill",
        "w58-window-skill",
        "W58 window gated skill description",
    );
    let conversation_id = unique_conversation_id("f1-6");

    // Act + Assert（小窗口）：技能清单被预算门挡掉，宪法照旧
    let small_provider = provider_manager(Some(4_096));
    let small_request = build_request(
        &small_provider,
        &retrieved_state(workspace.path(), &conversation_id, 1),
    );
    let small_text = semistable_text(&small_request);
    assert!(
        !small_text.contains(SKILL_MANIFEST_TITLE),
        "F1-6：context_window_tokens < 8192 时不得注入技能清单；实际：{small_text}"
    );
    assert!(
        small_text.contains("W58-SMALL-WINDOW-CONSTITUTION"),
        "F1-6：预算门只作用于技能清单，宪法必须照旧注入；实际：{small_text}"
    );

    // Act + Assert（足够大的窗口）：清单必须出现（证明上一条不是因为清单根本没接线）
    let large_provider = provider_manager(Some(128_000));
    let large_request = build_request(
        &large_provider,
        &retrieved_state(workspace.path(), &conversation_id, 1),
    );
    let large_text = semistable_text(&large_request);
    assert!(
        large_text.contains(SKILL_MANIFEST_TITLE),
        "F1-6 对照：上下文窗口充足时技能清单必须注入（否则「小窗口不注入」不可证）；实际：{large_text}"
    );
    assert!(
        large_text.contains("w58-window-skill"),
        "F1-6 对照：充足窗口下技能名必须出现在清单中；实际：{large_text}"
    );
}

// ============================================================
// F3-1：根层指令不得被触达层重复注入
// ============================================================

fn read_call(path: &str, call_id: &str) -> ToolCall {
    ToolCall {
        call_id: Some(call_id.to_string()),
        name: "workspace_read_file".to_string(),
        arguments: json!({ "path": path }),
        plan: None,
    }
}

#[test]
fn f3_1_root_layer_not_reinjected_on_touch() {
    // Arrange：根层 AGENTS.md 已在会话首轮进入 semistable 段，另有子文件可触达
    let workspace = TempDir::new("f3-1-workspace");
    workspace.write(
        "AGENTS.md",
        "# W58-F3-1-ROOT-MARKER\n根层规则，首轮已注入 semistable 段。\n",
    );
    workspace.write("src/main.rs", "fn main() {}\n");
    let root = workspace.canonical();
    let router = ToolRouter::with_workspace_root(root.clone());

    // Lead 裁决 D6'（Wave-58）：触达层跳过根层指令的**前提**是本次调用处于会话中
    // （session_id 非空 ⇒ 该会话前缀已由 semistable 段承载工作区根层指令）；
    // 无 session_id 的探针式调用保持既有行为（wave42 C4 系列契约不变）。
    let context = ToolExecutionContext {
        workspace_root: Some(root.clone()),
        workspace_id: None,
        session_id: Some("w58-f3-1-session".to_string()),
        dispatch_context: None,
        cancellation: None,
        provider_selection: None,
    };

    // Act：会话内触达工作区内文件
    let result = router.execute_with_context(&read_call("src/main.rs", "call-f3-1"), &context);

    // Assert
    assert_eq!(
        result.status, "ok",
        "F3-1 前置：workspace_read_file 必须成功，实际 status={} output={}",
        result.status, result.output
    );
    assert!(
        result.output.contains("读取成功"),
        "F3-1：既有结果关键字段不得被破坏，实际：{}",
        result.output
    );
    assert!(
        !result.output.contains("W58-F3-1-ROOT-MARKER"),
        "F3-1：根层 AGENTS.md 已由 semistable 段注入，触达层不得再次注入；实际工具输出：{}",
        result.output
    );
}

// ============================================================
// F3-2：子目录指令仍注入，且会话内只注入一次（回归护栏）
// ============================================================

#[test]
fn f3_2_sub_directory_layer_injected_once_per_session() {
    // Arrange：根层 + 子目录各一份指令，两个子文件供两次触达
    let workspace = TempDir::new("f3-2-workspace");
    workspace.write("AGENTS.md", "# W58-F3-2-ROOT-MARKER\n根层规则。\n");
    workspace.write("sub/AGENTS.md", "# W58-F3-2-SUB-MARKER\n子目录规则。\n");
    workspace.write("sub/x.rs", "pub fn x() {}\n");
    workspace.write("sub/y.rs", "pub fn y() {}\n");
    let root = workspace.canonical();
    let router = ToolRouter::with_workspace_root(root.clone());

    // Act 1：首次触达 sub/x.rs
    let first = router.execute_with_workspace_root(&read_call("sub/x.rs", "call-f3-2-a"), Some(&root));

    // Assert 1：子目录指令必须注入（F3-1 的修复不得把子目录一并种子化掉）
    assert_eq!(
        first.status, "ok",
        "F3-2 前置：首次触达必须成功，实际 status={} output={}",
        first.status, first.output
    );
    assert!(
        first.output.contains(TOUCH_PRELUDE_PREFIX),
        "F3-2：首次触达 sub/ 下的文件必须注入子目录指令段落，实际：{}",
        first.output
    );
    assert!(
        first.output.contains("W58-F3-2-SUB-MARKER"),
        "F3-2：子目录 AGENTS.md 必须在首次触达 `sub/x.rs` 时注入，实际：{}",
        first.output
    );

    // Act 2：会话内第二次触达 sub/y.rs
    let second = router.execute_with_workspace_root(&read_call("sub/y.rs", "call-f3-2-b"), Some(&root));

    // Assert 2：会话内去重 —— 同一子目录指令只注入一次
    assert_eq!(
        second.status, "ok",
        "F3-2 前置：第二次触达必须成功，实际 status={} output={}",
        second.status, second.output
    );
    assert!(
        !second.output.contains("W58-F3-2-SUB-MARKER"),
        "F3-2：同一会话内子目录指令只注入一次，第二次触达不得重复注入；实际：{}",
        second.output
    );
}

// ============================================================
// F3-3：根层与触达层共用同一候选常量集合
// ============================================================

#[test]
fn f3_3_instruction_candidates_single_source() {
    // Assert（常量契约）：唯一来源必须恰好是契约矩阵列举的 5 个候选且顺序冻结
    assert_eq!(
        INSTRUCTION_CANDIDATES,
        [
            "AGENTS.md",
            "CLAUDE.md",
            ".claude/CLAUDE.md",
            "AGENTS.local.md",
            "CLAUDE.local.md",
        ],
        "F3-3：指令候选必须由 workspace_hotloader 单一导出且顺序冻结（基础候选 3 项 + local overlay 2 项）"
    );

    // Arrange：两个工作区分别只放「触达层历史缺失」的候选
    let claude_layout = TempDir::new("f3-3-claude-layout");
    claude_layout.write("sub/.claude/CLAUDE.md", "# W58-F3-3-CLAUDE-LAYOUT-MARKER\n");
    claude_layout.write("sub/x.rs", "pub fn x() {}\n");

    let local_overlay = TempDir::new("f3-3-local-overlay");
    local_overlay.write("sub/AGENTS.local.md", "# W58-F3-3-LOCAL-OVERLAY-MARKER\n");
    local_overlay.write("sub/x.rs", "pub fn x() {}\n");

    // Act + Assert：触达层必须识别全部候选（不得再自带第二份字面量表）
    for (workspace, marker, candidate) in [
        (&claude_layout, "W58-F3-3-CLAUDE-LAYOUT-MARKER", ".claude/CLAUDE.md"),
        (&local_overlay, "W58-F3-3-LOCAL-OVERLAY-MARKER", "AGENTS.local.md"),
    ] {
        let root = workspace.canonical();
        let router = ToolRouter::with_workspace_root(root.clone());
        let result = router.execute_with_workspace_root(&read_call("sub/x.rs", "call-f3-3"), Some(&root));
        assert_eq!(
            result.status, "ok",
            "F3-3 前置：触达必须成功，实际 status={} output={}",
            result.status, result.output
        );
        assert!(
            result.output.contains(marker),
            "F3-3：触达层必须与根层共用候选集合，`{candidate}` 也必须被识别并注入；实际：{}",
            result.output
        );
    }
}

// ============================================================
// F3-4：constitution_source_paths 单一来源清单（供触达层种子化）
// ============================================================

#[test]
fn f3_4_constitution_source_paths_lists_injected_sources() {
    // Arrange：项目层 AGENTS.md + 用户级全局 AGENTS.md（显式注入全局根）
    let workspace = TempDir::new("f3-4-workspace");
    let global_home = TempDir::new("f3-4-global-home");
    let project_agents = workspace.write("AGENTS.md", "# W58-F3-4-PROJECT-MARKER\n");
    let global_agents = global_home.write("AGENTS.md", "# W58-F3-4-GLOBAL-MARKER\n");
    let workspace_root = workspace.canonical();
    let global_root = global_home.canonical();

    // Act
    let sources = constitution_source_paths(&workspace_root, &[global_root.clone()]);

    // Assert：必须包含两层实际会注入的指令文件
    let canonical_project = project_agents
        .canonicalize()
        .expect("项目层 AGENTS.md 必须可 canonicalize");
    let canonical_global = global_agents
        .canonicalize()
        .expect("全局层 AGENTS.md 必须可 canonicalize");
    assert!(
        sources.contains(&canonical_project),
        "F3-4：来源清单必须包含项目层 AGENTS.md 的 canonical 路径 {canonical_project:?}；实际：{sources:?}"
    );
    assert!(
        sources.contains(&canonical_global),
        "F3-4：来源清单必须包含用户级全局 AGENTS.md 的 canonical 路径 {canonical_global:?}；实际：{sources:?}"
    );

    // Assert：清单元素必须是「已 canonical 化的真实普通文件」（触达层按 canonical 路径去重）
    assert!(
        !sources.is_empty(),
        "F3-4：存在可注入指令时来源清单不得为空"
    );
    for path in &sources {
        assert!(
            path.is_absolute(),
            "F3-4：来源路径必须是绝对路径，实际：{path:?}"
        );
        let canonical = path
            .canonicalize()
            .unwrap_or_else(|error| panic!("F3-4：来源路径必须存在且可解析，{path:?} 失败：{error}"));
        assert_eq!(
            &canonical, path,
            "F3-4：来源路径必须已是 canonical 形态（否则触达层无法按路径去重）"
        );
        assert!(
            canonical.is_file(),
            "F3-4：来源路径必须是普通文件，实际：{path:?}"
        );
    }

    // Assert：与渲染层一致 —— 清单里的每个来源都必须是本次会注入的指令文件
    let hotloader = WorkspaceHotloader::new();
    let (constitution, _) = hotloader
        .resolve_constitution_with_globals(&workspace_root, &[global_root])
        .expect("F3-4：两层指令存在时宪法必须非空");
    for (marker, path) in [
        ("W58-F3-4-PROJECT-MARKER", &canonical_project),
        ("W58-F3-4-GLOBAL-MARKER", &canonical_global),
    ] {
        assert!(
            constitution.contains(marker),
            "F3-4：来源 {path:?} 必须在渲染结果中出现（清单与渲染不得各自计算来源集合）；实际宪法：{constitution}"
        );
    }
}
