//! 契约验收测试：Run 工具超时策略（`tool-timeout-policy.md` · 波次一 + v3 先行项）
//!
//! 契约真源：`.dev-team/contracts/tool-timeout-policy.md`（v3：CT-1..CT-4 与 CT-5/S2；
//! v2 裁决已撤销原 CT-6「指纹区分 timeoutMs」）。
//! 本文件是**外部黑盒验收**（integration test，独立 crate），只通过契约声明的可测接缝断言，
//! 不复制实现结构、不读取私有常量、不吞异常、不使用永真断言。
//!
//! ## 覆盖矩阵（CT → 用例，共 15 个）
//! - CT-1 → `ct1_contract_default_timeout_constant_observable_via_seam`、
//!   `ct1_contract_max_timeout_constant_observable_via_seam`
//! - CT-2 → `ct2_former_heuristic_commands_all_resolve_to_default`、
//!   `ct2_empty_command_falls_back_to_default`
//! - CT-3 → `ct3_zero_timeout_clamps_to_one_ms`、
//!   `ct3_timeout_above_max_clamps_to_max`、`ct3_non_numeric_timeout_falls_back_to_default`、
//!   `ct3_missing_timeout_uses_default`、`ct3_in_range_values_are_preserved_exactly`、
//!   `ct3_just_above_max_clamps_to_max`
//! - CT-4 → `ct4_timeout_path_evidence_marks_timed_out_true`、
//!   `ct4_success_path_evidence_marks_timed_out_false`、
//!   `ct4_success_path_reports_default_timeout_when_absent`
//! - CT-5 / S2 → `ct5_non_network_hint_points_to_background_jobs`、
//!   `ct5_network_hint_keeps_self_checks_and_points_to_background_jobs`
//!
//! ## 契约演进记录
//! - **CT-5（v3 新增，本波实施）**：`classify_timeout_hint` 由私有改 `pub`（S2）。原先「延后到
//!   波次二」的唯一阻塞项是撞 `wave9_green_acceptance_test.rs` 的逐字断言；Lead 已授权冻结
//!   断言迁移（契约演进，独立 `test(freeze)` 提交），该文件 `DEFAULT_TIMEOUT_HINT` 常量与逐字
//!   断言已同步为 CT-5 新文案，并追加 `job_start`/`job_output`/`600000` 三条标记断言；故本文件
//!   只断三条标记，避免两处重复维护同一逐字文本。
//! - **CT-5a（schema 文案同步）**：不在本文件验收范围（属 `tools.rs` schema 与前端副本的字符串
//!   相等性，由 executor 侧 src 单测覆盖，见契约 CT-5a）。
//! - **CT-6（撤销，本文件不含）**：`tool_failure_fingerprint` 含 `timeoutMs` 与冻结用例
//!   `harness_optimizations_test.rs:215` / `wave9_adversarial_test.rs` 的
//!   「仅 timeoutMs 不同 ⇒ 指纹相同」语义相反，该语义是刻意的反重试风暴设计。
//!
//! ## 「最小可证形式」与判读口径
//! 1) **CT-1**（Lead 裁决：不要求常量 pub）：`DEFAULT_RUN_TIMEOUT_MS` /
//!    `MAX_RUN_TIMEOUT_MS` 是 `agent/tools.rs` 内的私有 `const`，外部集成测试无法直读；
//!    故用最小可证形式——通过 S1 的**可观测输出**断言两常量的契约值
//!    （未传 `timeoutMs` ⇒ 120_000；clamp 上界 ⇒ 600_000）。
//! 2) **CT-4 的 evidence 落位**（Lead 裁决：断在载荷**根级**）：本仓既有证据键
//!    （`stdout`/`stderr`/`exitCode`/`timeoutMs`…）由 `error_result_with_evidence` 合并到载荷
//!    根级（见 `tools.rs` 该函数与既有
//!    `harness_optimizations_test::test_run_command_timeout_payload_keeps_evidence`），故在根级
//!    断言 `timedOut`。
//! 3) **CT-3 边界判读**（Lead 定稿）：`{timeoutMs:0}`/`{-5}` ⇒ 1（<1 夹紧下界）；
//!    `{1.5}` ⇒ 120_000（非整数走 fallback）；`{1}` ⇒ 1。
//! 4) **CT-4 的真实进程依赖**：超时用例必须真的让一个子进程跑过 deadline 才会产生超时证据，
//!    因此依赖本地 shell 的 `sleep`（Windows `ping`）真实进程；deadline 取 200ms，远大于调度
//!    抖动，属确定性触发而非 sleep 竞猜。被测子进程的"睡"不是测试自身的等待同步。
//! 5) **CT-5 为纯函数验收**（S2）：只断 `classify_timeout_hint` 文本标记，不真跑进程。
//! 6) **契约 v5 CT-P6a：本文件验证 promote 关闭时的超时语义。** 文件级在 `pin_promote_off()`
//!    一次性 pin `PONY_AGENT_RUN_PROMOTE_ON_TIMEOUT=0`（`Once` 幂等、同值，并行安全），
//!    CT-4 的三条断言因此在 promote 开关落地后仍按波次一语义成立（到期 ⇒ `error.code=timeout`）。
//!    promote **开启**路径由 `tests/run_timeout_promote_test.rs`（CT-P1..P5）覆盖，两文件互不重叠。

