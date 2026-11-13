//! PA-173 **红相**验收：上游调用策略三道防线（PA-172 收口后的残余缺陷）。
//!
//! 契约真值源：Lead 冻结的 PA-173 契约（2026-11-09）。本文件由独立 Test Agent 冻结；
//! Executor 只读，禁改。
//!
//! - **D1 钳制只读不落盘**：`normalize_storage` 的钳制结果**禁止**写盘（历史事故：用户的
//!   `0/0/0` 被静默改写成 `1000ms/1 次/1000ms` 并落盘，配置被单方面降级）。调用方给什么值
//!   就落什么值；钳制只发生在**解析生效策略**时（`ProviderRegistryStore::upstream_policy()`），
//!   且必须继续落 `upstream_policy_clamped` warn（`provided` / `clamped_to` / `bound` 三字段）。
//! - **D2 下界物理可达**：`UPSTREAM_REQUEST_TIMEOUT_MS_MIN` 1000 → 15000、
//!   `UPSTREAM_DECISION_MAX_ATTEMPTS_MIN` 1 → 2、`UPSTREAM_RETRY_BUDGET_MS_MIN` 1000 → 5000。
//!   依据（实测，非估计）：最小同步请求 1.76s、网关 p50 2.2s、模型 p50 4.67s、p99 36s
//!   ⇒ 1000ms 下界**物理不可达**（任何一次真实完成都不可能落在 1s 内）；attempts 下界 1
//!   等于「合法的零重试」。默认值 45000 / 5 / 30000 与上界 300000 / 10 / 300000 **不动**。
//! - **D3 批量 followup 同等重试**：`send_openai_tool_followup_batch_request`（多工具批量，
//!   公开入口 `continue_with_tool_results`）此前**没有重试层**，1:1 路径有 ⇒ 批量路径单次
//!   5xx 即终态，而 UI 报「上游服务暂时不可用，重试已用尽。」
//!   （`crates/pony-agent-core/src/bin/tui/errors.rs:76`），文案与实际行为脱节。
//!   目标：`max_retries = policy.decision_max_attempts - 1`，退避沿用
//!   `provider_timeout_backoff_config_for`，日志 scope 名 `followup_sync_batch`。
//! - **D3-v2 重试不得污染转录**（契约修订，Lead 裁决）：批量构造器在**每次尝试内**就把
//!   `assistant + N 条 tool_result` 写回 `accumulated_messages`（`provider/mod.rs:5904`
//!   同族），在外面加壳重试会让第 2 次尝试带着「已追加过的转录」发出 ⇒ 上游收到重复的
//!   tool_result / tool_use id（400 或上下文污染）。冻结行为：每次尝试都从 pristine base
//!   重建（第 N 次请求 messages 条数 == 第 1 次），成功后转录只追加 1 次。
//!
//! 判据口径（只锁可观察行为，不锁实现）：
//! 1. D1 的唯一证据是**落盘字节的语义值**（策略段三元组）与**生效策略**的分离；
//! 2. D3 的唯一证据是 mock 端**真实收到的 HTTP 请求计数**（`127.0.0.1:0` 回环 mock），
//!    不读日志、不断言 `attempts_total` 内部字段；
//! 3. D2 的唯一证据是常量与 `clamped()` 的输出，不读私有存储结构。
//!
//! 结构化日志面（`upstream_policy_clamped` 的 provided/clamped_to/bound）是**进程级单例
//! sink**、不可按用例隔离，登记为 non-machine（靠 review）；本文件只锁行为面。
//!
//! 红相预期（本 wave 实现前）：
//! - D1：落盘值被钳制结果覆盖（`0/0/0` 变 `1000/1/1000`）⇒ 断言失败；
//! - D2：三个 MIN 仍是 1000/1/1000 ⇒ 常量断言失败；
//! - D3：批量路径 0 重试 ⇒ `Ok` 与请求计数断言失败。

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use pony_agent_core::agent::config::{
    ProviderModelCapabilities, ProviderRegistryStore, ProviderRegistryView,
    ProviderSelectionResolver, ResolvedProviderSelection, ThinkingParamPattern, UpstreamPolicy,
    UPSTREAM_DECISION_MAX_ATTEMPTS_DEFAULT, UPSTREAM_DECISION_MAX_ATTEMPTS_MAX,
    UPSTREAM_DECISION_MAX_ATTEMPTS_MIN, UPSTREAM_REQUEST_TIMEOUT_MS_DEFAULT,
    UPSTREAM_REQUEST_TIMEOUT_MS_MAX, UPSTREAM_REQUEST_TIMEOUT_MS_MIN, UPSTREAM_RETRY_BUDGET_MS_DEFAULT,
    UPSTREAM_RETRY_BUDGET_MS_MAX, UPSTREAM_RETRY_BUDGET_MS_MIN,
};
use pony_agent_core::agent::provider::{
    ProviderAuthType, ProviderManager, ProviderMessage, ProviderProtocol, ProviderRequest,
};
use pony_agent_core::agent::tools::{ToolCall, ToolDefinition, ToolResult};
use serde_json::{json, Value};

