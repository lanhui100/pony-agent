// wave-38 L2-T / L2-AT：TUI 启动目录即默认工作区 —— 冻结验收契约（Test Agent 所有，Executor 只读）。
//
// 契约来源：.dev-team/contract-matrix-tui-workspace.md §2（E1–E7 / AT1–AT3）。
// 红相目标：`ensure_workspace_for_root`（R1.1）与 `build_turn_input`（R1.4）当前均为
// `unimplemented!` 空桩，因此本文件在红相阶段必须「可编译但运行失败（EXIT≠0）」。
//
// 本文件把 `src/bin/tui.rs` 以 bin-as-module 方式 include（与 tui_acceptance.rs 同模式），
// 以便对 `build_turn_input` / `TuiState::default()` 做纯内存断言；bin 的 main 不会被调用，
// 仅为消除 bin-as-module 固有的 dead_code 噪声而加最小 allow（不放宽任何断言）。
#[path = "../src/bin/tui.rs"]
#[allow(dead_code, unused_imports, unused_variables, unused_mut)]
mod tui;

use pony_agent_core::agent::workspace::{
    ensure_workspace_for_root, normalize_workspace_root, roots_match, validate_workspace_id,
    WorkspaceRecord, MAX_DISPLAY_NAME_CHARS,
};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// 临时目录夹具（谁创建谁清理）：std::env::temp_dir + uuid 唯一子目录。
// 不引入新依赖；Drop 统一回收，防止污染后续运行。
// ---------------------------------------------------------------------------
struct TempRoot {
    path: PathBuf,
}

