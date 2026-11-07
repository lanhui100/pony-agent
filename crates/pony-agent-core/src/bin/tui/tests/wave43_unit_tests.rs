//! 原 `tui.rs` 内联测试模块 `mod wave43_unit_tests`（Step1 机械外移，断言零改动）。

use super::*;

use serde_json::json;

fn long_question() -> &'static str {
    "请确认是否需要把当前的会话状态、未落地的回答以及图运行绑定一并迁移到新的存储层，并说明迁移期间的回滚策略与验收口径，以及失败后的补偿方式与用户可见提示。"
}

#[test]
fn test_wrap_ask_question_keeps_full_text_and_never_ellipsizes() {
    let wrapped = wrap_ask_question(long_question(), 32, 32);
    assert!(wrapped.len() > 1, "long question must wrap, got {wrapped:?}");
    assert_eq!(wrapped.concat(), long_question(), "换行不得丢字");
    assert!(
        !wrapped.iter().any(|line| line.contains('…')),
        "问题正文禁止省略号截断: {wrapped:?}"
    );
    for line in &wrapped {
        assert!(
            str_width(line) <= 32,
            "line exceeds width 32 ({}): {line}",
            str_width(line)
        );
    }
}

#[test]
fn test_wrap_ask_question_short_stays_single_line() {
    assert_eq!(
        wrap_ask_question("继续吗？", 60, 60),
        vec!["继续吗？".to_string()]
    );
}

#[test]
fn test_wrap_ask_question_keeps_punctuation_off_line_start() {
    let wrapped = wrap_ask_question(
        "请选择本次交付的执行架构与部署策略，然后确认是否继续执行后续步骤。",
        20,
        20,
    );
    assert!(wrapped.len() > 1);
    for line in &wrapped {
        let first = line.chars().next().unwrap_or(' ');
        assert!(!is_no_line_start(first), "行首不得是避让标点: {line}");
    }
}

#[test]
fn test_inline_ask_height_grows_with_wrapped_question() {
    let mut long_state = TuiState::default();
    long_state.mode = InteractionMode::AskUserQuestion;
    long_state.ask_user_state = Some(AskUserState::new(
        "req-long".to_string(),
        vec![AskUserQuestion {
            id: "q1".to_string(),
            header: None,
            question: long_question().to_string(),
            multi_select: false,
            options: vec![
                QuestionOption {
                    label: "是".to_string(),
                    description: None,
                },
                QuestionOption {
                    label: "否".to_string(),
                    description: None,
                },
            ],
        }],
    ));

    let mut short_state = TuiState::default();
    short_state.mode = InteractionMode::AskUserQuestion;
    short_state.ask_user_state = Some(AskUserState::new(
        "req-short".to_string(),
        vec![AskUserQuestion {
            id: "q1".to_string(),
            header: None,
            question: "继续吗？".to_string(),
            multi_select: false,
            options: vec![QuestionOption {
                label: "是".to_string(),
                description: None,
            }],
        }],
    ));

    let long_h = inline_ask_height(&long_state, 80);
    let short_h = inline_ask_height(&short_state, 80);
    assert!(
        long_h > short_h,
        "换行后的问题必须让面板增高: long={long_h} short={short_h}"
    );
    // 窄终端下同样成立（换行更多）
    assert!(inline_ask_height(&long_state, 48) >= long_h);
}

#[test]
fn test_pending_ui_request_from_value() {
    let val_ask = json!({
        "requestId": "ask-101",
        "version": 3,
        "runId": "run-xyz",
        "requestKind": "interaction"
    });
    let handle = pending_ui_request_from_value(&val_ask).expect("must parse handle");
    assert_eq!(handle.request_id, "ask-101");
    assert_eq!(handle.version, 3);
    assert_eq!(handle.run_id, Some("run-xyz".to_string()));
    assert!(!handle.is_approval);

    let val_appr = json!({
        "requestId": "appr-202",
        "version": 1,
        "requestKind": "approval"
    });
    let handle_appr = pending_ui_request_from_value(&val_appr).expect("must parse handle");
    assert_eq!(handle_appr.request_id, "appr-202");
    assert!(handle_appr.is_approval);

    let val_appr_req = json!({
        "requestId": "appr-203",
        "version": 1,
        "requestKind": "approval_required"
    });
    let handle_appr_req = pending_ui_request_from_value(&val_appr_req).expect("must parse handle");
    assert!(handle_appr_req.is_approval);
}

#[test]
fn test_pending_ui_request_reads_expiry() {
    // `expiresAtMs` 只是宿主快照的留痕字段：解析出来可以，但**不参与任何判定**
    // （ADR 0074：墙钟 TTL 已退出产品语义，能否作答只由宿主生命周期决定）。
    let with_ttl = json!({
        "requestId": "control-000000000000",
        "version": 1,
        "requestKind": "interaction",
        "expiresAtMs": 1_700_000_060_000u64
    });
    let handle = pending_ui_request_from_value(&with_ttl).expect("must parse handle");
    assert_eq!(handle.expires_at_ms, Some(1_700_000_060_000));

    let without_ttl = json!({
        "requestId": "control-000000000001",
        "version": 1,
        "requestKind": "interaction"
    });
    assert_eq!(
        pending_ui_request_from_value(&without_ttl).expect("must parse handle").expires_at_ms,
        None
    );

    // 0 只表示「宿主没给期限」，不是「此刻过期」。
    let zero_ttl = json!({
        "requestId": "control-000000000002",
        "version": 1,
        "requestKind": "interaction",
        "expiresAtMs": 0
    });
    assert_eq!(
        pending_ui_request_from_value(&zero_ttl).expect("must parse handle").expires_at_ms,
        None
    );
}

