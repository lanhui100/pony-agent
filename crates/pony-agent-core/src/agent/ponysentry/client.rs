use super::config::PonySentryConfig;
use super::models::{Breadcrumb, Exception, Frame, IngestPayload};
use super::sanitizer::{sanitize, sanitize_json};
use std::backtrace::Backtrace;
use std::collections::{HashMap, VecDeque};
use std::panic::PanicHookInfo;
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const MAX_BREADCRUMBS: usize = 64;
const QUEUE_CAPACITY: usize = 1024;
const REQUEST_TIMEOUT_SECS: u64 = 3;
/// 栈帧采集上限（超出截断，避免把超深调用栈写穿 ingest 载荷）。
const MAX_STACK_FRAMES: usize = 32;
/// 退出前排空上报队列的有界等待上限（Wave-5 P0-B：不因网络悬挂卡住进程退出）。
const FLUSH_TIMEOUT_MS: u64 = 1_500;

#[derive(Debug)]
enum SentryEvent {
    /// 一次上报：payload + 入队时（调用方上下文）已解析好的 traceparent + 待惰性符号化的原始栈。
    ///
    /// `traceparent` 必须在**入队线程**解析：活跃 turn 位于调用方上下文，worker 线程看不到。
    /// `backtrace` 保留原始帧指针，符号化（昂贵）推迟到 worker，保证入队路径非阻塞。
    Ingest {
        payload: IngestPayload,
        traceparent: Option<String>,
        backtrace: Option<Backtrace>,
    },
    /// 排空屏障（Wave-5 P0-B）：worker 收到它即表示**此前所有事件已处理完**，
    /// 用于进程退出前有界 flush，避免 CLI `--prompt` 这种「报错即退出」形态丢 issue。
    Flush(mpsc::SyncSender<()>),
}

#[derive(Clone)]
pub struct PonySentryClient {
    pub config: PonySentryConfig,
    sender: Option<SyncSender<SentryEvent>>,
    breadcrumbs: Arc<Mutex<VecDeque<Breadcrumb>>>,
}

/// 过滤用于离线自测、E2E 诊断以及探针预埋的诊断桩错误，避免污染生产监控。
/// 与前端 `shouldIgnoreTelemetryError` 语义严格保持对齐。
pub fn should_ignore_telemetry_error(message: &str) -> bool {
    let msg = message.trim();
    if msg.contains("DIAG_STUB") || msg.contains("DIAG:") {
        return true;
    }
    if msg.contains("cannot be consumed: replay, expiry, cross-session, or version/nonce mismatch")
        || msg.contains("stale mutation rejected")
        || msg.contains("unknown control request")
        || (msg.contains("control request") && msg.contains("kind mismatch"))
    {
        return true;
    }
    false
}

impl PonySentryClient {
    pub fn new(config: PonySentryConfig) -> Self {
        let breadcrumbs = Arc::new(Mutex::new(VecDeque::with_capacity(MAX_BREADCRUMBS)));

        // fail-closed 门禁：仅当 client_token 已配置时才 spawn worker（无 token 即零 egress）。
        let sender = if config.enabled
            && config
                .client_token
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .is_some()
        {
            let (tx, rx) = mpsc::sync_channel::<SentryEvent>(QUEUE_CAPACITY);
            let worker_cfg = config.clone();
            let _handle: Option<JoinHandle<()>> = thread::Builder::new()
                .name("ponysentry-worker".to_string())
                .spawn(move || {
                    worker_loop(worker_cfg, rx);
                })
                .ok();
            Some(tx)
        } else {
            None
        };

        Self {
            config,
            sender,
            breadcrumbs,
        }
    }

    pub fn add_breadcrumb(
        &self,
        category: &str,
        message: &str,
        data: Option<HashMap<String, String>>,
    ) {
        let sanitized_msg = sanitize(message);
        let sanitized_data = data.map(|map| {
            map.into_iter()
                .map(|(k, v)| (k, sanitize(&v)))
                .collect::<HashMap<String, String>>()
        });

        let mut queue = self
            .breadcrumbs
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        if queue.len() >= MAX_BREADCRUMBS {
            queue.pop_front();
        }

        queue.push_back(Breadcrumb {
            category: category.to_string(),
            message: sanitized_msg,
            data: sanitized_data,
        });
    }