use std::sync::Once;

use pony_agent_core::agent::sandbox::TestSandboxBackend;
use pony_agent_core::agent::tools::{
    classify_timeout_hint, resolve_run_timeout_ms, ToolCall, ToolRouter,
};
use serde_json::{json, Value};

/// 契约 v5 CT-P6a：文件级 pin「promote 关闭」。
///
/// promote 默认开启后，Run 到期会返回 ok + `promoted/jobId` 而非 `error.code=timeout`；
/// 本文件的 CT-4 断言语义是**关闭路径**（波次一原状）。`Once` 保证同值只写一次：
/// 所有用例在触达 Run 之前都经过这里，写入 happens-before 后续读取，恰好一次 + 同值 ⇒ 并行安全。
fn pin_promote_off() {
    static PIN: Once = Once::new();
    PIN.call_once(|| std::env::set_var("PONY_AGENT_RUN_PROMOTE_ON_TIMEOUT", "0"));
}

/// 契约值（来自契约 CT-1/CT-2/CT-3，不是对实现的镜像复制：
/// 实现若与契约不符，本常量保持不变，用例必须红）。
const CONTRACT_DEFAULT_TIMEOUT_MS: u64 = 120_000;
const CONTRACT_MAX_TIMEOUT_MS: u64 = 600_000;

// ---------------------------------------------------------------------------
// CT-1：两常量契约值的可观测形式（理由见文件头说明 1）
// ---------------------------------------------------------------------------

/// CT-1 的默认值 120_000 通过 S1 观测：任何命令、任何缺失/非数字 `timeoutMs` 都落到默认值。
#[test]
fn ct1_contract_default_timeout_constant_observable_via_seam() {
    // Arrange：契约要求默认值对所有命令一致（CT-2 已删除子串启发式）。
    let cases = [
        (json!({}), "echo hi"),
        (json!({}), "cargo build --release"),
        (json!({ "timeoutMs": Value::Null }), "echo hi"),
    ];

    for (arguments, command) in cases {
        // Act
        let actual = resolve_run_timeout_ms(&arguments, command);
        // Assert
        assert_eq!(
            actual, CONTRACT_DEFAULT_TIMEOUT_MS,
            "CT-1/CT-2：未给出有效 timeoutMs 时 {} (arguments={}) 必须取默认值 {}，实际 {}",
            command, arguments, CONTRACT_DEFAULT_TIMEOUT_MS, actual
        );
    }
}

