//! 启动期输入守卫（黑盒）：TUI 启动时**不得**向终端发能力查询、**不得**吞掉用户按键。
//!
//! 缺陷来源：`ratatui-image` 的 stdio 能力查询会另起线程对 stdin 做阻塞读（超时后线程不
//! 回收、继续吃后续按键），并自行 `disable_raw_mode()`；`init_splash_image()` 曾因此在
//! 启动窗口内吞掉用户输入。品牌标改为纯环境判定后，这条链路必须保持零 stdin 读。
//!
//! 断言（真实二进制 + 伪终端，`CARGO_BIN_EXE_pony`）：
//!   1. `g1_no_capability_query`：整个会话输出**不含**任何终端能力查询序列
//!      （DA1 `CSI c` / DSR `CSI 5n` / `CSI 16t` / `CSI 14t` / iTerm2 `CSI 1337n`）。
//!      查询字节出现即说明又走回了 stdio 探测。
//!   2. `g2_early_typing_survives`：启动后**立刻**键入 `abc`（不等输入框就绪），输入框
//!      就绪后这三个字符必须完整出现在界面上——被吞则必然缺失。
//!   3. `g0_harness_detects_probe`：负向对照（变异检查）。同一套驱动跑一个**故意**发
//!      `CSI c` 的假程序时，`g1` 的判定必须变红——否则说明守卫是空转的。
//!
//! 判定只依赖真实进程输出，不做单元级打桩；驱动脚本只负责「按键 + 抓字节」，不做业务判定。

use std::path::{Path, PathBuf};
use std::process::Command;

/// 会话结束后交给 Rust 侧断言的 JSON 结果行前缀。
const RESULT_MARKER: &str = "RESULT_JSON ";

// Windows 不可用：驱动依赖 Python 的 `pty` / `termios`（Unix-only，Windows 无这两个模块；
// 等价能力需 ConPTY，当前未接线），且判据只能由真实伪终端字节流给出。
// 缺口归属（如实登记）：Windows 上本用例 = **已知缺口 B067，无等价替代覆盖** —— 渲染层用例
// 不产生 stdin 读，无法替代「真实 PTY 启动期零 stdin 读」；`TUI_SNAPSHOT=1` 的 dump 是
// opt-in 过程产物、无消费者，不算覆盖。缺口恢复条件与登记见 B067。
#[cfg_attr(
    windows,
    ignore = "Windows 无 PTY 驱动（Python pty 仅 Unix）；已知缺口 B067（无等价替代覆盖）"
)]
#[test]
fn g1_no_capability_query_and_g2_early_typing_survives() {
    let result = run_driver(Path::new(env!("CARGO_BIN_EXE_pony")));

    // 契约 g1：启动期零能力查询（查询字节一旦出现，就存在被查询线程吃键的可能）。
    assert_eq!(
        result["query_free"].as_bool(),
        Some(true),
        "启动期发出了终端能力查询序列 {:?}（必须保持零 stdin 读的环境判定）",
        result["queries"].as_str().unwrap_or_default()
    );

    // 契约 g2：启动窗口内键入的字符不得被吞。
    assert_eq!(
        result["composer_ready"].as_bool(),
        Some(true),
        "输入框 45s 内未出现，无法验证按键是否被吞"
    );
    assert_eq!(
        result["early_typing_visible"].as_bool(),
        Some(true),
        "启动后立刻键入的 `abc` 未出现在界面上（启动期吞键回归）；可见区尾部：{}",
        result["tail"].as_str().unwrap_or_default()
    );
}

/// 负向对照：故意发能力查询的假程序必须被 `g1` 判定抓到，证明守卫不是空转。
///
/// Windows 不可用：负向对照同样经 Python PTY 驱动（`pty/termios` Unix-only）跑假终端程序，
/// 无伪终端则无从采集查询字节，判定不可信。
/// 缺口归属（如实登记）：Windows 上本用例 = **已知缺口 B067，无等价替代覆盖** —— 负向对照的
/// 判别力证明只能由真实 PTY 字节流给出，渲染层用例无从替代。恢复条件与登记见 B067。
#[cfg_attr(
    windows,
    ignore = "Windows 无 PTY 驱动（Python pty 仅 Unix）；已知缺口 B067（无等价替代覆盖）"
)]
#[test]
fn g0_harness_detects_capability_probe() {
    let work = scratch_dir("fake-probe");
    let fake = work.join("fake_probe_app.py");
    std::fs::write(&fake, FAKE_PROBE_APP).expect("write fake probe app");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755))
            .expect("chmod fake probe app");
    }

    let result = run_driver(&fake);
    assert_eq!(
        result["composer_ready"].as_bool(),
        Some(true),
        "驱动未走到输入框阶段，负向对照无效"
    );
    assert_eq!(
        result["query_free"].as_bool(),
        Some(false),
        "守卫未能识别能力查询字节，`g1` 判定不可信"
    );
}

/// 驱动一次 PTY 会话并解析结果行；驱动自身失败必须显式报错（不得静默通过）。
fn run_driver(bin: &Path) -> serde_json::Value {
    let work = scratch_dir("run");
    let driver: PathBuf = work.join("driver.py");
    std::fs::write(&driver, DRIVER_PY).expect("write pty driver");

    let output = Command::new("python3")
        .arg(&driver)
        .arg("--bin")
        .arg(bin)
        .output()
        .unwrap_or_else(|error| panic!("HARNESS ERROR：无法启动 python3 伪终端驱动：{error}"));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let payload = stdout
        .lines()
        .find_map(|line| line.strip_prefix(RESULT_MARKER))
        .unwrap_or_else(|| {
            panic!(
                "HARNESS ERROR：伪终端驱动未输出结果行。\nbin:{}\nstdout:\n{stdout}\nstderr:\n{stderr}\nstatus:{:?}",
                bin.display(),
                output.status
            )
        });
    serde_json::from_str(payload)
        .unwrap_or_else(|error| panic!("HARNESS ERROR：结果行不是合法 JSON（{error}）：{payload}"))
}

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "pony-startup-guard-{}-{tag}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.subsec_nanos())
            .unwrap_or_default()
    ));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// 负向对照用的假终端程序：发一次 DA1 查询，然后回显按键（够驱动走到「输入框就绪」）。
