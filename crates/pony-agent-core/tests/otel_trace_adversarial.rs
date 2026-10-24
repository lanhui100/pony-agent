//! Wave43 · Agent Trace OTel 迁移 —— **L2-AT 对抗破坏测试（Red Team）**。
//!
//! 本文件只写**破坏类**用例；正向契约由同目录 `otel_trace_migration_test.rs`（L2-T）覆盖，
//! 此处不重复（不重复断言门禁默认值、属性正常映射、手动导出数量契约、退役符号扫描、设置落盘）。
//!
//! 破坏面（每条都要求确定性，禁止概率等待、禁止 `#[ignore]`、禁止无断言放行）：
//! - **AT-1 端点黑洞 / 超时注入**：`127.0.0.1` 上只 `accept` 不响应的 TCP 监听器做 OTLP 端点，
//!   断言导出失败被吞掉（不 panic）、`force_flush` 落在 `[2000ms, 3250ms]`（下界防「根本没发」，
//!   上界防「导出层没配超时」）、总连接数符合 `retry.max_attempts=1`、以及 `log_json("warn"`
//!   降级调用点在 `src/agent/otel/**` 静态存在。
//! - **AT-2 畸形 / 超大 payload**：64KB×4 字段的 `TurnSpanInput` 不 panic、不乘性放大（近 OOM 代理断言）、
//!   输出仍是合法且唯一的冻结键；空 `session_id` / 空 `user_id` 按 A2 不输出该键；
//!   注入形状字符串（引号/换行/NUL/花括号）逐字节保留、绝不被提升为属性**键**。
//! - **AT-3 并发门禁翻转**：`Barrier::new(3)` + 3×200 次翻转 + 1 条持续读线程，整体 5s 硬截止
//!   （超时即判死锁），无 panic、最终态可被确定设置。
//!
//! 契约来源（只读、已冻结）：`.dev-team/wave43-adjudication.md`（A2/A3）、
//! `.dev-team/nfr-baseline-wave43.json`（external_call_timeout_ms=3000、retry.max_attempts=1、
//! 并发约束）、`crates/pony-agent-core/src/agent/otel/mod.rs`（空桩签名）。
//!
//! 当前实现为 `unimplemented!` 空桩 ⇒ 本文件全部用例**必然红**，这是预期红相。

use std::any::Any;
use std::collections::HashSet;
use std::io::ErrorKind;
use std::net::{TcpListener, TcpStream};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Barrier, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use pony_agent_core::agent::otel::{
    self, AttrValue, OtelConfig, TurnSpanInput, DEFAULT_SAMPLING_RATIO,
};

// ---------------------------------------------------------------- 全局态串行锁
//
// `otel::init` / `set_reporting_enabled` / `force_flush` 都作用在进程级单例上；cargo test 默认
// 多线程跑用例，若 AT-1（要求门禁开启 + 独占 provider）与 AT-3（疯狂翻转门禁）并行，
// 断言会互相污染。凡触达全局态的用例一律持锁串行，且退出时把门禁复位为 false。

static GLOBAL_STATE_LOCK: Mutex<()> = Mutex::new(());

fn state_lock() -> MutexGuard<'static, ()> {
    GLOBAL_STATE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

// ---------------------------------------------------------------- 通用小工具

/// 把 panic payload 转成可打印文本（`unimplemented!` 的 payload 是 `&str`）。
fn panic_payload(payload: &(dyn Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&'static str>() {
        (*text).to_string()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "<不可解码的 panic payload>".to_string()
    }
}

/// 断言「此段代码不得 panic」，panic 时带破坏点标签升级为失败（不吞异常放行）。
fn catch_labeled<T>(label: &str, body: impl FnOnce() -> T) -> T {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(value) => value,
        Err(payload) => panic!(
            "{label} 发生 panic（对抗面要求：失败必须被吞掉降级，不得 panic）：{}",
            panic_payload(payload.as_ref())
        ),
    }
}

fn attr<'a>(attrs: &'a [(String, AttrValue)], key: &str) -> Option<&'a AttrValue> {
    attrs
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value)
}

