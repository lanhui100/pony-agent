//! `/models` 面板上下文窗口编辑器（`w`）的验收测试。
//!
//! 覆盖三条用户可见承诺（ADR 2026-11-14-model-declared-window-tui-entry）：
//! 1. **逐模型声明**：改一个模型的窗口不会动到别的模型，也不会把「未声明」固化成默认值；
//! 2. **来源可见**：面板徽标区分「声明」与「预设补缺」，编辑器回显当前生效值与来源；
//! 3. **失败不静默**：解析错误 / 未命中模型留在面板里显示，不关面板、不落盘。
//!
//! 全部用**真实** `ProviderRegistryStore`（临时目录）+ 真实渲染驱动，不 mock 成功。

use super::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::Terminal;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn screen(state: &mut TuiState, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
    terminal.draw(|f| ui(f, state)).expect("draw");
    terminal.backend().buffer().clone()
}

/// 与 `tui_six_fixes_acceptance` 同口径的取字：宽字符右半格是占位空格，必须跳过。
fn row_text(buf: &ratatui::buffer::Buffer, y: u16) -> String {
    let mut out = String::new();
    let mut skip = 0usize;
    for x in 0..buf.area.width {
        if skip > 0 {
            skip -= 1;
            continue;
        }
        let symbol = buf[(x, y)].symbol();
        let width = str_width(symbol).max(1);
        if width > 1 {
            skip = width - 1;
        }
        out.push_str(symbol);
    }
    out
}