// ── 临时注册表（谁创建谁清理；不污染仓库与用户目录） ────────────────────────────────

struct TempRegistry {
    dir: PathBuf,
}

impl TempRegistry {
    fn new(tag: &str) -> Self {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let unique = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "pa173-policy-guard-{}-{tag}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("PA-173：必须能创建临时配置目录");
        Self { dir }
    }

    fn providers_path(&self) -> PathBuf {
        self.dir.join("providers.json")
    }

    fn store(&self) -> ProviderRegistryStore {
        ProviderRegistryStore::with_path(self.providers_path())
    }

    fn read_json(&self) -> Value {
        let raw = std::fs::read_to_string(self.providers_path())
            .expect("PA-173：必须能读回临时 providers.json");
        serde_json::from_str(&raw).expect("PA-173：落盘的 providers.json 必须是合法 JSON")
    }

    fn write_json(&self, value: &Value) {
        std::fs::write(
            self.providers_path(),
            serde_json::to_string_pretty(value).expect("PA-173：配置 JSON 必须可序列化"),
        )
        .expect("PA-173：必须能写入临时 providers.json");
    }
}

impl Drop for TempRegistry {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 在落盘 JSON 中定位策略段外层键（存储层 snake_case / 视图层 camelCase 均容忍，
/// 不把命名当契约）。
fn locate_policy_key(on_disk: &Value) -> &'static str {
    if on_disk.get("upstream_policy").is_some() {
        "upstream_policy"
    } else if on_disk.get("upstreamPolicy").is_some() {
        "upstreamPolicy"
    } else {
        panic!(
            "PA-173 D1：落盘 providers.json 必须携带策略段（`upstream_policy`/`upstreamPolicy`\
             至少其一），实际顶层键={:?}",
            on_disk
                .as_object()
                .map(|object| object.keys().cloned().collect::<Vec<_>>())
        )
    }
}

/// 把策略段内**三个字段全部置 0**（不假定内层字段命名：按段内现有键逐个改写）。
fn zero_policy_fields(on_disk: &mut Value) {
    let key = locate_policy_key(on_disk);
    let policy = match on_disk.get_mut(key).and_then(Value::as_object_mut) {
        Some(policy) => policy,
        None => panic!("PA-173 D1：策略段 `{key}` 必须是 JSON 对象"),
    };
    assert_eq!(
        policy.len(),
        3,
        "PA-173 D1：策略段必须恰有 3 个字段（requestTimeoutMs/decisionMaxAttempts/\
         retryBudgetMs），实际 {:?}",
        policy.keys().collect::<Vec<_>>()
    );
    for value in policy.values_mut() {
        *value = json!(0);
    }
}

/// 从落盘 JSON 读出策略三元组 `(requestTimeoutMs, decisionMaxAttempts, retryBudgetMs)`。
///
/// 内层字段命名容忍 camelCase / snake_case（按语义子串归类），避免把存储层命名当契约。
fn policy_triple_on_disk(on_disk: &Value) -> (u64, u64, u64) {
    let key = locate_policy_key(on_disk);
    let policy = match on_disk.get(key).and_then(Value::as_object) {
        Some(policy) => policy,
        None => panic!("PA-173 D1：策略段 `{key}` 必须是 JSON 对象"),
    };
    let mut timeout = None;
    let mut attempts = None;
    let mut budget = None;
    for (field, value) in policy {
        let normalized = field.to_ascii_lowercase().replace('_', "");
        let number = value.as_u64().unwrap_or_else(|| {
            panic!("PA-173 D1：策略字段 `{field}` 必须是 u64 整数（禁止字符串/浮点），实际 {value}")
        });
        if normalized.contains("timeout") {
            timeout = Some(number);
        } else if normalized.contains("attempt") {
            attempts = Some(number);
        } else if normalized.contains("budget") {
            budget = Some(number);
        } else {
            panic!("PA-173 D1：策略段出现契约外字段 `{field}`");
        }
    }
    (
        timeout.unwrap_or_else(|| panic!("PA-173 D1：策略段缺少请求超时字段，实际 {policy:?}")),
        attempts.unwrap_or_else(|| panic!("PA-173 D1：策略段缺少尝试次数字段，实际 {policy:?}")),
        budget.unwrap_or_else(|| panic!("PA-173 D1：策略段缺少退避预算字段，实际 {policy:?}")),
    )
}

/// 保存一份含指定策略的视图（走 store 自己的保存路径落盘）。
fn save_policy_view(
    store: &ProviderRegistryStore,
    policy: Value,
) -> ProviderRegistryView {
    let mut view_json =
        serde_json::to_value(store.load_view()).expect("PA-173：ProviderRegistryView 必须可序列化");
    view_json["upstreamPolicy"] = policy;
    let view: ProviderRegistryView = serde_json::from_value(view_json)
        .expect("PA-173：注入策略后的视图 JSON 必须可反序列化");
    store
        .save_view_without_env_sync(view)
        .expect("PA-173：保存视图必须成功")
}

// ── D1：钳制只读不落盘 ──────────────────────────────────────────────────────────

