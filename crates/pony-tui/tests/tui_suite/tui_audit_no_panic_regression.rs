//! 批4（task-11）N19 验收测试：`tools/tui-audit/checks.py` 的 `no_panic` **双源三态**判定（F-K）。
//!
//! ## 为什么需要这条用例（原缺陷是「恒真审计断言」）
//! TUI 启动后 stderr 被 dup2 重定向进 `~/.pony-agent/tui-logs/tui-runtime.log`，因此 tmux capture
//! 里**结构上不可能**出现 panic 文本 —— 旧版 `<cap>.no_panic` 只扫 capture，等于永远 PASS。
//! 修复后必须「capture 无 panic **且** 运行时日志无 panic；日志缺失时显式 skipped 而不是静默算通过」。
//!
//! ## 测试口径（与 exec-rust 落地的可测入口一一对应）
//! 1. 纯函数层：`panic_scan(text) -> list[str]`、`no_panic_verdict(capture_text, log_text) ->
//!    (status, evidence)`（`status ∈ {"pass","fail","skipped"}`）。用 `python3 -c` + importlib 直接
//!    调用 —— 这是最精确的三态判别（能区分「capture 干净但日志有 panic」这一关键场景）。
//! 2. 接线层：真实跑 `python3 tools/tui-audit/checks.py check --dir <tmp证据目录>`，用环境变量
//!    `PONY_TUI_RUNTIME_LOG` 指向临时日志，读回 `assertions.json` 断言 `main.no_panic` 的
//!    `status` 三态，并校验顶层 `pass + fail + skipped == len(checks)` 的结构不变量。
//!
//! ## 隔离
//! 全部证据/日志都在 `std::env::temp_dir()` 下的唯一目录内，`TempDir` 守卫 Drop 时递归删除；
//! 不读写真实 `~/.pony-agent`（日志路径由 env 显式指向临时文件）。
//!
//! 铁律：真实断言、无 `#[ignore]`/skip；不修改实现代码。

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

/// 仓库根（`CARGO_MANIFEST_DIR` = crates/pony-agent-core）。
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/pony-agent-core 应位于仓库根下两层")
        .to_path_buf()
}

fn checks_py() -> PathBuf {
    repo_root().join("tools/tui-audit/checks.py")
}

