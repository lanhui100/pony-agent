# wave-40 红相收据（Shift+Enter 换行 · L2-T）

- 角色：独立 Test Agent（只写测试/审计工具；未碰 `tui.rs`、未改既有冻结测试、未放宽断言）
- 任务板：`task-13`（W40-T 红相：单元矩阵 + PTY Shift+Enter E2E）
- 契约：`.dev-team/contract-matrix-tui-enter-key.md` §3 A1/A2（stub 基线 `af999a2`）
- 分级判定：**L2**（跨终端协议/输入事件边界的组件级契约；含真实 PTY E2E）
- 结论：**红相成立 —— 单元矩阵「编译 Exit 0 + 运行 Exit 101（4/4 unimplemented panic）」；
  PTY E2E「Exit 1」，仅断言 1（probe/push 未接线）失败，行为类断言 2–4 已通过**

## 1. 交付物（写域内）

| 交付 | 路径 | 说明 |
|---|---|---|
| A1 单元矩阵 | `crates/pony-agent-core/tests/tui_enter_key_wave40.rs`（102 行） | 4 组断言，独立 `--test` 目标 |
| A2 真实 PTY E2E | `tools/tui-audit/check_shift_enter.py`（605 行，stdlib-only） | pty.fork + 最小 ANSI 屏模型 + kitty 探测应答 |
| 物理凭证 | `.dev-team/evidence/wave40-shift-enter/{screen-before,screen-after-shift-enter,screen-after-enter}.txt` + `report.json` | 每次运行重生成 |
| 本收据 | `.dev-team/wave40-red-receipts.md` | — |

写域审计：`git status --porcelain` 中本波次新增仅上述测试/脚本/证据/收据；`tui.rs` 零改动（HEAD `af999a2` 即空桩基线），既有冻结测试零改动。

## 2. 门禁命令与真实退出码

### 2.1 A1 单元矩阵（红相必须是「编译通过 + 运行失败」）

```
$ cd /home/dm/pony-agent && cargo test -p pony-agent-core --test tui_enter_key_wave40 --no-run
    Finished `test` profile [unoptimized] target(s) in 2.44s
EXIT_NO_RUN=0

$ cd /home/dm/pony-agent && cargo test -p pony-agent-core --test tui_enter_key_wave40 2>&1 | tail -25
running 4 tests
test a1_keyboard_enhancement_flags_follow_support ... FAILED
test a1_modified_enter_inserts_newline ... FAILED
test a1_non_enter_key_submits ... FAILED
test a1_plain_enter_submits ... FAILED
[4× panicked at src/bin/tui.rs:116:5 / 122:5: not implemented]
test result: FAILED. 0 passed; 4 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
EXIT_UNIT=101
```

失败性质：全部为 `unimplemented!()` panic（`composer_enter_action` @116、`keyboard_enhancement_flags` @122），无编译错误。

### 2.2 A2 PTY E2E（真实二进制 `target/debug/pony`，sha256 `6f805145…`）

```
$ cd /home/dm/pony-agent && python3 tools/tui-audit/check_shift_enter.py --bin target/debug/pony
RESULT scenario=shift-enter pass=3 fail=1 exit=1
FAIL a2.1_probe_answered_and_flags_pushed ::
     probe_seen=False push_seen=False push_flags=None raw_push=
PASS a2.2_shift_enter_inserts_newline_without_submit ::
     composer_row='   › abc' submit_markers=[] prompt_row_before=31 prompt_row_after=30
     composer_grew(second visual line)=True
PASS a2.3_plain_enter_submits_and_clears_composer ::
     submitted=True composer_row='   › 写下任务，或输入 / 查看命令'
PASS a2.4_ctrl_c_exits_without_panic_and_restores_terminal ::
     exited=True child_exit=0 panic=False leave_alt_screen=True pop_seen=False pop_required=False
EXIT_PTY=1          # 真实 PIPESTATUS[0]
```