/// 断言键存在且为字符串，返回值；键缺失时把全集打进失败信息便于定位。
fn require_str<'a>(attrs: &'a [(String, AttrValue)], key: &str) -> &'a str {
    match attr(attrs, key) {
        Some(AttrValue::Str(value)) => value.as_str(),
        other => panic!(
            "属性 `{key}` 必须以字符串输出，实际 {:?}；当前键集：{:?}",
            other,
            attrs.iter().map(|(name, _)| name).collect::<Vec<_>>()
        ),
    }
}

/// 断言键存在且为整数（数值键被落成字符串会破坏 Langfuse 数值过滤）。
fn require_i64(attrs: &[(String, AttrValue)], key: &str) -> i64 {
    match attr(attrs, key) {
        Some(AttrValue::I64(value)) => *value,
        other => panic!(
            "属性 `{key}` 必须以整数（AttrValue::I64）输出，实际 {:?}；当前键集：{:?}",
            other,
            attrs.iter().map(|(name, _)| name).collect::<Vec<_>>()
        ),
    }
}

/// A2：缺失值一律不输出该键（禁止空串占位）。
fn assert_absent(attrs: &[(String, AttrValue)], key: &str) {
    assert!(
        attr(attrs, key).is_none(),
        "A2：缺失/空值不得输出键 `{key}`，实际存在 {:?}",
        attr(attrs, key)
    );
}

// ---------------------------------------------------------------- 属性键合法性
//
// A2 冻结的 turn 键集（此处用**字面量**，避免常量被改后自指自证）。
const AT2_ALLOWED_TURN_KEYS: [&str; 7] = [
    "langfuse.session.id",
    "langfuse.trace.name",
    "gen_ai.operation.name",
    "agent.turn.id",
    "agent.turn.sequence",
    "agent.turn.phase",
    "langfuse.user.id",
];

/// 校验输出仍是「合法键值对」：键非空、可打印 ASCII、属于冻结键集、无重复、值无空串占位。
/// 返回键 + 字符串值占用的总字节数（作为「不乘性放大 / 不 OOM」的可确定上界量）。
fn assert_valid_key_value_pairs(
    attrs: &[(String, AttrValue)],
    allowed: &[&str],
    label: &str,
) -> usize {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut total_bytes = 0usize;

    for (key, value) in attrs {
        assert!(
            !key.is_empty(),
            "{label}：属性键不得为空串（用户可控值不得变成键）"
        );
        assert!(
            key.is_ascii() && !key.chars().any(|c| c.is_whitespace() || c.is_control()),
            "{label}：属性键必须是可打印 ASCII，实际 `{key}`"
        );
        assert!(
            allowed.contains(&key.as_str()),
            "{label}：出现冻结键集之外的属性键 `{key}`（键注入 ⇒ Langfuse 侧字段不可控）；实际键集：{:?}",
            attrs.iter().map(|(name, _)| name).collect::<Vec<_>>()
        );
        assert!(
            seen.insert(key.as_str()),
            "{label}：属性键 `{key}` 重复（OTel attribute map 会静默去重，末值不可预期）"
        );

        total_bytes += key.len();
        if let AttrValue::Str(text) = value {
            assert!(
                !text.is_empty(),
                "{label}：键 `{key}` 不得以空串占位（A2 禁止空 observation 字段）"
            );
            total_bytes += text.len();
        }
    }

    total_bytes
}

// ---------------------------------------------------------------- AT-1 端点黑洞

