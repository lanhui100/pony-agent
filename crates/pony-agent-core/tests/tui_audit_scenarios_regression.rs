//! 批7（task-17）回归：审计**场景存在性 / 对比度纯函数 / 反伪造护栏**。
//!
//! 契约背景：审计脚本是「物理凭证」的来源，一旦出现「只加场景名、不接断言」或「断言恒真」，
//! 门禁就会退化成形式主义。本文件只读调用 `tools/tui-audit/**`（不修改），用三类证据锁死：
//!
//! - **A1 场景存在性（静态）**：`run_audit.sh` 的场景清单 + `case` 分派 + `scn_*` 函数，
//!   `checks.py` 的 `SCENARIOS` 注册 + `check_*` 函数，两侧必须**同时**存在；
//!   并做**全量对账**：`run_audit.sh` 清单里的每个场景都必须在 `checks.py` 里有注册与断言函数
//!   （任何一侧漏接 ⇒ 红）。`ask-panel` 若被提及但未接入断言，则必须带 `documented-gap` 说明。
//! - **A2 对比度纯函数**：`contrast_ratio(fg, bg)` 的 WCAG 语义（白/黑 = 21.0、同色 = 1.0），
//!   并用主题实测三色（FAINT/GHOST/ACCENT_DIM 对 CANVAS）断言 ≥ 3.0。
//!   主题色值同时与 `tui.rs` 源码对账（颜色若被改动，用例会明确要求同步基线，而不是悄悄失效）。
//!   若 `checks.py` 未暴露 `contrast_ratio`，按 Lead 允许的替代口径断言 `selfcheck` 输出含
//!   `contrast`（**无跳过分支**：两条分支都是硬断言）。
//! - **A3 反伪造护栏（行为）**：把新场景的证据目录做成「空 capture + 齐全 meta」后跑
//!   `checks.py check --dir`，断言该场景**至少有一条断言 fail** —— 锁死「场景存在但断言恒真」。
//!
//! 隔离：全部证据目录在 `std::env::temp_dir()` 下的唯一目录内（`TempDir` Drop 递归删除）。
//! 铁律：真实断言、无 `#[ignore]`/skip；不修改 `src/**` 与 `tools/**`。

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

/// 本批要求同时落地的三个新场景。
const REQUIRED_SCENARIOS: [&str; 3] = ["paste-multiline", "resize-invalidate", "contrast"];

/// 主题实测三色（与 `tui.rs` 源码对账，见 A2）。
const THEME_FAINT: (u8, u8, u8) = (124, 118, 110);
const THEME_GHOST: (u8, u8, u8) = (104, 98, 90);
const THEME_ACCENT_DIM: (u8, u8, u8) = (180, 92, 62);
const THEME_CANVAS: (u8, u8, u8) = (18, 16, 14);

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/pony-agent-core 应位于仓库根下两层")
        .to_path_buf()
}

fn run_audit_sh() -> PathBuf {
    repo_root().join("tools/tui-audit/run_audit.sh")
}