/// D1（缺陷锚点）：`0/0/0` 经「读出 → 保存」往返后，**落盘仍是 0/0/0**；
/// 而生效策略 `upstream_policy()` 必须被钳到新下界 `(15000, 2, 5000)`。
///
/// 这正是历史事故的复现路径：用户把三项都填 0，保存时 `normalize_storage` 先把越界值
/// 钳成合法值再落盘 ⇒ 用户配置被静默降级，且再也回不到他填的值。
#[test]
fn d1_clamped_policy_is_not_persisted_but_still_takes_effect() {
    // ── Arrange ────────────────────────────────────────────────────────────────
    // 先落一份**合法非默认**策略（保证策略段真在盘上），再把三个字段全部改写为 0。
    let sandbox = TempRegistry::new("d1-no-persist");
    let store = sandbox.store();
    save_policy_view(
        &store,
        json!({
            "requestTimeoutMs": 60_000,
            "decisionMaxAttempts": 7,
            "retryBudgetMs": 12_345
        }),
    );
    let mut on_disk = sandbox.read_json();
    zero_policy_fields(&mut on_disk);
    sandbox.write_json(&on_disk);
    let disk_before_round_trip = sandbox.read_json();
    assert_eq!(
        policy_triple_on_disk(&disk_before_round_trip),
        (0, 0, 0),
        "PA-173 D1 夹具自检：Act 前盘上策略必须已是 0/0/0"
    );

    // ── Act ────────────────────────────────────────────────────────────────────
    // 用户/前端视角的完整保存路径：读出视图 → 原样保存回去。
    let reload_store = sandbox.store();
    let reloaded_view = reload_store.load_view();
    reload_store
        .save_view_without_env_sync(reloaded_view)
        .expect("PA-173 D1：越界策略的保存必须成功（越界不是致命错误，禁止因此吞掉整份配置）");
    let disk_after_round_trip = sandbox.read_json();

    // ── Assert ─────────────────────────────────────────────────────────────────
    // A1：落盘必须与调用方给的值逐字段一致（`0/0/0`），不得被钳制结果覆盖。
    assert_eq!(
        policy_triple_on_disk(&disk_after_round_trip),
        (0, 0, 0),
        "PA-173 D1：钳制结果**禁止**落盘——用户填的 0/0/0 保存后必须仍是 0/0/0，\
         实际落盘被改写成 {:?}",
        policy_triple_on_disk(&disk_after_round_trip)
    );

    // A2：生效策略仍然被钳制到新下界（钳制只读不落盘 ⇒ 语义不能退化成「不钳制」）。
    let effective = sandbox.store().upstream_policy();
    assert_eq!(
        (
            effective.request_timeout_ms,
            u64::from(effective.decision_max_attempts),
            effective.retry_budget_ms
        ),
        (15_000, 2, 5_000),
        "PA-173 D1：生效策略必须在解析时钳到 (15000, 2, 5000)，实际 ({}, {}, {})",
        effective.request_timeout_ms,
        effective.decision_max_attempts,
        effective.retry_budget_ms
    );
}

/// D1 反向钉子（防过度修复）：**合法**策略 `(45000, 10, 60000)`（= 2026-11-14 起的默认值）
/// 走带 env-sync 的公开保存入口（`save_view`）往返后，盘上逐字段不变，生效策略等于该值且
/// `is_default()`。
#[test]
fn d1_legal_policy_round_trips_unchanged() {
    // ── Arrange ────────────────────────────────────────────────────────────────
    let sandbox = TempRegistry::new("d1-legal-roundtrip");
    let store = sandbox.store();
    let mut view_json =
        serde_json::to_value(store.load_view()).expect("PA-173：ProviderRegistryView 必须可序列化");
    view_json["upstreamPolicy"] = json!({
        "requestTimeoutMs": 45_000,
        "decisionMaxAttempts": 10,
        "retryBudgetMs": 60_000
    });
    let view: ProviderRegistryView =
        serde_json::from_value(view_json).expect("PA-173：视图 JSON 必须可反序列化");

    // ── Act 1：走 `save_view`（带 env-sync 的另一条保存路径）落盘。 ────────────────
    store
        .save_view(view)
        .expect("PA-173 D1：合法策略保存必须成功");
    let first_write = sandbox.read_json();
    assert_eq!(
        policy_triple_on_disk(&first_write),
        (45_000, 10, 60_000),
        "PA-173 D1：合法策略必须原样落盘"
    );

    // ── Act 2：再往返一次（读 → 存），确认无漂移。 ────────────────────────────────
    let round_trip_store = sandbox.store();
    let reloaded = round_trip_store.load_view();
    round_trip_store
        .save_view(reloaded)
        .expect("PA-173 D1：合法策略二次保存必须成功");
    let second_write = sandbox.read_json();

    // ── Assert ─────────────────────────────────────────────────────────────────
    assert_eq!(
        policy_triple_on_disk(&second_write),
        (45_000, 10, 60_000),
        "PA-173 D1：合法策略往返后必须逐字段不变（防「修复钳制时顺手改写合法值」）"
    );
    let effective = sandbox.store().upstream_policy();
    assert_eq!(
        (
            effective.request_timeout_ms,
            effective.decision_max_attempts,
            effective.retry_budget_ms
        ),
        (45_000, 10, 60_000),
        "PA-173 D1：合法策略的生效值必须等于用户值"
    );
    assert!(
        effective.is_default(),
        "PA-173 D1：合法默认策略必须判 `is_default()`（G2 默认零行为变更）"
    );
}