/// CT-1 的上界 600_000 通过 S1 观测：clamp 上限即协议上限。
#[test]
fn ct1_contract_max_timeout_constant_observable_via_seam() {
    // Arrange：远超上界的显式请求。
    let arguments = json!({ "timeoutMs": u64::MAX });

    // Act
    let actual = resolve_run_timeout_ms(&arguments, "cargo build");

    // Assert
    assert_eq!(
        actual, CONTRACT_MAX_TIMEOUT_MS,
        "CT-1/CT-3：超出上限的 timeoutMs 必须夹紧到契约上限 {}，实际 {}",
        CONTRACT_MAX_TIMEOUT_MS, actual
    );
}

// ---------------------------------------------------------------------------
// CT-2：删除子串启发式（build/test/cargo/check/install 不再获得 60_000）
// ---------------------------------------------------------------------------

/// 原启发式命中的五类子串命令（含 `cargo build`）一律取 120_000，不得再返回 60_000。
#[test]
fn ct2_former_heuristic_commands_all_resolve_to_default() {
    // Arrange：逐一覆盖旧启发式的五个子串（tools.rs 旧 2862-2877）。
    let commands = [
        "cargo build --release",
        "cargo test -p pony-agent-core",
        "pnpm install",
        "cargo check",
        "make check",
        "npm run test:unit",
        "git checkout main",
        "echo hi",
    ];

    for command in commands {
        // Act
        let actual = resolve_run_timeout_ms(&json!({}), command);
        // Assert：正向断言具体值，而非"不等于 60_000"这种可被 0/其他值蒙混的形式。
        assert_eq!(
            actual, CONTRACT_DEFAULT_TIMEOUT_MS,
            "CT-2：`{}` 未传 timeoutMs 必须等于默认值 {}（旧子串启发式的 60_000 必须已删除），实际 {}",
            command, CONTRACT_DEFAULT_TIMEOUT_MS, actual
        );
    }
}

/// 命令文本为空串（边界输入）时同样落到默认值，不得 panic 或返回 0。
#[test]
fn ct2_empty_command_falls_back_to_default() {
    // Arrange
    let arguments = json!({});

    // Act
    let actual = resolve_run_timeout_ms(&arguments, "");

    // Assert
    assert_eq!(
        actual, CONTRACT_DEFAULT_TIMEOUT_MS,
        "CT-2：空命令文本也必须取默认值 {}，实际 {}",
        CONTRACT_DEFAULT_TIMEOUT_MS, actual
    );
}

// ---------------------------------------------------------------------------
// CT-3：显式 timeoutMs 夹紧
// ---------------------------------------------------------------------------

/// `{timeoutMs: 0}` ⇒ 1（下界夹紧，不是 0、不是默认值）；负数 `-5` 同样落在「<1」分支 ⇒ 1
/// （Lead 裁决 CT-3 定稿：负数按 clamp 下界处理，而非走非数字 fallback）。
#[test]
fn ct3_zero_timeout_clamps_to_one_ms() {
    // Arrange：0 与负数都属「<1」分支。
    let cases = [json!({ "timeoutMs": 0 }), json!({ "timeoutMs": -5 })];

    for arguments in cases {
        // Act
        let actual = resolve_run_timeout_ms(&arguments, "echo hi");
        // Assert
        assert_eq!(
            actual, 1,
            "CT-3：timeoutMs<1（arguments={}）必须夹紧到 1，实际 {}",
            arguments, actual
        );
    }
}

/// `{timeoutMs: 900000}` ⇒ 600000（上界夹紧）。
#[test]
fn ct3_timeout_above_max_clamps_to_max() {
    // Arrange
    let arguments = json!({ "timeoutMs": 900_000u64 });

    // Act
    let actual = resolve_run_timeout_ms(&arguments, "echo hi");

    // Assert
    assert_eq!(
        actual, CONTRACT_MAX_TIMEOUT_MS,
        "CT-3：timeoutMs=900000 必须夹紧到 {}，实际 {}",
        CONTRACT_MAX_TIMEOUT_MS, actual
    );
}

