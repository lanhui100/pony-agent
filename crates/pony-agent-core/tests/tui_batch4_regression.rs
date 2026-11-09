//! 批4（task-11）验收测试：滚动偏移 u32 化 / ModelsDiscovered 生产函数 / 审计双源。
//!
//! 契约真值源：`.dev-team/contract-matrix-slash-commands.md` 修订9（a）(c）(d)（Lead 批准·批4）。
//! 沿用 bin-as-module 模式（`#[path = "../src/bin/tui.rs"]`）：`src/bin/tui.rs` 是 `[[bin]]` 目标，
//! 只能引入**同一份真实源码**才能在编译期断言其行为。
//!
//! 用例映射：
//! - **N16**（F-H）滚动偏移由 u16 → u32：大偏移（100_000 行）不得被截断/回绕；渲染侧用
//!   `clamp_render_scroll(u32) -> u16` 做**饱和** cast 喂给 ratatui（`as u16` 会把 100_000
//!   回绕成 34464 → 画面直接跳顶，这是本用例要钉死的缺陷形态）。
//! - **N17**（F-J）`apply_models_discovered` 生产映射函数：写入 / 未知 provider 返回 Err 且
//!   view 不被改动 / 重复调用幂等。
//! - **N18** 在 `tests/tui_acceptance.rs` 就地治理（原「测试内重写映射逻辑」改为调用生产函数）。
//! - **N19**（F-K）`tools/tui-audit/checks.py` 的 `no_panic` 双源三态判定，独立于本文件，
//!   待 exec-rust 落地可测入口后按实际口径补（见任务回报；不写猜测式断言）。
//!
//! 铁律：真实断言、无 `#[ignore]`/skip、不修改实现代码；不触碰真实 `~/.pony-agent`。

#[path = "../src/bin/tui.rs"]
#[allow(dead_code, unused_imports, unused_variables, unused_mut)]
mod tui;

use pony_agent_core::agent::config::{ProviderConfigView, ProviderModelConfig, ProviderRegistryView};
use pony_agent_core::agent::provider::{ProviderAuthType, ProviderProtocol};

// ── N16：大滚动偏移不得截断（F-H） ──────────────────────────────────────────────────

#[test]
fn n16a_large_scroll_offset_is_preserved_in_state() {
    const HUGE: u32 = 100_000; // > u16::MAX(65535)：旧 u16 实现下这个值根本存不下

    let mut state = tui::TuiState::default();
    state.scroll_offset = HUGE;
    assert_eq!(
        state.scroll_offset, HUGE,
        "N16a: scroll_offset 必须能原样保存 > 65535 的偏移（u32 语义），不得截断"
    );

    state.max_scroll_offset = HUGE;
    assert_eq!(
        state.max_scroll_offset, HUGE,
        "N16a: max_scroll_offset 必须能原样保存 > 65535 的偏移"
    );

    // scroll_to_bottom 必须等于 max（旧实现里 `max_offset as u16` 会把 100_000 截断成 34464，
    // 于是「跳到底部」反而跳到接近顶部）。
    state.scroll_offset = 0;
    state.scroll_to_bottom();
    assert_eq!(
        state.scroll_offset, HUGE,
        "N16a: scroll_to_bottom 必须落到真实 max_scroll_offset，不得因截断跳回顶部"
    );
    assert!(state.auto_scroll, "N16a: 跳到底部后必须恢复自动贴底");

    // 大偏移上的逐格上翻必须是精确减法（不得饱和到 0 或小值）。
    state.scroll_up(1);
    assert_eq!(
        state.scroll_offset,
        HUGE - 1,
        "N16a: 大偏移上 scroll_up 必须是精确减法"
    );

    // 小值路径同样要保持原有「滚到底部即贴底」语义（防止 u32 化时改坏边界）。
    state.scroll_offset = 0;
    state.scroll_down(1, 5);
    assert_eq!(state.scroll_offset, 1, "N16a: 未到底部时 scroll_down 正常累加");
    state.scroll_down(10, 5);
    assert_eq!(state.scroll_offset, 5, "N16a: 越过底部时钳到 max_offset");
    assert!(state.auto_scroll, "N16a: 到底后必须恢复自动贴底");
}

#[test]
fn n16b_clamp_render_scroll_saturates_instead_of_wrapping() {
    // 渲染侧只接受 u16：必须**饱和**到 u16::MAX，而不是 `as u16` 回绕。
    assert_eq!(
        tui::clamp_render_scroll(100_000),
        u16::MAX,
        "N16b: 100_000 行必须饱和到 u16::MAX"
    );
    assert_ne!(
        tui::clamp_render_scroll(100_000),
        34_464u16,
        "N16b: 100_000 as u16 会回绕成 34464 —— 那正是「大偏移跳顶」缺陷的形态，必须被禁止"
    );

    // 正常区间原样透传。
    assert_eq!(tui::clamp_render_scroll(1_234), 1_234, "N16b: 正常值必须原样透传");
    assert_eq!(tui::clamp_render_scroll(0), 0, "N16b: 0 必须原样透传");

    // 边界：65535 恰好可表示；65536 起饱和。
    assert_eq!(tui::clamp_render_scroll(65_535), u16::MAX, "N16b: 65535 边界应等于 u16::MAX");
    assert_eq!(tui::clamp_render_scroll(65_536), u16::MAX, "N16b: 65536 起必须饱和");
    assert_eq!(
        tui::clamp_render_scroll(u32::MAX),
        u16::MAX,
        "N16b: u32::MAX 必须饱和而不是回绕"
    );
}