// ── D2：三个下界物理可达 ────────────────────────────────────────────────────────

/// D2（缺陷锚点）：三个 `*_MIN` 必须抬到物理可达区间，且默认值 / 上界**一字不动**。
///
/// 依据（实测，非估计）：最小同步请求 1.76s、网关 p50 2.2s、模型 p50 4.67s、p99 36s。
/// 1000ms 下界意味着「配置成 1s 的请求必然先超时再谈完成」——物理不可达；
/// attempts=1 允许「合法的零重试」，用户配置即等于关闭重试，而 UI 仍宣称会重试。
#[test]
fn d2_policy_floors_are_physically_reachable() {
    // A1：三个下界（D2 的唯一变更面）。
    assert_eq!(
        UPSTREAM_REQUEST_TIMEOUT_MS_MIN, 15_000,
        "PA-173 D2：请求超时下界必须 ≥ 实测最小完成时间的数量级（1.76s），实际 {}",
        UPSTREAM_REQUEST_TIMEOUT_MS_MIN
    );
    assert_eq!(
        UPSTREAM_DECISION_MAX_ATTEMPTS_MIN, 2,
        "PA-173 D2：尝试次数下界必须是 2（1 = 合法的零重试），实际 {}",
        UPSTREAM_DECISION_MAX_ATTEMPTS_MIN
    );
    assert_eq!(
        UPSTREAM_RETRY_BUDGET_MS_MIN, 5_000,
        "PA-173 D2：退避预算下界必须 ≥ 首轮指数退避的可用区间，实际 {}",
        UPSTREAM_RETRY_BUDGET_MS_MIN
    );

    // A2：默认值不动（G2 零行为变更）。
    assert_eq!(
        (
            UPSTREAM_REQUEST_TIMEOUT_MS_DEFAULT,
            UPSTREAM_DECISION_MAX_ATTEMPTS_DEFAULT,
            UPSTREAM_RETRY_BUDGET_MS_DEFAULT
        ),
        (45_000, 10, 60_000),
        "PA-173 D2：下界上调**不得**动默认值；2026-11-14 默认值另行上调为 45000 / 10 / 60000"
    );

    // A3：上界不动。
    assert_eq!(
        (
            UPSTREAM_REQUEST_TIMEOUT_MS_MAX,
            UPSTREAM_DECISION_MAX_ATTEMPTS_MAX,
            UPSTREAM_RETRY_BUDGET_MS_MAX
        ),
        (300_000, 10, 300_000),
        "PA-173 D2：下界上调**不得**动上界（300000 / 10 / 300000）"
    );

    // A4：全 0 配置必须被钳到**新**下界，且三条越界都要报（禁止静默）。
    let (clamped, fields) = UpstreamPolicy {
        request_timeout_ms: 0,
        decision_max_attempts: 0,
        retry_budget_ms: 0,
    }
    .clamped();
    assert_eq!(
        (
            clamped.request_timeout_ms,
            clamped.decision_max_attempts,
            clamped.retry_budget_ms
        ),
        (15_000, 2, 5_000),
        "PA-173 D2：0/0/0 必须钳到新下界 (15000, 2, 5000)，实际 ({}, {}, {})",
        clamped.request_timeout_ms,
        clamped.decision_max_attempts,
        clamped.retry_budget_ms
    );
    assert_eq!(
        fields.len(),
        3,
        "PA-173 D2：三个字段全部越界 ⇒ 必须报 3 条钳制，实际 {fields:?}"
    );

    // A5：下界本身合法（边界内不得误报钳制）。
    let (legal_floor, floor_fields) = UpstreamPolicy {
        request_timeout_ms: 15_000,
        decision_max_attempts: 2,
        retry_budget_ms: 5_000,
    }
    .clamped();
    assert_eq!(
        legal_floor,
        UpstreamPolicy {
            request_timeout_ms: 15_000,
            decision_max_attempts: 2,
            retry_budget_ms: 5_000,
        },
        "PA-173 D2：恰好等于下界的配置必须原样保留"
    );
    assert!(
        floor_fields.is_empty(),
        "PA-173 D2：边界内不得报钳制，实际 {floor_fields:?}"
    );
}

// ── 回环 mock provider（D3 的唯一证据面：真实到达的 HTTP 请求计数） ──────────────────

/// 503 报文：码表 `UPSTREAM_SERVER_ERROR_HINTS`（`502/503/504/408`）⇒
/// `FailureKind::TransientRetryable`。**不得**含 `quota` / `rate limit` / `429` 等
/// 会改变分类的词（否则会走冷却分支，重试被短路）。
const SERVER_ERROR_BODY: &str =
    "{\"error\":{\"type\":\"server_error\",\"message\":\"upstream temporarily unavailable\"}}";