const FAKE_PROBE_APP: &str = r#"#!/usr/bin/env python3
import os, sys, tty

sys.stdout.write("\x1b[?1049h\x1b[c\u203a ")
sys.stdout.flush()
try:
    tty.setraw(0)
except Exception:
    pass
while True:
    ch = os.read(0, 1)
    if not ch or ch == b"\x03":
        break
    sys.stdout.write(ch.decode("utf-8", "replace"))
    sys.stdout.flush()
"#;

const DRIVER_PY: &str = r#"
import argparse, fcntl, json, os, pty, re, select, signal, struct, subprocess, sys, tempfile, termios, time

# 终端能力查询序列：出现任何一条都说明又走了 stdio 探测（会读 stdin，可能吞键）。
# 不把 kitty `ESC _ G` 计入：品牌标**渲染**也会发它，而 stdio 探测必然同帧带 DA1 `CSI c`。
QUERIES = [b"\x1b[c", b"\x1b[5n", b"\x1b[16t", b"\x1b[14t", b"\x1b[1337n"]
MULTIPLEXER_ENV_KEYS = ("HERDR", "HERDR_ENV", "HERDR_PANE_ID", "HERDR_SOCKET_PATH", "HERDR_TAB_ID", "HERDR_WORKSPACE_ID", "TMUX", "ZELLIJ", "STY", "ABDUCO")
# 图形协议探测相关环境：剥离后固定为「不支持图形协议」，使判定与开发者终端无关。
IMAGE_ENV_KEYS = ("KITTY_WINDOW_ID", "TERM_PROGRAM", "LC_TERMINAL")


def norm(raw):
    text = re.sub(rb"\x1b\[[0-9;?]*[a-zA-Z]", b" ", bytes(raw))
    text = re.sub(rb"\x1b\][^\x07]*\x07", b" ", text)
    text = re.sub(rb"\x1b[()][A-Za-z0-9]", b" ", text)
    return re.sub(r"[\s\u2800-\u28ff]", "", text.decode("utf-8", "replace"))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", required=True)
    args = ap.parse_args()

    work = tempfile.mkdtemp(prefix="pony-startup-guard-")
    iso = os.path.join(work, "iso")
    for sub in ("xdg-config/pony-agent", "xdg-data", "xdg-cache", "home"):
        os.makedirs(os.path.join(iso, sub), exist_ok=True)

    env = {k: v for k, v in os.environ.items()
           if k.upper() not in (
               "HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "NO_COLOR",
               "PONY_KEYBOARD_ENHANCEMENT", "PONY_TUI_SPLASH_IMAGE_PROTOCOL",
               *MULTIPLEXER_ENV_KEYS, *IMAGE_ENV_KEYS,
           )}
    env.update({
        "XDG_CONFIG_HOME": os.path.join(iso, "xdg-config"),
        "XDG_DATA_HOME": os.path.join(iso, "xdg-data"),
        "XDG_CACHE_HOME": os.path.join(iso, "xdg-cache"),
        "HOME": os.path.join(iso, "home"),
        "TERM": "xterm-256color",
        "LANG": "C.UTF-8",
        "PONYSENTRY_INGEST_URL": "",
        "PONYSENTRY_CLIENT_TOKEN": "",
        "OTEL_EXPORTER_OTLP_ENDPOINT": "http://127.0.0.1:9/v1/traces",
        "OTEL_TRACES_SAMPLER": "always_off",
        "OTEL_TRACES_SAMPLER_ARG": "0",
    })

    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 34, 120, 0, 0))
    proc = subprocess.Popen([args.bin], stdin=slave, stdout=slave, stderr=slave,
                            env=env, cwd=work, close_fds=True)
    os.close(slave)
    buf = bytearray()

    def pump(duration):
        end = time.time() + duration
        while time.time() < end:
            ready, _, _ = select.select([master], [], [], 0.05)
            if not ready:
                continue
            try:
                data = os.read(master, 65536)
            except OSError:
                return
            if not data:
                return
            buf.extend(data)

    # 关键动作：不等输入框就绪，进程一启动就键入（复现「启动期吃键」的现场）。
    os.write(master, b"abc")

    ready = False
    end = time.time() + 45
    while time.time() < end:
        pump(0.1)
        if "\u203a" in norm(buf):
            ready = True
            break
    pump(1.0)  # 让输入框把已回显的字符渲染出来

    found = [q.hex() for q in QUERIES if q in bytes(buf)]
    result = {
        "query_free": not found,
        "queries": ",".join(found),
        "composer_ready": ready,
        "early_typing_visible": "abc" in norm(buf),
        "tail": re.sub(r"[^a-zA-Z0-9\u4e00-\u9fff]", "", norm(buf))[-120:],
    }

    try:
        proc.send_signal(signal.SIGKILL)
        proc.wait(timeout=5)
    except Exception:
        pass
    try:
        os.close(master)
    except Exception:
        pass

    print("RESULT_JSON " + json.dumps(result, ensure_ascii=False))


main()
"#;
