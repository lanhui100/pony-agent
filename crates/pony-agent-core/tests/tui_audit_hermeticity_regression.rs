//! 批4 补丁 / 审计封闭性（hermeticity）回归：`no_panic` 双源判定**不得被旧日志污染**。
//!
//! ## 背景（真实事故）
//! 真终端 tmux 审计跑出 336/369，31 条 `no_panic` 全判失败，根因不是 TUI 崩了，而是判定进程
//! 读了**真实** `~/.pony-agent/tui-logs/tui-runtime.log` 里 2026-10-22 的历史 panic —— 即
//! F-K 的双源判定在「作用域」上不封闭：`runtime_log_path()` 在缺省时 `expanduser("~")` 解析到
//! 真实用户目录，而 `run_audit.sh` 调 `checks.py check` 时没有把日志作用域绑到「本次运行」。
//!
//! ## 本文件锁死两件事
//! - **H1 旧日志不得污染判定**：把「运行时日志」显式指向干净日志 B ⇒ 必须 `pass`；指向含
//!   `panicked at` 的日志 A ⇒ 必须 `fail`；路径不存在 ⇒ 必须 `skipped`。三态严格区分，
//!   且额外断言注入优先级（显式参数 > per-run override > 环境变量 > `$HOME` 缺省）——
//!   即「判定结果由我指定的作用域决定」，而不是由真实用户目录的历史内容决定。
//! - **H2 作用域必须绑定到本次运行**：`run_audit.sh` 调用 `checks.py check` 时必须把运行时日志
//!   作用域注入该次运行（环境变量 `PONY_TUI_RUNTIME_LOG` / `extra.json["runtime_log"]` /
//!   该调用行上的 `HOME=`），否则判定仍会漂到真实 `~/.pony-agent`。
//!   若实现改为「只扫本次运行新增字节」的偏移入口（`read_runtime_log(path, since=...)` 之类），
//!   H2 会**自动升级**为语义断言：旧内容 + 新增干净内容 + `since=旧长度` ⇒ pass；`since=0` ⇒ fail。
//!   （当前实现尚未提供偏移入口，故 H2 走静态证据分支——文件头如实注明，不做跳过式放行。）
//!
//! ## 隔离
//! 所有日志/证据目录都在 `std::env::temp_dir()` 下的唯一目录内（`TempDir` 守卫 Drop 递归删除）；
//! 需要 `$HOME` 语义的地方**只对子进程**设置 `HOME`，**不修改本进程环境**（cargo 同二进制内并行
//! 跑用例，进程级 env 改动会互相污染）。
//!
//! 铁律：真实断言、无 `#[ignore]`/skip；不修改 `src/**` 与 `tools/**`。

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

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

fn run_audit_sh() -> PathBuf {
    repo_root().join("tools/tui-audit/run_audit.sh")
}

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
            "pony-agent-it-hermetic-{label}-{}-{stamp}",
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