/// 200 成功报文：`first_openai_message` 契约 + `tool_calls`（批量回填的可解析尾巴）。
const BATCH_OK_BODY: &str = "{\"id\":\"pa173-batch-ok\",\"choices\":[{\"index\":0,\"message\":{\"role\":\"assistant\",\"content\":\"pa173-batch-ok\",\"tool_calls\":[{\"id\":\"call-pa173-1\",\"type\":\"function\",\"function\":{\"name\":\"todo_write\",\"arguments\":\"{\\\"todos\\\":[]}\"}}]},\"finish_reason\":\"tool_calls\"}],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":2,\"total_tokens\":7}}";

struct MockReply {
    status: u16,
    reason: &'static str,
    body: &'static str,
    content_type: &'static str,
}

/// 按剧本应答的 mock provider：第 N 个请求取第 N 条应答，超出后**重复最后一条**。
/// `requests` 统计真实到达的 HTTP 请求数——D3 的「同等重试」只能靠它证明。
/// `message_counts` 记录每次真实请求**请求体**里 `messages` 数组的条数（D3-v2「重试不得
/// 污染转录」的唯一证据面：不读被测进程内部变量，只读线上真实字节）。
struct MockProvider {
    base_url: String,
    addr: String,
    requests: Arc<AtomicUsize>,
    message_counts: Arc<std::sync::Mutex<Vec<usize>>>,
    shutdown: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl MockProvider {
    fn script(replies: Vec<MockReply>) -> Self {
        assert!(!replies.is_empty(), "PA-173：mock 剧本不得为空");
        let listener = TcpListener::bind("127.0.0.1:0")
            .expect("PA-173：mock provider 必须能绑定回环端口");
        let port = listener
            .local_addr()
            .expect("PA-173：mock provider 必须能取本地地址")
            .port();
        let requests = Arc::new(AtomicUsize::new(0));
        let message_counts = Arc::new(std::sync::Mutex::new(Vec::new()));
        let shutdown = Arc::new(AtomicBool::new(false));
        let requests_for_thread = Arc::clone(&requests);
        let message_counts_for_thread = Arc::clone(&message_counts);
        let shutdown_for_thread = Arc::clone(&shutdown);

        let handle = std::thread::spawn(move || {
            while !shutdown_for_thread.load(Ordering::SeqCst) {
                let Ok((mut stream, _addr)) = listener.accept() else {
                    return;
                };
                // 读完请求（含 body）再应答，避免客户端写一半就被 close。
                let body = read_http_request(&mut stream);
                // 唤醒用的空连接（Drop 里发起）不得计入真实请求数。
                if shutdown_for_thread.load(Ordering::SeqCst) {
                    return;
                }
                let index = requests_for_thread.fetch_add(1, Ordering::SeqCst);
                // 捕获真实请求体的 `messages` 条数（畸形/空体不记账，避免计数器与应答错位）。
                if let Ok(payload) = serde_json::from_str::<Value>(&body) {
                    if let Some(messages) = payload.get("messages").and_then(Value::as_array) {
                        message_counts_for_thread
                            .lock()
                            .expect("PA-173：mock 请求体捕获锁不得中毒")
                            .push(messages.len());
                    }
                }
                debug_assert_eq!(
                    message_counts_for_thread
                        .lock()
                        .expect("PA-173：mock 请求体捕获锁不得中毒")
                        .len(),
                    index + 1,
                    "PA-173：真实请求计数与请求体捕获必须一一对应"
                );
                let reply = &replies[index.min(replies.len() - 1)];
                let mut head = format!(
                    "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    reply.status,
                    reply.reason,
                    reply.content_type,
                    reply.body.as_bytes().len()
                );
                head.push_str(reply.body);
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.flush();
            }
        });

        Self {
            base_url: format!("http://127.0.0.1:{port}/v1"),
            addr: format!("127.0.0.1:{port}"),
            requests,
            message_counts,
            shutdown,
            handle: Some(handle),
        }
    }

    fn request_count(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }

    /// 每次真实请求的 `messages` 条数（按到达顺序）。
    fn message_counts(&self) -> Vec<usize> {
        self.message_counts
            .lock()
            .expect("PA-173：mock 请求体捕获锁不得中毒")
            .clone()
    }

    fn selection(&self, provider_name: &str) -> ResolvedProviderSelection {
        provider_selection(self.base_url.clone(), provider_name)
    }
}

impl Drop for MockProvider {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        // 空连接唤醒 accept，令 mock 线程及时退出。
        let _ = TcpStream::connect(self.addr.clone());
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn read_http_request(stream: &mut TcpStream) -> String {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    let mut header_end: Option<usize> = None;
    let mut content_length = 0usize;
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => buffer.extend_from_slice(&chunk[..read]),
            Err(_) => break,
        }
        if header_end.is_none() {
            if let Some(index) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
                header_end = Some(index + 4);
                let headers = String::from_utf8_lossy(&buffer[..index + 4]).to_lowercase();
                for line in headers.lines() {
                    if let Some(value) = line.strip_prefix("content-length:") {
                        content_length = value.trim().parse::<usize>().unwrap_or(0);
                    }
                }
            }
        }
        if let Some(header_end) = header_end {
            if buffer.len().saturating_sub(header_end) >= content_length {
                break;
            }
        }
    }
    let body_start = header_end.unwrap_or(buffer.len());
    String::from_utf8_lossy(&buffer[body_start..]).to_string()
}

