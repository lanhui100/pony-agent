//! 技能调用策略四象限契约测试（PLAN-20261111-02 / ISSUE-20261111-008）。
//!
//! 契约矩阵：
//! | 象限 | user-invocable | disable-model-invocation | model_invocable | 目录可见 | skill_load | 人类面 (/skill:) |
//! | Q1   | true (缺省)   | false (缺省)             | true            | 可见     | 允许       | 允许             |
//! | Q2   | true          | true                     | false           | 过滤(标记)| 拒绝       | 允许             |
//! | Q3   | false         | false                    | true            | 可见     | 允许       | 拒绝             |
//! | Q4   | false         | true                     | false           | 过滤(标记)| 拒绝       | 拒绝             |

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use pony_agent_core::agent::skill_body::{load_skill_body_for_workspace, resolve_skill_by_name};
use pony_agent_core::agent::tools::{ToolCall, ToolExecutionContext, ToolRouter};
use pony_agent_core::agent::workspace_hotloader::{
    resolve_custom_skill_dirs_merged, SkillCatalogBudget, WorkspaceHotloader,
};

static TEST_DIR_SEQ: AtomicU64 = AtomicU64::new(1);

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let seq = TEST_DIR_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("pony_test_{prefix}_{nanos}_{seq}"));
        fs::create_dir_all(&dir).unwrap();
        Self { path: dir }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn create_skill(root: &Path, name: &str, frontmatter_content: &str, body: &str) -> PathBuf {
    let skill_dir = root.join(".agents").join("skills").join(name);
    fs::create_dir_all(&skill_dir).unwrap();
    let skill_file = skill_dir.join("SKILL.md");
    let content = format!("---\n{frontmatter_content}\n---\n\n{body}\n");
    fs::write(&skill_file, content).unwrap();
    skill_file
}

#[test]
fn test_skill_frontmatter_four_quadrant_parsing() {
    let temp = TempDir::new("skill_parsing_4q");
    let root = temp.path();

    create_skill(
        root,
        "q1-default",
        "description: Q1 default skill",
        "# Q1 Body",
    );
    create_skill(
        root,
        "q2-human-only",
        "description: Q2 human only\nuser-invocable: true\ndisable-model-invocation: true",
        "# Q2 Body",
    );
    create_skill(
        root,
        "q3-model-only",
        "description: Q3 model only\nuser-invocable: false\ndisable-model-invocation: false",
        "# Q3 Body",
    );
    create_skill(
        root,
        "q4-disabled",
        "description: Q4 disabled\nuser-invocable: false\ndisable-model-invocation: true",
        "# Q4 Body",
    );

    let hotloader = WorkspaceHotloader::new();
    let (skills, _) = hotloader.scan_skills(root);

    let q1 = skills.iter().find(|s| s.name == "q1-default").expect("q1 must exist");
    assert!(q1.user_invocable, "Q1 user_invocable default true");
    assert!(!q1.disable_model_invocation, "Q1 disable_model_invocation default false");
    assert!(q1.model_invocable, "Q1 model_invocable default true");

    let q2 = skills.iter().find(|s| s.name == "q2-human-only").expect("q2 must exist");
    assert!(q2.user_invocable, "Q2 user_invocable true");
    assert!(q2.disable_model_invocation, "Q2 disable_model_invocation true");
    assert!(!q2.model_invocable, "Q2 model_invocable false");

    let q3 = skills.iter().find(|s| s.name == "q3-model-only").expect("q3 must exist");
    assert!(!q3.user_invocable, "Q3 user_invocable false");
    assert!(!q3.disable_model_invocation, "Q3 disable_model_invocation false");
    assert!(q3.model_invocable, "Q3 model_invocable true");

    let q4 = skills.iter().find(|s| s.name == "q4-disabled").expect("q4 must exist");
    assert!(!q4.user_invocable, "Q4 user_invocable false");
    assert!(q4.disable_model_invocation, "Q4 disable_model_invocation true");
    assert!(!q4.model_invocable, "Q4 model_invocable false");
}

#[test]
fn test_prompt_prefix_filters_model_invocable_and_emits_omission_marker() {
    let temp = TempDir::new("skill_prefix_4q");
    let root = temp.path();

    create_skill(
        root,
        "q1-visible",
        "description: Visible skill",
        "# Q1 Body",
    );
    create_skill(
        root,
        "q2-hidden",
        "description: Hidden skill\ndisable-model-invocation: true",
        "# Q2 Body",
    );

    let hotloader = WorkspaceHotloader::new();
    let report = hotloader.scan_skills_report_with_globals(root, &[]);
    let budget = SkillCatalogBudget::unlimited();

    let prefix = hotloader.build_cached_prompt_prefix_with_omissions(
        None,
        &report.skills,
        budget,
        &report.omitted,
    );

    assert!(
        prefix.contains("- `q1-visible`"),
        "q1-visible must be present in prompt prefix; actual prefix:\n{prefix}"
    );
    assert!(
        !prefix.contains("- `q2-hidden`"),
        "q2-hidden must NOT be in AVAILABLE REUSABLE SKILLS list; actual prefix:\n{prefix}"
    );
    assert!(
        prefix.contains("omitted skill:q2-hidden (model_invocation_disabled)"),
        "q2-hidden must have model_invocation_disabled omission marker; actual prefix:\n{prefix}"
    );
}

