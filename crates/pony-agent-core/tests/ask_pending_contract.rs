//! Wave-15 acceptance (red phase): the Ask pending list must only expose requests that are both
//! `Interaction` **and** `Pending`, and the host projection must inherit that filter.
//!
//! Frozen contract: `.dev-team/contracts/wave-15-ask-cas.md` (C1-1 ~ C1-6).
//!
//! Red-phase intent: every helper below drives the *real* dispatcher store through
//! `dispatch_governed` / `answer_ask` / `cancel_ask` / `expire_control_requests`, so a failing
//! assertion is evidence about the SUT (the list filter drops terminal states), never about a
//! missing or renamed symbol. All referenced items are existing `pub` API.
//!
//! Symptom under test: `pending_requests()` returns every stored control request (including
//! `Consumed` / `Cancelled` / `Expired`), and `ask_control::filter_interaction_requests()` only
//! filters by `request_kind` — so a consumed Ask is re-listed as if it were still awaiting an
//! answer, and the user's next submit hits a CAS error.

use pony_agent_core::agent::ask_control::{answer_ask, cancel_ask, list_pending_asks};
use pony_agent_core::agent::control_plane::{HostControlPlane, HostControlPlaneBuilder};
use pony_agent_core::agent::dispatcher::{
    ControlRequestAuthorization, DispatchContext, GovernedDispatcher,
};
use pony_agent_core::agent::governed_executor::LegacyCompatiblePolicyEvaluator;
use pony_agent_core::agent::tool_runtime::{
    FakeClock, InvocationOrigin, PendingControlRequest, PendingControlRequestKind,
    PendingControlRequestState, RuntimeClock, ToolDispatchRequest,
};
use pony_agent_core::agent::tools::ToolRegistrySnapshot;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::sync::Arc;

/// Contract-conformant Ask descriptor id (builtin first-class `ask_user`).
const ASK_DESCRIPTOR_ID: &str = "builtin:ask_user";

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Harness (real registry + real governed dispatcher + deterministic clock)
// ─────────────────────────────────────────────────────────────────────────────────────────────

struct AskHarness {
    dispatcher: Arc<GovernedDispatcher>,
    clock: Arc<FakeClock>,
}

fn harness() -> AskHarness {
    let registry = Arc::new(
        ToolRegistrySnapshot::builtin().expect("builtin tool registry must construct cleanly"),
    );
    let clock = Arc::new(FakeClock::new(1_000));
    let dispatcher = GovernedDispatcher::new(
        registry,
        Arc::clone(&clock) as Arc<dyn RuntimeClock>,
    );
    // The builtin `ask_user` descriptor is host-mediated; the legacy-compatible evaluator is what
    // turns it into a `WaitingHost` control outcome (same wiring as the product runtime).
    dispatcher.register_policy_evaluator(Arc::new(LegacyCompatiblePolicyEvaluator));
    AskHarness {
        dispatcher: Arc::new(dispatcher),
        clock,
    }
}

fn ask_context() -> DispatchContext {
    DispatchContext {
        session_id: Some("session-1".to_string()),
        run_id: Some("run-1".to_string()),
        turn_id: Some("turn-1".to_string()),
        host_control_available: true,
        ..Default::default()
    }
}

/// Dispatch a real model-origin Ask and return the persisted pending control request.
fn dispatch_ask(
    dispatcher: &GovernedDispatcher,
    call_id: &str,
    question: &str,
) -> PendingControlRequest {
    let outcome = dispatcher.dispatch_governed(
        ToolDispatchRequest {
            origin: InvocationOrigin::Model,
            descriptor_id: ASK_DESCRIPTOR_ID.to_string(),
            call_id: call_id.to_string(),
            arguments: json!({
                "question": question,
                "options": ["是", "否"],
                "description": "wave-15 契约测试"
            }),
        },
        &ask_context(),
    );
    let outcome_debug = format!("{outcome:?}");
    let request_id = outcome
        .control_outcome
        .unwrap_or_else(|| {
            panic!("host-mediated ask must surface a control outcome (call_id={call_id}, outcome={outcome_debug})")
        })
        .request_id;
    dispatcher
        .pending_request(&request_id)
        .expect("dispatched ask must be persisted in the control-request store")
}