impl TempRoot {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!("wave38-{}-{}", tag, uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).expect("创建临时根目录失败");
        Self { path }
    }

    /// 在临时根下创建（必要时递归创建）子目录并返回其绝对路径。
    fn dir(&self, rel: &str) -> PathBuf {
        let path = self.path.join(rel);
        std::fs::create_dir_all(&path).expect("创建临时子目录失败");
        path
    }

    /// 在临时根下写一个文件并返回其绝对路径。
    fn file(&self, rel: &str, content: &str) -> PathBuf {
        let path = self.path.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("创建临时文件父目录失败");
        }
        std::fs::write(&path, content).expect("写入临时文件失败");
        path
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        // 只读夹具可能残留 0o555 目录，先尽力恢复可写再删除。
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o755));
        }
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn pstr(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

fn record(id: &str, name: &str, root: &str) -> WorkspaceRecord {
    WorkspaceRecord {
        id: id.to_string(),
        name: name.to_string(),
        root_path: root.to_string(),
    }
}

// ---------------------------------------------------------------------------
// E1：新 root 登记——root 规范化、id 合法、records 长度 +1。
// ---------------------------------------------------------------------------
#[test]
fn e1_ensure_new_root_registers_normalized_record() {
    let tmp = TempRoot::new("e1");
    let root = tmp.dir("project");
    let root_str = pstr(&root);

    let mut records: Vec<WorkspaceRecord> = Vec::new();
    let created = ensure_workspace_for_root(&mut records, &root_str)
        .expect("E1：合法目录 root 应登记成功，而非 Err");

    let expected_root = normalize_workspace_root(&root_str).expect("E1：夹具 root 必须可规范化");
    assert_eq!(
        created.root_path, expected_root,
        "E1：返回记录的 root_path 必须等于 normalize_workspace_root(root)"
    );
    assert!(
        validate_workspace_id(&created.id).is_ok(),
        "E1：返回 id 必须合法（ws- 前缀或 default 规则），实际 = {:?}",
        created.id
    );
    assert_eq!(records.len(), 1, "E1：新 root 登记后注册表长度必须 +1");
    assert_eq!(records[0], created, "E1：注册表内记录必须与返回记录一致");
    assert!(
        !created.name.trim().is_empty(),
        "E1：显示名不得为空（取自 canonical root 的 file_name）"
    );
    assert!(
        !created.name.contains('/') && !created.name.contains('\\'),
        "E1：显示名必须是 file_name 而非整条路径，实际 = {:?}",
        created.name
    );
    assert!(
        created.name.chars().count() <= MAX_DISPLAY_NAME_CHARS,
        "E1：显示名必须 ≤{} 字符，实际 = {:?}",
        MAX_DISPLAY_NAME_CHARS,
        created.name
    );
}

// ---------------------------------------------------------------------------
// E2：幂等——同 root 二次调用返回同 id，records 长度不变。
// ---------------------------------------------------------------------------
#[test]
fn e2_ensure_is_idempotent_for_same_root() {
    let tmp = TempRoot::new("e2");
    let root = tmp.dir("project");
    let root_str = pstr(&root);

    let mut records: Vec<WorkspaceRecord> = Vec::new();
    let first = ensure_workspace_for_root(&mut records, &root_str).expect("E2：首次登记应成功");
    let len_after_first = records.len();

    let second = ensure_workspace_for_root(&mut records, &root_str).expect("E2：二次登记应成功（幂等）");

    assert_eq!(first.id, second.id, "E2：同 root 二次调用必须返回同一 id");
    assert_eq!(
        records.len(),
        len_after_first,
        "E2：幂等路径不得新增注册表记录"
    );
    assert_eq!(records.len(), 1, "E2：注册表应只有一条记录");
    assert_eq!(
        roots_match(&first.root_path, &second.root_path),
        true,
        "E2：两次返回的 root 必须视为同一 root"
    );
}

// ---------------------------------------------------------------------------
// E3：既有同 root 记录（名称不同）→ 原样返回，不新增、不改名。
// ---------------------------------------------------------------------------
#[test]
fn e3_ensure_returns_existing_record_untouched() {
    let tmp = TempRoot::new("e3");
    let root = tmp.dir("project");
    let root_str = pstr(&root);
    let canonical = normalize_workspace_root(&root_str).expect("E3：夹具 root 必须可规范化");

    let mut records = vec![record("ws-existing-1", "既有工作区名", &canonical)];
    let snapshot = records.clone();

    let returned = ensure_workspace_for_root(&mut records, &root_str).expect("E3：命中既有 root 应成功");

    assert_eq!(records, snapshot, "E3：既有同 root 记录必须原样保留（不新增、不改名、不迁移）");
    assert_eq!(returned.id, "ws-existing-1", "E3：必须返回既有记录 id");
    assert_eq!(returned.name, "既有工作区名", "E3：必须返回既有记录名称（不得改名为 file_name）");
    assert_eq!(returned.root_path, canonical, "E3：root 必须保持不变");
}

// ---------------------------------------------------------------------------
// E4：同名不同 root——第一个占名，第二个确定性去重为 `name (2)`（或 ` (3)`）。
// ---------------------------------------------------------------------------
#[test]
fn e4_duplicate_basename_gets_deterministic_suffix() {
    let tmp = TempRoot::new("e4");
    let root_a = tmp.dir("a/shared-proj");
    let root_b = tmp.dir("b/shared-proj");

    let mut records: Vec<WorkspaceRecord> = Vec::new();
    let first = ensure_workspace_for_root(&mut records, &pstr(&root_a)).expect("E4：第一个 root 应登记成功");
    let second = ensure_workspace_for_root(&mut records, &pstr(&root_b)).expect("E4：第二个同名 root 应登记成功");

    assert_eq!(first.name, "shared-proj", "E4：第一个同名 root 应原样占用名称");
    assert_ne!(first.id, second.id, "E4：不同 root 必须分配不同 id");
    assert_ne!(
        second.name, first.name,
        "E4：同名不同 root 必须去重，实际第二个名称 = {:?}",
        second.name
    );
    assert!(
        second.name == "shared-proj (2)" || second.name == "shared-proj (3)",
        "E4：去重名称必须是 `shared-proj (2)`（后接 (3)），实际 = {:?}",
        second.name
    );
    assert_eq!(records.len(), 2, "E4：两个不同 root 必须各占一条记录");
    for rec in &records {
        assert!(
            rec.name.chars().count() <= MAX_DISPLAY_NAME_CHARS,
            "E4：所有记录名必须 ≤{} 字符，实际 = {:?}",
            MAX_DISPLAY_NAME_CHARS,
            rec.name
        );
    }
    let mut names: Vec<&str> = records.iter().map(|r| r.name.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), records.len(), "E4：注册表内不得出现重复名称");
    let mut ids: Vec<&str> = records.iter().map(|r| r.id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), records.len(), "E4：注册表内不得出现重复 id");
}