/// 只 `accept`、永不响应的 OTLP 端点（TCP 层黑洞：连接建立成功 ⇒ 导出层只能靠自身超时脱身）。
struct BlackHoleEndpoint {
    port: u16,
    accepted: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl BlackHoleEndpoint {
    /// `hold_budget` 是「保持黑洞」的安全网（必须远大于导出超时），真正退出由 [`Self::finish`]
    /// 置停止位触发，避免绿灯路径白等。
    fn start(hold_budget: Duration) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("AT-1：黑洞端点必须能绑定回环端口");
        listener
            .set_nonblocking(true)
            .expect("AT-1：黑洞端点必须为非阻塞监听");
        let port = listener
            .local_addr()
            .expect("AT-1：黑洞端点 local_addr 必须可读")
            .port();

        let accepted = Arc::new(AtomicUsize::new(0));
        let accepted_in_thread = Arc::clone(&accepted);
        let stop = Arc::new(AtomicBool::new(false));
        let stop_in_thread = Arc::clone(&stop);

        let handle = thread::spawn(move || {
            let deadline = Instant::now() + hold_budget;
            // hold 住连接句柄：既不发响应，也不主动关闭（真正的黑洞）。
            let mut held: Vec<TcpStream> = Vec::new();
            while Instant::now() < deadline && !stop_in_thread.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _peer)) => {
                        let _ = stream.set_nodelay(true);
                        held.push(stream);
                        accepted_in_thread.fetch_add(1, Ordering::SeqCst);
                    }
                    Err(err) if err.kind() == ErrorKind::WouldBlock => {
                        // 不是概率等待：仅避免自旋烧 CPU，不影响任何断言判定。
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(_) => break,
                }
            }
            drop(held);
        });

        Self {
            port,
            accepted,
            stop,
            handle: Some(handle),
        }
    }

    fn endpoint(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// 停黑屏并等线程退出，返回本次窗口内被 accept 的连接总数。
    fn finish(mut self) -> usize {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            handle.join().expect("AT-1：黑洞端点线程不得 panic");
        }
        self.accepted.load(Ordering::SeqCst)
    }
}

fn blackhole_config(port_endpoint: String) -> OtelConfig {
    OtelConfig {
        endpoint: port_endpoint,
        service_name: "pony-agent-wave43-at1-blackhole".to_string(),
        sampling_ratio: DEFAULT_SAMPLING_RATIO,
        headers: Vec::new(),
    }
}

/// AT-1 主用例：黑洞端点下「导出失败被吞掉 + 有界超时 + 单次尝试」。
///
/// 时间带 `[2000ms, 3250ms]` 的判读口径：
/// - 下界 2000ms：`force_flush` 秒回（或根本没发）说明 span 没进队列 / 出口被短路，属假通过；
/// - 上界 3250ms：nfr 冻结 `external_call_timeout_ms = 3000`。测得值超过上界时按落点归因：
///   ≈ 本用例给 `force_flush` 的窗口（6000ms）⇒ 导出层**根本没有**自身超时，全靠调用方截止；
///   ≈ 其它值（如 5000ms）⇒ 导出层有自己的超时但**不等于** nfr 的 3000ms（超时口径漂移）。
///
/// `force_flush` 的实参故意取 6000ms（远大于 nfr 的 3000ms），使测得耗时由导出层自身超时主导，
/// 从而把「导出层缺超时 / 超时口径不等于 3000」暴露成上界红，而不是被调用方窗口掩盖。
#[test]
fn at1_blackhole_endpoint_swallows_export_failure_within_bounded_time() {
    let _guard = state_lock();
    otel::shutdown(); // A3：幂等，作为「未 init」可复现前置

    // 黑洞保持预算 8s（安全网）>> 任何合理导出超时：先由导出层超时，绝不靠端点断链「伪装」成超时。
    let blackhole = BlackHoleEndpoint::start(Duration::from_millis(8_000));

    otel::set_reporting_enabled(true);
    otel::init(blackhole_config(blackhole.endpoint()))
        .expect("AT-1：回环端点装配不得失败（装配失败只降级，但此处端点合法）");

    // 破坏点 1：turn 主链路（TurnScope 生命周期）在导出端点为黑洞时不得 panic。
    catch_labeled("AT-1 TurnScope::new/end（黑洞端点）", || {
        let scope = otel::TurnScope::new("sess-at1-blackhole", "turn-at1-blackhole", 0);
        // 采样率 1.0 且门禁开启，此处允许 Some/None 两种实现取向，只要求不 panic。
        let _ = scope.trace_id();
        scope.end();
    });

    // 破坏点 2：强制导出必须被有界地吞掉（不 panic、不死等）。
    let started = Instant::now();
    let flush_result = catch_labeled("AT-1 force_flush（黑洞端点）", || otel::force_flush(6_000));
    let elapsed = started.elapsed();

    assert!(
        elapsed >= Duration::from_millis(2_000),
        "AT-1 下界失败：force_flush 仅耗时 {elapsed:?}（< 2000ms）——span 没进队列或出口被短路，\
         本用例等于没测到超时路径（防「根本没发」）"
    );
    assert!(
        elapsed <= Duration::from_millis(3_250),
        "AT-1 上界失败：force_flush 耗时 {elapsed:?}（> 3250ms）——nfr 冻结 external_call_timeout_ms=3000；\
         落点若≈6000ms（本用例窗口）说明导出层没配自身超时、只有调用方在干等，\
         落点若为其它值说明导出层超时口径≠3000ms（防「没配超时 / 超时口径漂移」）"
    );

    // 破坏点 3：失败被吞掉 —— 不向上传播成 panic，只以可诊断的 Err 返回（A3 失败语义）。
    match flush_result {
        Err(message) => assert!(
            !message.is_empty(),
            "AT-1：导出失败必须以非空原因降级，不得返回空字符串（不可诊断）"
        ),
        Ok(()) => panic!(
            "AT-1：黑洞端点下导出不可能成功，force_flush 却返回 Ok(()) —— 失败被静默改写成成功（伪绿）"
        ),
    }

    // 破坏点 4：失败不得污染门禁末态，主链路继续可用。
    assert!(
        otel::is_reporting_enabled(),
        "AT-1：一次导出失败不得翻转全局门禁（末态必须仍为调用前的 true）"
    );

    otel::shutdown();
    otel::set_reporting_enabled(false);

    let accepted = blackhole.finish();
    assert!(
        accepted >= 1,
        "AT-1：黑洞端点始终没收到连接 —— 导出根本没发出去，上面的耗时断言失去意义"
    );
    assert!(
        accepted <= 2,
        "AT-1：nfr 冻结 retry.max_attempts=1（OTLP 导出失败不重试），本窗口内实际建立 {accepted} 条连接，\
         出现重试风暴"
    );
}

