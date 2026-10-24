//! 最小 JSON 日志面（Wave43 D8）。
//!
//! 契约：只服务本次新增的 trace 相关代码路径，**不替换**存量 `eprintln!`
//! （PA-119 债务豁免范围不得借本次扩大）。

use std::io::Write;

/// 输出一行 JSON 日志。
///
/// 调用方：trace 装配与导出失败降级路径（例如导出错误时 `warn` 一条）。
/// 语义：单行 JSON，至少含 `level` / `message` / `ts` / `trace_id` 四个字段；
/// 输出目标为进程 **stdout**（供宿主/运维按行采集），不做异步排队、不依赖日志框架。
/// `level` 取字面量 `"debug"` / `"info"` / `"warn"` / `"error"`——**调用点必须直接写
/// 字面量**（本模块不提供 level 常量，以免静态扫描断言失效）。
/// `trace_id` 为 `None` 时该字段输出 JSON `null`，不得省略键。
/// 失败语义：本函数不返回错误、不 panic；写 stdout 失败时静默丢弃。
pub fn log_json(level: &str, message: &str, trace_id: Option<&str>) {
    let timestamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let record = serde_json::json!({
        "level": level,
        "message": message,
        "ts": timestamp,
        // Option 序列化为 null 时键仍然存在（契约：不得省略 trace_id 键）。
        "trace_id": trace_id,
    });

    let Ok(line) = serde_json::to_string(&record) else {
        return;
    };
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    let _ = writeln!(handle, "{line}");
    let _ = handle.flush();
}