fn provider_selection(base_url: String, provider_name: &str) -> ResolvedProviderSelection {
    ResolvedProviderSelection {
        requested_name: provider_name.to_string(),
        provider_name: provider_name.to_string(),
        protocol: ProviderProtocol::OpenAiCompletions,
        base_url,
        auth_type: ProviderAuthType::Auto,
        api_key_env_var: "PA173_BATCH_RETRY_TEST_API_KEY".to_string(),
        api_key: Some("pa173-dummy-key".to_string()),
        model: format!("{provider_name}-model"),
        temperature: 0.0,
        max_output_tokens: 64,
        reasoning_effort: None,
        reasoning_budget_tokens: None,
        capabilities: ProviderModelCapabilities {
            context_window_tokens: Some(8_192),
            supports_tools: true,
            supports_streaming: true,
            supports_image_input: false,
            supports_reasoning: false,
            ..Default::default()
        },
        thinking_param_pattern: ThinkingParamPattern::EffortStandard,
    }
}

fn request_probe() -> ProviderRequest {
    ProviderRequest {
        model: "pa173-batch-model".to_string(),
        input: vec![ProviderMessage::user("pa173 batch probe")],
        images: Vec::new(),
        native_messages: Vec::new(),
        observation: Default::default(),
        temperature: 0.0,
        max_output_tokens: 64,
    }
}

fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "todo_write",
            description: "write todos",
            input_schema: json!({"type": "object"}),
        },
        ToolDefinition {
            name: "read_file",
            description: "read a file",
            input_schema: json!({"type": "object"}),
        },
    ]
}

/// 两个工具调用 + 两个结果（N>1 才会进入**批量**回填通道）。
fn batch_calls_and_results() -> (Vec<ToolCall>, Vec<ToolResult>) {
    let calls = vec![
        ToolCall {
            call_id: Some("call-pa173-batch-a".to_string()),
            name: "todo_write".to_string(),
            arguments: json!({"todos": []}),
            plan: None,
        },
        ToolCall {
            call_id: Some("call-pa173-batch-b".to_string()),
            name: "read_file".to_string(),
            arguments: json!({"path": "README.md"}),
            plan: None,
        },
    ];
    let results = vec![
        ToolResult {
            tool_name: "todo_write".to_string(),
            status: "ok".to_string(),
            output: "todos written".to_string(),
            duration_ms: 3,
        },
        ToolResult {
            tool_name: "read_file".to_string(),
            status: "ok".to_string(),
            output: "pa173 content".to_string(),
            duration_ms: 5,
        },
    ];
    (calls, results)
}

fn transient_503_reply() -> MockReply {
    MockReply {
        status: 503,
        reason: "Service Unavailable",
        body: SERVER_ERROR_BODY,
        content_type: "application/json",
    }
}

/// 网关常见形态：**空正文**的 503/502（`Content-Length: 0`）。
/// 用于证明耗尽错误在空正文下也不得降级成空串。
fn empty_body_503_reply() -> MockReply {
    MockReply {
        status: 503,
        reason: "Service Unavailable",
        body: "",
        content_type: "application/json",
    }
}

fn ok_reply() -> MockReply {
    MockReply {
        status: 200,
        reason: "OK",
        body: BATCH_OK_BODY,
        content_type: "application/json",
    }
}

/// 驱动**批量**（N=2）同步 followup 公开入口，并返回调用方可见的转录
/// `accumulated_messages`（D3-v2「重试不得污染转录」的断言口径）。
fn drive_batch_followup_with_transcript(
    mock: &MockProvider,
    provider_name: &str,
    decision_max_attempts: u32,
) -> (
    Result<pony_agent_core::agent::provider::ProviderResponse, String>,
    Vec<Value>,
) {
    let manager = ProviderManager::new(mock.selection(provider_name)).with_policy(UpstreamPolicy {
        decision_max_attempts,
        ..UpstreamPolicy::default()
    });
    let request = request_probe();
    let tools = tool_definitions();
    let (calls, results) = batch_calls_and_results();
    let mut accumulated_messages: Vec<Value> = Vec::new();
    let result = manager.continue_with_tool_results(
        &request,
        &tools,
        &mut accumulated_messages,
        None,
        &calls,
        &results,
    );
    (result, accumulated_messages)
}

/// 驱动**批量**（N=2）同步 followup 公开入口（只关心结果面）。
fn drive_batch_followup(
    mock: &MockProvider,
    provider_name: &str,
    decision_max_attempts: u32,
) -> Result<pony_agent_core::agent::provider::ProviderResponse, String> {
    drive_batch_followup_with_transcript(mock, provider_name, decision_max_attempts).0
}