/// AT-1 静态面：导出失败必须降级为 `log_json("warn", ...)`（进程内 stderr 不可捕获，故做源码扫描）。
///
/// 扫描范围是**整个** `src/agent/otel/` 目录（不假定失败处理落在哪个文件），并要求证据源含锚点，
/// 避免「扫了个空文件」式的假通过。
#[test]
fn at1_export_failure_has_warn_json_degradation_callsite() {
    let module_dir = format!("{}/src/agent/otel", env!("CARGO_MANIFEST_DIR"));
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&module_dir)
        .unwrap_or_else(|err| panic!("AT-1：otel 模块目录必须可读（{module_dir}）：{err}"))
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().map(|ext| ext == "rs").unwrap_or(false))
        .collect();
    files.sort();

    assert!(
        !files.is_empty(),
        "AT-1：{module_dir} 下没有 .rs 文件，静态扫描不可信"
    );

    let mut evidence = String::new();
    for file in &files {
        let text = std::fs::read_to_string(file)
            .unwrap_or_else(|err| panic!("AT-1：证据源必须可读（{}）：{err}", file.display()));
        evidence.push_str(&text);
        evidence.push('\n');
    }

    // 防假绿锚点：证据源确实是 otel 契约面（含 log_json 定义与 force_flush 签名）。
    assert!(
        evidence.contains("pub fn log_json"),
        "AT-1：证据源缺少 `pub fn log_json` 锚点，扫描结果不可信"
    );
    assert!(
        evidence.contains("pub fn force_flush"),
        "AT-1：证据源缺少 `pub fn force_flush` 锚点，扫描结果不可信"
    );

    // 调用点可能跨行书写（`log_json(\n    "warn",`），故先剥掉空白再匹配，避免格式脆断
    // 造成的假红（也只接受 argument 位置上就是字面量 "warn" 的写法）。
    let compact: String = evidence.chars().filter(|c| !c.is_whitespace()).collect();
    let warn_callsites = compact.matches(r#"log_json("warn""#).count();
    assert!(
        warn_callsites >= 1,
        "AT-1：`src/agent/otel/**` 内没有任何 `log_json(\"warn\", ...)` 调用点 —— 导出失败没有降级日志，\
         用户侧不可观测（D8：失败只降级为 warn JSON 日志）"
    );

    // 反向：D8 要求 level 直接写字面量，禁止用常量拼接绕开静态扫描。
    assert!(
        !compact.contains("log_json(LEVEL_WARN")
            && !compact.contains("log_json(&LEVEL_WARN")
            && !compact.contains("log_json(LEVEL::WARN")
            && !compact.contains("log_json(&\"warn\""),
        "AT-1：warn 级别不得经常量/引用间接传入（D8 要求调用点直接写字面量，否则本扫描断言失效）"
    );
}

