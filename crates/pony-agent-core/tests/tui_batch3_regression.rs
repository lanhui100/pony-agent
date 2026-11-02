//! 批3（task-9）验收测试：provider 生效与持久化 / 会话作用域复位 / 覆盖层 mode 栈 /
//! Ctrl+C 语义 / 非流式状态提示。
//!
//! 契约真值源：`.dev-team/contract-matrix-slash-commands.md` 修订8（a）~（e）（Lead 批准·批3 修复）。
//! 沿用 bin-as-module 模式（`#[path = "../src/bin/tui.rs"]`）：`src/bin/tui.rs` 是 `[[bin]]` 目标，
//! 只能引入**同一份真实源码**才能在编译期断言其行为。
//!
//! ## 隔离（禁止触碰真实用户数据）
//! N11 用**临时目录**内的 `providers.json` + 注入式 `FileSecretStore`（同一临时目录）构造
//! `ProviderRegistryStore`，因此既不读写 `~/.pony-agent`，也不触达系统钥匙串；用例结束由
//! `TempDir` 守卫递归删除。
//!
//! ## 可观测性边界（如实声明，不写假断言）
//! `TuiState::turn_usage` 是**私有字段**（bin-as-module 下仍受模块隐私约束），集成测试无法设置
//! 也无法直接断言其复位。故 N12 逐项断言契约列出的**pub 字段**，并对 `turn_usage` 不做断言——
//! 其复位口径由实现侧单测/代码评审覆盖，本文件不伪造该断言（也不写「只运行不报错」的空用例）。
//!
//! ## TUI 选择路径的持久化（已升级为端到端断言）
//! exec-rust 提供了可注入入口 `TuiState::provider_registry_path: Option<PathBuf>`（`registry_store()`
//! 据此构造 store）与 `TuiState::select_current_provider()`，故本文件**不需要**退化：
//! - `n11`  断言 store 层往返（`save_view` → 重开 `load_view`，并校验落盘文件内容含该 id）；
//! - `n11b` 端到端断言 TUI 选择路径——把 `provider_registry_path` 指向临时目录 → 调
//!   `select_current_provider()` → 断言 `current_provider_id` 立刻更新**且**重开 store 后
//!   `selected_provider_id` 已落盘，直接锁死「界面报已切换、发送仍用旧 provider」的假成功。
//!
//! 若后续有人移除 `provider_registry_path` 注入点，n11b 会编译失败——这正是期望的护栏信号。

#[path = "../src/bin/tui.rs"]
#[allow(dead_code, unused_imports, unused_variables, unused_mut)]
mod tui;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use pony_agent_core::agent::config::ProviderRegistryStore;
use pony_agent_core::agent::secret_store::FileSecretStore;