#[test]
fn test_skill_load_blocks_when_disable_model_invocation() {
    let temp = TempDir::new("skill_load_block_4q");
    let root = temp.path();

    create_skill(
        root,
        "q1-loadable",
        "description: Visible skill",
        "# Q1 Content",
    );
    create_skill(
        root,
        "q2-blocked",
        "description: Blocked skill\ndisable-model-invocation: true",
        "# Q2 Content",
    );

    let router = ToolRouter::with_workspace_root(root.to_path_buf());
    let ctx = ToolExecutionContext {
        workspace_root: Some(root.to_path_buf()),
        workspace_id: None,
        session_id: Some("test-session".to_string()),
        dispatch_context: None,
        cancellation: None,
        provider_selection: None,
    };

    // Q1: skill_load should succeed
    let call_q1 = ToolCall {
        call_id: Some("call-1".to_string()),
        name: "skill_load".to_string(),
        arguments: serde_json::json!({ "name": "q1-loadable" }),
        plan: None,
    };
    let res_q1 = router.execute_with_context(&call_q1, &ctx);
    assert_eq!(res_q1.status, "ok", "q1-loadable should succeed; got: {:?}", res_q1);
    assert!(res_q1.output.contains("=== SKILL q1-loadable ==="));

    // Q2: skill_load must be blocked with skill_not_available_for_model_invocation
    let call_q2 = ToolCall {
        call_id: Some("call-2".to_string()),
        name: "skill_load".to_string(),
        arguments: serde_json::json!({ "name": "q2-blocked" }),
        plan: None,
    };
    let res_q2 = router.execute_with_context(&call_q2, &ctx);
    assert_eq!(
        res_q2.status, "error",
        "q2-blocked must fail with error status; got: {:?}", res_q2
    );
    assert!(
        res_q2.output.contains("skill_not_available_for_model_invocation"),
        "Output must contain error code skill_not_available_for_model_invocation; got: {}", res_q2.output
    );

    // But physical load for human (resolve_skill_by_name + load_skill_body_for_workspace) must still succeed!
    let skill_q2 = resolve_skill_by_name(root, "q2-blocked").expect("q2-blocked resolves physically");
    let body_q2 = load_skill_body_for_workspace(&skill_q2, 10240, Some(root))
        .expect("q2-blocked physical load succeeds for human");
    assert!(body_q2.content.contains("# Q2 Content"));
}

#[test]
fn test_skill_frontmatter_boolean_is_case_and_alias_insensitive() {
    // 缺陷背景（L3 评审可绕过项）：`bool::from_str` 只认小写，`disable-model-invocation: True`
    // 会被 `unwrap_or(false)` 静默吃掉 ⇒ 策略闸门 fail-open。对齐 dsh `frontmatterBoolean`：
    // 大小写不敏感 + 接受 yes/on/1 与 no/off/0。
    let temp = TempDir::new("skill_parsing_bool_alias");
    let root = temp.path();

    create_skill(
        root,
        "q5-upper-true",
        "description: Q5 uppercase true\nuser-invocable: True\ndisable-model-invocation: TRUE",
        "# Q5 Body",
    );
    create_skill(
        root,
        "q6-alias-off",
        "description: Q6 alias off\nuser-invocable: off\ndisable-model-invocation: yes",
        "# Q6 Body",
    );
    create_skill(
        root,
        "q7-unparsable",
        "description: Q7 unparsable\nuser-invocable: maybe\ndisable-model-invocation: maybe",
        "# Q7 Body",
    );

    let hotloader = WorkspaceHotloader::new();
    let (skills, _) = hotloader.scan_skills(root);

    let q5 = skills.iter().find(|s| s.name == "q5-upper-true").expect("q5 must exist");
    assert!(q5.user_invocable, "`True` 必须解析为 true，不得回退默认值");
    assert!(q5.disable_model_invocation, "`TRUE` 必须解析为 true");
    assert!(!q5.model_invocable, "大写 TRUE 不得绕过模型调用闸门（fail-open 回归）");

    let q6 = skills.iter().find(|s| s.name == "q6-alias-off").expect("q6 must exist");
    assert!(!q6.user_invocable, "`off` 必须解析为 false");
    assert!(q6.disable_model_invocation, "`yes` 必须解析为 true");
    assert!(!q6.model_invocable, "`yes` 不得绕过模型调用闸门");

    let q7 = skills.iter().find(|s| s.name == "q7-unparsable").expect("q7 must exist");
    assert!(q7.user_invocable, "非法值回退 user-invocable 默认 true");
    assert!(!q7.disable_model_invocation, "非法值回退 disable-model-invocation 默认 false");
}