// ---------------------------------------------------------------- AT-2 畸形 / 超大 payload

/// 64KB×4 字段（session/user/turn/phase 全部拉满）+ 数值边界 `u64::MAX`。
#[test]
fn at2_oversized_payload_stays_bounded_and_does_not_panic() {
    const FIELD_LEN: usize = 64 * 1024; // 64KB
    let huge = |head: char| format!("{head}{}", "x".repeat(FIELD_LEN - 1));

    let input = TurnSpanInput {
        session_id: huge('s'),
        user_id: Some(huge('u')),
        turn_id: huge('t'),
        sequence: u64::MAX, // 数值边界：饱和/截断都是实现自由，但不得 panic、不得丢键
        phase: huge('p'),
    };

    let attrs = catch_labeled("AT-2 turn_span_attributes（64KB×4 超大 payload）", || {
        otel::turn_span_attributes(&input)
    });

    // 断言 1：属性条数有界（输入变大不得导致键数乘性膨胀）。
    assert!(
        attrs.len() <= AT2_ALLOWED_TURN_KEYS.len(),
        "AT-2：属性条数 {} 超过冻结键数 {} —— 输入规模污染了键集（疑似按内容/分段造键）",
        attrs.len(),
        AT2_ALLOWED_TURN_KEYS.len()
    );

    // 断言 2：输出仍是合法键值对（键集/唯一性/无空串占位）。
    let output_bytes = assert_valid_key_value_pairs(&attrs, &AT2_ALLOWED_TURN_KEYS, "AT-2 超大 payload");

    // 断言 3：不 OOM 的可确定代理 —— 输出字节数不得超输入的 4 倍（平方级复制会直接越界）。
    let input_bytes = input.session_id.len()
        + input.user_id.as_ref().map(|value| value.len()).unwrap_or(0)
        + input.turn_id.len()
        + input.phase.len();
    let budget = input_bytes * 4 + 1024;
    assert!(
        output_bytes <= budget,
        "AT-2：输出字节 {output_bytes} 超过输入 {input_bytes} 的 4 倍预算 {budget} —— \
         疑似线性以下不成立的重复复制/转义放大（OOM 前兆）"
    );

    // 断言 4：值必须逐字节原样搬运（不截断、不改写）。
    assert_eq!(
        require_str(&attrs, "langfuse.session.id").len(),
        FIELD_LEN,
        "AT-2：64KB session_id 被截断（长度不符）"
    );
    assert_eq!(
        require_str(&attrs, "langfuse.session.id"),
        input.session_id.as_str(),
        "AT-2：session_id 内容必须原样保留"
    );
    require_i64(&attrs, "agent.turn.sequence"); // 存在且为整数（不得因 u64::MAX 丢键）
    assert_eq!(
        require_str(&attrs, "gen_ai.operation.name"),
        "agent.turn",
        "AT-2：恒输出键在畸形 payload 下也不得消失"
    );

    // 断言 5：同输入必须确定性同输出（无隐藏全局态/无规模爬升）。
    let again = otel::turn_span_attributes(&input);
    assert_eq!(
        again, attrs,
        "AT-2：同一输入两次映射结果不同（存在隐式状态或非确定性）"
    );
}

/// 空值契约：`session_id=""`、`user_id=Some("")` ⇒ 不输出该键（A2：缺失值不输出，禁止空串占位）。
#[test]
fn at2_empty_session_and_user_values_omit_keys() {
    let input = TurnSpanInput {
        session_id: String::new(),
        user_id: Some(String::new()),
        turn_id: "turn-at2-empty".to_string(),
        sequence: 0,
        phase: "completed".to_string(),
    };

    let attrs = catch_labeled("AT-2 turn_span_attributes（空 session/user）", || {
        otel::turn_span_attributes(&input)
    });

    assert_absent(&attrs, "langfuse.session.id");
    assert_absent(&attrs, "langfuse.user.id");
    // 全面禁止空串占位（空 turn_id/phase 若被写成 "" 同样违反 A2）。
    assert_valid_key_value_pairs(&attrs, &AT2_ALLOWED_TURN_KEYS, "AT-2 空值");

    // 反向防过度删除：非空字段与恒输出键仍须在位。
    assert_eq!(require_str(&attrs, "agent.turn.id"), "turn-at2-empty");
    assert_eq!(require_str(&attrs, "gen_ai.operation.name"), "agent.turn");
    assert_eq!(require_i64(&attrs, "agent.turn.sequence"), 0);
}

