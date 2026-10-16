//! Wave-15 C4 对抗破坏用例（L2-AT，红相）——ask pending 列表 / CAS 终态投影。
//!
//! 契约依据：`.dev-team/contracts/wave-15-ask-cas.md` §C4（冻结）。
//!
//! 红相目标（根因，契约 §0）：
//! `ask_control::list_pending_asks` =
//! `filter_interaction_requests(dispatcher.pending_requests())`，
//! 而 `filter_interaction_requests` **只按 `request_kind == Interaction` 过滤，未按
//! `state == Pending` 过滤**；`GovernedDispatcher::pending_requests()` 返回存储中**全部**
//! 控制请求（含 Consumed / Cancelled / Expired 终态）。
//! ⇒ 已消费的 Ask 仍被列为「待答」，用户再次点击即触发 CAS 拒绝（通用文案）。
//!
//! 断言纪律：全部断言打在 SUT 行为面（`ask_control::list_pending_asks` /
//! `ask_control::answer_ask` / `ask_control::cancel_ask` / `GovernedDispatcher`
//! 的 CAS 与存储快照 `pending_request` / `pending_requests` /
//! `expire_control_requests`）之上，不依赖测试基建异常，不使用 sleep 概率等待。

use pony_agent_core::agent::ask_control::{answer_ask, cancel_ask, list_pending_asks};
use pony_agent_core::agent::dispatcher::{ControlRequestAuthorization, GovernedDispatcher};
use pony_agent_core::agent::tool_runtime::{FakeClock, PendingControlRequestState};
use std::collections::BTreeSet;
use std::sync::{Arc, Barrier};

const ASK_DESCRIPTOR_ID: &str = "builtin:ask_user";
const BASE_NOW_MS: u64 = 1_000;
/// 契约 C4-1 指定的固定并发度（4 × answer_ask + 4 × cancel_ask）。
const RACE_THREADS: usize = 8;
/// 契约 C4-4 指定的批量体量下限。
const BATCH: usize = 200;

// ─────────────────────────────────────────────────────────────────────────────
// 测试基建（全部为既有 pub API 之上的最小封装，与 ask_user_acceptance.rs 同构）
// ─────────────────────────────────────────────────────────────────────────────

/// 真实 dispatcher：builtin 注册表 + FakeClock(1000) + LegacyCompatiblePolicyEvaluator。
/// 返回 (dispatcher, 具体 FakeClock)，后者供 C4-3 注入超期时刻（同一实例亦作为注入时钟）。
fn ask_dispatcher() -> (GovernedDispatcher, Arc<FakeClock>) {
    use pony_agent_core::agent::governed_executor::LegacyCompatiblePolicyEvaluator;
    use pony_agent_core::agent::tool_runtime::RuntimeClock;
    use pony_agent_core::agent::tools::ToolRegistrySnapshot;

    let registry = Arc::new(
        ToolRegistrySnapshot::builtin().expect("builtin tool registry must construct cleanly"),
    );
    let clock = Arc::new(FakeClock::new(BASE_NOW_MS));
    let injected: Arc<dyn RuntimeClock> = Arc::clone(&clock) as Arc<dyn RuntimeClock>;
    let dispatcher = GovernedDispatcher::new(Arc::clone(&registry), injected);
    dispatcher.register_policy_evaluator(Arc::new(LegacyCompatiblePolicyEvaluator));
    (dispatcher, clock)
}

fn ask_context(session: &str) -> pony_agent_core::agent::dispatcher::DispatchContext {
    pony_agent_core::agent::dispatcher::DispatchContext {
        session_id: Some(session.to_string()),
        run_id: Some("run-1".to_string()),
        turn_id: Some("turn-1".to_string()),
        host_control_available: true,
        ..Default::default()
    }
}