/// 唯一临时目录守卫（谁创建谁清理）。
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "pony-agent-it-batch4-{label}-{}-{stamp}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("create isolated temp dir");
        Self { path }
    }

    fn join(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// 跑一段 Python 代码并把它 stdout 解析为 JSON。
/// 失败时把 stderr 带进 panic 信息（便于红相定位，而不是只报「断言失败」）。
fn python_json(code: &str) -> Value {
    let output = Command::new("python3")
        .arg("-c")
        .arg(code)
        .output()
        .expect("python3 必须可用（审计脚本的运行前置）");
    assert!(
        output.status.success(),
        "python3 脚本执行失败：stdout={:?} stderr={:?}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    serde_json::from_str(stdout.trim())
        .unwrap_or_else(|error| panic!("stdout 不是合法 JSON：{error}; raw={stdout:?}"))
}

/// 构造最小可用的审计证据目录（`check --dir` 的输入契约：meta.json + <cap>.txt/…ansi/…png）。
fn write_evidence_dir(dir: &TempDir, capture_lines: &[&str]) {
    std::fs::write(
        dir.join("meta.json"),
        "{\"scenario\":\"empty\",\"cols\":100,\"rows\":30}\n",
    )
    .expect("write meta.json");
    let mut lines: Vec<String> = capture_lines.iter().map(|line| (*line).to_string()).collect();
    while lines.len() < 30 {
        lines.push(String::new());
    }
    std::fs::write(dir.join("main.txt"), lines.join("\n")).expect("write capture text");
    // 审计脚本会顺带读取 ansi/png 附属产物；本用例只关心 no_panic 判定，故给最小占位。
    std::fs::write(dir.join("main.ansi"), "ansi-stub").expect("write capture ansi stub");
    std::fs::write(dir.join("main.png"), "png-stub").expect("write capture png stub");
}

/// 跑 `checks.py check --dir`，回传解析后的 assertions.json。
fn run_check_cli(evidence_dir: &TempDir, runtime_log: Option<&Path>) -> Value {
    let mut command = Command::new("python3");
    command
        .arg(checks_py())
        .arg("check")
        .arg("--dir")
        .arg(&evidence_dir.path)
        .current_dir(repo_root());
    match runtime_log {
        Some(path) => {
            command.env("PONY_TUI_RUNTIME_LOG", path);
        }
        None => {
            command.env_remove("PONY_TUI_RUNTIME_LOG");
        }
    }

    let output = command.output().expect("python3 必须可用");
    assert!(
        output.status.success(),
        "checks.py check 失败：stdout={:?} stderr={:?}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let assertions_path = evidence_dir.join("assertions.json");
    let raw = std::fs::read_to_string(&assertions_path)
        .expect("checks.py check 必须产出 assertions.json");
    serde_json::from_str(&raw).expect("assertions.json 必须是合法 JSON")
}

/// 取出 `no_panic` 条目（id 形如 `<capture>.no_panic`）。
fn no_panic_item(payload: &Value) -> &Value {
    payload["checks"]
        .as_array()
        .expect("assertions.json 必须含 checks 数组")
        .iter()
        .find(|item| {
            item["id"]
                .as_str()
                .map(|id| id.ends_with(".no_panic"))
                .unwrap_or(false)
        })
        .expect("assertions.json 必须包含 <capture>.no_panic 条目")
}

fn status_of(item: &Value) -> &str {
    item["status"]
        .as_str()
        .unwrap_or_else(|| panic!("no_panic 条目必须带 status 字段（三态判别），实际条目 = {item}"))
}

// ── N19a：纯函数三态矩阵（最精确的判别面） ────────────────────────────────────────

#[test]
fn n19a_no_panic_verdict_distinguishes_three_states() {
    let checks = checks_py();
    let code = format!(
        r#"
import importlib.util, json
spec = importlib.util.spec_from_file_location("tui_checks", {path:?})
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
out = {{
    "skipped": list(module.no_panic_verdict("all good", None)),
    "capture_fail": list(module.no_panic_verdict("thread 'main' panicked at src/x.rs:1:2", "all good")),
    "log_fail": list(module.no_panic_verdict("all good", "thread 'turn' panicked at tui.rs:42:1")),
    "pass": list(module.no_panic_verdict("all good", "session started\nready\n")),
    "scan_clean": list(module.panic_scan("session started\nready\n")),
    "scan_capture": list(module.panic_scan("thread 'main' panicked at src/x.rs:1:2")),
    "scan_backtrace": list(module.panic_scan("stack backtrace:\n   0: std::panicking")),
}}
print(json.dumps(out, ensure_ascii=False))
"#,
        path = checks.to_string_lossy(),
    );
    let value = python_json(&code);

    let pair = |key: &str| -> (String, String) {
        let entry = value[key]
            .as_array()
            .unwrap_or_else(|| panic!("no_panic_verdict 必须返回二元组，key={key}"));
        assert_eq!(entry.len(), 2, "no_panic_verdict 必须返回 (status, evidence)");
        (
            entry[0].as_str().expect("status 必须是字符串").to_string(),
            entry[1].as_str().expect("evidence 必须是字符串").to_string(),
        )
    };

    // ① 日志缺失 ⇒ skipped（**不得**算 pass）。
    let (status, evidence) = pair("skipped");
    assert_eq!(
        status, "skipped",
        "N19a: 运行时日志缺失时必须标记 skipped（静默 pass 会让审计门禁重新变成恒真）"
    );
    assert!(
        evidence.to_lowercase().contains("log"),
        "N19a: skipped 证据必须说明日志不可用，实际 = {evidence:?}"
    );

    // ② capture 命中 panic ⇒ fail。
    let (status, evidence) = pair("capture_fail");
    assert_eq!(status, "fail", "N19a: capture 出现 panic 必须判 fail");
    assert!(
        evidence.contains("capture"),
        "N19a: fail 证据必须指明 panic 来自 capture，实际 = {evidence:?}"
    );

    // ③ capture 干净但运行时日志有 panic ⇒ fail（**这是旧实现必然漏判的场景**）。
    let (status, evidence) = pair("log_fail");
    assert_eq!(
        status, "fail",
        "N19a: capture 干净但运行时日志出现 panic 时必须仍判 fail（双源的核心）"
    );
    assert!(
        evidence.to_lowercase().contains("log"),
        "N19a: fail 证据必须指明 panic 来自运行时日志，实际 = {evidence:?}"
    );

    // ④ 双源都干净 ⇒ pass。
    let (status, _evidence) = pair("pass");
    assert_eq!(status, "pass", "N19a: capture 与日志都干净时必须判 pass");

    // ⑤ panic_scan 自身：干净返回空列表；命中/回溯各自返回非空。
    let scan_clean = value["scan_clean"].as_array().expect("panic_scan 必须返回 list");
    assert!(scan_clean.is_empty(), "N19a: 干净日志的 panic_scan 必须为空");
    assert!(
        !value["scan_capture"]
            .as_array()
            .expect("panic_scan 必须返回 list")
            .is_empty(),
        "N19a: 含 panicked at 的文本必须被 panic_scan 命中"
    );
    assert!(
        !value["scan_backtrace"]
            .as_array()
            .expect("panic_scan 必须返回 list")
            .is_empty(),
        "N19a: 含 stack backtrace 的文本必须被 panic_scan 命中"
    );
}

// ── N19b：真实 CLI 接线（env 注入日志路径 + assertions.json 三态） ─────────────────

#[test]
fn n19b_check_cli_fails_when_runtime_log_has_panic() {
    let evidence = TempDir::new("audit-fail");
    write_evidence_dir(&evidence, &["pony agent", "  键入 / 查看全部命令"]);

    let log_dir = TempDir::new("audit-fail-log");
    let log_path = log_dir.join("tui-runtime.log");
    std::fs::write(
        &log_path,
        "session started\nthread 'turn-1' panicked at crates/pony-agent-core/src/bin/tui.rs:42:1\n",
    )
    .expect("write panicking runtime log");

    let payload = run_check_cli(&evidence, Some(&log_path));
    let item = no_panic_item(&payload);

    assert_eq!(
        status_of(item),
        "fail",
        "N19b: 运行时日志含 panic 时 no_panic 必须判 fail（capture 本身是干净的，这正是双源要抓的场景）"
    );
    assert!(
        payload["skipped"].is_number(),
        "N19b: assertions.json 顶层必须含 skipped 计数（三态统计口径），实际 keys = {:?}",
        payload.as_object().map(|map| map.keys().cloned().collect::<Vec<_>>())
    );
}

#[test]
fn n19c_check_cli_passes_with_clean_runtime_log() {
    let evidence = TempDir::new("audit-pass");
    write_evidence_dir(&evidence, &["pony agent", "  键入 / 查看全部命令"]);

    let log_dir = TempDir::new("audit-pass-log");
    let log_path = log_dir.join("tui-runtime.log");
    std::fs::write(&log_path, "session started\nready\n").expect("write clean runtime log");

    let payload = run_check_cli(&evidence, Some(&log_path));
    let item = no_panic_item(&payload);

    assert_eq!(
        status_of(item),
        "pass",
        "N19c: 双源都干净时 no_panic 必须判 pass"
    );
    assert_eq!(
        payload["skipped"].as_u64(),
        Some(0),
        "N19c: 日志可用时 skipped 计数必须为 0"
    );
}

#[test]
fn n19d_check_cli_skips_instead_of_silently_passing_when_log_missing() {
    let evidence = TempDir::new("audit-skipped");
    write_evidence_dir(&evidence, &["pony agent", "  键入 / 查看全部命令"]);

    // 指向一个确定不存在的日志文件。
    let log_dir = TempDir::new("audit-skipped-log");
    let missing_log = log_dir.join("does-not-exist.log");
    assert!(!missing_log.exists(), "N19d 前置条件：该日志路径必须不存在");

    let payload = run_check_cli(&evidence, Some(&missing_log));
    let item = no_panic_item(&payload);
    let status = status_of(item);

    assert_eq!(
        status, "skipped",
        "N19d: 日志缺失必须显式 skipped，实际 status = {status:?}（旧实现会静默算 pass）"
    );
    assert_ne!(
        status, "pass",
        "N19d: 日志缺失绝不可计为通过（这正是被修掉的恒真断言形态）"
    );
    assert!(
        payload["skipped"].as_u64().unwrap_or(0) >= 1,
        "N19d: 顶层 skipped 计数必须 >= 1，实际 = {}",
        payload["skipped"]
    );

    // 结构不变量：三态计数必须覆盖全部条目（不允许有既非 pass/fail/skipped 的漏计）。
    let checks_len = payload["checks"]
        .as_array()
        .expect("checks 必须是数组")
        .len() as u64;
    let pass = payload["pass"].as_u64().expect("pass 必须是数字");
    let fail = payload["fail"].as_u64().expect("fail 必须是数字");
    let skipped = payload["skipped"].as_u64().expect("skipped 必须是数字");
    assert_eq!(
        pass + fail + skipped,
        checks_len,
        "N19d: pass+fail+skipped 必须等于 checks 总数（skipped 不得被重复计入 pass 或 fail）"
    );
}