/// 注入形状 payload：引号 / 换行 / NUL / 花括号 / 非 ASCII 必须原样保留，且绝不能变成属性**键**。
#[test]
fn at2_injection_shaped_strings_are_verbatim_not_promoted_to_keys() {
    let hostile_session = "sess\"}\n{\"level\":\"warn\",\"message\":\"forged\"}\u{0}\ttail-中文";
    let hostile_user = "user\"\n\u{0}inj";
    let hostile_turn = "turn\"-inj\n";

    let input = TurnSpanInput {
        session_id: hostile_session.to_string(),
        user_id: Some(hostile_user.to_string()),
        turn_id: hostile_turn.to_string(),
        sequence: 1,
        phase: "completed\"\n".to_string(),
    };

    let attrs = catch_labeled("AT-2 turn_span_attributes（注入形状 payload）", || {
        otel::turn_span_attributes(&input)
    });

    // 键集白名单是最关键的一条：用户可控字符串若被拼进键名，即键注入。
    assert_valid_key_value_pairs(&attrs, &AT2_ALLOWED_TURN_KEYS, "AT-2 注入形状 payload");

    assert_eq!(
        require_str(&attrs, "langfuse.session.id"),
        hostile_session,
        "AT-2：注入形状 session_id 必须逐字节保留（不得截断/转义/二次编码/改写）"
    );
    assert_eq!(
        require_str(&attrs, "langfuse.session.id").len(),
        hostile_session.len(),
        "AT-2：session_id 长度必须与输入一致"
    );
    assert_eq!(require_str(&attrs, "langfuse.user.id"), hostile_user);
    assert_eq!(require_str(&attrs, "agent.turn.id"), hostile_turn);
}

// ---------------------------------------------------------------- AT-3 并发门禁翻转

const AT3_FLIPPERS: usize = 3;
const AT3_ITERATIONS_PER_FLIPPER: usize = 200;

struct GateStormStats {
    total_flips: usize,
    reader_reads: u64,
}