fn checks_py() -> PathBuf {
    repo_root().join("tools/tui-audit/checks.py")
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
            "pony-agent-it-scenarios-{label}-{}-{stamp}",
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

fn python_json(code: &str) -> Value {
    let output = Command::new("python3")
        .arg("-c")
        .arg(code)
        .current_dir(repo_root())
        .output()
        .expect("python3 必须可用（审计脚本运行前置）");
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

/// 场景名 → 断言函数/函数后缀（`paste-multiline` → `paste_multiline`）。
fn snake(name: &str) -> String {
    name.replace('-', "_")
}

/// 解析 `run_audit.sh` 里的 `for scenario in a b c \\\n d; do` 场景清单。
///
/// 只取**第一处** `for scenario in` 到其后第一个 `; do` 之间的 token，避免把 `case "$scenario" in`
/// 的分派臂或 `--only` 的示例列表混进来（口径写死在这里，失败消息会打印解析结果）。
fn parse_runner_scenarios(runner: &str) -> Vec<String> {
    let Some(start) = runner.find("for scenario in ") else {
        return Vec::new();
    };
    let region = &runner[start..];
    let Some(end) = region.find("; do") else {
        return Vec::new();
    };
    region[..end]
        .replace("for scenario in", " ")
        .replace('\\', " ")
        .split_whitespace()
        .map(|token| token.trim().to_string())
        .filter(|token| !token.is_empty())
        .collect()
}

/// 解析 `checks.py` 的 `SCENARIOS = { "name": check_x, ... }` 键集合。
///
/// 取 `SCENARIOS = {` 起、到**行首** `}` 为止的区间，只收「行内以 `"` 开头且含 `":`」的条目，
/// 从而不受 dict 内注释/嵌套结构影响。
fn parse_scenarios_keys(checks: &str) -> Vec<String> {
    let Some(start) = checks.find("SCENARIOS = {") else {
        return Vec::new();
    };
    let region = &checks[start..];
    let mut keys: Vec<String> = Vec::new();
    for line in region.lines().skip(1) {
        let trimmed = line.trim_start();
        if trimmed.starts_with('}') {
            break;
        }
        if !trimmed.starts_with('"') || !trimmed.contains("\":") {
            continue;
        }
        let rest = &trimmed[1..];
        if let Some(length) = rest.find('"') {
            keys.push(rest[..length].to_string());
        }
    }
    keys.sort();
    keys
}

// ── A1：场景存在性（两侧同时存在 + 全量对账） ──────────────────────────────────────

#[test]
fn a1_required_scenarios_are_wired_on_both_sides() {
    let runner = std::fs::read_to_string(run_audit_sh()).expect("必须能读取 run_audit.sh");
    let checks = std::fs::read_to_string(checks_py()).expect("必须能读取 checks.py");

    for scenario in REQUIRED_SCENARIOS {
        let underscored = snake(scenario);

        // ① runner 侧：场景清单中出现（否则跑不到）。
        assert!(
            runner.contains(scenario),
            "A1: run_audit.sh 的场景清单必须包含 {scenario}（否则该场景永远不会被执行）"
        );
        // ② runner 侧：case 分派臂存在（否则场景名只是注释）。
        assert!(
            runner.contains(&format!("{scenario}) scn_{underscored}")),
            "A1: run_audit.sh 必须有 `{scenario}) scn_{underscored}` 分派臂，不得只列名字不接执行"
        );
        // ③ runner 侧：场景函数定义存在。
        assert!(
            runner.contains(&format!("scn_{underscored}()")),
            "A1: run_audit.sh 必须定义 `scn_{underscored}()` 场景函数"
        );

        // ④ checks 侧：断言函数存在（否则「场景存在但无断言」= 反伪造失败）。
        assert!(
            checks.contains(&format!("def check_{underscored}(")),
            "A1: checks.py 必须定义 `def check_{underscored}(` 断言函数，不得只注册场景名"
        );
        // ⑤ checks 侧：SCENARIOS 注册项存在（否则 `check --dir` 会 FATAL）。
        assert!(
            checks.contains(&format!("\"{scenario}\": check_{underscored}")),
            "A1: checks.py 的 SCENARIOS 必须注册 `\"{scenario}\": check_{underscored}`"
        );
    }

    // ⑥ 双向对账（强度不降）：runner 的 `for scenario in ... ; do` 清单 与 checks.py 的
    //    `SCENARIOS = {...}` 键集合，**双向差集都必须为空**（21 == 21 完全对齐）。
    //    失败消息里同时打印两个解析结果，便于一眼定位是解析口径问题还是真的漏接。
    let listed = parse_runner_scenarios(&runner);
    let registered = parse_scenarios_keys(&checks);
    assert!(
        !listed.is_empty(),
        "A1: 无法解析 run_audit.sh 的场景清单（预期形如 `for scenario in a b c; do`）"
    );
    assert!(
        !registered.is_empty(),
        "A1: 无法解析 checks.py 的 `SCENARIOS = {{...}}` 键集合"
    );

    let runner_only: Vec<&str> = listed
        .iter()
        .filter(|scenario| !registered.contains(scenario))
        .map(String::as_str)
        .collect();
    let checks_only: Vec<&str> = registered
        .iter()
        .filter(|scenario| !listed.contains(scenario))
        .map(String::as_str)
        .collect();
    assert!(
        runner_only.is_empty() && checks_only.is_empty(),
        "A1: runner 与 SCENARIOS 必须双向完全对齐，实际 runner_only={runner_only:?} checks_only={checks_only:?}\n\
runner=[{listed:?}]\nscenarios=[{registered:?}]"
    );
    assert_eq!(
        listed.len(),
        registered.len(),
        "A1: 两侧场景数量必须一致（runner=[{listed:?}] scenarios=[{registered:?}]）"
    );

    // 每个注册项都必须指向一个真实存在的 `check_*` 函数（注册悬空 ⇒ 红）。
    for scenario in &registered {
        let key = format!("\"{scenario}\":");
        let line = checks
            .lines()
            .find(|line| line.contains(&key) && line.contains("check_"))
            .unwrap_or_else(|| {
                panic!("A1: SCENARIOS 键 {scenario} 必须映射到一个 check_* 函数")
            });
        let function = line
            .split("check_")
            .nth(1)
            .map(|tail| tail.trim_end_matches(',').trim())
            .unwrap_or("");
        assert!(
            !function.is_empty() && checks.contains(&format!("def check_{function}(")),
            "A1: {scenario} 注册到 `check_{function}`，但该函数在 checks.py 中未定义（注册悬空）"
        );
    }

    for scenario in REQUIRED_SCENARIOS {
        assert!(
            listed.iter().any(|item| item == scenario),
            "A1: {scenario} 必须出现在 runner 的可执行场景清单里（不能只写在 case 臂）\nrunner=[{listed:?}]"
        );
    }

    // ⑦ documented-gap 必须是**真缺口**（不得过期）：
    //    - 未注册的场景（上一条分支）必须具名声明 documented-gap；
    //    - 反过来，任何被点名的 documented-gap 场景都**不得**已注册断言 —— 否则说明缺口已被真
    //      断言替代、却仍留着过期声明（批8 已把 ask-panel 的缺口关闭，故 ask-panel 现在必须
    //      不再作为「未接断言」出现）。
    let mut stale_gaps: Vec<String> = Vec::new();
    for line in checks.lines().chain(runner.lines()) {
        if !line.contains("documented-gap") {
            continue;
        }
        for scenario in listed.iter().chain(registered.iter()) {
            if line.contains(scenario.as_str()) && registered.contains(scenario) {
                stale_gaps.push(scenario.clone());
            }
        }
    }
    stale_gaps.sort();
    stale_gaps.dedup();
    assert!(
        stale_gaps.is_empty(),
        "A1: 以下场景已注册真断言，却仍留有 documented-gap 声明（过期缺口必须删除）：{stale_gaps:?}"
    );
}

#[test]
fn a2_contrast_ratio_matches_wcag_and_theme_colors_are_readable() {
    // 主题色值先与源码对账：颜色若被改动，本用例要明确要求同步基线，而不是悄悄测旧值。
    let tui_source = std::fs::read_to_string(
        repo_root().join("crates/pony-agent-core/src/bin/tui.rs"),
    )
    .expect("必须能读取 tui.rs");
    let expect_rgb = |label: &str, (r, g, b): (u8, u8, u8)| {
        assert!(
            tui_source.contains(&format!("Color::Rgb({r}, {g}, {b})")),
            "A2: 源码中找不到 {label} 的 RGB({r}, {g}, {b}) —— 若主题色被调整，请同步本用例的对比度基线"
        );
    };
    expect_rgb("FAINT", THEME_FAINT);
    expect_rgb("GHOST", THEME_GHOST);
    expect_rgb("ACCENT_DIM", THEME_ACCENT_DIM);
    expect_rgb("CANVAS", THEME_CANVAS);

    let code = format!(
        r#"
import importlib.util, json
spec = importlib.util.spec_from_file_location("tui_checks", {checks:?})
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
out = {{"has_contrast_ratio": hasattr(module, "contrast_ratio")}}
if out["has_contrast_ratio"]:
    ratio = module.contrast_ratio
    out["white_on_black"] = ratio((255, 255, 255), (0, 0, 0))
    out["black_on_white"] = ratio((0, 0, 0), (255, 255, 255))
    out["same_color"] = ratio({faint}, {faint})
    out["faint_on_canvas"] = ratio({faint}, {canvas})
    out["ghost_on_canvas"] = ratio({ghost}, {canvas})
    out["accent_dim_on_canvas"] = ratio({accent_dim}, {canvas})
print(json.dumps(out, ensure_ascii=False))
"#,
        checks = checks_py().to_string_lossy(),
        // 以 Rust 的 tuple 调试格式注入 Python 元组字面量（如 "(124, 118, 110)"）。
        faint = format!("{:?}", THEME_FAINT),
        ghost = format!("{:?}", THEME_GHOST),
        accent_dim = format!("{:?}", THEME_ACCENT_DIM),
        canvas = format!("{:?}", THEME_CANVAS),
    );
    let value = python_json(&code);

    if value["has_contrast_ratio"] == Value::Bool(true) {
        let number = |key: &str| -> f64 {
            value[key]
                .as_f64()
                .unwrap_or_else(|| panic!("A2: contrast_ratio 结果必须是数值，key={key}，实际 {value}"))
        };
        let white_on_black = number("white_on_black");
        assert!(
            (white_on_black - 21.0).abs() <= 0.01,
            "A2: 纯白对纯黑的 WCAG 对比度必须是 21.0±0.01，实际 {white_on_black}"
        );
        let black_on_white = number("black_on_white");
        assert!(
            (black_on_white - 21.0).abs() <= 0.01,
            "A2: 对比度必须对称（黑/白同为 21.0±0.01），实际 {black_on_white}"
        );
        let same = number("same_color");
        assert!(
            (same - 1.0).abs() <= 0.01,
            "A2: 同色对比度必须是 1.0，实际 {same}"
        );

        for (key, label) in [
            ("faint_on_canvas", "FAINT"),
            ("ghost_on_canvas", "GHOST"),
            ("accent_dim_on_canvas", "ACCENT_DIM"),
        ] {
            let ratio = number(key);
            assert!(
                ratio >= 3.0,
                "A2: 主题色 {label} 对 CANVAS 的对比度必须 >= 3.0（可读性下限），实际 {ratio}"
            );
        }
        // 主题三色必须彼此可区分（否则「弱化层次」在视觉上不存在）。
        let faint = number("faint_on_canvas");
        let ghost = number("ghost_on_canvas");
        assert!(
            (faint - ghost).abs() > 0.01,
            "A2: FAINT 与 GHOST 的对比度不得相同（弱化层级需要可分辨），实际 faint={faint} ghost={ghost}"
        );
        return;
    }

    // 替代口径（Lead 允许，**无跳过分支**）：未暴露 contrast_ratio 时，selfcheck 必须已经把
    // contrast 纳入自检面（关键词出现在 selfcheck 输出里），否则等于该场景没有任何可执行断言。
    let output = Command::new("python3")
        .arg(checks_py())
        .arg("selfcheck")
        .current_dir(repo_root())
        .output()
        .expect("python3 必须可用");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert!(
        output.status.success(),
        "A2: 未暴露 contrast_ratio 时必须改走 selfcheck 兜底，但 selfcheck 执行失败：stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        stdout.to_lowercase().contains("contrast"),
        "A2: checks.py 既没有 contrast_ratio，selfcheck 输出也未包含 contrast —— 对比度场景不可验证 \
（stdout={stdout:?}）"
    );
}

// ── A3：反伪造护栏（空 capture 必须让新场景至少一条断言失败） ─────────────────────

/// 造「空 capture + 齐全 meta」的证据目录：只有 0 字节的 `main.txt`，其余 meta/ansi/png 齐全。
///
/// 口径（Lead 裁定）：**不得**为该场景补齐它想要的所有 capture —— 缺 capture 时 `check_*`
/// 必须判 FAIL（记录成断言失败），而不是抛 `KeyError` 让审计进程崩掉。当前
/// `check_resize_invalidate` 在缺 `original`/`narrow`/`restored` 时抛 KeyError，故本用例为红相，
/// 等实现改为「缺证据 ⇒ fail」后转绿；断言本身不做任何放宽。
fn write_empty_capture_evidence(dir: &TempDir, scenario: &str) {
    std::fs::write(
        dir.join("meta.json"),
        format!("{{\"scenario\":\"{scenario}\",\"cols\":100,\"rows\":30}}\n"),
    )
    .expect("write meta.json");
    std::fs::write(dir.join("main.txt"), "").expect("write empty capture");
    std::fs::write(dir.join("main.ansi"), "ansi-stub").expect("write ansi stub");
    std::fs::write(dir.join("main.png"), "png-stub").expect("write png stub");
}

#[test]
fn a3_empty_capture_makes_each_new_scenario_fail_some_assertion() {
    for scenario in REQUIRED_SCENARIOS {
        let dir = TempDir::new(&format!("a3-{}", snake(scenario)));
        write_empty_capture_evidence(&dir, scenario);

        let output = Command::new("python3")
            .arg(checks_py())
            .arg("check")
            .arg("--dir")
            .arg(&dir.path)
            .current_dir(repo_root())
            .env("PONY_TUI_RUNTIME_LOG", dir.join("no-such-runtime.log"))
            .output()
            .expect("python3 必须可用");
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        assert!(
            output.status.success(),
            "A3: `checks.py check --dir` 对场景 {scenario} 必须能产出断言（当前失败：stdout={stdout:?} stderr={stderr:?}）"
        );

        let raw = std::fs::read_to_string(dir.join("assertions.json")).unwrap_or_else(|error| {
            panic!("A3: 场景 {scenario} 必须产出 assertions.json，实际错误：{error}；stdout={stdout:?}")
        });
        let payload: Value =
            serde_json::from_str(&raw).expect("A3: assertions.json 必须是合法 JSON");
        let fail = payload["fail"]
            .as_u64()
            .unwrap_or_else(|| panic!("A3: assertions.json 必须含 fail 计数，实际 {payload}"));
        let total = payload["checks"]
            .as_array()
            .map(|items| items.len())
            .unwrap_or(0);

        assert!(
            total > 0,
            "A3: 场景 {scenario} 必须有断言（当前 0 条 = 只加场景名不接断言）"
        );
        assert!(
            fail > 0,
            "A3: 空 capture 下场景 {scenario} 必须至少有一条断言 fail（fail={fail}, total={total}）\
—— 全部通过说明断言恒真，等于没有门禁；stdout={stdout:?}"
        );
    }
}
