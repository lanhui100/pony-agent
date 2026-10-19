# wave-39 v3 测试缺陷修正（A12/A13 · Test-Fix）

- 角色：独立 Test Agent（唯一可改冻结测试的成员；本次仅修**用例自身缺陷**，契约与实现均未动）
- 授权：Lead 裁决 (a)「用例自身缺陷重写豁免」，不计熔断
- 背景：Executor 已落地 v3（`tui.rs` 工作树），v3 红相已由 commit `c233c34` 记录；本文件不重造红相
- 修正前实测：`tui_dialog_wave39` → **10 passed / 2 failed**（A12/A13，其余 10 条绿）
- 修正后实测：`tui_dialog_wave39` → **12 passed / Exit 0**；`tui_usage_wave39_delta` → **4 passed / Exit 0**
- 改动面：仅 `crates/pony-agent-core/tests/tui_dialog_wave39.rs`（`git diff --stat`：+55 / −15）；`tui.rs`/`checks.py`/A11 冻结文件零改动

## 1. 缺陷 1：A12 宽字符续格误判（TestBackend 首帧语义）

### 根因
ratatui 0.29 `Buffer::diff` 对宽字符（CJK）右侧续格置 `to_skip`，TestBackend 首帧不会为续格写入样式，该格保持 `Cell::default`（`fg/bg = Reset`）；而 `ui()` 实际已把指标行整行背景填成 `CANVAS`、整行字形前景设为 `GHOST`（Executor 插桩已证，真实终端由宽字符渲染器覆盖续格）。原断言对**每一格**都查 `bg == CANVAS`，于是在宽字符续格上读到 `Reset` 而假失败：

```
thread 'a12_...' panicked at tests/tui_dialog_wave39.rs:767:9:
assertion `left == right` failed: 指标行第 87 列背景必须是 CANVAS（非色带 SURFACE），实际 Reset
  left: Reset
 right: Rgb(18, 16, 14)
```

### 修法（语义不变）
遍历指标行时维护 `skip_continuation`：若**前一格** `symbol()` 为单个字符且显示宽度为 2（复用测试内 `disp_width()`，与 `tui.rs::char_width` 同构），则跳过其右侧续格的 fg/bg 断言；其余格（含 workspace 标签、各指标段字形、行尾空白）仍逐格严格断言 `fg == GHOST` / `bg == CANVAS`。

- 间距行 28 无宽字符，**逐格严格断言保持不变**（空白 + `bg == CANVAS`）；
- `GHOST = Rgb(104,98,90)`、`CANVAS = Rgb(18,16,14)` 两个字面契约未放宽；
- 另断言「至少有一个字形格」，防止整行空白时“跳过”变成免检。

## 2. 缺陷 2：A13 孤立分隔符检测误杀合法 `" · "`

### 根因
`assert_no_orphan_separator` 对每个指标段做「前缀 trim 后是否以 `·` 结尾」检查。合法分隔符是 `" · "`（空格-点-空格），因此 `"↑500 ↓0 · 缓存 20%"` 中 `缓存` 段的前缀 trim 后正是 `"…↓0 ·"`，被误判为孤立分隔符：

```
thread 'a13_...' panicked at tests/tui_dialog_wave39.rs:742:13:
[A13-1 in>0] 首个指标段 "缓存" 之前不得有孤立分隔符:
"     ~/pony-agent/crates/pony-agent-core          ↑500 ↓0 · 缓存 20%   "
```

### 修法（只查真正的孤立分隔符）
先裁出右簇（行内首个 `↑`/`缓存`/`上下文` 到行尾、`trim()`），再断言：

1. 簇首不得为 `·`、簇尾不得为 `·`（行首/行尾孤立）；
2. 不得出现连续分隔符：`"· ·"` 或 `"·  ·"`（空段）；
3. 结构：按 `" · "` split 后**每段非空、且 `part == part.trim()`**（无首尾空格）——等价于「段间恰好一个 `" · "`」；
4. 簇为空（无值不显示）时直接返回，段的存在/不存在由各用例断言负责。

**未删除任何隐藏语义断言**：A13 三个用例的「有值段必须出现 / 无值段必须隐藏」、`—` 占位符禁止、以及 A8 全零/单段隐藏断言全部保留。

## 3. 门禁命令与真实退出码（修正后）

```
$ cd /home/dm/pony-agent && cargo test -p pony-agent-core --test tui_dialog_wave39 2>&1 | tail -22
running 12 tests
...
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s
EXIT_WAVE39=0

$ cd /home/dm/pony-agent && cargo test -p pony-agent-core --test tui_usage_wave39_delta 2>&1 | tail -4
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
EXIT_A11=0

$ python3 -m py_compile tools/tui-audit/checks.py
EXIT_PYCOMPILE=0

$ bash -n tools/tui-audit/run_audit.sh
EXIT_BASH_N=0
```

逐条 12/12：A1–A10、A12、A13 全绿（A11 在独立 `--test` 目标 4/4）。

## 4. 写域与冻结声明

```
$ git status --porcelain（排除并行前端/E2E 目录）
 M crates/pony-agent-core/tests/tui_dialog_wave39.rs   # 本次唯一改动（Test Agent）
 M crates/pony-agent-core/src/bin/tui.rs               # Executor v3 实现（未触碰）
?? tools/                                              # Test Agent 工具目录（未跟踪）
```

- `tui.rs`、`docs/decisions/0042-*`、前端文件、`tools/tui-audit/checks.py`、A11 冻结文件：本轮零改动。
- 测试修复未降低对实现的约束：A12 仍逐格校验 GHOST/CANVAS（续格除外），A13 仍校验段可见性与分隔符结构；后续 E2E 复跑应能通过（空态 workspace-only 断言已在 checks.py 中）。