    fn take_breadcrumbs(&self) -> Vec<Breadcrumb> {
        let mut queue = self
            .breadcrumbs
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        queue.drain(..).collect()
    }

    pub fn capture_error(&self, error_type: &str, message: &str, extra: Option<serde_json::Value>) {
        self.capture_error_with_trace_hint(error_type, message, extra, None);
    }

    /// 与 [`Self::capture_error`] 相同，但允许调用方显式提供 **trace_id 提示**。
    ///
    /// 语义：hint 必须是合法 32 位小写十六进制，否则忽略并回落到 `exported_trace_id()`。
    /// 用途（Wave-6 BLOCKING）：`capture_turn_failure` 可能在与 turn 登记不同的任务/线程
    /// 上下文执行，此时按 turn_id 精确回查的导出态 id 通过本参数传入，避免 issue 丢 trace_id。
    pub fn capture_error_with_trace_hint(
        &self,
        error_type: &str,
        message: &str,
        extra: Option<serde_json::Value>,
        trace_id_hint: Option<String>,
    ) {
        // fail-closed：无 token 即零 egress（enabled 恒开，门禁由 token 承担）
        if !self.config.enabled || self.config.client_token.is_none() {
            return;
        }

        // 过滤诊断桩/测试预埋错误，避免污染生产监控（与前端 shouldIgnoreTelemetryError 保持一致）
        if should_ignore_telemetry_error(message) {
            return;
        }

        let sanitized_msg = sanitize(message);
        let sanitized_extra = extra.map(|ex| sanitize_json(&ex));
        let breadcrumbs = self.take_breadcrumbs();

        let mut tags = HashMap::new();
        tags.insert("project".to_string(), "pony-agent".to_string());

        let payload = IngestPayload {
            platform: "rust".to_string(),
            release: self.config.release.clone(),
            environment: self.config.environment.clone(),
            message: Some(sanitized_msg.clone()),
            exception: Some(Exception {
                error_type: error_type.to_string(),
                value: Some(sanitized_msg),
                // 栈帧由 worker 侧从原始 backtrace 惰性符号化后回填（C2 要求填充 stacktrace，
                // 但符号化昂贵，不得阻塞上报入队路径）。
                stacktrace: None,
            }),
            tags: Some(tags),
            extra: sanitized_extra,
            breadcrumbs: if breadcrumbs.is_empty() {
                None
            } else {
                Some(breadcrumbs)
            },
            event_id: Some(new_event_id()),
            // B1：issue 的 trace_id **只**接受导出态 id（门禁关闭/未 init/未采样 ⇒ None），
            // 绝不回落到本地相关性 id（那会让 issue 带一个「查不到链路」的假关联）。
            // Wave-6：跨上下文场景由 hint 提供（仍必须是导出态 id，非法 hint 被忽略）。
            trace_id: trace_id_hint
                .filter(|value| is_lower_hex_32(value))
                .or_else(crate::agent::otel::exported_trace_id),
            fingerprint: Some(fingerprint_for(error_type)),
            culprit: None,
        };

        // H1：本路径同样走统一清洗（culprit/stacktrace/tags 等全字段），不留脱敏旁路。
        let mut payload = payload;
        normalize_payload(&mut payload);
        // 原始帧指针采集很便宜（不符号化），符号化推迟到 worker 线程。
        self.enqueue(payload, Some(Backtrace::force_capture()));
    }