// ---------------------------------------------------------------------------
// E5：非法 root（空串 / 不存在 / 不是目录）→ Err，且 records 不变。
// ---------------------------------------------------------------------------
#[test]
fn e5_invalid_root_returns_err_without_mutating_records() {
    let tmp = TempRoot::new("e5");
    let not_a_dir = tmp.file("plain-file.txt", "not a dir");
    let missing = tmp.path.join("does-not-exist");

    let invalid_inputs = vec![
        String::new(),
        "   ".to_string(),
        pstr(&not_a_dir),
        pstr(&missing),
    ];

    for input in invalid_inputs {
        let mut records = vec![record("ws-keep-1", "保持不动", "/tmp/keep-untouched")];
        let snapshot = records.clone();

        let result = ensure_workspace_for_root(&mut records, &input);

        assert!(
            result.is_err(),
            "E5：非法 root {input:?} 必须返回 Err，实际 = {result:?}"
        );
        assert_eq!(records, snapshot, "E5：非法 root 失败后 records 必须保持不变（input={input:?}）");
    }
}

// ---------------------------------------------------------------------------
// E6：build_turn_input —— workspace_id 透传 + 其余字段与现发送点一致。
// ---------------------------------------------------------------------------
#[test]
fn e6_build_turn_input_propagates_active_workspace_id() {
    let mut state = tui::TuiState::default();
    state.active_workspace_id = Some("ws-demo-1".to_string());

    let input = tui::build_turn_input(
        &state,
        "读取 package.json 并总结".to_string(),
        "fakep".to_string(),
        "mock-alpha".to_string(),
    );

    assert_eq!(
        input.workspace_id.as_deref(),
        Some("ws-demo-1"),
        "E6：active_workspace_id 必须透传为 TurnInput.workspace_id"
    );
    assert_eq!(input.message, "读取 package.json 并总结", "E6：message 必须透传");
    assert_eq!(input.provider_id.as_deref(), Some("fakep"), "E6：provider_id 必须透传");
    assert_eq!(input.model_id.as_deref(), Some("mock-alpha"), "E6：model_id 必须透传");
    assert_eq!(
        input.session_id.as_deref(),
        Some(state.session_id.as_str()),
        "E6：session_id 必须取 state.session_id"
    );
    assert_eq!(
        input.reasoning_effort, state.reasoning_effort,
        "E6：reasoning_effort 必须取 state.reasoning_effort"
    );
    assert!(input.display_message.is_none(), "E6：display_message 必须保持 None");
    assert!(input.workspace_mode.is_none(), "E6：workspace_mode 必须保持 None");
    assert!(input.node_id.is_none(), "E6：node_id 必须保持 None");
    assert!(input.history.is_empty(), "E6：history 必须为空");
    assert!(input.images.is_empty(), "E6：images 必须为空");
    assert!(
        input.plan_execution_approval.is_none(),
        "E6：plan_execution_approval 必须保持 None"
    );

    // 未登记（None）时不得凭空产生 workspace_id，保持既有默认工作区行为。
    let mut plain = tui::TuiState::default();
    plain.active_workspace_id = None;
    let input_none = tui::build_turn_input(&plain, "hi".to_string(), "fakep".to_string(), "mock-alpha".to_string());
    assert!(
        input_none.workspace_id.is_none(),
        "E6：active_workspace_id=None 时 workspace_id 必须为 None"
    );
}

// ---------------------------------------------------------------------------
// E7：回归——TuiState::default().workspace == canonicalize(current_dir)。
// ---------------------------------------------------------------------------
#[test]
fn e7_default_tui_state_workspace_is_canonical_cwd() {
    let state = tui::TuiState::default();
    let cwd = std::env::current_dir().expect("E7：current_dir 必须可读");
    let expected = std::fs::canonicalize(&cwd).unwrap_or(cwd);
    assert_eq!(
        state.workspace, expected,
        "E7：TuiState::default().workspace 必须等于 canonicalize(current_dir)"
    );
    assert!(
        state.active_workspace_id.is_none(),
        "E7：Default 的 active_workspace_id 必须为 None（由 main 在接线后填充）"
    );
}