// ── D3：批量 followup 必须带重试 ────────────────────────────────────────────────

/// D3 夹具自检（**现在必须绿**，防「红相因夹具写错才红」）：200 一次成功 ⇒
/// 恰好 1 次真实 HTTP 且 `Ok`。若本用例红了，说明 mock/请求体/解析尾巴写错，
/// 其余 D3 断言的红相无效。
#[test]
fn d3_batch_followup_success_path_is_wired() {
    // Arrange
    let mock = MockProvider::script(vec![ok_reply()]);

    // Act
    let result = drive_batch_followup(&mock, "pa173-batch-happy", 2);
    let hits = mock.request_count();

    // Assert
    let response = result.expect("PA-173 D3：200 应答必须一次成功（夹具自检）");
    assert_eq!(hits, 1, "PA-173 D3：200 一次成功 ⇒ 恰好 1 次真实 HTTP，实际 {hits} 次");
    assert_eq!(
        response.output_text, "pa173-batch-ok",
        "PA-173 D3：批量回填必须产出真实文本，实际 {:?}",
        response.output_text
    );
}

/// D3（缺陷锚点）：批量 followup 必须与 1:1 路径**同等重试**——
/// 首次 503（transient）后第二次 200 ⇒ `Ok`，且 mock 端**观察到 2 次请求**。
///
/// 现状缺陷：批量路径零重试层 ⇒ 首次 503 即终态（1 次请求 + `Err`），
/// 而 UI 文案却是「重试已用尽」。
#[test]
fn d3_batch_followup_retries_transient_failure_then_succeeds() {
    // ── Arrange ────────────────────────────────────────────────────────────────
    let mock = MockProvider::script(vec![transient_503_reply(), ok_reply()]);
    // attempts=2 ⇒ max_retries=1（含首次共 2 次），退避 400ms。
    let attempts = 2;

    // ── Act ────────────────────────────────────────────────────────────────────
    let result = drive_batch_followup(&mock, "pa173-batch-retry", attempts);
    let hits = mock.request_count();

    // ── Assert ─────────────────────────────────────────────────────────────────
    assert_eq!(
        hits, 2,
        "PA-173 D3：首次 503 必须触发**恰好** 1 次重发（共 2 次真实 HTTP）——批量路径\
         必须与 1:1 路径同等重试，实际 {hits} 次"
    );
    let response = result.expect(
        "PA-173 D3：首次 503 后第二次 200 必须返回 Ok（禁止把 transient 单次失败当终态）",
    );
    assert_eq!(
        response.output_text, "pa173-batch-ok",
        "PA-173 D3：重试成功后必须返回第二次的真实产出，实际 {:?}",
        response.output_text
    );
    assert!(
        !response.tool_calls.is_empty(),
        "PA-173 D3：重试成功后必须保留整批 tool_calls（不得因重试丢字段）"
    );
}

/// D3-v2（契约修订，缺陷锚点）：批量重试**不得污染转录**。
///
/// 缺陷机制：批量构造器在**每次尝试内**就把「assistant + N 条 tool_result」写回
/// `accumulated_messages`（`provider/mod.rs:5904` 同族：`*accumulated_messages = messages`）。
/// 直接在外面加壳重试 ⇒ 第 2 次尝试的 base 已是「追加过的转录」，上游收到**重复的
/// tool_result / tool_use id**（可能 400 或污染上下文）——这是加重试后的新回归。
///
/// 冻结行为（口径 = mock 端捕获的**真实请求体**，不断言内部变量）：
/// 1. 每次尝试必须从 pristine base 重建 ⇒ 第 N 次请求与第 1 次请求的 `messages` 条数**相等**；
/// 2. 成功返回后 `accumulated_messages` 恰好 = base + **1 次**追加（不得为 N 次追加）。
///
/// 夹具基数：pristine base = 1 条 user；每次尝试追加 1 assistant + 2 tool ⇒ 每次请求 4 条。
#[test]
fn d3_batch_retry_does_not_duplicate_transcript() {
    // ── Arrange ────────────────────────────────────────────────────────────────
    let mock = MockProvider::script(vec![transient_503_reply(), ok_reply()]);
    let attempts = 2;
    const PRISTINE_BASE: usize = 1;
    const ONE_APPEND: usize = 1 + 2; // 1 assistant + N(=2) tool_result
    let expected_messages = PRISTINE_BASE + ONE_APPEND;

    // ── Act ────────────────────────────────────────────────────────────────────
    let (result, transcript) =
        drive_batch_followup_with_transcript(&mock, "pa173-batch-transcript", attempts);
    let counts = mock.message_counts();

    // ── Assert ─────────────────────────────────────────────────────────────────
    // A1：必须先有 2 次真实请求，否则「第 N 次 vs 第 1 次」无从比较。
    assert_eq!(
        counts.len(),
        2,
        "PA-173 D3-v2：transient 失败后必须重发 1 次（共 2 次真实请求），\
         否则无法验证转录是否被污染，实际各次 messages 条数={counts:?}"
    );
    // A2：重试请求必须与首次请求携带**相同条数**（从 pristine base 重建，不得单调增长）。
    assert_eq!(
        counts[1], counts[0],
        "PA-173 D3-v2：重试不得污染转录——第 2 次请求 messages 必须与第 1 次相同\
         （第 1 次 {} 条 / 第 2 次 {} 条；相等 = 每次 attempt 都从 pristine base 重建）",
        counts[0], counts[1]
    );
    assert_eq!(
        counts[0], expected_messages,
        "PA-173 D3-v2：第 1 次请求 messages 必须 = base({PRISTINE_BASE}) + 1 assistant + 2 tool\
         = {expected_messages}，实际 {}",
        counts[0]
    );

    // A3：请求确实成功，且调用方可见转录**只追加一次**（不得为 N 次追加 = {expected_messages}
    // + 3 = 7 条）。
    let response = result.expect(
        "PA-173 D3-v2：首次 503 后第二次 200 必须返回 Ok（重试成功是转录断言的前提）",
    );
    assert_eq!(
        response.output_text, "pa173-batch-ok",
        "PA-173 D3-v2：重试成功后必须返回真实产出，实际 {:?}",
        response.output_text
    );
    let duplicated = expected_messages + ONE_APPEND;
    assert_eq!(
        transcript.len(),
        expected_messages,
        "PA-173 D3-v2：成功返回后 `accumulated_messages` 必须恰好 = base + 1 次追加\
         （{expected_messages} 条），实际 {} 条（若为 {duplicated} 条 ⇒ 每次都追加，转录被重复污染）",
        transcript.len()
    );
}