    pub fn capture_panic(&self, info: &PanicHookInfo) {
        // fail-closed：无 token 即零 egress（enabled 恒开，门禁由 token 承担）
        if !self.config.enabled || self.config.client_token.is_none() {
            return;
        }

        let message = info
            .payload()
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| {
                info.payload()
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| "unknown panic".to_string());

        let location = info.location().map(|l| l.to_string());
        let sanitized_msg = sanitize(&message);
        let breadcrumbs = self.take_breadcrumbs();

        let mut tags = HashMap::new();
        tags.insert("project".to_string(), "pony-agent".to_string());

        let payload = IngestPayload {
            platform: "rust".to_string(),
            release: self.config.release.clone(),
            environment: self.config.environment.clone(),
            message: Some(format!("panic: {sanitized_msg}")),
            exception: Some(Exception {
                error_type: "Panic".to_string(),
                value: Some(sanitized_msg),
                // panic 现场至少保留一条定位帧；真实调用栈由 worker 侧追加在前。
                stacktrace: Some(vec![Frame {
                    filename: location.clone(),
                    function: Some("panic_hook".to_string()),
                    lineno: None,
                    in_app: Some(true),
                }]),
            }),
            tags: Some(tags),
            extra: None,
            breadcrumbs: if breadcrumbs.is_empty() {
                None
            } else {
                Some(breadcrumbs)
            },
            event_id: Some(new_event_id()),
            // B1：导出态 id only（见 capture_error 同条注释）。
            trace_id: crate::agent::otel::exported_trace_id(),
            fingerprint: Some(fingerprint_for("Panic")),
            culprit: location.clone(),
        };

        // H1：panic 现场的 culprit（含文件路径）与栈帧同样必须脱敏。
        let mut payload = payload;
        normalize_payload(&mut payload);
        self.enqueue(payload, Some(Backtrace::force_capture()));
    }

    pub fn capture_payload(&self, payload: IngestPayload) {
        // fail-closed：无 token 即零 egress（enabled 恒开，门禁由 token 承担）
        if !self.config.enabled || self.config.client_token.is_none() {
            return;
        }

        // 诊断桩噪音硬过滤：message 或 exception.value 命中 DIAG_STUB 等模式则静默忽略
        if let Some(ref msg) = payload.message {
            if should_ignore_telemetry_error(msg) {
                return;
            }
        }
        if let Some(ref exc) = payload.exception {
            if let Some(ref val) = exc.value {
                if should_ignore_telemetry_error(val) {
                    return;
                }
            }
        }

        let mut payload = payload;
        // C2：IPC 路径（前端已带 event_id/trace_id）不得被丢弃；缺失项在 Rust 侧补齐。
        normalize_payload(&mut payload);
        // 前端自带 stacktrace 时不重复采集调用栈（避免无谓开销）。
        let needs_stack = payload
            .exception
            .as_ref()
            .map(|exception| exception.stacktrace.is_none())
            .unwrap_or(false);
        self.enqueue(
            payload,
            needs_stack.then(Backtrace::force_capture),
        );
    }

    /// 统一入队：在**调用方线程**解析 traceparent（活跃 turn 只在该上下文可见），
    /// 再投递到有界队列（满即丢弃，绝不阻塞）。
    fn enqueue(&self, payload: IngestPayload, backtrace: Option<Backtrace>) {
        if !self.config.enabled || self.config.client_token.is_none() {
            return;
        }
        let traceparent = crate::agent::otel::current_traceparent();
        // Wave-6：记录最终 payload（已 normalize/脱敏），供验收用例断言 trace_id 等字段。
        remember_enqueued_payload(&payload);

        if let Some(ref tx) = self.sender {
            match tx.try_send(SentryEvent::Ingest {
                payload,
                traceparent,
                backtrace,
            }) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => {
                    eprintln!("[ponysentry] queue full (1024), dropping event to protect runtime");
                }
                Err(TrySendError::Disconnected(_)) => {
                    eprintln!("[ponysentry] worker disconnected");
                }
            }
        }
    }

    /// 排空上报队列（有界等待，最多 1500ms）。
    ///
    /// 语义：投递一个 FIFO 屏障，worker 处理到屏障时说明此前事件都已尝试发送；
    /// 队列已满/worker 已断开时**立即返回**（绝不阻塞调用方，绝不 panic）。
    pub fn flush(&self) {
        let Some(tx) = self.sender.as_ref() else {
            return;
        };
        let (ack_tx, ack_rx) = mpsc::sync_channel::<()>(1);
        if tx.try_send(SentryEvent::Flush(ack_tx)).is_err() {
            return;
        }
        let _ = ack_rx.recv_timeout(Duration::from_millis(FLUSH_TIMEOUT_MS));
    }
}