// ---------------------------------------------------------------------------
// AT1（对抗）：只读/不可写 HOME —— 不得 panic，行为必须确定。
// 说明：ensure_workspace_for_root 为纯内存函数（不写盘），build_turn_input 为纯函数；
// 本用例把 HOME/XDG 指向不可写目录，验证二者仍返回确定结果、绝不 panic。
// ---------------------------------------------------------------------------
#[test]
fn at1_readonly_home_never_panics() {
    let tmp = TempRoot::new("at1");
    let home = tmp.dir("home");
    let xdg_config = tmp.dir("xdg-config");
    let xdg_data = tmp.dir("xdg-data");
    let xdg_cache = tmp.dir("xdg-cache");
    let root = tmp.dir("project-readable");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o555))
            .expect("AT1：设置只读 HOME 失败");
    }

    // 记录真实可写性（root 身份可能绕过权限；此时显式标注，不静默放行）。
    let probe = home.join("write-probe");
    let home_writable = std::fs::write(&probe, b"probe").is_ok();
    let _ = std::fs::remove_file(&probe);

    let old_home = std::env::var_os("HOME");
    let old_cfg = std::env::var_os("XDG_CONFIG_HOME");
    let old_data = std::env::var_os("XDG_DATA_HOME");
    let old_cache = std::env::var_os("XDG_CACHE_HOME");
    std::env::set_var("HOME", &home);
    std::env::set_var("XDG_CONFIG_HOME", &xdg_config);
    std::env::set_var("XDG_DATA_HOME", &xdg_data);
    std::env::set_var("XDG_CACHE_HOME", &xdg_cache);

    let mut records: Vec<WorkspaceRecord> = Vec::new();
    let root_str = pstr(&root);

    let valid = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ensure_workspace_for_root(&mut records, &root_str)
    }));
    let invalid = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut scratch: Vec<WorkspaceRecord> = Vec::new();
        ensure_workspace_for_root(&mut scratch, "/wave38/definitely/not/here")
    }));
    let turn_input = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut state = tui::TuiState::default();
        state.active_workspace_id = Some("ws-readonly-home".to_string());
        let first = tui::build_turn_input(&state, "m".to_string(), "p".to_string(), "mid".to_string());
        let second = tui::build_turn_input(&state, "m".to_string(), "p".to_string(), "mid".to_string());
        (first.workspace_id, second.workspace_id)
    }));

    // 立刻恢复环境，避免影响同进程其它用例。
    match old_home {
        Some(value) => std::env::set_var("HOME", value),
        None => std::env::remove_var("HOME"),
    }
    match old_cfg {
        Some(value) => std::env::set_var("XDG_CONFIG_HOME", value),
        None => std::env::remove_var("XDG_CONFIG_HOME"),
    }
    match old_data {
        Some(value) => std::env::set_var("XDG_DATA_HOME", value),
        None => std::env::remove_var("XDG_DATA_HOME"),
    }
    match old_cache {
        Some(value) => std::env::set_var("XDG_CACHE_HOME", value),
        None => std::env::remove_var("XDG_CACHE_HOME"),
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o755));
    }

    assert!(
        valid.is_ok(),
        "AT1：只读 HOME 下 ensure_workspace_for_root 不得 panic（home_writable={home_writable}）"
    );
    let valid_record = valid
        .expect("AT1：unwind payload 已在上一条断言给出")
        .expect("AT1：合法 root 在只读 HOME 下仍应 Ok（纯函数不写盘）");
    assert_eq!(
        valid_record.root_path,
        normalize_workspace_root(&root_str).expect("AT1：夹具 root 必须可规范化"),
        "AT1：只读 HOME 不得改变 root 规范化结果"
    );

    assert!(
        invalid.is_ok(),
        "AT1：只读 HOME 下非法 root 的 Err 路径不得 panic（home_writable={home_writable}）"
    );
    assert!(
        invalid.expect("AT1：unwind payload 已在上一条断言给出").is_err(),
        "AT1：非法 root 必须返回 Err 而非 Ok"
    );

    assert!(
        turn_input.is_ok(),
        "AT1：只读 HOME 下 build_turn_input 不得 panic（home_writable={home_writable}）"
    );
    let (first_ws, second_ws) = turn_input.expect("AT1：unwind payload 已在上一条断言给出");
    assert_eq!(
        first_ws.as_deref(),
        Some("ws-readonly-home"),
        "AT1：只读 HOME 下 build_turn_input 仍必须透传 workspace_id"
    );
    assert_eq!(first_ws, second_ws, "AT1：build_turn_input 必须确定性（重复调用同结果）");
}