- 红相只允许断言 1 失败（契约 §3 明文）：当前实现未调用 `supports_keyboard_enhancement()` → 8s 内无 `CSI ? u` 探测。
- 断言 2–4 在红相已通过，原因如实披露：crossterm 对 `CSI 13;2u` 的解析**不依赖**是否 push（契约 §0 已核实），且 `tui.rs` 既有内联 Shift+Enter 分支本就可用。即：**本波次真正缺失的是「让终端把 Shift+Enter 编码成 CSI u」的协议启用（R3）**，A1 的 4 条 panic 正是 R1/R2 未实现。
- 证据：`screen-after-shift-enter.txt` 可见输入区 `› abc` 保留（未提交、无思考/写入中）；`screen-after-enter.txt` 输入区回到 `› 写下任务，或输入 / 查看命令`（已提交/清空）；`report.json` 含 4 条断言与关键序列 hex（`shift_enter=1b5b31333b3275`、`plain_enter=0d`、`ctrl_c=03`）。

## 3. 契约字面值勘误（实现侧必须按实际协议，不按错字）

E2E 脚本按「真实协议 + crossterm 0.28.1 源码」实现，并与契约字面值有两处必要校正（均物理验证，非放宽断言）：

1. **push flags 是 `ESC[>1u`，不是契约写的 `ESC[>2u`**
   - `KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES = 0b1`，`PushKeyboardEnhancementFlags` 序列化为 `CSI > {bits} u`（`crossterm-0.28.1/src/event.rs:293,494-496`）→ 实际字节 `1b5b3e3175`。
   - `>2u` 对应 `REPORT_EVENT_TYPES`，与契约 R2（只要求 DISAMBIGUATE）不一致；若按 `>2u` 断言，绿相必然假失败。
   - 脚本断言改为「push 序列存在且 DISAMBIGUATE 位（bit0）为 1」，并把**实际 flags 与原始 hex** 写入 report；这比字面匹配更严格（值必须真含 disambiguate）。
2. **DA1 应答必须是 `ESC[?1;2c`，不能回写裸 `ESC[c`**
   - `ESC[c` 是 DA1 **查询**；终端应答形如 `CSI ? 1 ; 2 c`。用裸 `ESC[c` 回复会让 crossterm 解析器读不到 PrimaryDeviceAttributes，探测循环挂起（实测子进程 8s 不退出）。
   - 已用一次性 crossterm 探针程序（/tmp，非仓库）验证：回写 `ESC[?1u` + `ESC[?1;2c` 后 `supports_keyboard_enhancement()` 返回 `true`；探测应答与 push 均出现在输出流中。
   - 正确应答序列：`PROBE_REPLY = b"\x1b[?1u\x1b[?1;2c"`（顺序：flags 应答在前，DA1 应答在后——反序会被判为不支持）。

## 4. 交接给 Executor 的绿相门禁

1. `cargo test -p pony-agent-core --test tui_enter_key_wave40` → **4 passed / Exit 0**（R1/R2 实现）。
2. R3：`run()` 在 raw mode + alternate screen + mouse 之后调用 `supports_keyboard_enhancement()`，按 R2 决策 push；退出路径（Ctrl-C、/quit、panic catch）在 `disable_raw_mode` 前 pop。
3. `python3 tools/tui-audit/check_shift_enter.py --bin target/debug/pony` → **Exit 0**：
   - 断言 1 需看到 `CSI ? u` 探测收到 `ESC[?1u` + `ESC[?1;2c` 应答后，应用输出含 `CSI > 1 u`（DISAMBIGUATE）且退出时含 `CSI < u`（pop）。
4. 实现仅限 `tui.rs`；不得改本测试/脚本/既有冻结测试；`PONY_KEYBOARD_ENHANCEMENT=0/1` 兜底按 R3 实现（脚本默认不设该变量，走探测路径）。

## 5. 环境与清理

- 执行环境：本机 Linux PTY（`pty.fork`，120×34，`TERM=xterm-256color`，隔离 HOME/XDG 临时目录）。
- 脚本每次运行后 `SIGKILL` 兜底回收子进程并删除临时 HOME/XDG（`shutil.rmtree`）；本次运行后无残留 pony 进程。
- 一次性验证件 `/tmp/w40-probe`（crossterm 探测程序）仅用于协议应答验证，可随时删除。