/// 非数字（字符串 `"x"`）与非整数（`1.5`）⇒ 120_000：非法输入绝不产生 0 或极小超时。
/// Lead 裁决 CT-3 定稿：非整数按「非数字」处理，走 fallback。
#[test]
fn ct3_non_numeric_timeout_falls_back_to_default() {
    // Arrange：字符串非法值 + 小数非法值。
    let cases = [json!({ "timeoutMs": "x" }), json!({ "timeoutMs": 1.5 })];

    for arguments in cases {
        // Act
        let actual = resolve_run_timeout_ms(&arguments, "echo hi");
        // Assert
        assert_eq!(
            actual, CONTRACT_DEFAULT_TIMEOUT_MS,
            "CT-3：非数字/非整数 timeoutMs（arguments={}）必须回落默认值 {}，实际 {}",
            arguments, CONTRACT_DEFAULT_TIMEOUT_MS, actual
        );
    }
}

/// 缺失 `timeoutMs` ⇒ 120_000。
#[test]
fn ct3_missing_timeout_uses_default() {
    // Arrange
    let arguments = json!({ "command": "echo hi" });

    // Act
    let actual = resolve_run_timeout_ms(&arguments, "echo hi");

    // Assert
    assert_eq!(
        actual, CONTRACT_DEFAULT_TIMEOUT_MS,
        "CT-3：缺失 timeoutMs 必须取默认值 {}，实际 {}",
        CONTRACT_DEFAULT_TIMEOUT_MS, actual
    );
}

/// 区间内的合法值必须原样保留（夹紧不得误伤）：1 / 599_999 / 600_000。
#[test]
fn ct3_in_range_values_are_preserved_exactly() {
    // Arrange：含上下界本身，验证 clamp 是闭区间。
    let cases = [
        (1u64, 1u64),
        (2_000u64, 2_000u64),
        (599_999u64, 599_999u64),
        (600_000u64, 600_000u64),
    ];

    for (requested, expected) in cases {
        // Act
        let actual = resolve_run_timeout_ms(&json!({ "timeoutMs": requested }), "echo hi");
        // Assert
        assert_eq!(
            actual, expected,
            "CT-3：区间内 timeoutMs={} 必须原样保留为 {}，实际 {}",
            requested, expected, actual
        );
    }
}

/// 上界外一个单位即夹紧到上界（600_001 ⇒ 600_000），证明边界属于闭区间而非开区间。
#[test]
fn ct3_just_above_max_clamps_to_max() {
    // Arrange
    let arguments = json!({ "timeoutMs": 600_001u64 });

    // Act
    let actual = resolve_run_timeout_ms(&arguments, "echo hi");

    // Assert
    assert_eq!(
        actual, CONTRACT_MAX_TIMEOUT_MS,
        "CT-3：timeoutMs=600001 必须夹紧到 {}，实际 {}",
        CONTRACT_MAX_TIMEOUT_MS, actual
    );
}

// ---------------------------------------------------------------------------
// CT-4：工具结果 evidence 的 timedOut / timeoutMs
// ---------------------------------------------------------------------------