fn all_text(buf: &ratatui::buffer::Buffer) -> String {
    (0..buf.area.height)
        .map(|y| row_text(buf, y))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 夹具：一个 provider、两个模型（`model-declared` 显式 200000；`model-undeclared` 没有
/// `capabilities` 段 ⇒ 未声明 ⇒ 生效值来自预设补缺）。返回 (注册表路径, provider id)。
fn fixture(tag: &str) -> (std::path::PathBuf, String) {
    let root = std::env::temp_dir().join(format!(
        "pony-tui-window-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&root).expect("create temp root");
    let path = root.join("providers.json");
    let raw = serde_json::json!({
        "providers": [{
            "id": "provider-demo",
            "name": "Demo",
            "protocol": "openai-completions",
            "base_url": "https://example.test/v1",
            "api_key_env_var": "DEMO_API_KEY",
            "models": [
                {
                    "id": "model-declared",
                    "name": "Declared 200K",
                    "model": "declared-200k",
                    "temperature": 0.2,
                    "maxOutputTokens": 64000,
                    "capabilityPreset": "open-ai-chat",
                    "capabilities": { "contextWindowTokens": 200000 }
                },
                {
                    "id": "model-undeclared",
                    "name": "Undeclared",
                    "model": "undeclared-1m",
                    "temperature": 0.2,
                    "maxOutputTokens": 64000,
                    "capabilityPreset": "open-ai-chat"
                }
            ],
            "selectedModelId": "model-declared"
        }],
        "selectedProviderId": "provider-demo"
    });
    std::fs::write(&path, serde_json::to_string_pretty(&raw).unwrap()).expect("write fixture");
    (path, "provider-demo".to_string())
}

/// 打开 `/models` 面板并把高亮锚到指定模型。
fn open_panel(state: &mut TuiState, provider_id: &str, model_id: &str) {
    state.current_provider_id = provider_id.to_string();
    let store = state.registry_store();
    let items = model_items_for_provider(&store, provider_id);
    state.open_model_selector(items);
    state.selected_model_index =
        model_panel_index_of(&state, model_id).expect("模型必须在面板索引空间内");
}

fn registry_json(state: &TuiState) -> serde_json::Value {
    let path = state.provider_registry_path.clone().expect("temp registry");
    serde_json::from_str(&std::fs::read_to_string(path).expect("read registry"))
        .expect("parse registry")
}

#[test]
fn editor_mode_is_modal_so_enter_cannot_leak_into_composer() {
    assert!(
        queue::is_modal_mode(&InteractionMode::EditingModelWindow),
        "窗口编辑器必须被判为模态：否则排队消息回灌的 Enter 会被当成「保存窗口」"
    );
}

#[test]
fn open_prefills_declared_value_and_reports_source() {
    let (path, provider_id) = fixture("prefill");
    let mut state = TuiState::default();
    state.provider_registry_path = Some(path);
    open_panel(&mut state, &provider_id, "model-declared");

    state.open_model_window_editor().expect("open editor");

    assert_eq!(state.mode, InteractionMode::EditingModelWindow);
    let edit = state.model_window_edit.as_ref().expect("edit state");
    assert_eq!(edit.buffer, "200000", "已声明模型应预填声明值");
    assert_eq!(edit.declared, Some(200_000));
    assert_eq!(edit.effective, Some(200_000));
    assert_eq!(edit.effective_label(), "200K（声明）");
}

#[test]
fn open_leaves_buffer_blank_when_window_inherited_from_preset() {
    let (path, provider_id) = fixture("blank");
    let mut state = TuiState::default();
    state.provider_registry_path = Some(path);
    open_panel(&mut state, &provider_id, "model-undeclared");

    state.open_model_window_editor().expect("open editor");

    let edit = state.model_window_edit.as_ref().expect("edit state");
    assert!(
        edit.buffer.is_empty(),
        "未声明窗口不能预填默认值，否则回车就把「跟随预设」变成了「声明 1000000」"
    );
    assert_eq!(edit.declared, None);
    assert_eq!(
        edit.effective,
        Some(pony_agent_core::agent::config::DEFAULT_CONTEXT_WINDOW_TOKENS)
    );
    assert!(edit.effective_label().ends_with("（预设）"));
}

#[test]
fn commit_writes_only_the_edited_model_and_keeps_siblings_intact() {
    let (path, provider_id) = fixture("commit");
    let mut state = TuiState::default();
    state.provider_registry_path = Some(path);
    state.model = "model-undeclared".to_string();
    open_panel(&mut state, &provider_id, "model-undeclared");
    state.open_model_window_editor().expect("open editor");

    for ch in "200K".chars() {
        state.handle_model_window_editor_key(&key(KeyCode::Char(ch)));
    }
    assert!(state.handle_model_window_editor_key(&key(KeyCode::Enter)));

    // 编辑器关闭并回到模型面板（不是裸 Normal：面板仍开着）。
    assert_eq!(state.mode, InteractionMode::SelectingModel);
    assert!(state.model_window_edit.is_none());

    let raw = registry_json(&state);
    assert_eq!(
        raw["providers"][0]["models"][1]["capabilities"]["contextWindowTokens"],
        serde_json::json!(200_000),
        "200K 必须按十进制口径落成 200000"
    );
    assert_eq!(
        raw["providers"][0]["models"][0]["capabilities"]["contextWindowTokens"],
        serde_json::json!(200_000),
        "另一个模型的声明值不得被顺带改写"
    );
    // 在用的模型 ⇒ 右上角上下文分母就地更新，不必等下一轮重读注册表。
    assert_eq!(state.context_window_tokens, 200_000);
    // 面板行也立刻反映新值。
    let row = state
        .model_list
        .iter()
        .find(|item| item.id == "model-undeclared")
        .expect("panel row");
    assert_eq!(row.context_window_tokens, Some(200_000));
    assert_eq!(row.declared_context_window, Some(200_000));
}

#[test]
fn clearing_declaration_restores_preset_inheritance_on_disk() {
    let (path, provider_id) = fixture("clear");
    let mut state = TuiState::default();
    state.provider_registry_path = Some(path);
    open_panel(&mut state, &provider_id, "model-declared");
    state.open_model_window_editor().expect("open editor");

    // Delete 清空缓冲 = 恢复跟随预设。
    state.handle_model_window_editor_key(&key(KeyCode::Delete));
    assert!(state.handle_model_window_editor_key(&key(KeyCode::Enter)));

    let raw = registry_json(&state);
    assert!(
        raw["providers"][0]["models"][0]["capabilities"]
            .get("contextWindowTokens")
            .is_none(),
        "清除声明必须删键，不能写 0 或写预设默认值"
    );
    let row = state
        .model_list
        .iter()
        .find(|item| item.id == "model-declared")
        .expect("panel row");
    assert_eq!(row.declared_context_window, None);
    assert_eq!(
        row.context_window_tokens,
        Some(pony_agent_core::agent::config::DEFAULT_CONTEXT_WINDOW_TOKENS)
    );
}

#[test]
fn out_of_range_input_keeps_editor_open_with_error_and_writes_nothing() {
    let (path, provider_id) = fixture("invalid");
    let mut state = TuiState::default();
    state.provider_registry_path = Some(path.clone());
    open_panel(&mut state, &provider_id, "model-declared");
    state.open_model_window_editor().expect("open editor");
    let before = std::fs::read_to_string(&path).expect("read before");

    state.handle_model_window_editor_key(&key(KeyCode::Delete));
    // 9900 万 token：超过上限，必须拒绝而不是写进去。
    for ch in "99M".chars() {
        state.handle_model_window_editor_key(&key(KeyCode::Char(ch)));
    }
    state.handle_model_window_editor_key(&key(KeyCode::Enter));

    assert_eq!(
        state.mode,
        InteractionMode::EditingModelWindow,
        "失败必须留在编辑器里，不能关面板假装成功"
    );
    let edit = state.model_window_edit.as_ref().expect("edit state");
    assert!(
        edit.error.as_deref().unwrap_or_default().contains("过大"),
        "错误原因必须落在面板上：{:?}",
        edit.error
    );
    assert_eq!(std::fs::read_to_string(&path).expect("read after"), before);
}

#[test]
fn esc_discards_edits_without_touching_disk() {
    let (path, provider_id) = fixture("cancel");
    let mut state = TuiState::default();
    state.provider_registry_path = Some(path.clone());
    open_panel(&mut state, &provider_id, "model-declared");
    state.open_model_window_editor().expect("open editor");
    let before = std::fs::read_to_string(&path).expect("read before");

    state.handle_model_window_editor_key(&key(KeyCode::Delete));
    state.handle_model_window_editor_key(&key(KeyCode::Char('1')));
    state.handle_model_window_editor_key(&key(KeyCode::Esc));

    assert_eq!(state.mode, InteractionMode::SelectingModel);
    assert!(state.model_window_edit.is_none());
    assert_eq!(std::fs::read_to_string(&path).expect("read after"), before);
}

#[test]
fn typing_rejects_characters_that_cannot_be_part_of_a_window() {
    let (path, provider_id) = fixture("chars");
    let mut state = TuiState::default();
    state.provider_registry_path = Some(path);
    open_panel(&mut state, &provider_id, "model-undeclared");
    state.open_model_window_editor().expect("open editor");

    for ch in ['2', 'x', '中', '0', '/'] {
        state.handle_model_window_editor_key(&key(KeyCode::Char(ch)));
    }
    assert_eq!(
        state.model_window_edit.as_ref().expect("edit").buffer,
        "20",
        "只有窗口表达式允许的字符能进缓冲"
    );
}

#[test]
fn model_panel_shows_window_and_its_source() {
    let (path, provider_id) = fixture("render-panel");
    let mut state = TuiState::default();
    state.provider_registry_path = Some(path);
    open_panel(&mut state, &provider_id, "model-declared");

    let text = all_text(&screen(&mut state, 100, 30));
    assert!(text.contains("Declared 200K"), "{text}");
    // 声明值裸写；预设补缺加后缀——两者绝不能在界面上长得一样。
    assert!(text.contains("200K"), "已声明窗口应显示声明值：{text}");
    assert!(
        text.contains("1M·预设"),
        "预设补缺窗口必须标注来源：{text}"
    );
    assert!(text.contains("w 能力"), "面板底栏必须给出入口提示：{text}");
}

#[test]
fn editor_panel_render_makes_the_choice_explicit() {
    let (path, provider_id) = fixture("render-editor");
    let mut state = TuiState::default();
    state.provider_registry_path = Some(path);
    open_panel(&mut state, &provider_id, "model-undeclared");
    state.open_model_window_editor().expect("open editor");

    let text = all_text(&screen(&mut state, 100, 30));
    assert!(text.contains("模型能力"), "{text}");
    assert!(text.contains("上下文窗口"), "{text}");
    assert!(text.contains("Undeclared"), "{text}");
    // 空缓冲时必须直接写出「后果」：跟随预设以及它当前解析成什么。
    assert!(text.contains("跟随预设（1M）"), "{text}");
    // 常用规格与多模态开关必须同屏可见（可快选，也可手输）；快选档位下限 256K。
    for preset in ["256K", "512K", "1M"] {
        assert!(text.contains(preset), "缺少规格 {preset}：{text}");
    }
    assert!(text.contains("多模态输入"), "{text}");
    assert!(text.contains("图片"), "{text}");
    assert!(text.contains("↵ 保存"), "{text}");
}

#[test]
fn right_arrow_fills_common_preset_and_commit_writes_it() {
    let (path, provider_id) = fixture("preset");
    let mut state = TuiState::default();
    state.provider_registry_path = Some(path);
    open_panel(&mut state, &provider_id, "model-undeclared");
    state.open_model_window_editor().expect("open editor");

    // 未声明 ⇒ 跟随预设 1M（档位表末项）；→ 从末项回绕到首项 256K（下限档位）。
    state.handle_model_window_editor_key(&key(KeyCode::Right));
    assert_eq!(
        state.model_window_edit.as_ref().expect("edit").buffer,
        "256K",
        "常用规格必须能一键填入"
    );
    assert!(state.handle_model_window_editor_key(&key(KeyCode::Enter)));

    let raw = registry_json(&state);
    assert_eq!(
        raw["providers"][0]["models"][1]["capabilities"]["contextWindowTokens"],
        serde_json::json!(256_000),
        "256K 必须按十进制口径落成 256000"
    );
}

#[test]
fn modality_toggles_switch_to_custom_and_persist() {
    let (path, provider_id) = fixture("inputs");
    let mut state = TuiState::default();
    state.provider_registry_path = Some(path);
    open_panel(&mut state, &provider_id, "model-undeclared");
    state.open_model_window_editor().expect("open editor");

    // tab → 模态焦点；默认高亮「图片」，空格关掉；→ 到「视频」，空格打开。
    state.handle_model_window_editor_key(&key(KeyCode::Tab));
    state.handle_model_window_editor_key(&key(KeyCode::Char(' ')));
    state.handle_model_window_editor_key(&key(KeyCode::Right));
    state.handle_model_window_editor_key(&key(KeyCode::Char(' ')));

    let edit = state.model_window_edit.as_ref().expect("edit");
    assert!(!edit.inputs.image, "图片应被关掉");
    assert!(edit.inputs.video, "视频应被打开");
    assert!(!edit.inputs.audio);

    assert!(state.handle_model_window_editor_key(&key(KeyCode::Enter)));

    let raw = registry_json(&state);
    let model = &raw["providers"][0]["models"][1];
    // 预设分支会忽略声明布尔值 ⇒ 必须切 custom 才能让勾选生效。
    assert_eq!(model["capabilityPreset"], serde_json::json!("custom"));
    let capabilities = &model["capabilities"];
    assert_eq!(capabilities["supportsImageInput"], serde_json::json!(false));
    assert_eq!(capabilities["supportsVideoInput"], serde_json::json!(true));
    assert_eq!(capabilities["supportsAudioInput"], serde_json::json!(false));
    // 未动窗口 ⇒ 不写窗口键（继续跟随 preset/custom 的默认 1M）。
    assert!(
        capabilities.get("contextWindowTokens").is_none(),
        "只改模态不得顺带声明窗口：{capabilities:?}"
    );
}