#[test]
fn test_catalog_budget_description_500_chars_and_100_skills_fidelity() {
    let temp = TempDir::new("catalog_budget_100");
    let root = temp.path();

    let long_desc = "a".repeat(800);
    create_skill(
        root,
        "long-desc-skill",
        &format!("description: {long_desc}"),
        "# Body",
    );

    let hotloader = WorkspaceHotloader::new();
    let report = hotloader.scan_skills_report_with_globals(root, &[]);
    let budget = SkillCatalogBudget::unlimited();
    let prefix = hotloader.build_cached_prompt_prefix_with_omissions(
        None,
        &report.skills,
        budget,
        &report.omitted,
    );

    // 目录描述必须被截断到 <= 500 字符
    let line = prefix
        .lines()
        .find(|l| l.contains("long-desc-skill"))
        .expect("long-desc-skill line must exist");
    let desc_part = line.split("]: ").nth(1).expect("desc part must exist");
    assert!(
        desc_part.len() <= 500,
        "Manifest description must be truncated to <= 500 chars; actual len: {}",
        desc_part.len()
    );

    // 构造 100 个技能，验证在 16KiB 预算下，被裁剪描述的技能仍保留名称（消除「可载而不可知」）
    for i in 0..100 {
        let name = format!("skill-{i:03}");
        let desc = format!("Description for {name} with some padding text {}", "x".repeat(300));
        create_skill(root, &name, &format!("description: {desc}"), "# Body");
    }

    let report_100 = hotloader.scan_skills_report_with_globals(root, &[]);
    // 隔离纪律：扫描会合并宿主 `settings.json.skillDirs` 与 `PONY_AGENT_SKILL_DIRS`，
    // 因此**不得**断言扫描总数（本机配置了自定义技能根的机器上会假红）；只断言本夹具的 100 个。
    let fixture_names: Vec<String> = (0..100).map(|i| format!("skill-{i:03}")).collect();
    for name in &fixture_names {
        assert!(
            report_100.skills.iter().any(|skill| &skill.name == name),
            "fixture skill `{name}` must be scanned"
        );
    }

    let constrained_budget = SkillCatalogBudget::new(16384, 64);
    let prefix_100 = hotloader.build_cached_prompt_prefix_with_omissions(
        None,
        &report_100.skills,
        constrained_budget,
        &report_100.omitted,
    );

    // 16KiB 内必须保留全部技能名称（要么全行，要么最短清单）
    for name in &fixture_names {
        assert!(
            prefix_100.contains(&format!("- `{name}`")),
            "Skill `{name}` must be discoverable in catalog even under 16KiB budget constraint"
        );
        // 「最短清单渲染」不是「丢弃」：已渲染行不得同时出现在预算丢弃诊断行里，
        // 否则同一技能既可见又被报告为 omitted（目录自相矛盾，误导模型）。
        assert!(
            !prefix_100.contains(&format!("omitted skill:{name} (budget)")),
            "Skill `{name}` 已渲染为目录行，不得同时被标记为预算丢弃"
        );
    }

    // 诊断行必须与「真正被丢弃」的集合一致：`omitted skills: N more` 的计数口径
    // 只能来自本次条数（不在本夹具内），故只断言计数行存在时其数字可解析且非零。
    if let Some(counter) = prefix_100
        .lines()
        .find(|line| line.starts_with("omitted skills: "))
    {
        let count_part = counter
            .trim_start_matches("omitted skills: ")
            .trim_end_matches(" more (catalog budget)")
            .trim();
        let count: usize = count_part
            .parse()
            .unwrap_or_else(|_| panic!("omitted 计数行格式不可解析：{counter}"));
        assert!(count > 0, "零丢弃不应输出计数行：{counter}");
    }
}