/// 确定性并发风暴：`Barrier::new(3)` 同步起跑 3 个翻转线程，另 1 条线程持续读门禁。
///
/// 全程无 `sleep`、无随机数：翻转方向由 `(线程号 + 步号) % 2` 决定，读线程就绪用有界自旋握手。
fn run_gate_storm() -> GateStormStats {
    let barrier = Arc::new(Barrier::new(AT3_FLIPPERS));
    let stop = Arc::new(AtomicBool::new(false));
    let reader_ready = Arc::new(AtomicBool::new(false));
    let reader_reads = Arc::new(AtomicU64::new(0));
    let reader_panic: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

    let reader = {
        let stop = Arc::clone(&stop);
        let reader_ready = Arc::clone(&reader_ready);
        let reader_reads = Arc::clone(&reader_reads);
        let reader_panic = Arc::clone(&reader_panic);
        thread::spawn(move || {
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                // 先报到，再由主线程确认「至少完成一次真实读取」：两个握手都是构造出的事实，
                // 不依赖线程调度概率。
                reader_ready.store(true, Ordering::SeqCst);
                let _ = otel::is_reporting_enabled();
                reader_reads.fetch_add(1, Ordering::SeqCst);
                while !stop.load(Ordering::SeqCst) {
                    let _ = otel::is_reporting_enabled();
                    reader_reads.fetch_add(1, Ordering::SeqCst);
                    std::hint::spin_loop();
                }
            }));
            if let Err(payload) = outcome {
                *reader_panic
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                    Some(panic_payload(payload.as_ref()));
            }
        })
    };

    // 握手 1：读线程已上线（有界自旋，不 sleep）。
    let ready_deadline = Instant::now() + Duration::from_secs(2);
    while !reader_ready.load(Ordering::SeqCst) {
        assert!(
            Instant::now() < ready_deadline,
            "AT-3：读线程 2s 内未上线（线程调度层面已死锁）"
        );
        thread::yield_now();
    }

    // 握手 2：读线程已完成至少一次真实读取；失败时把读线程自身的 panic 原因带出来。
    let read_deadline = Instant::now() + Duration::from_secs(2);
    while reader_reads.load(Ordering::SeqCst) == 0 {
        if let Some(message) = reader_panic
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
        {
            panic!("AT-3：读线程 is_reporting_enabled() panic：{message}");
        }
        assert!(
            Instant::now() < read_deadline,
            "AT-3：读线程 2s 内未完成首次读取（读路径死锁）"
        );
        thread::yield_now();
    }

    let mut flippers = Vec::with_capacity(AT3_FLIPPERS);
    for index in 0..AT3_FLIPPERS {
        let barrier = Arc::clone(&barrier);
        flippers.push(thread::spawn(move || {
            barrier.wait(); // 三线程同一时刻起跑（不是概率窗口）
            for step in 0..AT3_ITERATIONS_PER_FLIPPER {
                otel::set_reporting_enabled((index + step) % 2 == 0);
            }
        }));
    }

    for handle in flippers {
        if let Err(payload) = handle.join() {
            panic!(
                "AT-3：翻转线程 set_reporting_enabled() panic：{}",
                panic_payload(payload.as_ref())
            );
        }
    }
    stop.store(true, Ordering::SeqCst);
    if let Err(payload) = reader.join() {
        panic!(
            "AT-3：读线程 is_reporting_enabled() panic：{}",
            panic_payload(payload.as_ref())
        );
    }
    if let Some(message) = reader_panic
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
    {
        panic!("AT-3：读线程 is_reporting_enabled() panic：{message}");
    }

    // 最终态可确定设置：所有写者退出后再显式写两次，末态必须等于最后一次写入。
    otel::set_reporting_enabled(true);
    assert!(
        otel::is_reporting_enabled(),
        "AT-3：并发写者全部退出后显式置 true，读回必须为 true（末态不可确定）"
    );
    otel::set_reporting_enabled(false);
    assert!(
        !otel::is_reporting_enabled(),
        "AT-3：并发写者全部退出后显式置 false，读回必须为 false（末态不可确定）"
    );

    GateStormStats {
        total_flips: AT3_FLIPPERS * AT3_ITERATIONS_PER_FLIPPER,
        reader_reads: reader_reads.load(Ordering::SeqCst),
    }
}

/// AT-3：无 panic、无死锁（整体 5s 硬截止）、无脏末态。
#[test]
fn at3_gate_flip_storm_no_panic_no_deadlock() {
    let _guard = state_lock();

    let (tx, rx) = mpsc::channel();
    let worker = thread::Builder::new()
        .name("at3-gate-storm".to_string())
        .spawn(move || {
            let outcome = catch_unwind(AssertUnwindSafe(run_gate_storm))
                .map_err(|payload| panic_payload(payload.as_ref()));
            // 发送失败只可能是主线程判死锁后提前返回，此处无需再报错。
            let _ = tx.send(outcome);
        })
        .expect("AT-3：必须能创建风暴线程");

    let started = Instant::now();
    let outcome = match rx.recv_timeout(Duration::from_secs(5)) {
        Ok(outcome) => outcome,
        Err(RecvTimeoutError::Timeout) => panic!(
            "AT-3 死锁：{}×{} 次门禁翻转 + 持续读在 5s 硬截止内未完成（已等 {:?}）",
            AT3_FLIPPERS,
            AT3_ITERATIONS_PER_FLIPPER,
            started.elapsed()
        ),
        Err(RecvTimeoutError::Disconnected) => {
            panic!("AT-3：风暴线程未回报结果即退出（疑似 panic 未被捕获或线程被吞）")
        }
    };

    let stats = match outcome {
        Ok(stats) => stats,
        Err(message) => panic!("AT-3 并发翻转发生 panic：{message}"),
    };

    assert_eq!(
        stats.total_flips, 600,
        "AT-3：翻转次数必须是确定值 3×200=600，实际 {}",
        stats.total_flips
    );
    assert!(
        stats.reader_reads > 0,
        "AT-3：读线程一次都没读到门禁值（并发未发生，用例等于没测）"
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "AT-3：整体耗时 {:?} 触及 5s 上限",
        started.elapsed()
    );

    worker.join().expect("AT-3：风暴线程不得 panic");
    // 复位，避免污染同进程其他用例（本文件内由 GLOBAL_STATE_LOCK 串行）。
    otel::set_reporting_enabled(false);
}
