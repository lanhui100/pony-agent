//! `append_turn` 事件源物化回归（从 `control_plane/mod.rs` 内联测试外移，避开规模棘轮）。
//!
//! PA-095 #2（防架空探针）：caller 文本与事件文本不同时，history 必须取事件文本。
//! PA-095 #3（B146）：并行测试下全局单槽事件通道会被其他 control plane 构建覆盖，
//! 故本用例把事件通道**按 session 绑定**到本控制面（守卫随作用域解绑），把
//! 「靠重试碰运气」的结构性竞态换成确定性路由。

use super::*;

/// PA-095 #2：append_turn 事件源物化生效——caller 文本与事件
/// 文本不同时，history 必须取事件文本；assistant reasoning_content 随事件
/// 物化（caller 参数无此数据）。
#[test]
fn append_turn_materializes_history_from_events_over_caller_text() {
    // 同 graph_sync_run_turn...：先 cp 锁再起 mock server，保持获取顺序一致。
    let _cp_lock = control_plane_test_lock();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
    let dir = {
        let raw = std::env::temp_dir().join(format!("pony-mat-events-{stamp}"));
        raw.canonicalize().unwrap_or(raw)
    };
    fs::create_dir_all(&dir).expect("create temp dir");

    let mut snapshot_history: Option<Vec<(String, Option<String>)>> = None;
    let mut last_counters = None;
    for attempt in 0..8 {
        let save_store_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let append_message_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let update_meta_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let sessions = SessionStore::with_backend(Box::new(CountingBackend {
            inner: crate::agent::sqlite_session::SqliteSessionBackend::new_with_trace_mode(
                dir.join(format!("sessions-{attempt}.db")),
                crate::agent::session::SeparateTraceTableMode::WriteSeparate,
            ),
            save_store_count: Arc::clone(&save_store_count),
            append_message_count: Arc::clone(&append_message_count),
            update_meta_count: Arc::clone(&update_meta_count),
        }));
        let runtime = AgentRuntime::with_dependencies(
            sessions,
            Box::new(StaticResolver {
                selection: test_provider_selection("http://127.0.0.1:1/v1".to_string()),
            }),
            Box::new(ToolRouter::new()),
            Box::new(LocalTurnPlanner),
            Box::new(DefaultTurnContextBuilder),
            Box::new(DefaultTurnTelemetryBuilder),
        );
        // builder.build() 注册 persist + flush 通道（绑定本 store）。
        let control_plane = HostControlPlane::with_runtime(runtime);
        // PA-095 #3（B146）：会话绑定路由——本控制面的事件通道按 session 绑定，
        // 全局单槽被并行测试的 control plane 构建覆盖也不影响本用例
        // （守卫在 attempt 结束时解绑）。
        let _persist_guard = crate::agent::turn_flow::bind_event_persist_session(
            "mat-s1",
            control_plane.event_persist_channel(),
        );

        let noop_sink = crate::agent::turn_flow::NoopTurnEventSink;
        let emit = |name: &str, text: Option<String>, reasoning: Option<String>| {
            crate::agent::turn_flow::emit_stream_event(
                &noop_sink,
                name,
                format!("mat-turn-{attempt}"),
                if name == "turn:started" {
                    "started"
                } else {
                    "completed"
                },
                Some(if name == "turn:started" {
                    "calling_model"
                } else {
                    "completed"
                }),
                text,
                reasoning,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some("mat-s1".to_string()),
            );
        };
        emit("turn:started", Some("事件用户文本".to_string()), None);
        emit(
            "turn:completed",
            Some("事件助手文本".to_string()),
            Some("事件推理".to_string()),
        );

        // append_turn 的 caller 文本故意与事件不同——物化必须取事件。
        // 基线计数：store 构造期的迁移写（workspace 播种等）不计入
        // append_turn 的结构性断言——只看 append_turn 前后的增量。
        let baseline_save = save_store_count.load(std::sync::atomic::Ordering::SeqCst);
        let baseline_appends = append_message_count.load(std::sync::atomic::Ordering::SeqCst);
        let baseline_metas = update_meta_count.load(std::sync::atomic::Ordering::SeqCst);
        let snapshot = control_plane
            .sessions_rwlock
            .write()
            .expect("sessions lock poisoned")
            .append_turn(
                Some("mat-s1"),
                "caller 用户文本",
                "caller 助手文本",
                None,
                Vec::new(),
            );
        let delta_save = save_store_count.load(std::sync::atomic::Ordering::SeqCst) - baseline_save;
        let delta_appends =
            append_message_count.load(std::sync::atomic::Ordering::SeqCst) - baseline_appends;
        let delta_metas =
            update_meta_count.load(std::sync::atomic::Ordering::SeqCst) - baseline_metas;
        let entries: Vec<(String, Option<String>)> = snapshot
            .history
            .iter()
            .map(|message| (message.content.clone(), message.reasoning_content.clone()))
            .collect();
        // 物化生效判据：user 条目取事件文本（会话绑定后确定性命中；循环仅作兜底）。
        if entries.first().map(|(content, _)| content.as_str()) == Some("事件用户文本") {
            snapshot_history = Some(entries);
            last_counters = Some((delta_save, delta_appends, delta_metas));
            break;
        }
    }

    let entries = snapshot_history
        .expect("materialization never took effect across retries (channel theft every attempt)");
    assert_eq!(entries.len(), 2);
    assert_eq!(
        entries[0].0, "事件用户文本",
        "user entry materialized from event, not caller"
    );
    assert_eq!(
        entries[1].0, "事件助手文本",
        "assistant entry materialized from event, not caller"
    );
    assert_eq!(
        entries[1].1.as_deref(),
        Some("事件推理"),
        "reasoning_content derives from the event"
    );

    // 写放大消除的结构性证据（persist-command 拦截，append_turn 前后增量）：
    // 消息级 turn 的整包 store 写（save_store）= 0；AppendMessage = 2
    // （user+assistant）、UpdateSessionMeta = 1。
    let (save_store, appends, metas) =
        last_counters.expect("counters captured on successful attempt");
    assert_eq!(
        save_store, 0,
        "message-only turn must not trigger whole-store writes"
    );
    assert_eq!(appends, 2, "two AppendMessage commands per turn");
    assert_eq!(metas, 1, "one UpdateSessionMeta command per turn");

    // 不 clear 全局注册表——并行测试的 control plane 可能正持有通道；
    // 泄漏的闭包随进程结束回收（覆盖语义：下一个 builder 注册即替换）。
    let _ = fs::remove_dir_all(&dir);
}