fn handle_for(request_id: &str) -> PendingUiRequest {
    PendingUiRequest {
        request_id: request_id.to_string(),
        version: 3,
        run_id: Some("run-1".to_string()),
        // 刻意给一个**早已过去**的期限：新判定必须完全忽略它。
        expires_at_ms: Some(1),
        is_approval: false,
    }
}

#[test]
fn test_snapshot_contains_request_matches_request_id_only() {
    let items = vec![
        json!({ "requestId": "control-a", "version": 1 }),
        json!({ "requestId": "control-b", "version": 2 }),
    ];
    assert!(snapshot_contains_request(&items, "control-a"));
    assert!(snapshot_contains_request(&items, "control-b"));
    assert!(!snapshot_contains_request(&items, "control-c"));
}

#[test]
fn test_pending_panel_should_close_is_lifecycle_driven_not_wall_clock() {
    // 契约 §6.4：宿主列表即真值 —— 句柄仍出现在待决列表里就**不许**关，
    // 哪怕它的 `expiresAtMs` 早已过去（旧实现会在这里误判为过期）。
    let stale_ttl_but_pending = vec![json!({
        "requestId": "control-000000000000",
        "version": 3,
        "requestKind": "interaction",
        "expiresAtMs": 1
    })];
    let handle = handle_for("control-000000000000");
    assert!(!pending_panel_should_close(&stale_ttl_but_pending, Some(&handle)));

    // 请求已不在列表（Answered / Cancelled / Superseded / Orphaned）⇒ 必须收面板。
    let empty: Vec<serde_json::Value> = Vec::new();
    assert!(pending_panel_should_close(&empty, Some(&handle)));

    // 列表里是**别的** request：本面板同样必须消失（同一会话只应挂当前未决项）。
    let other = vec![json!({ "requestId": "control-other", "version": 1 })];
    assert!(pending_panel_should_close(&other, Some(&handle)));

    // 没有句柄就无面板可收。
    assert!(!pending_panel_should_close(&empty, None));
}

#[test]
fn test_is_stale_control_request_error() {
    // 复现原缺陷时宿主原样回给用户的英文串必须被识别为「已失效」家族。
    assert!(is_stale_control_request_error(
        "control request `control-000000000000` cannot be consumed: replay, expiry, cross-session, or version/nonce mismatch"
    ));
    assert!(is_stale_control_request_error(
        "Ask `control-1` expected version 1 but current version is 2; stale mutation rejected."
    ));
    assert!(is_stale_control_request_error("unknown control request `control-9`"));
    assert!(!is_stale_control_request_error("failed to serialize ask resume outcome: io"));
}

#[test]
fn test_is_stale_control_request_error_recognises_persisted_cas_codes() {
    // 契约 §4 的五个稳定错误码是终态冲突/作用域缺失：重试永远不会成功，
    // 必须归入「已失效」家族（统一中文文案），不得落到「请重试」分支。
    for code in STALE_CONTROL_REQUEST_CODES {
        let message = format!("{code}: whatever the host says afterwards");
        assert!(
            is_stale_control_request_error(&message),
            "稳定错误码 `{code}` 必须被认定为已失效家族"
        );
        // 大小写不敏感：宿主偶有替换大小写的实现差异。
        assert!(is_stale_control_request_error(&message.to_uppercase()));
    }
    // 反例：普通网络/IO 故障不得被误判为终态冲突（否则会丢掉一个仍可作答的面板）。
    assert!(!is_stale_control_request_error("network down"));
    assert!(!is_stale_control_request_error("failed to serialize ask answer: io"));
}

#[test]
fn test_ask_question_from_pending_mapping() {
    let val = json!({
        "descriptorId": "confirmation_box",
        "prompt": "确定要继续吗？",
        "options": [
            { "label": "选项 A", "description": "说明 A" },
            { "label": "选项 B" }
        ]
    });
    let q = ask_question_from_pending(&val);
    assert_eq!(q.question, "确定要继续吗？");
    assert_eq!(q.header, Some("confirmation_box".to_string()));
    assert_eq!(q.options.len(), 2);
    assert_eq!(q.options[0].label, "选项 A");
    assert_eq!(q.options[0].description, Some("说明 A".to_string()));
}

#[test]
fn test_permission_request_from_pending_mapping() {
    let val = json!({
        "requestId": "req-999",
        "sessionId": "sess-abc",
        "descriptorId": "bash",
        "prompt": "执行危险删除命令",
        "callId": "rm -rf /",
        "expiresAtMs": 55555
    });
    let req = permission_request_from_pending(&val, "fallback-sess");
    assert_eq!(req.request_id, "req-999");
    assert_eq!(req.session_id, "sess-abc");
    assert_eq!(req.tool_name, "bash");
    assert_eq!(req.description, "执行危险删除命令");
    assert_eq!(req.command_or_path, Some("rm -rf /".to_string()));
    assert_eq!(req.timestamp_ms, 55555);
}