/// 通过真实受管派发路径持久化一个 Interaction（Ask）PendingControlRequest，
/// 返回 (request_id, 持久化快照)。宿主控制面可用 ⇒ WaitingHost ⇒ Interaction。
fn persist_ask(
    dispatcher: &GovernedDispatcher,
    call_id: &str,
    session: &str,
) -> (String, pony_agent_core::agent::tool_runtime::PendingControlRequest) {
    use pony_agent_core::agent::tool_runtime::{InvocationOrigin, ToolDispatchRequest};

    let outcome = dispatcher.dispatch_governed(
        ToolDispatchRequest {
            origin: InvocationOrigin::Model,
            descriptor_id: ASK_DESCRIPTOR_ID.to_string(),
            call_id: call_id.to_string(),
            arguments: serde_json::json!({
                "question": format!("继续？{call_id}"),
                "options": ["是", "否"],
                "description": "对抗用例"
            }),
        },
        &ask_context(session),
    );
    let request_id = outcome
        .control_outcome
        .expect("host-mediated ask must surface a WaitingHost control outcome")
        .request_id;
    let pending = dispatcher
        .pending_request(&request_id)
        .expect("persisted ask must be readable from the dispatcher store");
    (request_id, pending)
}

// ─────────────────────────────────────────────────────────────────────────────
// C4-1 并发竞态（确定性，无 sleep）
// ─────────────────────────────────────────────────────────────────────────────

