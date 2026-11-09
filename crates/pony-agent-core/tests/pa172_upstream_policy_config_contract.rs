//! PA-172 **P1/P2 红相**验收：上游调用策略配置化（`UpstreamPolicy`）+ 钳制语义 +
//! 候选链卫生 + 配置 round-trip 不丢字段 + 类型错乱容错。
//!
//! 契约真值源：`.dev-team/contract-matrix-pa172.md`（v2 定稿）
//! - §2.1 冻结签名（9 个 `UPSTREAM_*` 常量 / `UpstreamPolicy` / `clamped` / `is_default`）；
//! - §2.2 wire schema（视图层 `upstreamPolicy` + `failoverProviderIds`，additive）；
//! - §2.3 钳制真值表（13 行：11 行字段级钳制 + 2 行 wire 默认）；
//! - C2-a 非致命 / C2-c 候选链卫生 / C2-d round-trip；
//! - §1 G2「默认零行为变更」：未配置时 45000 / 5 / 30000 与基线逐字一致。
//!
//! 本文件由独立 Test Agent 冻结；Executor 只读，禁改。
//!
//! **落地策略（重要）**：`providers.json` 的**存储层**是 snake_case（实测
//! `~/.config/pony-agent/providers.json`：`base_url` / `api_key_env_var` / `secret_ref`），
//! 而**视图层** `ProviderRegistryView` 是 camelCase（C2-d 冻结 `upstreamPolicy` /
//! `failoverProviderIds`）。故本文件**不手写 provider 条目**：
//! 一律「默认视图 JSON（camelCase）→ 注入字段 → `ProviderRegistryView` → 走 store 自己的
//! 保存路径落盘 → 再读回」，避免把存储层命名当成契约（那是实现细节）。
//!
//! 红相预期（wave-69 实现前）：`UpstreamPolicy` / `UPSTREAM_*` 常量 / `resolve_failover_chain`
//! 不存在 ⇒ 编译红（E0432/E0433），属冻结签名锚点的合法红相。

use std::path::PathBuf;

use pony_agent_core::agent::config::{
    ProviderRegistryStore, ProviderRegistryView, ProviderSelectionResolver, UpstreamPolicy,
    UPSTREAM_DECISION_MAX_ATTEMPTS_DEFAULT, UPSTREAM_DECISION_MAX_ATTEMPTS_MAX,
    UPSTREAM_DECISION_MAX_ATTEMPTS_MIN, UPSTREAM_REQUEST_TIMEOUT_MS_DEFAULT,
    UPSTREAM_REQUEST_TIMEOUT_MS_MAX, UPSTREAM_REQUEST_TIMEOUT_MS_MIN, UPSTREAM_RETRY_BUDGET_MS_DEFAULT,
    UPSTREAM_RETRY_BUDGET_MS_MAX, UPSTREAM_RETRY_BUDGET_MS_MIN,
};
use serde_json::{json, Value};

// ── 临时目录（谁创建谁清理；不污染仓库与用户目录） ────────────────────────────────

struct TempRegistry {
    dir: PathBuf,
}

impl TempRegistry {
    fn new(tag: &str) -> Self {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let unique = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "pa172-config-{}-{tag}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("PA-172：必须能创建临时配置目录");
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
            .expect("PA-172：必须能读回临时 providers.json");
        serde_json::from_str(&raw).expect("PA-172：落盘的 providers.json 必须是合法 JSON")
    }

    fn write_json(&self, value: &Value) {
        std::fs::write(
            self.providers_path(),
            serde_json::to_string_pretty(value).expect("PA-172：配置 JSON 必须可序列化"),
        )
        .expect("PA-172：必须能写入临时 providers.json");
    }
}