/// 在隔离环境里跑一段 Python 代码，返回 stdout 解析出的 JSON。
///
/// - `HOME`：仅对本子进程生效（避免污染同二进制其它用例）；
/// - `PONY_TUI_RUNTIME_LOG`：可显式设置或移除；
/// - 失败时把 stderr 带进 panic 信息。
fn python_json_with_env(code: &str, home: Option<&Path>, runtime_log: Option<Option<&Path>>) -> Value {
    let mut command = Command::new("python3");
    command.arg("-c").arg(code).current_dir(repo_root());
    if let Some(home) = home {
        command.env("HOME", home);
        // Windows 可移植性（wave-61-team CI 修复）：CPython 在 Windows 上用 `USERPROFILE`
        // （回退 HOMEDRIVE+HOMEPATH）解析 `~`，不读 `HOME` —— 只注入 HOME 时
        // `runtime_log_path()` 仍指向 runner 真实家目录，H1c 断言必红。
        // 这里同时注入 Windows 家目录变量，使「HOME 重定向」在两个平台语义一致。
        command.env("USERPROFILE", home);
        command.env("HOMEDRIVE", home.parent().unwrap_or(home));
        command.env("HOMEPATH", home.file_name().unwrap_or_default());
    }
    match runtime_log {
        Some(Some(path)) => {
            command.env("PONY_TUI_RUNTIME_LOG", path);
        }
        Some(None) => {
            command.env_remove("PONY_TUI_RUNTIME_LOG");
        }
        None => {}
    }

    let output = command.output().expect("python3 必须可用（审计脚本运行前置）");
    assert!(
        output.status.success(),
        "python3 执行失败：stdout={:?} stderr={:?}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    serde_json::from_str(stdout.trim())
        .unwrap_or_else(|error| panic!("stdout 不是合法 JSON：{error}; raw={stdout:?}"))
}

fn verdict_pair(value: &Value, key: &str) -> (String, String) {
    let entry = value[key]
        .as_array()
        .unwrap_or_else(|| panic!("no_panic_verdict 必须返回二元组，key={key}"));
    assert_eq!(entry.len(), 2, "no_panic_verdict 必须返回 (status, evidence)");
    (
        entry[0].as_str().expect("status 必须是字符串").to_string(),
        entry[1].as_str().expect("evidence 必须是字符串").to_string(),
    )
}

/// 造一份含 panic 的历史日志（模拟真实事故里 2026-10-22 的旧 `tui-runtime.log`）。
fn write_panicking_log(path: &Path) {
    std::fs::write(
        path,
        "2026-10-22T10:11:12Z session started\n\
2026-10-22T10:11:13Z thread 'turn-7' panicked at crates/pony-agent-core/src/bin/tui.rs:42:1\n\
2026-10-22T10:11:13Z stack backtrace:\n",
    )
    .expect("write panicking legacy log");
}

fn write_clean_log(path: &Path) {
    std::fs::write(path, "2026-11-02T09:00:00Z session started\nready\n")
        .expect("write clean runtime log");
}

// ── H1：旧日志不得污染判定（三态严格区分 + 作用域优先级） ──────────────────────────

#[test]
fn h1a_verdict_is_driven_by_the_log_it_is_pointed_at() {
    let dir = TempDir::new("h1a");
    let log_a = dir.join("legacy-panicking.log");
    let log_b = dir.join("this-run-clean.log");
    let log_missing = dir.join("never-written.log");
    write_panicking_log(&log_a);
    write_clean_log(&log_b);
    assert!(!log_missing.exists(), "H1a 前置条件：缺失日志路径必须不存在");

    let code = format!(
        r#"
import importlib.util, json
spec = importlib.util.spec_from_file_location("tui_checks", {checks:?})
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
out = {{
    "at_clean": list(module.no_panic_verdict("all good", module.read_runtime_log({clean:?}))),
    "at_panicking": list(module.no_panic_verdict("all good", module.read_runtime_log({panicking:?}))),
    "at_missing": list(module.no_panic_verdict("all good", module.read_runtime_log({missing:?}))),
}}
print(json.dumps(out, ensure_ascii=False))
"#,
        checks = checks_py().to_string_lossy(),
        clean = log_b.to_string_lossy(),
        panicking = log_a.to_string_lossy(),
        missing = log_missing.to_string_lossy(),
    );
    let value = python_json_with_env(&code, None, None);

    let (clean_status, clean_evidence) = verdict_pair(&value, "at_clean");
    let (dirty_status, dirty_evidence) = verdict_pair(&value, "at_panicking");
    let (missing_status, missing_evidence) = verdict_pair(&value, "at_missing");

    // ① 指向**本次运行的干净日志** ⇒ pass（旧日志不在作用域内，不得影响判定）。
    assert_eq!(
        clean_status, "pass",
        "H1a: 指向干净日志必须 pass，实际 evidence = {clean_evidence:?}"
    );
    // ② 指向含 panic 的日志 ⇒ fail。
    assert_eq!(
        dirty_status, "fail",
        "H1a: 指向含 panicked at 的日志必须 fail，实际 evidence = {dirty_evidence:?}"
    );
    // ③ 日志不可得 ⇒ skipped（既不算 pass 也不算 fail）。
    assert_eq!(
        missing_status, "skipped",
        "H1a: 日志缺失必须 skipped，实际 evidence = {missing_evidence:?}"
    );

    // 三态必须严格互不相同（否则「skipped 混进 pass」这类回归会漏判）。
    assert_ne!(clean_status, dirty_status, "H1a: pass 与 fail 必须可区分");
    assert_ne!(clean_status, missing_status, "H1a: pass 与 skipped 必须可区分");
    assert_ne!(dirty_status, missing_status, "H1a: fail 与 skipped 必须可区分");
}

#[test]
fn h1b_log_scope_precedence_keeps_real_home_out_of_the_verdict() {
    // 造一个「假真实 HOME」：里面正是事故形态的历史 panic 日志。
    let home = TempDir::new("h1b-home");
    let legacy_log = home
        .join(".pony-agent")
        .join("tui-logs")
        .join("tui-runtime.log");
    std::fs::create_dir_all(legacy_log.parent().expect("legacy log parent"))
        .expect("create legacy log dir");
    write_panicking_log(&legacy_log);

    // 本次运行的干净日志 B。
    let run_dir = TempDir::new("h1b-run");
    let run_log = run_dir.join("tui-runtime.log");
    write_clean_log(&run_log);

    let code = format!(
        r#"
import importlib.util, json
spec = importlib.util.spec_from_file_location("tui_checks", {checks:?})
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

def verdict_for(path):
    return list(module.no_panic_verdict("all good", module.read_runtime_log(path)))

out = {{
    # ① 环境变量注入：作用域 = 本次运行日志
    "env_path": module.runtime_log_path(),
    "env_verdict": verdict_for(module.runtime_log_path()),
    # ② 显式参数注入：作用域 = 指定日志
    "explicit_path": module.runtime_log_path({run_log:?}),
    "explicit_verdict": verdict_for(module.runtime_log_path({run_log:?})),
    # ③ per-run override（extra.json 通道）：优先级高于 env
    "override_verdict": (module.set_runtime_log_override({run_log:?}) or verdict_for(module.runtime_log_path())),
    "override_path": module.runtime_log_path(),
}}
module.set_runtime_log_override(None)
print(json.dumps(out, ensure_ascii=False))
"#,
        checks = checks_py().to_string_lossy(),
        run_log = run_log.to_string_lossy(),
    );
    // 关键：子进程 HOME 指向「假真实 HOME」（内含历史 panic 日志），env 指向本次运行日志。
    let value = python_json_with_env(&code, Some(&home.path), Some(Some(&run_log)));

    let env_path = value["env_path"].as_str().expect("env_path 必须是字符串");
    assert_eq!(
        env_path,
        run_log.to_string_lossy(),
        "H1b: 环境变量注入必须压过 $HOME 缺省路径（这正是事故的边界）"
    );
    let (env_status, env_evidence) = verdict_pair(&value, "env_verdict");
    assert_eq!(
        env_status, "pass",
        "H1b: HOME 里躺着历史 panic 日志时，只要作用域指向本次运行日志就必须 pass，实际 evidence = {env_evidence:?}"
    );

    let explicit_path = value["explicit_path"].as_str().expect("explicit_path 必须是字符串");
    assert_eq!(
        explicit_path,
        run_log.to_string_lossy(),
        "H1b: 显式参数必须压过 $HOME 与 env"
    );
    let (explicit_status, _) = verdict_pair(&value, "explicit_verdict");
    assert_eq!(explicit_status, "pass", "H1b: 显式参数指向干净日志必须 pass");

    let override_path = value["override_path"].as_str().expect("override_path 必须是字符串");
    assert_eq!(
        override_path,
        run_log.to_string_lossy(),
        "H1b: per-run override（extra.json 通道）必须压过 env"
    );
    let (override_status, _) = verdict_pair(&value, "override_verdict");
    assert_eq!(override_status, "pass", "H1b: override 指向干净日志必须 pass");
}

#[test]
fn h1c_home_fallback_path_points_at_the_real_user_log() {
    // 反向证据：**没有任何注入**时，缺省路径就是 `$HOME/.pony-agent/tui-logs/tui-runtime.log`。
    // 这条用例锁死「事故成因」——只要 runner 不注入作用域，判定就会读到真实用户目录的历史日志。
    let home = TempDir::new("h1c-home");
    let legacy_log = home
        .join(".pony-agent")
        .join("tui-logs")
        .join("tui-runtime.log");
    std::fs::create_dir_all(legacy_log.parent().expect("legacy log parent"))
        .expect("create legacy log dir");
    write_panicking_log(&legacy_log);

    let code = format!(
        r#"
import importlib.util, json
spec = importlib.util.spec_from_file_location("tui_checks", {checks:?})
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
path = module.runtime_log_path()
print(json.dumps({{"path": path, "verdict": list(module.no_panic_verdict("all good", module.read_runtime_log(path)))}}, ensure_ascii=False))
"#,
        checks = checks_py().to_string_lossy(),
    );
    // 不注入 env、不注入 override：只剩 $HOME 缺省。
    let value = python_json_with_env(&code, Some(&home.path), Some(None));

    let path = value["path"].as_str().expect("path 必须是字符串");
    assert_eq!(
        Path::new(path),
        legacy_log,
        "H1c: 缺省路径必须解析到 $HOME/.pony-agent/tui-logs/tui-runtime.log（可被 HOME 重定向）"
    );
    let (status, evidence) = verdict_pair(&value, "verdict");
    assert_eq!(
        status, "fail",
        "H1c: 未注入作用域时会读到旧日志并判 fail —— 这正是事故形态，runner 必须注入作用域（见 H2）; evidence = {evidence:?}"
    );
}

// ── H2：作用域必须绑定到本次运行（偏移入口优先，否则静态证据） ──────────────────────

#[test]
fn h2_audit_runner_scopes_the_runtime_log_to_this_run() {
    let script = std::fs::read_to_string(run_audit_sh()).expect("必须能读取 run_audit.sh");
    let lines: Vec<&str> = script.lines().collect();

    // ① 判定入口仍在：`checks.py check --dir` 是产出 assertions.json 的唯一路径。
    assert!(
        lines
            .iter()
            .any(|line| line.contains("checks.py\" check") || line.contains("checks.py check")),
        "H2: run_audit.sh 必须仍然调用 `checks.py check --dir` 产出断言"
    );

    // ② 环境变量注入必须存在，且绑定到**本次运行**的变量（不得硬编码真实 HOME 路径）。
    let injection = lines
        .iter()
        .find(|line| line.contains("PONY_TUI_RUNTIME_LOG="))
        .unwrap_or_else(|| {
            panic!(
                "H2: run_audit.sh 必须注入 PONY_TUI_RUNTIME_LOG 把 no_panic 的日志作用域绑到本次运行（否则判定会漂到真实 ~/.pony-agent 的旧日志，复现 336/369 事故）"
            )
        });
    let injected_value = injection
        .split("PONY_TUI_RUNTIME_LOG=")
        .nth(1)
        .expect("注入行必须形如 PONY_TUI_RUNTIME_LOG=<value>");
    assert!(
        injected_value.trim_start().starts_with('$'),
        "H2: 注入值必须是 shell 变量（每场景独占），不得硬编码路径：{injection}"
    );
    assert!(
        !injection.contains("$HOME"),
        "H2: 注入值不得指向 $HOME（那正是真实用户目录 = 污染源）：{injection}"
    );

    // ③ 被注入的那个变量必须指向**场景目录内**的日志文件（每场景独占，不复用历史字节）。
    let log_var = injected_value
        .trim_start()
        // 去掉 shell 变量前缀 `$`，否则首个分隔片段会是空串。
        .trim_start_matches('$')
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
        .next()
        .unwrap_or("")
        .to_string();
    assert!(!log_var.is_empty(), "H2: 无法解析注入的日志变量名：{injection}");
    let definition = lines
        .iter()
        .find(|line| line.contains(&format!("{log_var}=\"")) || line.contains(&format!("{log_var}=")))
        .unwrap_or_else(|| panic!("H2: 必须能在脚本内找到 {log_var} 的定义"));
    assert!(
        definition.contains("$SCEN_DIR") || definition.contains("$ISO"),
        "H2: {log_var} 必须定义在场景隔离目录下（每场景独占日志），实际定义：{definition}"
    );

    // ④ 作用域必须**带起始 offset**（只扫本次运行新增字节），并经 extra.json 下发给 checks.py。
    assert!(
        script.contains("runtime_log_offset"),
        "H2: run_audit.sh 必须记录并下发 runtime_log_offset（只扫本次运行新增字节）"
    );
    assert!(
        script.contains("runtime_log"),
        "H2: run_audit.sh 必须把 runtime_log 路径经 extra.json 下发给 checks.py"
    );

    // ⑤ 死接线护栏：runner 下发了 offset，checks.py 就必须真的消费它并支持偏移读取，
    //    否则「只扫本次运行新增字节」只是一句注释（这正是上一版缺陷的形态：有路径、无作用域）。
    let checks_src = std::fs::read_to_string(checks_py()).expect("必须能读取 checks.py");
    assert!(
        checks_src.contains("runtime_log_offset"),
        "H2: checks.py 必须消费 extra.json 的 runtime_log_offset（否则 run_audit.sh 传了也白传）"
    );
    assert!(
        checks_src.contains("def read_runtime_log(path: str, offset"),
        "H2: read_runtime_log 必须接受 offset 参数（按字节偏移只扫本次运行新增内容）"
    );
}

#[test]
fn h2b_offset_scoped_scan_excludes_old_bytes_or_static_scope_is_present() {
    let checks_src = std::fs::read_to_string(checks_py()).expect("必须能读取 checks.py");

    // 若实现提供了「只扫本次运行新增字节」的入口（签名里出现 since/offset/start 之一），
    // 则升级为语义断言：旧内容 + 新增干净内容 + since=旧长度 ⇒ pass；since=0 ⇒ fail。
    let has_offset_entry = checks_src.contains("since") || checks_src.contains("log_offset");

    if has_offset_entry {
        let dir = TempDir::new("h2b");
        let log = dir.join("tui-runtime.log");
        // 旧字节：含 panic（历史遗留）；新字节：干净。
        let legacy = "2026-10-22T10:11:13Z thread 'turn-7' panicked at tui.rs:42:1\n";
        let appended = "2026-11-02T09:00:00Z session started\nready\n";
        std::fs::write(&log, format!("{legacy}{appended}")).expect("write appended log");
        let legacy_len = legacy.len();

        let code = format!(
            r#"
import importlib.util, inspect, json
spec = importlib.util.spec_from_file_location("tui_checks", {checks:?})
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
sig = inspect.signature(module.read_runtime_log)
params = list(sig.parameters)
out = {{"params": params}}
if len(params) >= 2:
    out["since_legacy"] = module.read_runtime_log({path:?}, {legacy_len})
    out["since_zero"] = module.read_runtime_log({path:?}, 0)
    out["verdict_since_legacy"] = list(module.no_panic_verdict("all good", out["since_legacy"]))
    out["verdict_since_zero"] = list(module.no_panic_verdict("all good", out["since_zero"]))
print(json.dumps(out, ensure_ascii=False))
"#,
            checks = checks_py().to_string_lossy(),
            path = log.to_string_lossy(),
            legacy_len = legacy_len,
        );
        let value = python_json_with_env(&code, None, None);
        let params = value["params"].as_array().expect("params 必须是数组").len();
        if params >= 2 {
            let (fresh_status, fresh_evidence) = verdict_pair(&value, "verdict_since_legacy");
            assert_eq!(
                fresh_status, "pass",
                "H2b: since=旧长度 时只扫本次运行新增的干净字节，必须 pass，实际 evidence = {fresh_evidence:?}"
            );
            let (legacy_status, legacy_evidence) = verdict_pair(&value, "verdict_since_zero");
            assert_eq!(
                legacy_status, "fail",
                "H2b: since=0 时包含历史 panic 字节，必须 fail，实际 evidence = {legacy_evidence:?}"
            );
            return;
        }
    }

    // 未提供偏移入口 ⇒ 按 Lead 允许的静态证据分支：`run_audit.sh` 必须显式注入作用域。
    // （这不是跳过：静态证据缺失会立刻失败，见下。）
    let script = std::fs::read_to_string(run_audit_sh()).expect("必须能读取 run_audit.sh");
    assert!(
        script.contains("PONY_TUI_RUNTIME_LOG") || script.contains("runtime_log"),
        "H2b: 既没有偏移入口（read_runtime_log 第二参数/ since），run_audit.sh 里也没有 \
PONY_TUI_RUNTIME_LOG / runtime_log 注入 —— 旧日志污染判定的缺陷仍未闭环"
    );
}
