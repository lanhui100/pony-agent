# Agent Note: 修复 a23 日志字段用例在 `tui.rs` 模块化后失效（源码扫描漏掉 `tui/input.rs`）

Status: implemented

## Context

`99b3adce refactor(tui): tui.rs 原地模块化拆分（Step 1+2）` 把输入层从 `src/bin/tui.rs`
机械外移为 `src/bin/tui/input.rs`，`key_class_label` 随之搬到 `tui/input.rs:103`（`pub fn`）。

`crates/pony-agent-core/tests/tui_streaming_input_acceptance.rs` 的
`a23_dispatch_and_rejection_logs_carry_depth_and_key_class`（契约 C25）在 ④ 段用
`fn_body(TUI_SRC, "fn key_class_label")` 断言「按键分类函数对字符键只记类别 `"Char"`，
不记正文」——这是防把用户输入写进日志的结构护栏。它只扫描 `include_str!("../src/bin/tui.rs")`，
拆分后签名从被扫描文件中消失，`fn_body` 直接 panic：

```
thread 'a23_dispatch_and_rejection_logs_carry_depth_and_key_class' panicked at
crates/pony-agent-core/tests/tui_streaming_input_acceptance.rs:1083:28:
源码中找不到 `fn key_class_label`
```

这不是按键分类行为退化，而是**校验对象被搬走**：分类逻辑仍在（`tui/input.rs:103`），
只是扫描范围没跟上。在干净提交树 `/tmp/wt-splash` @ `47fa08ec`（无任何未提交改动）单跑该用例
同样失败（exit 101），证明这是 `99b3adce` 引入的既有结构性红，与启动页品牌标改造无关。

## Decision

为 a23 增加显式白名单的第二扫描源，只把 ④ 段指向它：

- 新增 `fn tui_input_src() -> String`：按 `env!("CARGO_MANIFEST_DIR")/src/bin/tui/input.rs`
  运行期读取，读不到即 panic（fail-closed，模块再次搬家时用例立刻红，而不是拿空字符串"通过"）；
- ④ 段改为 `fn_body(&input_src, "fn key_class_label")`；断言文本与语义不变；
- `TUI_SRC`（`tui.rs`）仍是其余断言（`tui_queue_log` / `status_line` / `status_word`）的唯一扫描源，
  不把两个文件拼接，避免同名字面量跨文件误命中、弱化校验强度。

只动测试，不动产品代码与运行时行为。

## Alternatives considered

1. **把 `TUI_SRC` 改成 `concat!(include_str!(tui.rs), include_str!(tui/input.rs))`** — 否决：
   全局扩大所有断言的扫描范围，`window_after` 等按字节窗口取片的断言会跨文件边界误命中，
   校验强度下降；且失败时无法判断命中来自哪个文件。
2. **把 `key_class_label` 搬回 `tui.rs`** — 否决：为迁就测试回退 `99b3adce` 的模块化方向，
   与 `scripts/check-file-size.mjs` 的代码规模门禁背道而驰。
3. **删除 ④ 段断言（只保留行为测试）** — 否决：该断言锁的是"日志里不得出现用户正文字符"这条
   隐私护栏，删掉即失去机器可验的防回归手段；而 `key_class_label` 是纯函数，
   行为测试无法证明"日志调用点用的是它"。
4. **递归扫描整个 `src/bin/tui/` 目录找签名** — 否决：目录内任意新增文件都可能被误判为
   "已对账"；与 `47fa08ec`（a2 修复）一致，采用显式白名单文件 + 读不到即 panic。

## Evidence

- 干净 HEAD `47fa08ec`：`cargo test -p pony-agent-core --test tui_streaming_input_acceptance a23`
  → `FAILED. 0 passed; 1 failed`，panic 于 `fn_body`（红在 HEAD）。
- 修复后：该测试目标 `all passed / 0 failed`（见提交信息中的实测输出）。
- `RUSTFLAGS="-D warnings"` 检查与 `node scripts/check-file-size.mjs` 见提交前门禁记录。
