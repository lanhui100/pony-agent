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

// ─────────────────────────────────────────────────────────────────────────────
// wave-ask-panel-multiselect：多选语义 + 版式（留白 / 折行）的纯函数契约
// ─────────────────────────────────────────────────────────────────────────────

fn multi_select_question() -> AskUserQuestion {
    AskUserQuestion {
        id: "q-multi".to_string(),
        header: None,
        question: "选择本次要一并迁移的存储层".to_string(),
        multi_select: true,
        options: vec![
            QuestionOption {
                label: "会话库".to_string(),
                description: Some("sessions.db".to_string()),
            },
            QuestionOption {
                label: "图运行库".to_string(),
                description: None,
            },
            QuestionOption {
                label: "向量索引".to_string(),
                description: None,
            },
        ],
    }
}

#[test]
fn test_ask_option_rows_wrap_without_exceeding_panel_width() {
    let label = "一个非常非常长的选项标签，用于验证折行后不会越过面板右缘并且不丢字";
    let desc = "说明文字同样很长，折行后必须继续与标签列对齐，而不是溢出到面板边界之外";
    let rows = ask_option_rows(true, "▢", label, desc, 40);
    assert!(rows.len() > 2, "长选项必须折行，实际只有 {} 行", rows.len());
    for row in &rows {
        assert!(
            line_display_width(row) <= 40,
            "选项行超过面板宽度：{} 列",
            line_display_width(row)
        );
    }
    // 每行末尾还有铺底的内边距空格，比较前必须先把空白抹掉，否则「包含」会被行间填充打断。
    let joined: String = rows
        .iter()
        .flat_map(|row| row.spans.iter())
        .map(|span| span.content.as_ref())
        .collect::<String>()
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect();
    assert!(joined.contains(label), "折行不得丢字：{joined:?}");
    assert!(!joined.contains('…'), "选项禁止省略号截断：{joined:?}");
    assert_eq!(
        rows[1].spans[0].content.chars().count(),
        ASK_OPTION_INDENT,
        "续行缩进必须与标签列对齐（{ASK_OPTION_INDENT} 列）"
    );
}

#[test]
fn test_ask_panel_height_counts_wrapped_option_rows() {
    let mut short_state = TuiState::default();
    short_state.mode = InteractionMode::AskUserQuestion;
    short_state.ask_user_state =
        Some(AskUserState::new("req-short-opt".to_string(), vec![multi_select_question()]));

    let mut long_state = TuiState::default();
    long_state.mode = InteractionMode::AskUserQuestion;
    let mut long_question = multi_select_question();
    long_question.options = (1..=3)
        .map(|index| QuestionOption {
            label: format!("选项{index}：{}", "很长的选项标签".repeat(4)),
            description: Some("说明文字".repeat(8)),
        })
        .collect();
    long_state.ask_user_state =
        Some(AskUserState::new("req-long-opt".to_string(), vec![long_question]));

    let short_h = inline_ask_height(&short_state, 80);
    let long_h = inline_ask_height(&long_state, 80);
    assert!(
        long_h > short_h,
        "折行后的选项必须让面板增高：long={long_h} short={short_h}"
    );
}

#[test]
fn test_ask_panel_height_is_fixed_rows_plus_question_plus_options() {
    let mut state = TuiState::default();
    state.mode = InteractionMode::AskUserQuestion;
    state.ask_user_state =
        Some(AskUserState::new("req-pad".to_string(), vec![multi_select_question()]));
    // 固定 6 行（发丝线 + 上内边距 + 标题 + 问答间距 + 键位提示 + 下内边距）
    // + 1 行问题 + 3 行选项 + 1 行自定义输入。
    let expected = 6 + 1 + 3 + 1;
    assert_eq!(
        inline_ask_height(&state, 80) as usize,
        expected,
        "面板行数口径（含上下内边距与问答间距）不得漂移"
    );
}

#[test]
fn test_ask_multi_select_collects_all_checked_in_option_order() {
    let mut ask = AskUserState::new("req-m".to_string(), vec![multi_select_question()]);
    ask.selected_option_index = 2;
    ask.toggle_current_option();
    ask.selected_option_index = 0;
    ask.toggle_current_option();
    assert_eq!(
        ask_answer_values(&ask),
        vec!["会话库".to_string(), "向量索引".to_string()],
        "多选答案必须按选项顺序给出全部勾选项（不得按勾选顺序）"
    );
    let payload = ask_answer_json(&[("q-multi".to_string(), ask_answer_values(&ask))]);
    assert_eq!(payload["answer"], json!("会话库"));
    assert_eq!(
        payload["values"],
        json!(["会话库", "向量索引"]),
        "多选必须携带全部选中项，不得只留首个取值"
    );
}

#[test]
fn test_ask_single_select_answer_payload_shape_is_unchanged() {
    let mut ask = AskUserState::new(
        "req-s".to_string(),
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
    );
    ask.selected_option_index = 0;
    let payload = ask_answer_json(&[("q1".to_string(), ask_answer_values(&ask))]);
    assert_eq!(
        payload,
        json!({ "answer": "是" }),
        "单选载荷形状必须逐字不变（向后兼容控制面/图运行）"
    );
}

#[test]
fn test_ask_multi_select_marker_reflects_checked_set() {
    let mut ask = AskUserState::new("req-mark".to_string(), vec![multi_select_question()]);
    let question = ask.current_question().cloned().expect("question");
    assert_eq!(ask_option_marker(&question, &ask, 0), "▢");
    ask.toggle_current_option();
    assert_eq!(ask_option_marker(&question, &ask, 0), "▣");
    assert_eq!(ask_option_marker(&question, &ask, 1), "▢");
}

#[test]
fn test_ask_question_from_pending_reads_multi_select_flag() {
    let multi = ask_question_from_pending(&json!({
        "prompt": "选多个",
        "options": ["A", "B"],
        "multiSelect": true
    }));
    assert!(multi.multi_select, "multiSelect=true 必须映射为多选");
    let legacy = ask_question_from_pending(&json!({ "prompt": "选一个", "options": ["A", "B"] }));
    assert!(!legacy.multi_select, "缺省（旧宿主 / 旧记录）必须按单选处理");
    let loose = ask_question_from_pending(&json!({ "prompt": "选一个", "multiSelect": "true" }));
    assert!(!loose.multi_select, "多选只认结构化布尔，字符串不得宽松解释");
}

#[test]
fn test_ask_multi_select_enter_without_checks_submits_focused_option() {
    let mut ask = AskUserState::new("req-focus".to_string(), vec![multi_select_question()]);
    ask.next_option();
    let focused = ask.selected_option_index;
    assert_eq!(focused, 1, "前置：焦点必须已移到第二项");
    assert_eq!(
        ask_selected_index(&ask),
        Some(1),
        "多选未勾选任何项时，Enter 必须仍然可以提交焦点项（与键位提示语义一致）"
    );
    assert_eq!(
        ask_answer_values(&ask),
        vec!["图运行库".to_string()],
        "未勾选时答案取焦点项，不得是空答案"
    );
}