#[test]
fn test_custom_skill_dirs_merged_order_dedup_and_empty_filtering() {
    let store_dirs = vec![
        PathBuf::from("/store/skills-alpha"),
        PathBuf::from(""), // 空项被过滤
        PathBuf::from("/store/skills-beta"),
        PathBuf::from("/store/skills-alpha"), // 重复项被去重
    ];

    #[cfg(unix)]
    let env_raw = std::ffi::OsString::from("/store/skills-beta:/env/skills-gamma:/env/skills-delta");
    #[cfg(windows)]
    let env_raw = std::ffi::OsString::from("/store/skills-beta;/env/skills-gamma;/env/skills-delta");

    let merged = resolve_custom_skill_dirs_merged(&store_dirs, Some(&env_raw));

    assert_eq!(
        merged,
        vec![
            PathBuf::from("/store/skills-alpha"),
            PathBuf::from("/store/skills-beta"),
            PathBuf::from("/env/skills-gamma"),
            PathBuf::from("/env/skills-delta"),
        ],
        "Merged skill dirs must prioritize store_dirs, append env_dirs, skip empty items, and dedup by first appearance"
    );
}

#[test]
fn test_session_hot_reload_delta_notice_and_dynamic_skill_load() {
    let temp = TempDir::new("session_hot_reload");
    let root = temp.path();

    // 初始状态：只有 initial-skill
    create_skill(
        root,
        "initial-skill",
        "description: initial skill description",
        "# Initial Skill Body",
    );

    let hotloader = WorkspaceHotloader::new();

    // 首轮（turn 1）：固化会话基线
    let (prefix_t1, delta_t1) = hotloader.resolve_session_prompt_context_with_globals(
        "session-hotreload-test-1",
        1,
        root,
        &[],
    );
    assert!(prefix_t1.contains("initial-skill"));
    assert!(delta_t1.is_none(), "首轮基线固化，不应产生 delta_notice");

    // 会话进行中（mid-session）：动态写入 dynamic-skill
    create_skill(
        root,
        "dynamic-skill",
        "description: dynamically added skill mid-session",
        "# Dynamic Skill Body Content",
    );

    // 第二轮（turn 2）：应当检测到技能变更，生成 delta_notice 提示
    let (prefix_t2, delta_t2) = hotloader.resolve_session_prompt_context_with_globals(
        "session-hotreload-test-1",
        2,
        root,
        &[],
    );

    // KV-Cache 头保护：基础前缀保持原首轮基线不变
    assert_eq!(prefix_t2, prefix_t1, "KV-Cache 保护：首轮前缀保持不变");

    // 变更通过 delta_notice 通道通知模型
    let notice = delta_t2.expect("第二轮必须产出 delta_notice");
    assert!(
        notice.contains("Newly installed skills"),
        "delta_notice 必须报告新安装技能；实际为:\n{notice}"
    );
    assert!(
        notice.contains("dynamic-skill"),
        "delta_notice 必须包含 dynamic-skill；实际为:\n{notice}"
    );

    // 模型收到 notice 后直接调用 skill_load：必须成功加载正文，不可报 404 / unknown_skill
    let router = ToolRouter::with_workspace_root(root.to_path_buf());
    let ctx = ToolExecutionContext {
        workspace_root: Some(root.to_path_buf()),
        workspace_id: None,
        session_id: Some("session-hotreload-test-1".to_string()),
        dispatch_context: None,
        cancellation: None,
        provider_selection: None,
    };
    let call = ToolCall {
        call_id: Some("call-dynamic-load".to_string()),
        name: "skill_load".to_string(),
        arguments: serde_json::json!({ "name": "dynamic-skill" }),
        plan: None,
    };
    let res = router.execute_with_context(&call, &ctx);
    assert_eq!(
        res.status, "ok",
        "skill_load 必须成功载入会话内动态添加的技能；输出：{:?}",
        res
    );
    assert!(
        res.output.contains("# Dynamic Skill Body Content"),
        "skill_load 返回内容必须包含技能正文；实际输出：{}",
        res.output
    );

    // 第三轮（turn 3）：首轮前缀依然保持（KV-Cache 保护），且 notice 持续有效通知模型差异
    let (prefix_t3, delta_t3) = hotloader.resolve_session_prompt_context_with_globals(
        "session-hotreload-test-1",
        3,
        root,
        &[],
    );
    assert_eq!(prefix_t3, prefix_t1, "第三轮前缀依然必须与首轮严格相等");
    assert!(
        delta_t3.is_some(),
        "在技能与首轮基线存在差异期间，delta_notice 持续提供增量上下文"
    );

    // 移除 dynamic-skill，使工作区技能恢复为与首轮基线一致
    let dynamic_skill_dir = root.join(".agents").join("skills").join("dynamic-skill");
    fs::remove_dir_all(&dynamic_skill_dir).unwrap();

    // 第四轮（turn 4）：恢复与首轮基线一致后，delta_notice 消除为 None
    let (_prefix_t4, delta_t4) = hotloader.resolve_session_prompt_context_with_globals(
        "session-hotreload-test-1",
        4,
        root,
        &[],
    );
    assert!(
        delta_t4.is_none(),
        "恢复与首轮基线一致后，delta_notice 恢复为 None"
    );
}