/// 超时路径：真实执行一条必然超过 deadline 的命令，断言 `timedOut: true` + 生效 `timeoutMs`。
/// 说明：这里"睡"的是被测子进程（`sleep`/`ping`），不是测试自身的等待同步，
/// 200ms deadline 远大于调度抖动，仍是确定性断言而非 sleep 竞猜。
#[test]
fn ct4_timeout_path_evidence_marks_timed_out_true() {
    // Arrange
    #[cfg(windows)]
    let command = "ping -n 6 127.0.0.1";
    #[cfg(not(windows))]
    let command = "sleep 5";
    let requested_timeout_ms = 200u64;

    // Act
    let (status, payload) = execute_run_command(command, Some(requested_timeout_ms), "ct4-timeout");

    // Assert：先确认真的走的是超时路径，避免用非超时结果去断言超时证据。
    assert_eq!(
        status, "error",
        "CT-4：超时命令必须以 error 状态返回，实际 status={} payload={}",
        status, payload
    );
    assert_eq!(
        payload
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str),
        Some("timeout"),
        "CT-4：超时路径错误码必须保持 timeout（不得改名/删除），payload={}",
        payload
    );
    assert_eq!(
        payload.get("timedOut").and_then(Value::as_bool),
        Some(true),
        "CT-4：超时结果 evidence 根级必须含 timedOut=true，payload={}",
        payload
    );
    assert_eq!(
        payload.get("timeoutMs").and_then(Value::as_u64),
        Some(requested_timeout_ms),
        "CT-4：超时结果必须回显生效的 timeoutMs={}，payload={}",
        requested_timeout_ms, payload
    );
    // 既有证据键不得被删除（契约 CT-4「既有字段不删不改名」）。
    for key in ["stdout", "stderr", "exitCode", "command"] {
        assert!(
            payload.get(key).is_some(),
            "CT-4：超时结果根级既有证据键 {} 不得缺失，payload={}",
            key,
            payload
        );
    }
}

/// 成功路径：`timedOut: false` 且带生效 `timeoutMs`（证明固定证据不会伪造超时）。
#[test]
fn ct4_success_path_evidence_marks_timed_out_false() {
    // Arrange
    let command = "echo tool-timeout-policy-ct4";
    let requested_timeout_ms = 10_000u64;

    // Act
    let (status, payload) = execute_run_command(command, Some(requested_timeout_ms), "ct4-success");

    // Assert
    assert_eq!(
        status, "ok",
        "CT-4：未超时命令必须以 ok 返回，payload={}",
        payload
    );
    assert_eq!(
        payload.get("timedOut").and_then(Value::as_bool),
        Some(false),
        "CT-4：成功路径必须显式带 timedOut=false（不得缺失、不得为 true），payload={}",
        payload
    );
    assert_eq!(
        payload.get("timeoutMs").and_then(Value::as_u64),
        Some(requested_timeout_ms),
        "CT-4：成功路径必须带生效 timeoutMs={}，payload={}",
        requested_timeout_ms, payload
    );
    assert!(
        payload
            .get("stdout")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .contains("tool-timeout-policy-ct4"),
        "CT-4：成功路径必须保留真实 stdout，payload={}",
        payload
    );
}

/// 未传 `timeoutMs` 的成功路径：生效值必须是 120_000（CT-1/CT-2 在工具结果层的可见形式）。
#[test]
fn ct4_success_path_reports_default_timeout_when_absent() {
    // Arrange
    let command = "echo tool-timeout-policy-default";

    // Act
    let (status, payload) = execute_run_command(command, None, "ct4-default");

    // Assert
    assert_eq!(status, "ok", "CT-4：命令必须成功，payload={}", payload);
    assert_eq!(
        payload.get("timedOut").and_then(Value::as_bool),
        Some(false),
        "CT-4：成功路径 timedOut 必须为 false，payload={}",
        payload
    );
    assert_eq!(
        payload.get("timeoutMs").and_then(Value::as_u64),
        Some(CONTRACT_DEFAULT_TIMEOUT_MS),
        "CT-1/CT-2/CT-4：未传 timeoutMs 的工具结果必须回显默认生效值 {}，payload={}",
        CONTRACT_DEFAULT_TIMEOUT_MS,
        payload
    );
}

// ---------------------------------------------------------------------------
// CT-5 / S2：超时 hint 必须给出「后台执行」与「上限」两条出路
// ---------------------------------------------------------------------------