/// Dispatch a real Ask into an explicit session (supersede tests need two sessions).
fn dispatch_ask_in(
    dispatcher: &GovernedDispatcher,
    session_id: &str,
    call_id: &str,
    question: &str,
) -> PendingControlRequest {
    let outcome = dispatcher.dispatch_governed(
        ToolDispatchRequest {
            origin: InvocationOrigin::Model,
            descriptor_id: ASK_DESCRIPTOR_ID.to_string(),
            call_id: call_id.to_string(),
            arguments: json!({
                "question": question,
                "options": ["是", "否"],
                "description": "wave-15 契约测试"
            }),
        },
        &DispatchContext {
            session_id: Some(session_id.to_string()),
            run_id: Some("run-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            host_control_available: true,
            ..Default::default()
        },
    );
    let outcome_debug = format!("{outcome:?}");
    let request_id = outcome
        .control_outcome
        .unwrap_or_else(|| {
            panic!("host-mediated ask must surface a control outcome (call_id={call_id}, outcome={outcome_debug})")
        })
        .request_id;
    dispatcher
        .pending_request(&request_id)
        .expect("dispatched ask must be persisted in the control-request store")
}

fn ids(requests: &[PendingControlRequest]) -> BTreeSet<String> {
    requests.iter().map(|r| r.request_id.clone()).collect()
}

fn projection_ids(value: &Value) -> BTreeSet<String> {
    value
        .as_array()
        .expect("`list_pending_asks` projection must be a JSON array")
        .iter()
        .map(|item| {
            item.get("requestId")
                .and_then(Value::as_str)
                .expect("each projected ask must carry `requestId`")
                .to_string()
        })
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// C1-1: only `Interaction` **and** `Pending`
// ─────────────────────────────────────────────────────────────────────────────────────────────

#[test]
fn c1_1_list_pending_asks_only_exposes_pending_interaction_requests() {
    // Arrange: one ask gets consumed; the raw store keeps it, so filtering must do real work.
    let h = harness();
    let consumed_ask = dispatch_ask(&h.dispatcher, "call-1", "继续？");
    let waiting_ask = dispatch_ask(&h.dispatcher, "call-2", "再继续？");
    assert_eq!(
        consumed_ask.request_kind,
        PendingControlRequestKind::Interaction,
        "ask dispatch must persist an Interaction control request"
    );

    let authorization =
        ControlRequestAuthorization::for_request(&consumed_ask, Some(json!("是")));
    let consumed = answer_ask(&h.dispatcher, &consumed_ask.request_id, &authorization)
        .expect("answer_ask must consume the pending interaction request");
    assert_eq!(consumed.request.state, PendingControlRequestState::Consumed);

    // The store still holds both requests (terminal states are retained, not deleted).
    assert_eq!(
        h.dispatcher.pending_requests().len(),
        2,
        "the raw store retains terminal requests; only the Ask list may hide them"
    );
    assert_eq!(
        h.dispatcher
            .pending_request(&consumed_ask.request_id)
            .expect("stored")
            .state,
        PendingControlRequestState::Consumed
    );

    // Act
    let listed = list_pending_asks(&h.dispatcher);

    // Assert
    for request in &listed {
        assert_eq!(
            request.request_kind,
            PendingControlRequestKind::Interaction,
            "list_pending_asks must never expose non-Interaction requests"
        );
        assert_eq!(
            request.state,
            PendingControlRequestState::Pending,
            "list_pending_asks must never expose terminal request `{}` (state={:?})",
            request.request_id,
            request.state
        );
    }
    assert_eq!(
        ids(&listed),
        BTreeSet::from_iter([waiting_ask.request_id.clone()]),
        "only the still-pending ask may be listed"
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// C1-2: answering removes the request from the Ask list
// ─────────────────────────────────────────────────────────────────────────────────────────────

#[test]
fn c1_2_answered_ask_is_no_longer_listed() {
    // Arrange
    let h = harness();
    let pending = dispatch_ask(&h.dispatcher, "call-1", "继续？");
    assert_eq!(pending.state, PendingControlRequestState::Pending);
    assert_eq!(
        list_pending_asks(&h.dispatcher).len(),
        1,
        "the pending ask must be listed before it is answered"
    );

    // Act
    let authorization = ControlRequestAuthorization::for_request(&pending, Some(json!("是")));
    let consumed = answer_ask(&h.dispatcher, &pending.request_id, &authorization)
        .expect("answer_ask must consume the pending ask");
    assert_eq!(consumed.request.state, PendingControlRequestState::Consumed);
    assert_eq!(consumed.answer, Some(json!("是")));

    // Assert
    assert!(
        list_pending_asks(&h.dispatcher).is_empty(),
        "a consumed Ask must not be re-listed as pending (this is the user-visible CAS trap)"
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// C1-3: cancelling removes the request from the Ask list
// ─────────────────────────────────────────────────────────────────────────────────────────────

#[test]
fn c1_3_cancelled_ask_is_no_longer_listed() {
    // Arrange
    let h = harness();
    let pending = dispatch_ask(&h.dispatcher, "call-1", "继续？");
    assert_eq!(list_pending_asks(&h.dispatcher).len(), 1);

    // Act
    let authorization = ControlRequestAuthorization::for_request(&pending, None);
    let cancelled = cancel_ask(&h.dispatcher, &pending.request_id, &authorization)
        .expect("cancel_ask must cancel the pending ask");
    assert_eq!(
        cancelled.request.state,
        PendingControlRequestState::Cancelled
    );

    // Assert
    assert!(
        list_pending_asks(&h.dispatcher).is_empty(),
        "a cancelled Ask must not be re-listed as pending"
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// C1-4（ADR 0074 重冻）：**新用户消息**使旧 Ask 转 Superseded 后不再列出；墙钟不再是判据
// ─────────────────────────────────────────────────────────────────────────────────────────────
//
// 旧语义（已废止）：`expires_at_ms` 到期 ⇒ `Expired` ⇒ 不再列出。
// 新语义（ADR 0074 §3/§4）：只有「会话内出现新用户消息」才让旧 Ask 失格；时间既不拒绝作答、
// 也不清空列表。宿主的 `ask_expire` 退化为 GC（只回收终态/归档会话）。

#[test]
fn c1_4_superseded_ask_is_no_longer_listed() {
    // Arrange
    let h = harness();
    let pending = dispatch_ask(&h.dispatcher, "call-1", "继续？");
    assert_eq!(list_pending_asks(&h.dispatcher).len(), 1);

    // Act 1（时间不再是判据）：时钟越过旧 TTL 很久以后，Ask 仍然可列出。
    let long_after_ttl = pending.expires_at_ms + 24 * 60 * 60 * 1_000;
    h.clock.set_now_ms(long_after_ttl);
    assert_eq!(
        h.dispatcher.expire_control_requests(long_after_ttl),
        0,
        "ADR 0074：墙钟过期不再作用于 Ask（Interaction）——expire 只对 Approval 生效"
    );
    assert_eq!(
        list_pending_asks(&h.dispatcher).len(),
        1,
        "时间不得把未决 Ask 从列表里抹掉"
    );
    let authorization = ControlRequestAuthorization::for_request(&pending, Some(json!("是")));
    assert!(
        h.dispatcher
            .pending_request(&pending.request_id)
            .expect("stored")
            .state
            == PendingControlRequestState::Pending,
        "时间越过旧 TTL 后 Ask 必须仍为 Pending"
    );

    // Act 2：新用户消息进入会话 ⇒ 旧 Ask 被取代（宿主对内存镜像的关闭动作）
    let superseded = h.dispatcher.close_pending_asks_of_session("session-1");
    assert_eq!(superseded, 1, "新用户消息必须关闭该会话的 1 条未决 Ask");

    // Assert：不再列出，且不能再作答。
    assert!(
        list_pending_asks(&h.dispatcher).is_empty(),
        "被新用户消息取代的 Ask 不得再被列为待决"
    );
    assert!(
        answer_ask(&h.dispatcher, &pending.request_id, &authorization).is_err(),
        "被取代的 Ask 不得再被作答（终态不可迁移）"
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// C1-5（ADR 0074 重冻）：混合终态 → 列表恰好是仍为 Pending 的集合
// ─────────────────────────────────────────────────────────────────────────────────────────────

#[test]
fn c1_5_mixed_terminal_states_list_exactly_the_pending_set() {
    // Arrange：一条走「新用户消息取代」（放在 session-2，避免关掉 session-1 的等待项），
    // 其余三条在 session-1 分别走作答 / 取消 / 继续等待。
    let h = harness();
    let will_supersede = dispatch_ask_in(&h.dispatcher, "session-2", "call-supersede", "会被取代？");
    let will_consume = dispatch_ask(&h.dispatcher, "call-consume", "会消费？");
    let will_cancel = dispatch_ask(&h.dispatcher, "call-cancel", "会取消？");
    let will_wait = dispatch_ask(&h.dispatcher, "call-wait", "仍等待？");

    assert_eq!(
        h.dispatcher.pending_requests().len(),
        4,
        "the raw store holds every request regardless of state"
    );

    // Act：把三条推进到不同终态/等待态。
    let authorization = ControlRequestAuthorization::for_request(&will_consume, Some(json!("是")));
    answer_ask(&h.dispatcher, &will_consume.request_id, &authorization).expect("consume");
    let authorization = ControlRequestAuthorization::for_request(&will_cancel, None);
    cancel_ask(&h.dispatcher, &will_cancel.request_id, &authorization).expect("cancel");
    assert_eq!(
        h.dispatcher.close_pending_asks_of_session("session-2"),
        1,
        "只有 session-2 的那条 Ask 会被新用户消息取代"
    );

    // Assert：各终态落地，列表恰好等于仍为 Pending 的集合。
    let state_of = |request_id: &str| {
        h.dispatcher
            .pending_request(request_id)
            .expect("stored")
            .state
    };
    assert_ne!(
        state_of(&will_supersede.request_id),
        PendingControlRequestState::Pending,
        "被取代的 Ask 不得仍为 Pending"
    );
    assert_eq!(
        state_of(&will_consume.request_id),
        PendingControlRequestState::Consumed
    );
    assert_eq!(
        state_of(&will_cancel.request_id),
        PendingControlRequestState::Cancelled
    );
    assert_eq!(
        state_of(&will_wait.request_id),
        PendingControlRequestState::Pending
    );

    let listed = list_pending_asks(&h.dispatcher);
    assert_eq!(
        ids(&listed),
        BTreeSet::from_iter([will_wait.request_id.clone()]),
        "the list must be exactly the still-pending Interaction set — no terminal leaks, no missing waits"
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// C1-6: the host projection (`ask_list_pending` user-visible surface) inherits the filter
// ─────────────────────────────────────────────────────────────────────────────────────────────

#[test]
fn c1_6_host_control_plane_projection_excludes_terminal_requests() {
    use pony_agent_core::agent::ask_persistence::{open_ask_store, AskPersistenceStore};

    // Arrange（契约 §5 重冻）：宿主清单**必须有会话作用域**，且持久化层是唯一真值源。
    // 因此这里给 dispatcher 装上真实持久化 store，让派发出来的 Ask 真正落盘；作答走作用域入口。
    let h = harness();
    let db_path = unique_temp_db_path("c1-6");
    let store: Arc<dyn AskPersistenceStore> = Arc::from(
        open_ask_store(&db_path).expect("c1-6：持久化 Ask store 必须可构造"),
    );
    h.dispatcher.set_ask_store(Arc::clone(&store));

    let consumed_ask = dispatch_ask(&h.dispatcher, "call-1", "已答？");
    let waiting_ask = dispatch_ask(&h.dispatcher, "call-2", "等待？");

    let plane: HostControlPlane = HostControlPlaneBuilder::new()
        .ask_dispatcher(Arc::clone(&h.dispatcher))
        .ask_store(Arc::clone(&store))
        .build();

    let both = BTreeSet::from_iter([
        consumed_ask.request_id.clone(),
        waiting_ask.request_id.clone(),
    ]);
    let expected = BTreeSet::from_iter([waiting_ask.request_id.clone()]);

    // Act 1（作用域读）：两条都还在 Pending。
    let before = plane.list_pending_asks(Some("session-1"));
    assert_eq!(projection_ids(&before), both, "作用域清单必须包含两条未决 Ask");

    // Act 2（作用域作答，契约 §4）：只在显式会话作用域内消费。
    plane
        .answer_ask_in_session(
            "session-1",
            &consumed_ask.request_id,
            consumed_ask.version,
            json!("是"),
        )
        .expect("c1-6：作用域内作答必须成功（(session_id, request_id, version) CAS）");

    // Assert：作用域投影必须丢掉已终结请求、保留仍在等待的那条，且投影形态正确。
    let by_session = plane.list_pending_asks(Some("session-1"));
    for item in by_session
        .as_array()
        .expect("`list_pending_asks(Some(..))` must project a JSON array")
    {
        assert_eq!(
            item.get("state").and_then(Value::as_str),
            Some("pending"),
            "projected ask must be pending: {item}"
        );
        assert_eq!(
            item.get("requestKind").and_then(Value::as_str),
            Some("interaction"),
            "projected ask must be an Interaction: {item}"
        );
        assert!(
            item.get("nonce").is_none(),
            "契约 §5：ask 投影不得暴露 nonce（服务端自持 CAS 凭据）: {item}"
        );
    }
    assert_eq!(
        projection_ids(&by_session),
        expected,
        "session-filtered host projection must drop the answered ask"
    );

    // 反向断言（契约 §5）：**无会话作用域**的清单必须 fail-closed 为空数组，
    // 不得把其它会话的 requestId/version 泄露给无作用域调用。
    let unscoped = plane.list_pending_asks(None);
    assert_eq!(
        unscoped
            .as_array()
            .expect("`list_pending_asks(None)` must still project a JSON array")
            .len(),
        0,
        "无作用域清单必须返回空数组（ask_scope_required fail-closed），实际：{unscoped}"
    );

    // 反向断言（契约 §4）：三参 legacy 作答入口必须 fail-closed（ask_scope_required），
    // 且不得消费记录（更不得跨会话作答）。
    let legacy = plane.answer_ask(&waiting_ask.request_id, waiting_ask.version, json!("是"));
    let legacy_error = match legacy {
        Ok(value) => {
            assert!(
                false,
                "契约 §4：无会话作用域的三参 answer_ask 必须 fail-closed（ask_scope_required），                 实际返回 Ok({value})"
            );
            unreachable!("assert above always fails on Ok")
        }
        Err(error) => error,
    };
    assert!(
        legacy_error.contains("ask_scope_required"),
        "契约 §4：无作用域作答的错误码必须是 ask_scope_required，实际：{legacy_error}"
    );
    assert_eq!(
        projection_ids(&plane.list_pending_asks(Some("session-1"))),
        expected,
        "被拒绝的无作用域作答不得消费记录"
    );

    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(db_path.with_extension("db-wal"));
    let _ = std::fs::remove_file(db_path.with_extension("db-shm"));
}

/// 独立的临时 SQLite 库路径（谁创建谁清理；不污染真实会话库）。
fn unique_temp_db_path(tag: &str) -> std::path::PathBuf {
    let unique = format!(
        "pony-c1-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before epoch")
            .as_nanos()
    );
    let dir = std::env::temp_dir().join(unique);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir.join("sessions.db")
}