/// 进程内唯一临时目录守卫：用例结束即递归删除（谁创建谁清理）。
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "pony-agent-it-batch3-{label}-{}-{stamp}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("create isolated temp dir");
        Self { path }
    }

    fn join(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

// ── N10：pick_active_provider_id 真值表（修订8（a）） ──────────────────────────────
//
// 优先级：state 命中注册表 → registry 选中命中 → 首个 → None。
// 这条纯函数存在的意义就是消灭「界面报已切换、发送仍用旧 provider」。

#[test]
fn n10a_explicit_state_selection_wins_when_registered() {
    // ① state 命中 ⇒ 用 state（即便 registry 选中是另一个合法 id）。
    assert_eq!(
        tui::pick_active_provider_id(
            Some("provider-a"),
            Some("provider-b"),
            &["provider-a", "provider-b"]
        ),
        Some("provider-a".to_string()),
        "N10a: state 命中注册表时必须优先于 registry 选中"
    );

    // ② state 未命中（脏 id）而 registry 命中 ⇒ 回落 registry 选中。
    assert_eq!(
        tui::pick_active_provider_id(Some("ghost"), Some("provider-b"), &["provider-a", "provider-b"]),
        Some("provider-b".to_string()),
        "N10a: state 未命中时应回落 registry 选中"
    );
}

#[test]
fn n10b_falls_back_to_first_registered_provider_then_none() {
    // ③ state / registry 都是 None ⇒ 首个注册 provider。
    assert_eq!(
        tui::pick_active_provider_id(None, None, &["provider-a", "provider-b"]),
        Some("provider-a".to_string()),
        "N10b: 无任何选择时必须回落到首个 provider"
    );

    // ④ 注册表为空 ⇒ None（不得 panic、不得返回空串）。
    assert_eq!(
        tui::pick_active_provider_id(Some("provider-a"), Some("provider-b"), &[]),
        None,
        "N10b: 注册表为空时必须返回 None"
    );

    // ⑤ 两者都未命中 ⇒ 首个。
    assert_eq!(
        tui::pick_active_provider_id(Some("ghost"), Some("phantom"), &["provider-a", "provider-b"]),
        Some("provider-a".to_string()),
        "N10b: 两个候选都不在注册表时必须回落到首个"
    );

    // ⑥ registry 选中非法（不在注册表）而 state 为 None ⇒ 首个。
    assert_eq!(
        tui::pick_active_provider_id(None, Some("phantom"), &["provider-a", "provider-b"]),
        Some("provider-a".to_string()),
        "N10b: registry 选中非法时必须回落到首个"
    );

    // 单元素注册表的边界：两个候选都非法也必须回落到该唯一 provider。
    assert_eq!(
        tui::pick_active_provider_id(Some("ghost"), Some("phantom"), &["only-one"]),
        Some("only-one".to_string()),
        "N10b: 单元素注册表也必须回落成功"
    );
}

// ── N11：provider 选择持久化（修订8（a）真实落盘） ─────────────────────────────────

#[test]
fn n11_selected_provider_id_survives_reopen() {
    let dir = TempDir::new("provider-registry");
    let registry_path = dir.join("providers.json");
    let secret_path = dir.join("secrets.json");

    let store = ProviderRegistryStore::with_path_and_secret_store(
        &registry_path,
        Arc::new(FileSecretStore::new(secret_path.clone())),
    );
    let view = store.load_view();

    // 前置断言：默认注册表必须有 provider（否则后面的「选中落盘」没有意义）。
    assert!(
        !view.providers.is_empty(),
        "N11 前置条件失败：默认注册表应至少含一个 provider"
    );
    let default_selected = view.selected_provider_id.clone();
    // 选一个与当前选中不同的 id，确保「写进去」是可观测的变化（否则断言可能恒真）。
    let target = view
        .providers
        .iter()
        .map(|provider| provider.id.clone())
        .find(|id| Some(id) != default_selected.as_ref())
        .expect("默认注册表应至少含两个 provider（否则本用例无法区分 '写入' 与 '原样'）");

    let mut next_view = view.clone();
    next_view.selected_provider_id = Some(target.clone());
    let saved = store.save_view(next_view).expect("save_view 必须成功");
    assert_eq!(
        saved.selected_provider_id.as_deref(),
        Some(target.as_str()),
        "N11: save_view 的返回值必须携带新的选中 provider"
    );

    // 物理落盘断言：文件里必须出现该 id。
    let raw = std::fs::read_to_string(&registry_path).expect("providers.json 必须已落盘");
    assert!(
        raw.contains(&target),
        "N11: 落盘文件内容必须包含所选 provider id（{target}）"
    );

    // 重新打开一个 store 实例（模拟下次启动）⇒ 选中项必须仍是刚才所选。
    let reopened = ProviderRegistryStore::with_path_and_secret_store(
        &registry_path,
        Arc::new(FileSecretStore::new(secret_path)),
    );
    let persisted = reopened.load_view();
    assert_eq!(
        persisted.selected_provider_id.as_deref(),
        Some(target.as_str()),
        "N11: 重新加载后 selected_provider_id 必须仍为所选（持久化生效）"
    );
}

/// N11b（端到端）：TUI 的 `/provider` 选择路径**必须**真的落库 —— 锁死「界面报已切换、
/// 发送仍用旧 provider」这类假成功。`TuiState::provider_registry_path` 是可注入的 store 路径，
/// 因此本条不碰真实 `~/.pony-agent`：整个流程只读写临时目录。
#[test]
fn n11b_tui_provider_selection_path_persists_to_registry() {
    let dir = TempDir::new("tui-provider-select");
    let registry_path = dir.join("providers.json");

    // 用 store 层面的真实默认注册表准备候选（不伪造 provider 数据）。
    let seed_view = ProviderRegistryStore::with_path(&registry_path).load_view();
    assert!(
        seed_view.providers.len() >= 2,
        "N11b 前置条件失败：默认注册表应至少含两个 provider"
    );
    let target_index = seed_view
        .providers
        .iter()
        .position(|provider| Some(&provider.id) != seed_view.selected_provider_id.as_ref())
        .expect("必须存在一个与当前选中不同的 provider，才能证明选择真的生效");
    let target_id = seed_view.providers[target_index].id.clone();

    let mut state = tui::TuiState::default();
    state.provider_registry_path = Some(registry_path.clone());
    state.provider_list = seed_view
        .providers
        .iter()
        .map(|provider| tui::ProviderItem {
            id: provider.id.clone(),
            name: provider.name.clone(),
            protocol: format!("{:?}", provider.protocol),
            base_url: provider.base_url.clone(),
        })
        .collect();
    state.selected_provider_index = target_index;
    // 旧值故意置为一个不存在的 id：若选择路径是「假成功」，它会残留在 current_provider_id。
    state.current_provider_id = "stale-provider-before-switch".to_string();

    state.select_current_provider();

    assert_eq!(
        state.current_provider_id, target_id,
        "N11b: 选择后 current_provider_id 必须立刻更新为目标 provider"
    );

    // 真正落盘（下次启动 / 发送路径经注册表解析时才能看到）：重开 store 断言。
    let persisted = ProviderRegistryStore::with_path(&registry_path).load_view();
    assert_eq!(
        persisted.selected_provider_id.as_deref(),
        Some(target_id.as_str()),
        "N11b: 选择路径必须把 selected_provider_id 落库（否则就是『界面切换、实际仍用旧 provider』）"
    );
}

// ── N12：会话作用域复位（修订8（b）） ──────────────────────────────────────────────

/// 把 state 弄脏：契约列出的每个 pub 字段都置为非默认值。
fn dirty_state() -> tui::TuiState {
    let mut state = tui::TuiState::default();
    state.session_id = "session-keep-me".to_string();

    state.add_user_message("上一条用户消息");
    state.status_text = "Ready 之外的脏状态".to_string();
    state.is_streaming = true;
    state.turn_started_at = Some(std::time::Instant::now());
    state.runtime_mode = tui::RuntimeMode::Thinking;
    state.total_input_tokens = 123;
    state.total_output_tokens = 45;
    state.total_cache_hit_tokens = 6;
    state.last_context_tokens = 789;
    state.context_window_tokens = 128_000;
    state.plan_state = Some(tui::PlanState::new("plan-1".to_string(), "脏计划".to_string()));
    state.task_board_state = Some(tui::TaskBoardState::new(Vec::new()));
    state.goal_state = Some(tui::TuiGoal::new(
        "goal-1".to_string(),
        "脏目标".to_string(),
        Some(8),
    ));
    state.goal_banner_since = Some(std::time::Instant::now());
    state.pending_request = Some(tui::PendingUiRequest {
        request_id: "req-1".to_string(),
        version: 1,
        run_id: None,
        is_approval: true,
    });
    state.scroll_offset = 9;
    state.max_scroll_offset = 77;
    state.auto_scroll = false;
    state.selected_command_index = 3;
    state.selected_session_index = 2;
    state.selected_provider_index = 1;
    state.selected_model_index = 4;
    state.selected_reasoning_effort_index = 2;
    state.spinner_tick = 11;
    state
}

#[test]
fn n12a_reset_session_scope_clears_every_contract_field() {
    let mut state = dirty_state();

    state.reset_session_scope();

    assert!(state.messages.is_empty(), "N12a: messages 必须清空");
    assert_eq!(state.status_text, "Ready", "N12a: status_text 必须回到 Ready");
    assert!(!state.is_streaming, "N12a: is_streaming 必须复位为 false");
    assert!(state.turn_started_at.is_none(), "N12a: turn_started_at 必须为 None");
    assert!(
        matches!(state.runtime_mode, tui::RuntimeMode::Ready),
        "N12a: runtime_mode 必须回到 Ready"
    );
    assert_eq!(state.total_input_tokens, 0, "N12a: total_input_tokens 必须归零");
    assert_eq!(state.total_output_tokens, 0, "N12a: total_output_tokens 必须归零");
    assert_eq!(
        state.total_cache_hit_tokens, 0,
        "N12a: total_cache_hit_tokens 必须归零"
    );
    assert_eq!(
        state.last_context_tokens, 0,
        "N12a: last_context_tokens 必须归零"
    );
    assert_eq!(
        state.context_window_tokens, 0,
        "N12a: context_window_tokens 必须归零（0 = 未知）"
    );
    assert!(state.plan_state.is_none(), "N12a: plan_state 必须清空");
    assert!(state.task_board_state.is_none(), "N12a: task_board_state 必须清空");
    assert!(state.goal_state.is_none(), "N12a: goal_state 必须清空");
    assert!(
        state.goal_banner_since.is_none(),
        "N12a: goal_banner_since 必须清空（否则新会话横幅时间片错位）"
    );
    assert!(state.pending_request.is_none(), "N12a: pending_request 必须清空");
    assert_eq!(state.scroll_offset, 0, "N12a: scroll_offset 必须归零");
    assert_eq!(state.max_scroll_offset, 0, "N12a: max_scroll_offset 必须归零");
    assert!(state.auto_scroll, "N12a: auto_scroll 必须复位为贴底（true）");
    assert_eq!(
        state.selected_command_index, 0,
        "N12a: selected_command_index 必须归零"
    );
    assert_eq!(
        state.selected_session_index, 0,
        "N12a: selected_session_index 必须归零"
    );
    assert_eq!(
        state.selected_provider_index, 0,
        "N12a: selected_provider_index 必须归零"
    );
    assert_eq!(
        state.selected_model_index, 0,
        "N12a: selected_model_index 必须归零"
    );
    assert_eq!(
        state.selected_reasoning_effort_index, 0,
        "N12a: selected_reasoning_effort_index 必须归零"
    );
    assert_eq!(state.spinner_tick, 0, "N12a: spinner_tick 必须归零");

    // 契约明示：session_id **不**由本函数更换（/clear 自行换、/resume 复位后再装快照）。
    assert_eq!(
        state.session_id, "session-keep-me",
        "N12a: reset_session_scope 不得改写 session_id"
    );
}

#[test]
fn n12b_reset_session_scope_is_idempotent_on_clean_state() {
    let mut state = tui::TuiState::default();
    state.reset_session_scope();

    // 干净 state 上再复位一次：所有复位后不变量必须保持（不得把默认值改成非默认）。
    assert!(state.messages.is_empty(), "N12b: 复位后仍应无消息");
    assert!(!state.is_streaming, "N12b: 复位后仍应非流式");
    assert!(state.auto_scroll, "N12b: 复位后仍应贴底");
    assert!(state.plan_state.is_none(), "N12b: 复位后仍无计划");
    assert!(state.goal_state.is_none(), "N12b: 复位后仍无目标");
    assert_eq!(state.scroll_offset, 0, "N12b: 复位后滚动偏移仍为 0");
}

// ── N13：覆盖层 mode 栈（修订8（c）） ──────────────────────────────────────────────

#[test]
fn n13a_overlay_returns_to_previous_mode_on_end() {
    let mut state = tui::TuiState::default();
    assert!(
        matches!(state.mode, tui::InteractionMode::Normal),
        "N13a 前置条件：默认 mode 应为 Normal"
    );

    state.begin_overlay(tui::InteractionMode::SelectingModel);
    assert!(
        matches!(state.mode, tui::InteractionMode::SelectingModel),
        "N13a: begin_overlay 必须立刻把 mode 切到覆盖层"
    );

    state.end_overlay();
    assert!(
        matches!(state.mode, tui::InteractionMode::Normal),
        "N13a: end_overlay 必须按栈回退到进入前的 mode（Normal），不得留在覆盖层"
    );

    // 栈已空时再 end_overlay 不得 panic（幂等兜底）。
    state.end_overlay();
    assert!(
        matches!(state.mode, tui::InteractionMode::Normal),
        "N13a: 空栈 end_overlay 必须是无害 no-op"
    );
}

#[test]
fn n13b_returning_to_adding_provider_clears_secret_from_composer() {
    let mut state = tui::TuiState::default();
    state.mode = tui::InteractionMode::AddingProvider;
    // 模拟用户已在 composer 键入疑似密钥内容（真实缺陷：切回 AddingProvider 后它会被当消息发出）。
    state.set_input("sk-test-secret-should-not-leak");
    assert!(
        !state.input_buffer.is_empty(),
        "N13b 前置条件：composer 必须先有内容，否则『清空』断言恒真"
    );

    state.begin_overlay(tui::InteractionMode::PermissionApproval);
    assert!(
        matches!(state.mode, tui::InteractionMode::PermissionApproval),
        "N13b: 覆盖层必须生效"
    );

    state.end_overlay();
    assert!(
        matches!(state.mode, tui::InteractionMode::AddingProvider),
        "N13b: end_overlay 必须回到 AddingProvider"
    );
    assert!(
        state.input_buffer.is_empty(),
        "N13b: 回到 AddingProvider 时必须清空 input_buffer（api_key 不得落入 composer 被当消息发出），实际 = {:?}",
        state.input_buffer
    );
}

#[test]
fn n13c_nested_overlays_unwind_in_lifo_order() {
    let mut state = tui::TuiState::default();

    state.begin_overlay(tui::InteractionMode::SelectingModel);
    state.begin_overlay(tui::InteractionMode::PermissionApproval);
    state.begin_overlay(tui::InteractionMode::AskUserQuestion);
    assert!(
        matches!(state.mode, tui::InteractionMode::AskUserQuestion),
        "N13c: 最后一层覆盖层必须生效"
    );

    state.end_overlay();
    assert!(
        matches!(state.mode, tui::InteractionMode::PermissionApproval),
        "N13c: 第一层回退应回到 PermissionApproval（后进先出）"
    );

    state.end_overlay();
    assert!(
        matches!(state.mode, tui::InteractionMode::SelectingModel),
        "N13c: 第二层回退应回到 SelectingModel（后进先出）"
    );

    state.end_overlay();
    assert!(
        matches!(state.mode, tui::InteractionMode::Normal),
        "N13c: 第三层回退应回到最初的 Normal"
    );
}

// ── N14：Ctrl+C 语义（修订8（d）） ────────────────────────────────────────────────

#[test]
fn n14_streaming_ctrl_c_interrupts_turn_and_never_exits() {
    // 流式中无条件中断在跑的那一轮——**不得**退出进程（否则用户误触 Ctrl+C 就丢会话）。
    assert!(
        matches!(
            tui::ctrl_c_action(true, false),
            tui::CtrlCAction::InterruptTurn
        ),
        "N14: 流式中无草稿时 Ctrl+C 必须中断当前轮"
    );
    assert!(
        matches!(
            tui::ctrl_c_action(true, true),
            tui::CtrlCAction::InterruptTurn
        ),
        "N14: 流式中即使有草稿也必须中断当前轮（不得降级为清草稿）"
    );
}

#[test]
fn n14_non_streaming_ctrl_c_clears_draft_else_exits() {
    assert!(
        matches!(
            tui::ctrl_c_action(false, true),
            tui::CtrlCAction::ClearDraft
        ),
        "N14: 非流式且有草稿时 Ctrl+C 必须先清草稿（不得直接退出丢输入）"
    );
    assert!(
        matches!(tui::ctrl_c_action(false, false), tui::CtrlCAction::Exit),
        "N14: 非流式且无草稿时 Ctrl+C 才退出"
    );
}

// ── N15：非流式状态提示渲染判定（修订8（e）） ───────────────────────────────────────

#[test]
fn n15_streaming_never_renders_status_notice() {
    // streaming 期间提示由流式区承载，避免双份文案打架。
    assert!(
        !tui::should_render_status_notice(true, 0, 4_000),
        "N15: 流式中（age=0）不得渲染 status notice"
    );
    assert!(
        !tui::should_render_status_notice(true, 1, 4_000),
        "N15: 流式中（新鲜）不得渲染 status notice"
    );
    assert!(
        !tui::should_render_status_notice(true, 10_000, 4_000),
        "N15: 流式中即使已超 TTL 也不得渲染 status notice"
    );
}

#[test]
fn n15_non_streaming_notice_respects_ttl_boundary() {
    assert!(
        tui::should_render_status_notice(false, 0, 4_000),
        "N15: 非流式且刚设置（age=0）必须渲染"
    );
    assert!(
        tui::should_render_status_notice(false, 3_999, 4_000),
        "N15: 非流式且 age < ttl 必须渲染"
    );
    // 边界：age == ttl 即过期（TTL 语义为「最多可见 ttl」）。
    assert!(
        !tui::should_render_status_notice(false, 4_000, 4_000),
        "N15: age == ttl 时必须已过期（边界归入过期侧）"
    );
    assert!(
        !tui::should_render_status_notice(false, 4_001, 4_000),
        "N15: age > ttl 必须过期"
    );
}