// ---------------------------------------------------------------------------
// AT2（对抗）：同名目录对抗——两个不同 root 同名，顺序注册必须确定性去重。
// 正序与逆序分别跑一遍，两次结果形状必须一致（顺序决定名称归属，但去重规则不变）。
// ---------------------------------------------------------------------------
#[test]
fn at2_same_basename_dedup_is_order_deterministic() {
    let tmp = TempRoot::new("at2");
    let root_a = tmp.dir("first/proj");
    let root_b = tmp.dir("second/proj");

    let mut forward: Vec<WorkspaceRecord> = Vec::new();
    ensure_workspace_for_root(&mut forward, &pstr(&root_a)).expect("AT2：正序第一个 root 应成功");
    ensure_workspace_for_root(&mut forward, &pstr(&root_b)).expect("AT2：正序第二个 root 应成功");

    let mut reverse: Vec<WorkspaceRecord> = Vec::new();
    ensure_workspace_for_root(&mut reverse, &pstr(&root_b)).expect("AT2：逆序第一个 root 应成功");
    ensure_workspace_for_root(&mut reverse, &pstr(&root_a)).expect("AT2：逆序第二个 root 应成功");

    let names_forward: Vec<String> = forward.iter().map(|r| r.name.clone()).collect();
    let names_reverse: Vec<String> = reverse.iter().map(|r| r.name.clone()).collect();
    assert_eq!(
        names_forward, names_reverse,
        "AT2：同名去重结果必须确定性（顺序无关的形状），正序={names_forward:?} 逆序={names_reverse:?}"
    );
    assert_eq!(names_forward[0], "proj", "AT2：先登记者占用原名");
    assert_eq!(names_forward[1], "proj (2)", "AT2：后登记者必须去重为 `proj (2)`");

    for records in [&forward, &reverse] {
        let mut ids: Vec<&str> = records.iter().map(|r| r.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), records.len(), "AT2：不同 root 的 id 不得重复");
        let mut names: Vec<&str> = records.iter().map(|r| r.name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), records.len(), "AT2：注册表内名称不得重复");
    }
}

// ---------------------------------------------------------------------------
// AT3（对抗）：超长 / 怪异目录名（≥100 char）——登记后名称 ≤64 char 且 root 正确。
// ---------------------------------------------------------------------------
#[test]
fn at3_overlong_directory_name_is_truncated_but_root_is_exact() {
    let tmp = TempRoot::new("at3");
    // 混合 ASCII + 多字节 + 空白 + 标点，长度远超 64（按 char 计）。
    let long_name = format!("{}-中文-目录名-with spaces_and_symbols-{}", "x".repeat(80), "界".repeat(30));
    assert!(
        long_name.chars().count() >= 100,
        "AT3：夹具目录名必须 ≥100 char，实际 = {}",
        long_name.chars().count()
    );
    let long_dir = tmp.dir(&long_name);
    let long_dir_str = pstr(&long_dir);

    let mut records: Vec<WorkspaceRecord> = Vec::new();
    let record = ensure_workspace_for_root(&mut records, &long_dir_str).expect("AT3：超长目录名应登记成功");

    assert!(
        record.name.chars().count() <= MAX_DISPLAY_NAME_CHARS,
        "AT3：名称必须按 char 截断到 ≤{}，实际 = {}（{:?}）",
        MAX_DISPLAY_NAME_CHARS,
        record.name.chars().count(),
        record.name
    );
    assert!(!record.name.is_empty(), "AT3：截断后名称不得为空");
    assert!(
        long_name.starts_with(record.name.as_str()),
        "AT3：截断必须保留原名前缀，实际 = {:?}",
        record.name
    );
    assert_eq!(
        record.root_path,
        normalize_workspace_root(&long_dir_str).expect("AT3：夹具 root 必须可规范化"),
        "AT3：root 必须保持完整（不得随名称截断）"
    );
    assert!(validate_workspace_id(&record.id).is_ok(), "AT3：id 必须合法");
    assert_eq!(records.len(), 1, "AT3：登记后应只有一条记录");
}