impl Drop for TempRegistry {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 取「默认注册表」的**视图层 JSON**（camelCase），作为所有夹具的底版。
fn default_view_json() -> Value {
    let sandbox = TempRegistry::new("template");
    let store = sandbox.store();
    serde_json::to_value(store.load_view()).expect("PA-172：ProviderRegistryView 必须可序列化")
}

/// 把视图 JSON 交给 store 自己的保存路径落盘（⇒ 存储层命名由实现决定，不由测试假定）。
fn install_view(sandbox: &TempRegistry, view_json: Value) -> ProviderRegistryStore {
    let store = sandbox.store();
    let view: ProviderRegistryView =
        serde_json::from_value(view_json).expect("PA-172：注入后的视图 JSON 必须可反序列化");
    store
        .save_view_without_env_sync(view)
        .expect("PA-172：保存视图必须成功");
    sandbox.store()
}

/// 在视图 JSON 里追加 `count` 个合成 provider（id/name/baseUrl 互异）。
fn append_synth_providers(view_json: &mut Value, count: usize) {
    let template = view_json["providers"]
        .as_array()
        .and_then(|providers| providers.first())
        .cloned()
        .expect("PA-172：默认注册表必须至少有一个 provider");
    for index in 0..count {
        let mut synthesized = template.clone();
        synthesized["id"] = json!(format!("synth-{index}"));
        synthesized["name"] = json!(format!("synth-{index}"));
        synthesized["baseUrl"] = json!(format!("http://synth-{index}.pa172.test/v1"));
        view_json["providers"]
            .as_array_mut()
            .expect("PA-172：providers 必须是数组")
            .push(synthesized);
    }
}

/// 视图 JSON 里的策略对象（用于断言）。
fn policy_of(view_json: &Value) -> Option<&Value> {
    view_json.get("upstreamPolicy")
}

// ── §2.1：默认值 + is_default（G2 默认零行为变更） ───────────────────────────────

/// G2：未配置时默认 45000 / 5 / 30000，且 `is_default()` 为真。
#[test]
fn upstream_policy_defaults_match_frozen_baseline() {
    // Arrange + Act
    let policy = UpstreamPolicy::default();

    // Assert
    assert_eq!(
        policy.request_timeout_ms, 45_000,
        "G2：默认请求总超时必须沿用基线的 45s，实际 {}",
        policy.request_timeout_ms
    );
    assert_eq!(
        policy.decision_max_attempts, 5,
        "G2：默认决策尝试次数必须沿用基线的 5，实际 {}",
        policy.decision_max_attempts
    );
    assert_eq!(
        policy.retry_budget_ms, 30_000,
        "G2：默认退避预算必须沿用基线的 30s，实际 {}",
        policy.retry_budget_ms
    );
    assert!(policy.is_default(), "G2：`UpstreamPolicy::default()` 必须 `is_default()`");
    let expected = policy.clone();
    assert_eq!(
        policy.clamped(),
        (expected, Vec::new()),
        "§2.3：默认值必须落在边界内（钳制为空列表）"
    );
}

// ── §2.1：9 个常量边界值逐一断言 ────────────────────────────────────────────────

/// §2.1：9 个 `UPSTREAM_*` 常量取值逐一冻结（任一漂移即破坏 G2 默认零行为变更）。
///
/// v2 第 4 条（Lead 裁决）：`UPSTREAM_RETRY_BUDGET_MS_MIN` 由 0 上调为 **1000**。
#[test]
fn upstream_policy_constants_are_frozen() {
    assert_eq!(UPSTREAM_REQUEST_TIMEOUT_MS_DEFAULT, 45_000);
    assert_eq!(UPSTREAM_DECISION_MAX_ATTEMPTS_DEFAULT, 5);
    assert_eq!(UPSTREAM_RETRY_BUDGET_MS_DEFAULT, 30_000);
    assert_eq!(UPSTREAM_REQUEST_TIMEOUT_MS_MIN, 1_000);
    assert_eq!(UPSTREAM_REQUEST_TIMEOUT_MS_MAX, 300_000);
    assert_eq!(UPSTREAM_DECISION_MAX_ATTEMPTS_MIN, 1);
    assert_eq!(UPSTREAM_DECISION_MAX_ATTEMPTS_MAX, 10);
    assert_eq!(
        UPSTREAM_RETRY_BUDGET_MS_MIN, 1_000,
        "v2-4：retryBudgetMs 下界已裁决为 1000（不再是 0）"
    );
    assert_eq!(UPSTREAM_RETRY_BUDGET_MS_MAX, 300_000);
}

// ── §2.3：钳制真值表（11 行字段级钳制，逐行一个断言） ──────────────────────────────

/// 被钳制字段名列表必须包含指定语义词（容忍 camelCase / snake_case 两种书写）。
fn fields_mention(fields: &[&'static str], needle: &str) -> bool {
    fields.iter().any(|field| {
        field
            .to_ascii_lowercase()
            .replace('_', "")
            .contains(needle)
    })
}

/// §2.3 钳制真值表（13 行表中的前 11 行，字段级钳制）。
#[test]
fn upstream_policy_clamp_truth_table_row_by_row() {
    // 行 1：requestTimeoutMs = 0 ⇒ 1000，被钳制
    let (policy, fields) = UpstreamPolicy {
        request_timeout_ms: 0,
        ..UpstreamPolicy::default()
    }
    .clamped();
    assert_eq!(policy.request_timeout_ms, 1_000, "§2.3 行1：0 必须钳到 1000");
    assert_eq!(fields.len(), 1, "§2.3 行1：必须恰有 1 个字段被钳制，实际 {fields:?}");
    assert!(
        fields_mention(&fields, "timeout"),
        "§2.3 行1：被钳制字段名必须指向请求超时，实际 {fields:?}"
    );

    // 行 2：requestTimeoutMs = 999 ⇒ 1000，被钳制
    let (policy, fields) = UpstreamPolicy {
        request_timeout_ms: 999,
        ..UpstreamPolicy::default()
    }
    .clamped();
    assert_eq!(policy.request_timeout_ms, 1_000, "§2.3 行2：999 必须钳到 1000");
    assert_eq!(fields.len(), 1, "§2.3 行2：必须恰有 1 个字段被钳制，实际 {fields:?}");

    // 行 3：requestTimeoutMs = 45000 ⇒ 45000，不钳制
    let (policy, fields) = UpstreamPolicy {
        request_timeout_ms: 45_000,
        ..UpstreamPolicy::default()
    }
    .clamped();
    assert_eq!(policy.request_timeout_ms, 45_000, "§2.3 行3：45000 必须原样保留");
    assert!(fields.is_empty(), "§2.3 行3：边界内不得报钳制，实际 {fields:?}");

    // 行 4：requestTimeoutMs = 300001 ⇒ 300000，被钳制
    let (policy, fields) = UpstreamPolicy {
        request_timeout_ms: 300_001,
        ..UpstreamPolicy::default()
    }
    .clamped();
    assert_eq!(
        policy.request_timeout_ms, 300_000,
        "§2.3 行4：300001 必须钳到 300000"
    );
    assert_eq!(fields.len(), 1, "§2.3 行4：必须恰有 1 个字段被钳制，实际 {fields:?}");

    // 行 5：decisionMaxAttempts = 0 ⇒ 1，被钳制
    let (policy, fields) = UpstreamPolicy {
        decision_max_attempts: 0,
        ..UpstreamPolicy::default()
    }
    .clamped();
    assert_eq!(policy.decision_max_attempts, 1, "§2.3 行5：0 必须钳到 1");
    assert_eq!(fields.len(), 1, "§2.3 行5：必须恰有 1 个字段被钳制，实际 {fields:?}");
    assert!(
        fields_mention(&fields, "attempt"),
        "§2.3 行5：被钳制字段名必须指向尝试次数，实际 {fields:?}"
    );

    // 行 6：decisionMaxAttempts = 11 ⇒ 10，被钳制
    let (policy, fields) = UpstreamPolicy {
        decision_max_attempts: 11,
        ..UpstreamPolicy::default()
    }
    .clamped();
    assert_eq!(policy.decision_max_attempts, 10, "§2.3 行6：11 必须钳到 10");
    assert_eq!(fields.len(), 1, "§2.3 行6：必须恰有 1 个字段被钳制，实际 {fields:?}");

    // 行 7：decisionMaxAttempts = 5 ⇒ 5，不钳制
    let (policy, fields) = UpstreamPolicy {
        decision_max_attempts: 5,
        ..UpstreamPolicy::default()
    }
    .clamped();
    assert_eq!(policy.decision_max_attempts, 5, "§2.3 行7：5 必须原样保留");
    assert!(fields.is_empty(), "§2.3 行7：边界内不得报钳制，实际 {fields:?}");

    // 行 8：retryBudgetMs = 400000 ⇒ 300000，被钳制
    let (policy, fields) = UpstreamPolicy {
        retry_budget_ms: 400_000,
        ..UpstreamPolicy::default()
    }
    .clamped();
    assert_eq!(policy.retry_budget_ms, 300_000, "§2.3 行8：400000 必须钳到 300000");
    assert_eq!(fields.len(), 1, "§2.3 行8：必须恰有 1 个字段被钳制，实际 {fields:?}");
    assert!(
        fields_mention(&fields, "budget"),
        "§2.3 行8：被钳制字段名必须指向退避预算，实际 {fields:?}"
    );

    // 行 9（v2-4）：retryBudgetMs = 0 ⇒ 1000，被钳制（下界已由 0 上调为 1000）
    let (policy, fields) = UpstreamPolicy {
        retry_budget_ms: 0,
        ..UpstreamPolicy::default()
    }
    .clamped();
    assert_eq!(
        policy.retry_budget_ms, 1_000,
        "v2-4：retryBudgetMs=0 必须钳到 1000（下界不再是 0）"
    );
    assert_eq!(fields.len(), 1, "v2-4：retryBudgetMs=0 必须报 1 条钳制，实际 {fields:?}");
    assert!(
        fields_mention(&fields, "budget"),
        "v2-4：被钳制字段名必须指向退避预算，实际 {fields:?}"
    );

    // 行 10（v2-4）：retryBudgetMs = 999 ⇒ 1000，被钳制
    let (policy, fields) = UpstreamPolicy {
        retry_budget_ms: 999,
        ..UpstreamPolicy::default()
    }
    .clamped();
    assert_eq!(
        policy.retry_budget_ms, 1_000,
        "v2-4：retryBudgetMs=999 必须钳到 1000"
    );
    assert_eq!(fields.len(), 1, "v2-4：retryBudgetMs=999 必须报 1 条钳制，实际 {fields:?}");

    // 行 11（v2-4）：retryBudgetMs = 1000 ⇒ 1000，不钳制（下界本身合法）
    let (policy, fields) = UpstreamPolicy {
        retry_budget_ms: 1_000,
        ..UpstreamPolicy::default()
    }
    .clamped();
    assert_eq!(
        policy.retry_budget_ms, 1_000,
        "v2-4：retryBudgetMs=1000 必须原样保留"
    );
    assert!(fields.is_empty(), "v2-4：retryBudgetMs=1000 不得报钳制，实际 {fields:?}");
}

/// §2.3 行 12/13（wire 层默认，不经过 `clamped()`）：
/// 缺省整个 `upstreamPolicy` 与段内缺字段都必须落到 45000/5/30000。
#[test]
fn upstream_policy_wire_defaults_fill_missing_fields() {
    // 行 12：缺省整个 upstreamPolicy
    let absent: UpstreamPolicy = serde_json::from_value(json!({}))
        .expect("§2.3 行12：缺省 upstreamPolicy 必须能反序列化为默认值");
    assert_eq!(
        absent,
        UpstreamPolicy::default(),
        "§2.3 行12：缺省 upstreamPolicy ⇒ 45000/5/30000，实际 {absent:?}"
    );

    // 行 13：段内缺字段（只有 requestTimeoutMs）
    let partial: UpstreamPolicy = serde_json::from_value(json!({ "requestTimeoutMs": 60_000 }))
        .expect("§2.3 行13：段内缺字段必须能反序列化");
    assert_eq!(partial.request_timeout_ms, 60_000, "§2.3 行13：给值字段必须保留");
    assert_eq!(
        partial.decision_max_attempts,
        UPSTREAM_DECISION_MAX_ATTEMPTS_DEFAULT,
        "§2.3 行13：缺字段必须取默认决策尝试次数"
    );
    assert_eq!(
        partial.retry_budget_ms, UPSTREAM_RETRY_BUDGET_MS_DEFAULT,
        "§2.3 行13：缺字段必须取默认退避预算"
    );
    assert!(
        !partial.is_default(),
        "§2.3 行13：给了非默认超时 ⇒ 不得判 `is_default()`"
    );
}

// ── C2-d：配置 round-trip 不丢字段 ──────────────────────────────────────────────

/// C2-d：视图层必须携带 `upstreamPolicy` + `failoverProviderIds`，且
/// **保存路径落盘 → 再读回**后逐字段相等（前端保存后不得被清空）。
#[test]
fn upstream_policy_and_failover_ids_survive_registry_round_trip() {
    // Arrange：在默认视图 JSON 上注入非默认策略与候选链（视图层 camelCase，C2-d 冻结）。
    let sandbox = TempRegistry::new("round-trip");
    let mut view_json = default_view_json();
    view_json["upstreamPolicy"] = json!({
        "requestTimeoutMs": 60_000,
        "decisionMaxAttempts": 7,
        "retryBudgetMs": 12_345
    });
    view_json["failoverProviderIds"] = json!(["provider-openai", "provider-deepseek"]);

    // Act 1：视图可反序列化 + 走真实保存路径落盘。
    let store = install_view(&sandbox, view_json);

    // Act 2：从磁盘重新读回视图。
    let reloaded = serde_json::to_value(store.load_view())
        .expect("PA-172：ProviderRegistryView 必须可序列化");

    // Assert 1：落盘 JSON 里策略段键必须存在（存储层命名由实现决定）。
    let on_disk = sandbox.read_json();
    let policy_key = ["upstreamPolicy", "upstream_policy"]
        .iter()
        .find(|key| on_disk.get(**key).is_some());
    assert!(
        policy_key.is_some(),
        "C2-d：落盘 providers.json 必须携带策略段（`upstreamPolicy`/`upstream_policy` 至少其一），\
         实际顶层键={:?}",
        on_disk
            .as_object()
            .map(|object| object.keys().cloned().collect::<Vec<_>>())
    );

    // Assert 2：读回视图逐字段相等。
    assert_eq!(
        policy_of(&reloaded),
        Some(&json!({
            "requestTimeoutMs": 60_000,
            "decisionMaxAttempts": 7,
            "retryBudgetMs": 12_345
        })),
        "C2-d：保存→读回后 `upstreamPolicy` 必须逐字段相等（不得被清空），实际 {:?}",
        policy_of(&reloaded)
    );
    assert_eq!(
        reloaded.get("failoverProviderIds"),
        Some(&json!(["provider-openai", "provider-deepseek"])),
        "C2-d：保存→读回后 `failoverProviderIds` 必须保序相等，实际 {:?}",
        reloaded.get("failoverProviderIds")
    );
}

// ── L2-AT：极端值钳制不致命 + 候选链卫生（C2-a / C2-c） ────────────────────────────

/// L2-AT：`u64::MAX` / `u32::MAX` / 0 / 超长候选链 9 项 ⇒ 钳制 + 截断 8 + 去重/去自身，
/// 全程**不致命**（配置仍可用，provider 面不塌）。
#[test]
fn invalid_json_like_values_are_clamped_not_fatal() {
    // ── Arrange/Act 1：纯函数面（`clamped()`）的极端值 ──
    let (clamped, clamped_fields) = UpstreamPolicy {
        request_timeout_ms: u64::MAX,
        decision_max_attempts: u32::MAX,
        retry_budget_ms: u64::MAX,
    }
    .clamped();
    assert_eq!(
        clamped.request_timeout_ms, UPSTREAM_REQUEST_TIMEOUT_MS_MAX,
        "C2-a：u64::MAX 必须钳到上界，实际 {}",
        clamped.request_timeout_ms
    );
    assert_eq!(
        clamped.decision_max_attempts, UPSTREAM_DECISION_MAX_ATTEMPTS_MAX,
        "C2-a：u32::MAX 必须钳到上界，实际 {}",
        clamped.decision_max_attempts
    );
    assert_eq!(
        clamped.retry_budget_ms, UPSTREAM_RETRY_BUDGET_MS_MAX,
        "C2-a：u64::MAX 必须钳到上界，实际 {}",
        clamped.retry_budget_ms
    );
    assert_eq!(
        clamped_fields.len(),
        3,
        "C2-a：三个字段全部越界 ⇒ 必须报 3 条钳制，实际 {clamped_fields:?}"
    );

    let floor = UpstreamPolicy {
        request_timeout_ms: 0,
        decision_max_attempts: 0,
        retry_budget_ms: 0,
    }
    .clamped();
    assert_eq!(floor.0.request_timeout_ms, UPSTREAM_REQUEST_TIMEOUT_MS_MIN);
    assert_eq!(
        floor.0.decision_max_attempts, UPSTREAM_DECISION_MAX_ATTEMPTS_MIN
    );
    assert_eq!(
        floor.0.retry_budget_ms, UPSTREAM_RETRY_BUDGET_MS_MIN,
        "v2-4：retryBudgetMs 下界已抬到 1000"
    );
    assert_eq!(
        floor.1.len(),
        3,
        "v2-4：三个字段全部越界 ⇒ 必须报 3 条钳制，实际 {:?}",
        floor.1
    );

    // ── Arrange 2：落盘极端值 + 9 项互异候选链 ──
    // 候选必须**真实存在**：默认 5 个 provider 之外再造 9 个合成 provider（synth-0..synth-8），
    // 否则链上 id 会被 `unknown` 卫生规则丢掉，测不到「超长截断到 8」。
    let sandbox = TempRegistry::new("chain-hygiene");
    let mut view_json = default_view_json();
    append_synth_providers(&mut view_json, 9);
    let long_chain: Vec<String> = (0..9).map(|index| format!("synth-{index}")).collect();
    assert_eq!(long_chain.len(), 9, "C2-c：超长链必须是 9 项");
    view_json["failoverProviderIds"] = json!(long_chain);
    view_json["upstreamPolicy"] = json!({
        "requestTimeoutMs": u64::MAX,
        "decisionMaxAttempts": u32::MAX,
        "retryBudgetMs": u64::MAX
    });

    // ── Act 2：保存→读回（端到端非致命路径） ──
    let store = install_view(&sandbox, view_json);
    let reloaded = serde_json::to_value(store.load_view())
        .expect("PA-172：ProviderRegistryView 必须可序列化");

    // ── Assert 2a：极端值在端到端路径上被钳到边界（C2-a/C2-b 生效） ──
    assert_eq!(
        policy_of(&reloaded),
        Some(&json!({
            "requestTimeoutMs": UPSTREAM_REQUEST_TIMEOUT_MS_MAX,
            "decisionMaxAttempts": UPSTREAM_DECISION_MAX_ATTEMPTS_MAX,
            "retryBudgetMs": UPSTREAM_RETRY_BUDGET_MS_MAX
        })),
        "C2-a：落盘极端值读回时必须钳到边界，实际 {:?}",
        policy_of(&reloaded)
    );

    // ── Assert 2b：provider 面仍可用（非致命） ──
    let primary = store.resolve_selection(Some("provider-ppx"), None);
    assert_eq!(
        primary.provider_name, "ppx",
        "C2-a：极端值不得影响主 provider 选择"
    );

    // ── Assert 2c：候选链卫生（截断 8 / 去重 / 去自身） ──
    let chain = store.resolve_failover_chain(&primary);
    assert_eq!(
        chain.len(),
        8,
        "C2-c：9 项互异候选必须截断为 8 项（链长上限），实际 {}",
        chain.len()
    );
    let mut seen = std::collections::HashSet::new();
    for selection in &chain {
        let identity = (selection.provider_name.clone(), selection.base_url.clone());
        assert!(
            seen.insert(identity.clone()),
            "C2-c：候选链必须去重（{identity:?} 重复出现）"
        );
        assert_ne!(
            (selection.provider_name.as_str(), selection.base_url.as_str()),
            (primary.provider_name.as_str(), primary.base_url.as_str()),
            "C2-c：候选链必须去掉与主 provider 同 (provider_name, base_url) 的自身项"
        );
    }

    // ── Act/Assert 3：重复 id / 未知 id / 自身 id 必须被忽略，且不致命 ──
    let mut hygiene_view = default_view_json();
    hygiene_view["failoverProviderIds"] = json!([
        "provider-ppx",
        "provider-openai",
        "provider-openai",
        "provider-does-not-exist"
    ]);
    let hygiene_sandbox = TempRegistry::new("chain-dedupe");
    let hygiene_store = install_view(&hygiene_sandbox, hygiene_view);
    let hygiene_primary = hygiene_store.resolve_selection(Some("provider-ppx"), None);
    assert_eq!(
        hygiene_primary.provider_name, "ppx",
        "C2-a：非法候选链不得影响主 provider 选择（配置面必须仍可用）"
    );
    let hygiene_chain = hygiene_store.resolve_failover_chain(&hygiene_primary);
    assert_eq!(
        hygiene_chain.len(),
        1,
        "C2-c：自身 + 重复 + 未知必须被忽略，只剩 provider-openai 一个候选，实际 {:?}",
        hygiene_chain
            .iter()
            .map(|selection| selection.provider_name.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        hygiene_chain[0].provider_name, "openai",
        "C2-c：保留的候选必须是 provider-openai"
    );
}

// ── v2 第 3 条：类型错乱的配置必须容错（C2-a 非致命） ─────────────────────────────

/// 在落盘 JSON 中定位策略段的外层键（存储层命名可能是 camelCase 或 snake_case）。
fn locate_policy_key(on_disk: &Value) -> &'static str {
    if on_disk.get("upstreamPolicy").is_some() {
        "upstreamPolicy"
    } else if on_disk.get("upstream_policy").is_some() {
        "upstream_policy"
    } else {
        panic!(
            "v2-3：落盘 providers.json 未携带策略段（顶层键={:?}），无法注入类型错乱夹具",
            on_disk
                .as_object()
                .map(|object| object.keys().cloned().collect::<Vec<_>>())
        );
    }
}

/// 在落盘 JSON 中定位候选链的外层键。
fn locate_failover_key(on_disk: &Value) -> &'static str {
    if on_disk.get("failoverProviderIds").is_some() {
        "failoverProviderIds"
    } else if on_disk.get("failover_provider_ids").is_some() {
        "failover_provider_ids"
    } else {
        panic!(
            "v2-3：落盘 providers.json 未携带候选链（顶层键={:?}），无法注入非字符串夹具",
            on_disk
                .as_object()
                .map(|object| object.keys().cloned().collect::<Vec<_>>())
        );
    }
}

/// v2-3：`upstreamPolicy` 内**类型错乱**（`"retryBudgetMs":"30s"`、`"decisionMaxAttempts":null`）
/// 必须**不报错**、逐字段回落默认，且不得导致整份配置被丢弃（自定义 provider 仍在）。
///
/// 声明的「记录被忽略字段」属结构化日志面：日志 sink 是**进程级**单例、不可按用例隔离，
/// 故本用例只锁行为面（不报错 + 回落默认 + 配置未被整体丢弃），日志留痕登记为
/// **non-machine（靠 review）**。
#[test]
fn upstream_policy_tolerates_wrong_types() {
    // Arrange：先落一份**合法**配置（含自定义 provider 探针 + 非默认策略），确保策略段真在盘上。
    let sandbox = TempRegistry::new("wrong-types");
    let mut view_json = default_view_json();
    append_synth_providers(&mut view_json, 1);
    view_json["upstreamPolicy"] = json!({
        "requestTimeoutMs": 60_000,
        "decisionMaxAttempts": 7,
        "retryBudgetMs": 12_345
    });
    install_view(&sandbox, view_json);

    // Act：把盘上策略段替换成**类型错乱**的形态（两种命名都写，正确的那份生效、另一份被忽略）。
    let mut on_disk = sandbox.read_json();
    let policy_key = locate_policy_key(&on_disk);
    on_disk[policy_key] = json!({
        "requestTimeoutMs": "45s",
        "decisionMaxAttempts": null,
        "retryBudgetMs": "30s",
        "request_timeout_ms": "45s",
        "decision_max_attempts": null,
        "retry_budget_ms": "30s"
    });
    sandbox.write_json(&on_disk);

    // 重新加载（必须是非致命路径）。
    let store = sandbox.store();
    let reloaded = serde_json::to_value(store.load_view())
        .expect("v2-3：类型错乱不得让视图不可序列化");

    // Assert 1：配置未被整体丢弃（自定义 provider 仍在）。
    let ids: Vec<String> = reloaded["providers"]
        .as_array()
        .expect("v2-3：providers 必须是数组")
        .iter()
        .filter_map(|provider| provider.get("id").and_then(Value::as_str))
        .map(str::to_string)
        .collect();
    assert!(
        ids.iter().any(|id| id == "synth-0"),
        "v2-3/C2-a：类型错乱不得导致整份配置被丢弃（自定义 provider 必须仍在），实际 ids={ids:?}"
    );

    // Assert 2：错类型字段必须各自回落到默认值。
    assert_eq!(
        policy_of(&reloaded),
        Some(&json!({
            "requestTimeoutMs": UPSTREAM_REQUEST_TIMEOUT_MS_DEFAULT,
            "decisionMaxAttempts": UPSTREAM_DECISION_MAX_ATTEMPTS_DEFAULT,
            "retryBudgetMs": UPSTREAM_RETRY_BUDGET_MS_DEFAULT
        })),
        "v2-3：类型错乱字段必须回落默认 45000/5/30000，实际 {:?}",
        policy_of(&reloaded)
    );

    // Assert 3：provider 面仍可用（容错后仍能正常解析选择）。
    let resolved = store.resolve_selection(Some("synth-0"), None);
    assert_eq!(
        resolved.provider_name, "synth-0",
        "C2-a：容错后仍必须能正常选择 provider"
    );
}

/// v2-3：`failoverProviderIds` 含**非字符串项**（数字 / null）⇒ 丢弃不致命，其余候选保序生效。
#[test]
fn failover_provider_ids_tolerates_non_string_items() {
    // Arrange：先落合法配置，确保候选链键真在盘上。
    let sandbox = TempRegistry::new("non-string-ids");
    let mut view_json = default_view_json();
    view_json["failoverProviderIds"] = json!(["provider-openai", "provider-deepseek"]);
    install_view(&sandbox, view_json);

    // Act：把盘上候选链替换成混入非字符串项的形态。
    let mut on_disk = sandbox.read_json();
    let failover_key = locate_failover_key(&on_disk);
    on_disk[failover_key] = json!(["provider-openai", 42, null, "provider-deepseek"]);
    sandbox.write_json(&on_disk);

    let store = sandbox.store();
    let primary = store.resolve_selection(Some("provider-ppx"), None);
    let chain = store.resolve_failover_chain(&primary);

    // Assert
    assert_eq!(
        chain.len(),
        2,
        "v2-3/C2-c：非字符串项必须被丢弃且不致命，只剩两个合法候选，实际 {:?}",
        chain
            .iter()
            .map(|selection| selection.provider_name.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        chain[0].provider_name, "openai",
        "v2-3：合法候选必须保序（provider-openai 在前）"
    );
    assert_eq!(
        chain[1].provider_name, "deepseek",
        "v2-3：合法候选必须保序（provider-deepseek 在后）"
    );
}
