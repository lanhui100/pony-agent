//! D2 回归：模型侧 `drain_inbox` 只允许消费**调用者自己**的信箱。
//!
//! 事故背景（2026-11-07 trace）：Lead `drain_inbox(target=dev-tui)` 把派发给 dev-tui 的
//! 任务消息消费掉，被迫重发。修复后工具层按 `context.session_id` 反查调用者身份：
//! 队友会话 ⇒ 该队友；宿主会话 / 无上下文 ⇒ `lead`。跨成员 drain 返回
//! `inbox_scope_violation` 且**不消费**任何消息。
//!
//! 独立测试二进制（独占 OnceLock 引擎槽与 lead 信箱，避免与其它用例串扰）。

use std::sync::Arc;
use std::time::{Duration, Instant};

use pony_agent_core::agent::orchestration::{
    self, list_agents, send_message, teammate_session_id, teammate_target_for_session,
    MessageDeliveryMode, SendMessageArgs, SpawnTeammateArgs, TeammateStatus,
};
use pony_agent_core::agent::subagent_runtime::{
    set_subagent_engine, SubagentRunOutcome, SubagentTurnEngine,
};
use pony_agent_core::agent::tools::{ToolCall, ToolExecutionContext, ToolRouter};
use serde_json::{json, Value};

const STUB_SESSION: &str = "subagent-d2-stub-session";

struct StubEngine;

impl SubagentTurnEngine for StubEngine {
    fn run_turn(&self, _prompt: &str, _timeout_ms: u64) -> Result<SubagentRunOutcome, String> {
        Ok(SubagentRunOutcome {
            final_text: "d2-stub-reply".to_string(),
            session_id: STUB_SESSION.to_string(),
        })
    }
}

fn call(router: &ToolRouter, target: &str, session_id: Option<&str>) -> Value {
    let call = ToolCall {
        call_id: None,
        name: "drain_inbox".to_string(),
        arguments: json!({ "target": target }),
        plan: None,
    };
    let context = ToolExecutionContext {
        session_id: session_id.map(str::to_string),
        ..Default::default()
    };
    let result = router.execute_with_context(&call, &context);
    serde_json::from_str(&result.output).expect("drain_inbox 输出必须是合法 JSON")
}

#[test]
fn d2_tool_drain_is_scoped_to_caller_inbox() {
    // 1) 注入 Stub 引擎并派生一个不自动驱动的队友（空 prompt）。
    let _ = set_subagent_engine(Arc::new(StubEngine) as Arc<dyn SubagentTurnEngine>);
    let name = format!("scope-probe-{}", std::process::id());
    let spawned = orchestration::spawn_teammate(SpawnTeammateArgs {
        name: name.clone(),
        role: Some("tester".to_string()),
        description: "D2 作用域回归".to_string(),
        prompt: String::new(),
        context: None,
        provider_id: None,
        model_id: None,
    })
    .expect("spawn 必须成功");
    let target = spawned.member.target.clone();

    // 2) 派发一条消息触发真实驱动：Stub turn 成功后会话 id 登记进可见性缓存。
    let sent = send_message(SendMessageArgs {
        target: target.clone(),
        message: "d2 dispatch".to_string(),
        mode: Some(MessageDeliveryMode::Queue),
    })
    .expect("派发必须成功");
    assert!(sent.delivered);

    let deadline = Instant::now() + Duration::from_secs(5);
    while teammate_session_id(&target).as_deref() != Some(STUB_SESSION) {
        assert!(Instant::now() < deadline, "驱动必须在 5s 内登记会话 id");
        std::thread::sleep(Duration::from_millis(20));
    }
    // 收口状态：驱动完成后成员回到 Inactive（C7.2）。
    let roster = list_agents().expect("花名册必须可读");
    let member = roster
        .agents
        .iter()
        .find(|member| member.target == target)
        .expect("成员必须在册");
    assert_eq!(member.status, TeammateStatus::Inactive);

    // 3) 反查接口：队友会话 ⇒ 该队友；宿主会话 ⇒ None（调用者回落 lead）。
    assert_eq!(
        teammate_target_for_session(STUB_SESSION).as_deref(),
        Some(target.as_str())
    );
    assert_eq!(teammate_target_for_session("pa_host-session"), None);

    let router = ToolRouter::new();

    // 4) 队友调用者 drain 自己的信箱 ⇒ 允许（驱动已消费，此处应为空且 ok）。
    let self_drain = call(&router, &target, Some(STUB_SESSION));
    assert_eq!(self_drain["ok"], true, "队友 drain 自己的信箱必须允许: {self_drain}");

    // 5) Lead（宿主会话）drain 队友信箱 ⇒ 拒绝且不消费（事故场景的 fail-closed）。
    let cross = call(&router, &target, Some("pa_host-session"));
    assert_eq!(cross["ok"], false);
    assert_eq!(
        cross["error"]["code"], "inbox_scope_violation",
        "跨成员 drain 必须返回作用域错误: {cross}"
    );
    // 无上下文（session_id=None）同样回落 lead ⇒ 跨成员依旧拒绝。
    let cross_no_context = call(&router, &target, None);
    assert_eq!(cross_no_context["error"]["code"], "inbox_scope_violation");

    // 6) Lead drain 自己的信箱 ⇒ 允许，且能读到队友回执（证明步骤 5 未误消费）。
    let lead_drain = call(&router, "lead", Some("pa_host-session"));
    assert_eq!(lead_drain["ok"], true, "lead drain 自己的信箱必须允许: {lead_drain}");
    let messages = lead_drain["result"]["messages"]
        .as_array()
        .expect("messages 必须是数组");
    assert!(
        messages
            .iter()
            .any(|msg| msg["content"].as_str().unwrap_or("").contains("d2-stub-reply")),
        "lead 信箱必须保留队友回执（跨成员拒绝不得消费 lead 信箱）: {lead_drain}"
    );

    // 7) 大小写归一与作用域判定共用同一规则：target="Lead" 的宿主 drain 必须允许。
    let lead_upper = call(&router, "Lead", Some("pa_host-session"));
    assert_eq!(lead_upper["ok"], true, "D1 归一化必须先于作用域判定: {lead_upper}");
}