/// 非网络命令：hint 必须指向后台执行（`job_start` + `job_output` 取增量输出）并写明上限 600000。
///
/// 逐字文案由冻结用例 `wave9_green_acceptance_test::timeout_hint_classifies_network_commands`
/// 以 `DEFAULT_TIMEOUT_HINT` 常量驱动断言（契约演进授权迁移），此处只断 CT-5 的三条标记，
/// 避免同一逐字文本在两地重复维护。
#[test]
fn ct5_non_network_hint_points_to_background_jobs() {
    // Arrange
    let command = "cargo build --release";

    // Act
    let hint = classify_timeout_hint(command);

    // Assert
    assert!(
        !hint.trim().is_empty(),
        "CT-5：非网络命令 `{}` 的 hint 不得为空串，实测 hint={:?}",
        command,
        hint
    );
    for marker in ["job_start", "job_output", "600000"] {
        assert!(
            hint.contains(marker),
            "CT-5：非网络命令 `{}` 的 hint 必须同时给出后台执行与上限出路（缺标记 `{}`），实测 hint={}",
            command,
            marker,
            hint
        );
    }
}

/// 网络命令：四网络自检标记必须保留，且**追加**同样的后台执行/上限建议。
///
/// 契约 CT-5 原文「网络分支保留既有网络自检四标记并**追加**同样的后台/上限建议」⇒
/// 网络分支同样必须含 `job_start`/`job_output`/`600000`。
#[test]
fn ct5_network_hint_keeps_self_checks_and_points_to_background_jobs() {
    // Arrange
    let command = "git push --dry-run";

    // Act
    let hint = classify_timeout_hint(command);

    // Assert 1：既有四网络自检标记一字不少。
    for marker in [
        "网络/认证/代理",
        "git ls-remote",
        "git config --get remote.origin.url",
        "SSH 身份自检",
    ] {
        assert!(
            hint.contains(marker),
            "CT-5：网络命令 `{}` 的 hint 必须保留网络自检标记 `{}`，实测 hint={}",
            command,
            marker,
            hint
        );
    }
    // Assert 2：同样的后台执行/上限退出路必须追加。
    for marker in ["job_start", "job_output", "600000"] {
        assert!(
            hint.contains(marker),
            "CT-5：网络命令 `{}` 的 hint 必须追加后台执行/上限出路（缺标记 `{}`），实测 hint={}",
            command,
            marker,
            hint
        );
    }
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// 每个用例独立临时工作区，用例结束即删除（谁创建谁清理）。
fn unique_workspace(tag: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock must be after UNIX_EPOCH")
        .as_nanos();
    std::env::temp_dir().join(format!("tool_timeout_policy_{}_{}", tag, nanos))
}

/// 经真实 `ToolRouter` 执行 `workspace_run_command`，返回 `(status, 解析后的载荷)`。
/// 输出不是合法 JSON 时 panic —— 禁止静默放行。
fn execute_run_command(
    command: &str,
    timeout_ms: Option<u64>,
    tag: &str,
) -> (String, Value) {
    // 契约 v5 CT-P6a：本文件只验收「promote 关闭」的波次一语义。
    pin_promote_off();

    let workspace = unique_workspace(tag);
    std::fs::create_dir_all(&workspace).expect("create temp workspace");

    let router = ToolRouter::with_workspace_root(workspace.clone())
        .with_sandbox_backend(TestSandboxBackend::available());
    let mut arguments = json!({
        "command": command,
        "cwd": workspace.display().to_string(),
        "description": "tool-timeout-policy 验收",
    });
    if let Some(timeout_ms) = timeout_ms {
        arguments["timeoutMs"] = json!(timeout_ms);
    }

    let call = ToolCall {
        name: "workspace_run_command".to_string(),
        arguments,
        call_id: Some(format!("call-{}", tag)),
        plan: None,
    };

    let result = router.execute(&call);
    let _ = std::fs::remove_dir_all(&workspace);

    let payload: Value = serde_json::from_str(&result.output).unwrap_or_else(|error| {
        panic!(
            "workspace_run_command 输出必须是合法 JSON（{}）: {}",
            error, result.output
        )
    });
    (result.status, payload)
}