/// 最近一次**入队**的 payload（测试钩子，Wave-6 BLOCKING 用例需要检查最终 payload）。
///
/// 生产路径只写不读，成本为一次 clone + Mutex 写入；仅在入队成功后记录（已过 normalize/脱敏）。
static LAST_ENQUEUED_PAYLOAD: Mutex<Option<IngestPayload>> = Mutex::new(None);

/// 记录最近入队 payload（供测试断言 trace_id 等最终字段）。
fn remember_enqueued_payload(payload: &IngestPayload) {
    let mut guard = LAST_ENQUEUED_PAYLOAD
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *guard = Some(payload.clone());
}

/// 读取最近一次入队的 payload（仅测试/诊断用）。
pub fn last_enqueued_payload() -> Option<IngestPayload> {
    let guard = LAST_ENQUEUED_PAYLOAD
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.clone()
}

/// 生成事件 id：32 位小写十六进制（无连字符，符合 C2 的 `^[0-9a-f]{32}$` 契约）。
fn new_event_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// 32 位小写十六进制校验（event_id / trace_id 共用形态契约）。
fn is_lower_hex_32(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// 分组指纹：缺省按异常类型聚合（同类型错误在服务端归并为同一 issue）。
fn fingerprint_for(error_type: &str) -> Vec<String> {
    vec![error_type.trim().to_string()]
}

/// 补齐 / 校正 C2 互跳字段：非法 event_id 一律替换为新生成的合法 id，
/// trace_id 仅接受合法 32 位小写十六进制；否则回落**导出态** id（仍取不到则 `None`，
/// 绝不写占位值、也绝不回落本地相关性 id）。
/// 同时把 IPC 载荷统一过一遍脱敏（extra 走 `sanitize_json`，文本字段走 `sanitize`）。
fn normalize_payload(payload: &mut IngestPayload) {
    if !payload
        .event_id
        .as_deref()
        .map(is_lower_hex_32)
        .unwrap_or(false)
    {
        payload.event_id = Some(new_event_id());
    }
    let trace_is_valid = payload
        .trace_id
        .as_deref()
        .map(is_lower_hex_32)
        .unwrap_or(false);
    if !trace_is_valid {
        payload.trace_id = crate::agent::otel::exported_trace_id();
    }
    if payload.fingerprint.is_none() {
        if let Some(exception) = payload.exception.as_ref() {
            payload.fingerprint = Some(fingerprint_for(&exception.error_type));
        }
    }
    // 零信任：前端传入的载荷同样必须脱敏（此前只有 add_breadcrumb/capture_error 走了清洗）。
    // H1：**所有**文本承载字段（extra / message / tags / exception / stacktrace / breadcrumbs /
    // culprit / fingerprint）统一过清洗，杜绝任何未脱敏字符串进入 payload 的路径。
    if let Some(extra) = payload.extra.as_ref() {
        payload.extra = Some(sanitize_json(extra));
    }
    if let Some(message) = payload.message.as_ref() {
        payload.message = Some(sanitize(message));
    }
    if let Some(tags) = payload.tags.as_mut() {
        for value in tags.values_mut() {
            *value = sanitize(value);
        }
    }
    if let Some(exception) = payload.exception.as_mut() {
        exception.error_type = sanitize(&exception.error_type);
        if let Some(value) = exception.value.as_ref() {
            exception.value = Some(sanitize(value));
        }
        if let Some(frames) = exception.stacktrace.as_mut() {
            for frame in frames.iter_mut() {
                if let Some(filename) = frame.filename.as_ref() {
                    frame.filename = Some(sanitize(filename));
                }
                if let Some(function) = frame.function.as_ref() {
                    frame.function = Some(sanitize(function));
                }
            }
        }
    }
    if let Some(breadcrumbs) = payload.breadcrumbs.as_mut() {
        for breadcrumb in breadcrumbs.iter_mut() {
            breadcrumb.category = sanitize(&breadcrumb.category);
            breadcrumb.message = sanitize(&breadcrumb.message);
            if let Some(data) = breadcrumb.data.as_mut() {
                for value in data.values_mut() {
                    *value = sanitize(value);
                }
            }
        }
    }
    if let Some(culprit) = payload.culprit.as_ref() {
        payload.culprit = Some(sanitize(culprit));
    }
    if let Some(fingerprint) = payload.fingerprint.as_mut() {
        for part in fingerprint.iter_mut() {
            *part = sanitize(part);
        }
    }
}

/// 把原始调用栈符号化为 PonySentry `Frame` 列表（worker 侧调用，失败返回空）。
///
/// 解析 `std::backtrace` 的文本形态：`N: <function>` 后跟可选的 `at <file>:<line>:<col>`。
/// 语义：最多保留 [`MAX_STACK_FRAMES`] 帧；标准库/依赖帧标记 `in_app=false`；
/// 函数名与文件名同样过 [`sanitize`]（栈里可能含家目录/令牌）。
fn symbolize_backtrace(backtrace: &Backtrace) -> Vec<Frame> {
    let text = backtrace.to_string();
    let mut frames: Vec<Frame> = Vec::new();
    let mut current: Option<Frame> = None;

    for raw_line in text.lines() {
        let line = raw_line.trim();
        if let Some(function) = strip_frame_index(line) {
            if let Some(frame) = current.take() {
                frames.push(frame);
            }
            current = Some(Frame {
                filename: None,
                function: Some(sanitize(function)),
                lineno: None,
                in_app: None,
            });
        } else if let Some(location) = line.strip_prefix("at ") {
            let (filename, lineno) = split_location(location);
            let in_app = filename
                .as_deref()
                .map(|path| is_app_path(path))
                .unwrap_or(false);
            let sanitized_filename = filename.map(|path| sanitize(&path));
            match current.as_mut() {
                Some(frame) => {
                    frame.filename = sanitized_filename;
                    frame.lineno = lineno;
                    frame.in_app = Some(in_app);
                }
                None => {
                    current = Some(Frame {
                        filename: sanitized_filename,
                        function: None,
                        lineno,
                        in_app: Some(in_app),
                    })
                }
            }
        }
        if frames.len() >= MAX_STACK_FRAMES {
            break;
        }
    }
    if let Some(frame) = current.take() {
        frames.push(frame);
    }
    frames.truncate(MAX_STACK_FRAMES);
    frames
}

/// 识别 `N: function` 形态的帧头，返回函数名部分（非帧头返回 `None`）。
fn strip_frame_index(line: &str) -> Option<&str> {
    let (index, rest) = line.split_once(':')?;
    if index.is_empty() || !index.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let rest = rest.trim();
    if rest.is_empty() {
        None
    } else {
        Some(rest)
    }
}

/// 拆分 `file:line:col`：返回（文件名，行号）；列号丢弃，无法解析的部分置 `None`。
fn split_location(location: &str) -> (Option<String>, Option<u32>) {
    let trimmed = location.trim();
    let mut parts = trimmed.rsplitn(3, ':');
    let third = parts.next();
    let second = parts.next();
    match (third, second) {
        (Some(_column), Some(line)) => match line.trim().parse::<u32>() {
            Ok(lineno) => {
                // 第三个片段是列号；文件名 = 去掉最后两段后的前缀（用 rsplitn 再取一次）。
                let filename = trimmed
                    .rsplitn(3, ':')
                    .nth(2)
                    .map(|prefix| prefix.to_string())
                    .filter(|prefix| !prefix.is_empty());
                (filename, Some(lineno))
            }
            Err(_) => (Some(trimmed.to_string()), None),
        },
        _ => (Some(trimmed.to_string()), None),
    }
}

/// 是否属于本项目代码（非 registry 依赖 / 非标准库路径）。
fn is_app_path(path: &str) -> bool {
    !path.contains("/registry/") && !path.contains("library/std") && !path.contains("rustc/")
}

/// worker 侧回填：把符号化后的栈与 culprit 写入载荷（已有栈则追加在后，保留定位帧）。
fn apply_stacktrace(payload: &mut IngestPayload, backtrace: Option<Backtrace>) {
    let Some(backtrace) = backtrace else {
        return;
    };
    let captured = symbolize_backtrace(&backtrace);
    if captured.is_empty() {
        return;
    }
    if payload.culprit.is_none() {
        // 尽力填「肇事点」：优先本仓库中**非**上报模块自身的帧（避免 culprit 永远是 ponysentry），
        // 其次 in_app 帧，最后退回首个帧。
        payload.culprit = captured
            .iter()
            .find(|frame| {
                frame
                    .function
                    .as_deref()
                    .map(|name| name.contains("pony_agent_core") && !name.contains("ponysentry"))
                    .unwrap_or(false)
            })
            .or_else(|| captured.iter().find(|frame| frame.in_app == Some(true)))
            .or_else(|| captured.first())
            .and_then(|frame| frame.function.clone().or_else(|| frame.filename.clone()));
    }
    if let Some(exception) = payload.exception.as_mut() {
        match exception.stacktrace.as_mut() {
            Some(existing) if !existing.is_empty() => {
                existing.extend(captured);
                existing.truncate(MAX_STACK_FRAMES);
            }
            _ => exception.stacktrace = Some(captured),
        }
    }
}

fn worker_loop(config: PonySentryConfig, receiver: mpsc::Receiver<SentryEvent>) {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("[ponysentry] failed to create worker tokio runtime: {e}");
            return;
        }
    };
    let _guard = rt.enter();

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[ponysentry] failed to build HTTP client: {e}");
            return;
        }
    };

    let base_url = config.endpoint.trim_end_matches('/');
    let ingest_url = format!("{base_url}/api/v1/ingest");

    while let Ok(event) = receiver.recv() {
        let (mut payload, traceparent, backtrace) = match event {
            SentryEvent::Ingest {
                payload,
                traceparent,
                backtrace,
            } => (payload, traceparent, backtrace),
            // FIFO 屏障：此前事件都已处理完，回 ack 即可继续处理后续事件。
            SentryEvent::Flush(ack) => {
                let _ = ack.try_send(());
                continue;
            }
        };
        // 符号化（昂贵）落在 worker 线程，绝不占用上报入队路径。
        apply_stacktrace(&mut payload, backtrace);

        let url = ingest_url.clone();
        let body_json = serde_json::to_value(&payload);

        let body = match body_json {
            Ok(b) => b,
            Err(e) => {
                eprintln!("[ponysentry] Failed to serialize event: {e}");
                continue;
            }
        };

        let mut req = client
            .post(&url)
            .header("Content-Type", "application/json");

        if let Some(ref token) = config.client_token {
            req = req.header("X-Client-Token", token);
        }

        // Wave-4 C3：入队时（活跃 turn 上下文）解析好的 traceparent 在这里落到出网请求头。
        // 用 `headers()`（内部 `Entry::insert` 语义）而非 `header()`（append），避免重复头。
        if let Some(traceparent) = traceparent.as_deref() {
            if let Some(headers) = crate::agent::otel::traceparent_header_map_from(traceparent) {
                req = req.headers(headers);
            }
        }

        let send_fut = req.json(&body).send();

        rt.block_on(async {
            match send_fut.await {
                Ok(resp) => {
                    if !resp.status().is_success() {
                        eprintln!("[ponysentry] Ingest responded with status {}", resp.status());
                    }
                }
                Err(e) => {
                    eprintln!("[ponysentry] Failed to send event to {url}: {e}");
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

    /// H1：IPC 载荷的**所有**文本承载字段（culprit / stacktrace.filename /
    /// breadcrumbs / extra / message / tags）都必须脱敏，不存在旁路。
    #[test]
    fn h1_ipc_payload_fields_are_all_sanitized() {
        let mut breadcrumb_data = HashMap::new();
        breadcrumb_data.insert("path".to_string(), "/home/alice/secret.rs".to_string());
        breadcrumb_data.insert(
            "header".to_string(),
            "Bearer abcdefghij".to_string(),
        );

        let mut payload = IngestPayload {
            platform: "tauri".to_string(),
            release: "0.1.155".to_string(),
            environment: "production".to_string(),
            message: Some("boom Bearer abcdefghij".to_string()),
            exception: Some(Exception {
                error_type: "TypeError".to_string(),
                value: Some("failed at /home/alice/secret.rs".to_string()),
                stacktrace: Some(vec![Frame {
                    filename: Some("/home/alice/secret.rs".to_string()),
                    function: Some("do_thing Bearer abcdefghij".to_string()),
                    lineno: Some(12),
                    in_app: Some(true),
                }]),
            }),
            tags: Some(HashMap::from([(
                "path".to_string(),
                "/home/alice/secret.rs".to_string(),
            )])),
            extra: Some(json!({
                "access_tokens": "secret123",
                "input_tokens": 42,
                "trace": "Bearer abcdefghij"
            })),
            breadcrumbs: Some(vec![Breadcrumb {
                category: "ui Bearer abcdefghij".to_string(),
                message: "clicked /home/alice/secret.rs".to_string(),
                data: Some(breadcrumb_data),
            }]),
            event_id: None,
            trace_id: None,
            fingerprint: None,
            culprit: Some("/home/alice/secret.rs @ Bearer abcdefghij".to_string()),
        };

        normalize_payload(&mut payload);

        let rendered = serde_json::to_string(&payload).expect("载荷必须可序列化");
        for leaked in ["abcdefghij", "/home/alice", "secret123"] {
            assert!(
                !rendered.contains(leaked),
                "H1：脱敏旁路，payload 中仍出现 `{leaked}`：{rendered}"
            );
        }
        assert!(
            rendered.contains("[REDACTED_SECRET]"),
            "必须出现凭据脱敏标记：{rendered}"
        );
        assert!(
            rendered.contains("[USER_HOME]"),
            "必须出现家目录脱敏标记：{rendered}"
        );
        // 计量键仍保持数字原值（H2 精确白名单）。
        assert_eq!(payload.extra.as_ref().expect("extra")["input_tokens"], 42);
        // 互跳字段仍被补齐。
        assert_eq!(payload.event_id.as_deref().map(str::len), Some(32));
    }

    #[test]
    fn test_should_ignore_diag_stubs_and_control_noise() {
        assert!(should_ignore_telemetry_error("IPC command \"ask_list_pending\" failed: DIAG_STUB_UNIMPLEMENTED"));
        assert!(should_ignore_telemetry_error("load_session_runtime_view failed: DIAG: sessions.db read failed"));
        assert!(should_ignore_telemetry_error("cannot be consumed: replay, expiry, cross-session, or version/nonce mismatch"));
        assert!(should_ignore_telemetry_error("stale mutation rejected"));
        assert!(!should_ignore_telemetry_error("Network timeout connecting to LLM provider"));
        assert!(!should_ignore_telemetry_error("Panic occurred in worker thread"));
    }

    /// H1 反向：`capture_panic` 的 culprit（panic 文件路径）同样被清洗。
    #[test]
    fn h1_panic_culprit_shape_is_sanitized_by_normalize() {
        let mut payload = IngestPayload {
            platform: "rust".to_string(),
            release: "0.1.155".to_string(),
            environment: "dev".to_string(),
            message: Some("panic: boom".to_string()),
            exception: Some(Exception {
                error_type: "Panic".to_string(),
                value: Some("boom".to_string()),
                stacktrace: Some(vec![Frame {
                    filename: Some("/home/alice/secret.rs:1:1".to_string()),
                    function: Some("panic_hook".to_string()),
                    lineno: None,
                    in_app: Some(true),
                }]),
            }),
            tags: None,
            extra: None,
            breadcrumbs: None,
            event_id: None,
            trace_id: None,
            fingerprint: None,
            culprit: Some("/home/alice/secret.rs:1:1".to_string()),
        };
        normalize_payload(&mut payload);
        let rendered = serde_json::to_string(&payload).expect("serialize");
        assert!(!rendered.contains("/home/alice"), "culprit 未脱敏：{rendered}");
        assert!(rendered.contains("[USER_HOME]"), "缺少家目录脱敏位：{rendered}");
    }
}