/// D3 反向钉子：重试次数必须由 `decision_max_attempts` **精确**有界，
/// 不得无限重试（`5 ⇒ 5 次请求后 Err`，`2 ⇒ 恰好 2 次`）。
#[test]
fn d3_batch_followup_retry_is_bounded_by_decision_max_attempts() {
    for (attempts, expected_hits) in [(5_u32, 5_usize), (2_u32, 2_usize)] {
        // ── Arrange：恒 503（每次 attempt 都失败）。 ──────────────────────────────
        let mock = MockProvider::script(vec![transient_503_reply()]);

        // ── Act ────────────────────────────────────────────────────────────────
        let result = drive_batch_followup(&mock, &format!("pa173-batch-bounded-{attempts}"), attempts);
        let hits = mock.request_count();

        // ── Assert ─────────────────────────────────────────────────────────────
        let error = result.expect_err(
            "PA-173 D3：恒 503 必须返回 Err（禁止把失败改写成成功），attempts={attempts}",
        );
        assert!(
            !error.trim().is_empty(),
            "PA-173 D3：失败必须带可见原因（禁止空错误），attempts={attempts}"
        );
        // 契约修订 v3：耗尽错误**不得**被降级成「只剩响应正文」——
        // 原始错误的状态码必须保留（否则用户只看到 JSON 正文，看不到 503/耗时）。
        assert!(
            error.contains("503"),
            "PA-173 D3-v3：耗尽错误必须保留原始状态码 `503`（禁止降级为纯响应正文），\
             attempts={attempts}，实际错误：{error}"
        );
        assert_eq!(
            hits, expected_hits,
            "PA-173 D3：decision_max_attempts={attempts} ⇒ 必须恰好 {expected_hits} 次真实 HTTP\
             （含首次），实际 {hits} 次"
        );
    }
}

/// D3-v3（契约修订，缺陷锚点）：**空正文**的上游失败（网关常见 502/504/503 空体）
/// 耗尽后**不得**返回空错误串。
///
/// 缺陷机制：耗尽时返回 `extract_provider_error_detail(...)`，只取「响应正文：」之后
/// 的预览；空正文 ⇒ 预览为 `""` ⇒ 调用方拿到 `Err("")`，UI 只能显示空白/通用文案，
/// 真实失败原因（状态码、耗时、endpoint）全部丢失。
#[test]
fn d3_batch_failure_never_returns_empty_error_on_empty_body() {
    // ── Arrange：恒「空正文 503」。 ─────────────────────────────────────────────
    let mock = MockProvider::script(vec![empty_body_503_reply()]);
    let attempts = 2;

    // ── Act ────────────────────────────────────────────────────────────────────
    let result = drive_batch_followup(&mock, "pa173-batch-empty-body", attempts);
    let hits = mock.request_count();

    // ── Assert ─────────────────────────────────────────────────────────────────
    let error = result.expect_err("PA-173 D3-v3：恒 503（空正文）必须返回 Err，禁止伪造成功");
    assert_eq!(
        hits, 2,
        "PA-173 D3-v3：前置事实——空正文 503 同样要重试（attempts=2 ⇒ 2 次真实 HTTP），实际 {hits} 次"
    );
    assert!(
        !error.trim().is_empty(),
        "PA-173 D3-v3：空正文下错误串**不得**为空（必须至少保留状态码/耗时等本地事实），实际：{error:?}"
    );
    assert!(
        error.contains("503"),
        "PA-173 D3-v3：空正文下错误串必须保留原始状态码 `503`，实际：{error:?}"
    );
}