// ── N17：ModelsDiscovered 生产映射函数（F-J） ──────────────────────────────────────

fn empty_provider(id: &str) -> ProviderConfigView {
    ProviderConfigView {
        id: id.to_string(),
        name: id.to_string(),
        protocol: ProviderProtocol::OpenAiCompletions,
        base_url: format!("http://localhost:8000/{id}/v1"),
        auth_type: ProviderAuthType::Auto,
        api_key_env_var: format!("{}_KEY", id.to_uppercase()),
        api_key_value: "sk-test".to_string(),
        api_key_present: true,
        supported_protocols: Vec::new(),
        endpoints: Vec::new(),
        models: Vec::new(),
        selected_model_id: None,
    }
}

fn registry_view(provider_ids: &[&str]) -> ProviderRegistryView {
    ProviderRegistryView {
        // PA-172 §2.2 additive 字段（编译衍生补齐；本夹具不测策略面）。
        upstream_policy: Default::default(),
        failover_provider_ids: Vec::new(),
        providers: provider_ids
            .iter()
            .map(|id| empty_provider(id))
            .collect::<Vec<_>>(),
        selected_provider_id: provider_ids.first().map(|id| (*id).to_string()),
    }
}

/// 结构快照：用于断言「Err 路径 / 幂等路径不得改动 view」。
fn snapshot(view: &ProviderRegistryView) -> Vec<(String, Vec<String>)> {
    view.providers
        .iter()
        .map(|provider| {
            (
                provider.id.clone(),
                provider.models.iter().map(|model| model.id.clone()).collect(),
            )
        })
        .collect()
}

fn models_of<'a>(view: &'a ProviderRegistryView, provider_id: &str) -> &'a [ProviderModelConfig] {
    view.providers
        .iter()
        .find(|provider| provider.id == provider_id)
        .map(|provider| provider.models.as_slice())
        .unwrap_or(&[])
}

#[test]
fn n17a_apply_models_discovered_writes_models_to_target_provider() {
    let mut view = registry_view(&["ptarget", "pother"]);
    let discovered = vec!["m1".to_string(), "m2".to_string()];

    let result = tui::apply_models_discovered(&mut view, "ptarget", &discovered);

    assert!(
        result.is_ok(),
        "N17a: 目标 provider 存在时必须返回 Ok，实际 = {result:?}"
    );
    let ids: Vec<&str> = view
        .providers
        .iter()
        .find(|provider| provider.id == "ptarget")
        .expect("ptarget 必须仍在 view 中")
        .models
        .iter()
        .map(|model| model.id.as_str())
        .collect();
    assert_eq!(ids, vec!["m1", "m2"], "N17a: 模型 id 必须按发现顺序写入目标 provider");

    // 不得牵连其它 provider（映射必须精确定位到 provider_id）。
    assert!(
        models_of(&view, "pother").is_empty(),
        "N17a: 非目标 provider 不得被写入模型"
    );
}

#[test]
fn n17b_apply_models_discovered_rejects_unknown_provider_without_mutation() {
    let mut view = registry_view(&["pother"]);
    let before = snapshot(&view);

    let result = tui::apply_models_discovered(&mut view, "ghost", &["m1".to_string()]);

    let error = result.expect_err("N17b: provider 不存在时必须返回 Err（不得静默丢弃发现结果）");
    assert!(
        !error.trim().is_empty(),
        "N17b: Err 必须携带可诊断信息，实际 = {error:?}"
    );
    assert_eq!(
        snapshot(&view),
        before,
        "N17b: Err 路径不得对 view 做任何改动（半写入会让注册表状态不可预期）"
    );
}

#[test]
fn n17c_apply_models_discovered_is_idempotent() {
    let mut view = registry_view(&["ptarget"]);
    let discovered = vec!["m1".to_string(), "m2".to_string()];

    tui::apply_models_discovered(&mut view, "ptarget", &discovered).expect("首次映射必须成功");
    let after_first = snapshot(&view);
    assert_eq!(
        models_of(&view, "ptarget").len(),
        2,
        "N17c 前置条件：首次映射后应有 2 个模型"
    );

    // 重复下发同一批发现结果（每帧/重连都可能重复）必须幂等：不得追加成 4 个。
    tui::apply_models_discovered(&mut view, "ptarget", &discovered).expect("重复映射必须成功");
    assert_eq!(
        snapshot(&view),
        after_first,
        "N17c: 重复调用必须幂等（不得重复追加模型）"
    );
    assert_eq!(
        models_of(&view, "ptarget").len(),
        2,
        "N17c: 重复调用后模型数仍为 2"
    );
}