/// C4-1：固定 8 线程经 `Barrier` 同时释放，对**同一个** pending Ask 并发发起
/// 4 × `answer_ask` + 4 × `cancel_ask`（授权对象全部由同一份 pending 快照
/// `ControlRequestAuthorization::for_request` 派生，无任何线程内二次读快照）。
///
/// 断言：恰好 1 个 Ok、7 个 Err；事后 `list_pending_asks` 精确为空；
/// 存储快照 `pending_request(id).state` 为唯一终态；`version` 只自增一次（1 → 2）。
///
/// 红相预期：CAS 单胜者成立（版本不重复自增），但 `list_pending_asks` 仍列出已消费请求
/// （契约 §0 根因），故列表断言必须失败。
#[test]
fn test_c4_1_concurrent_answer_and_cancel_on_same_ask_has_single_winner() {
    let (dispatcher, _clock) = ask_dispatcher();
    let (request_id, pending) = persist_ask(&dispatcher, "call-c4-1", "session-c4-1");

    // 消费前基线：version 只有一个 prior 递增点（持久化即 version == 1）。
    assert_eq!(
        pending.version, 1,
        "a freshly persisted ask must start at version 1 so the single-consumption \
         invariant is observable as version == 2"
    );
    assert_eq!(pending.state, PendingControlRequestState::Pending);

    // 8 份授权全部从**同一份**快照派生：线程内不再读取任何可变状态。
    let authorizations: Vec<ControlRequestAuthorization> = (0..RACE_THREADS)
        .map(|index| {
            let answer = if index % 2 == 0 {
                Some(serde_json::json!("是"))
            } else {
                None
            };
            ControlRequestAuthorization::for_request(&pending, answer)
        })
        .collect();

    let dispatcher = Arc::new(dispatcher);
    let barrier = Arc::new(Barrier::new(RACE_THREADS));
    let mut handles = Vec::with_capacity(RACE_THREADS);

    for (index, authorization) in authorizations.into_iter().enumerate() {
        let dispatcher = Arc::clone(&dispatcher);
        let barrier = Arc::clone(&barrier);
        let request_id = request_id.clone();
        handles.push(std::thread::spawn(move || {
            // 固定并发度确定性同步：所有线程在此对齐后才进入 CAS，禁止 sleep 概率等待。
            barrier.wait();
            let result: Result<(), String> = if index % 2 == 0 {
                answer_ask(&dispatcher, &request_id, &authorization).map(|_| ())
            } else {
                cancel_ask(&dispatcher, &request_id, &authorization).map(|_| ())
            };
            (index, result)
        }));
    }

    let mut ok_indices = Vec::new();
    let mut err_messages = Vec::new();
    for handle in handles {
        let (index, result) = handle.join().expect("adversarial worker must not panic");
        match result {
            Ok(()) => ok_indices.push(index),
            Err(error) => err_messages.push((index, error)),
        }
    }

    assert_eq!(
        ok_indices.len(),
        1,
        "exactly one concurrent answer/cancel may win the CAS; winners={ok_indices:?}, \
         errors={err_messages:?}"
    );
    assert_eq!(
        err_messages.len(),
        RACE_THREADS - 1,
        "the 7 losing racers must fail closed: {:?}",
        err_messages
    );
    for (index, error) in &err_messages {
        assert!(
            error.contains("cannot be consumed") || error.contains("kind mismatch"),
            "racer #{index} must be rejected by the CAS/kind guard, got: {error}"
        );
    }

    // 终态唯一且 version 只自增一次。
    let stored = dispatcher
        .pending_request(&request_id)
        .expect("consumed ask must remain readable in the store snapshot");
    assert!(
        matches!(stored.state, PendingControlRequestState::Consumed | PendingControlRequestState::Cancelled),
        "the raced ask must land in exactly one terminal state, got {:?}",
        stored.state
    );
    assert_eq!(
        stored.version, 2,
        "version must be bumped exactly once by the single winner (1 → 2), got {}",
        stored.version
    );

    // SUT 列表投影：终态请求不得再列为待答。
    let asks = list_pending_asks(&dispatcher);
    assert!(
        asks.is_empty(),
        "after the single winner consumed the request, list_pending_asks must be empty, \
         but it still lists {:?}",
        asks.iter().map(|ask| &ask.request_id).collect::<Vec<_>>()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// C4-2 重放
// ─────────────────────────────────────────────────────────────────────────────

/// C4-2：消费后再用「消费后的版本 + 同一快照授权」重复 `answer_ask` / `cancel_ask`
/// 必须 Err，且 `list_pending_asks` 不得复活该请求。
///
/// 红相预期：重放拒绝成立（CAS fail-closed），但「不得复活」不成立——
/// 该请求从未从列表移除，因此「复活」在红相下恒真（列表包含已消费请求即失败）。
#[test]
fn test_c4_2_replay_after_consumption_must_fail_and_must_not_resurrect() {
    let (dispatcher, _clock) = ask_dispatcher();
    let (request_id, pending) = persist_ask(&dispatcher, "call-c4-2", "session-c4-2");

    // 首次消费（真实 CAS 路径）。
    let first = ControlRequestAuthorization::for_request(&pending, Some(serde_json::json!("是")));
    let consumed = answer_ask(&dispatcher, &request_id, &first)
        .expect("first answer_ask must consume the pending ask");
    assert_eq!(consumed.request.state, PendingControlRequestState::Consumed);
    let consumed_version = consumed.request.version;
    assert_eq!(
        consumed_version, 2,
        "the first consumption must be the only version bump (1 → 2)"
    );

    // 消费后的版本 + 同一快照授权：用消费返回的快照重新派生授权再重放。
    let replay_authorization =
        ControlRequestAuthorization::for_request(&consumed.request, Some(serde_json::json!("是")));
    assert!(
        answer_ask(&dispatcher, &request_id, &replay_authorization).is_err(),
        "replaying answer_ask with the post-consumption version must fail closed"
    );
    assert!(
        cancel_ask(&dispatcher, &request_id, &replay_authorization).is_err(),
        "replaying cancel_ask after consumption must fail closed"
    );
    // 极端变体：沿用首次授权（expected_version == 1，落后于存储的 2）也必须 Err。
    assert!(
        answer_ask(&dispatcher, &request_id, &first).is_err(),
        "replaying the stale pre-consumption authorization must fail closed"
    );

    // 存储快照未被重放改写。
    let stored = dispatcher
        .pending_request(&request_id)
        .expect("consumed ask must remain readable");
    assert_eq!(
        stored.state,
        PendingControlRequestState::Consumed,
        "replay attempts must not mutate the terminal state"
    );
    assert_eq!(
        stored.version, consumed_version,
        "replay attempts must not bump the version again"
    );

    // SUT 列表投影：不得复活该请求。
    let asks = list_pending_asks(&dispatcher);
    assert!(
        !asks.iter().any(|ask| ask.request_id == request_id),
        "a consumed ask must never reappear in list_pending_asks (resurrection), \
         but the list contains {request_id}"
    );
    assert!(
        asks.is_empty(),
        "list_pending_asks must be empty after the only ask was consumed, got {:?}",
        asks.iter().map(|ask| &ask.request_id).collect::<Vec<_>>()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// C4-3 时钟超期注入
// ─────────────────────────────────────────────────────────────────────────────

/// C4-3：`FakeClock` 推进到 `expires_at_ms + 1` 后，`answer_ask` / `cancel_ask`
/// 必须 fail-closed（Err）且 `pending_request(id).state` 仍为 `Pending`；
/// 随后 `dispatcher.expire_control_requests(now)` → `list_pending_asks` 必须为空。
///
/// 红相预期：超期 fail-closed 与「不经对账不改状态」成立；`expire_control_requests`
/// 后列表清空**不成立**（该函数把 state 置为 Expired，但列表未按 state 过滤 → 仍列出）。
#[test]
fn test_c4_3_expired_ask_fails_closed_then_expiry_reconciles_the_list() {
    use pony_agent_core::agent::tool_runtime::RuntimeClock;

    let (dispatcher, clock) = ask_dispatcher();
    let (request_id, pending) = persist_ask(&dispatcher, "call-c4-3", "session-c4-3");

    // 基线：未超期时列表可见该请求（证明后续失败不是「本来就列不出来」）。
    let visible = list_pending_asks(&dispatcher);
    assert!(
        visible.iter().any(|ask| ask.request_id == request_id),
        "baseline: the fresh ask must be visible before expiry injection"
    );

    // 注入：推进到 expires_at_ms + 1。
    let expired_now = pending.expires_at_ms + 1;
    clock.set_now_ms(expired_now);
    assert_eq!(clock.now_ms(), expired_now);
    assert!(
        expired_now > pending.expires_at_ms,
        "clock injection must strictly exceed the request TTL"
    );

    let authorization =
        ControlRequestAuthorization::for_request(&pending, Some(serde_json::json!("是")));
    assert!(
        answer_ask(&dispatcher, &request_id, &authorization).is_err(),
        "answering an expired ask must fail closed"
    );
    assert!(
        cancel_ask(&dispatcher, &request_id, &authorization).is_err(),
        "cancelling an expired ask must fail closed"
    );

    // 失败的提交不得改变状态（仅 `expire_control_requests` 可迁移到 Expired）。
    let after_failed = dispatcher
        .pending_request(&request_id)
        .expect("expired-but-unreconciled ask must remain readable");
    assert_eq!(
        after_failed.state,
        PendingControlRequestState::Pending,
        "failed expired submissions must leave state == Pending until explicit reconciliation"
    );
    assert_eq!(
        after_failed.version, 1,
        "failed expired submissions must not bump the version"
    );

    // 宿主对账：显式过期迁移。
    let expired_count = dispatcher.expire_control_requests(expired_now);
    assert_eq!(
        expired_count, 1,
        "exactly the single stale ask must transition to Expired"
    );
    let reconciled = dispatcher
        .pending_request(&request_id)
        .expect("reconciled ask must remain readable");
    assert_eq!(reconciled.state, PendingControlRequestState::Expired);

    // SUT 列表投影：对账后必须清空。
    let asks = list_pending_asks(&dispatcher);
    assert!(
        asks.is_empty(),
        "after expire_control_requests, list_pending_asks must be empty, \
         but it still lists {:?}",
        asks.iter().map(|ask| &ask.request_id).collect::<Vec<_>>()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// C4-4 体量 / 边界
// ─────────────────────────────────────────────────────────────────────────────

/// C4-4：批量 200 个 Ask（各自唯一 call_id）；对偶数序号者 answer/cancel 混合消费。
/// `list_pending_asks` 的 request_id 集合必须**精确等于**仍 Pending 的 100 个
/// （集合相等，非仅长度相等）；且 `pending_requests()` 长度仍为 200
/// （证明过滤发生在列表投影层，而非从存储中删除）。
#[test]
fn test_c4_4_bulk_200_mixed_terminal_states_list_projection_is_exact() {
    let (dispatcher, _clock) = ask_dispatcher();

    let mut persisted = Vec::with_capacity(BATCH);
    for index in 0..BATCH {
        let call_id = format!("call-c4-4-{index:04}");
        let session = format!("session-c4-4-{index:04}");
        let (request_id, pending) = persist_ask(&dispatcher, &call_id, &session);
        assert_eq!(
            pending.state,
            PendingControlRequestState::Pending,
            "every freshly persisted ask must be Pending (index {index})"
        );
        persisted.push((request_id, pending));
    }

    // 全部 200 个请求一开始都应在待答列表中（基线的完整性）。
    let all_pending_ids: BTreeSet<String> = list_pending_asks(&dispatcher)
        .into_iter()
        .map(|ask| ask.request_id)
        .collect();
    assert_eq!(
        all_pending_ids.len(),
        BATCH,
        "all {BATCH} asks must be listed while every one is Pending"
    );

    // 偶数序号者消费：交替 answer / cancel，覆盖两条终态路径。
    let mut consumed_ids = BTreeSet::new();
    for (index, (request_id, _)) in persisted.iter().enumerate() {
        if index % 2 != 0 {
            continue;
        }
        // 授权必须从消费前的实时快照派生（version == 1），故此处按需读取存储。
        let snapshot = dispatcher
            .pending_request(request_id)
            .unwrap_or_else(|| panic!("ask {request_id} must exist before consumption"));
        let authorization =
            ControlRequestAuthorization::for_request(&snapshot, Some(serde_json::json!("是")));
        let result = if (index / 2) % 2 == 0 {
            answer_ask(&dispatcher, request_id, &authorization).map(|consumed| consumed.request.state)
        } else {
            cancel_ask(&dispatcher, request_id, &authorization).map(|consumed| consumed.request.state)
        };
        let state = result.unwrap_or_else(|error| {
            panic!("bulk consumption of {request_id} (index {index}) must succeed: {error}")
        });
        assert!(
            matches!(state, PendingControlRequestState::Consumed | PendingControlRequestState::Cancelled),
            "bulk consumption must land on a terminal state, got {state:?}"
        );
        consumed_ids.insert(request_id.clone());
    }
    assert_eq!(consumed_ids.len(), BATCH / 2, "half of the batch must be consumed");

    // 期望集合：从**消费后**的真实存储判定仍 Pending 的 100 个
    // （不得用消费前的陈旧快照，也不得用待答列表自证）。
    let store = dispatcher.pending_requests();
    let expected_pending_ids: BTreeSet<String> = store
        .iter()
        .filter(|request| request.state == PendingControlRequestState::Pending)
        .map(|request| request.request_id.clone())
        .collect();
    assert_eq!(
        expected_pending_ids.len(),
        BATCH / 2,
        "exactly {BATCH}/2 asks must still be Pending in the post-consumption store snapshot"
    );
    // 交叉校验：本用例消费过的 id 一律不得仍为 Pending。
    for consumed_id in &consumed_ids {
        assert!(
            !expected_pending_ids.contains(consumed_id),
            "consumed ask {consumed_id} must have left the Pending state in the store"
        );
    }

    // SUT 列表投影：集合精确相等（不得多列、不得漏列）。
    let listed_ids: BTreeSet<String> = list_pending_asks(&dispatcher)
        .into_iter()
        .map(|ask| ask.request_id)
        .collect();
    let over_listed: Vec<&String> = listed_ids.difference(&expected_pending_ids).collect();
    let missing: Vec<&String> = expected_pending_ids.difference(&listed_ids).collect();
    assert!(
        over_listed.is_empty(),
        "list_pending_asks must not list terminal requests; over-listed {} id(s), e.g. {:?}",
        over_listed.len(),
        over_listed.iter().take(3).collect::<Vec<_>>()
    );
    assert!(
        missing.is_empty(),
        "list_pending_asks must not drop still-pending requests; missing {} id(s)",
        missing.len()
    );
    assert_eq!(
        listed_ids, expected_pending_ids,
        "the listed request_id set must equal exactly the still-Pending set"
    );

    // 过滤必须发生在投影层：底层存储不得被删除。
    assert_eq!(
        store.len(),
        BATCH,
        "the dispatcher store must still hold all {BATCH} control requests \
         (filtering belongs to the list projection, not to storage deletion)"
    );
    for consumed_id in &consumed_ids {
        let stored = dispatcher
            .pending_request(consumed_id)
            .unwrap_or_else(|| panic!("consumed request {consumed_id} must remain in the store"));
        assert!(
            matches!(stored.state, PendingControlRequestState::Consumed | PendingControlRequestState::Cancelled),
            "consumed request {consumed_id} must keep its terminal state in the store, got {:?}",
            stored.state
        );
    }
}
