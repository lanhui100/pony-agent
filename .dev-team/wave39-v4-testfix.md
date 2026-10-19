# wave-39 v4b 测试缺陷修正（A12 越界断言）

- 角色：独立 Test Agent（唯一可改冻结测试；本次属「用例自身缺陷重写豁免」，不计熔断）
- 授权：Lead 裁决 —— A12 整行断言迫使实现把呼吸行 SURFACE 铺满终端整行，破坏 ADR 0038 沟槽纪律
- 改动面：`crates/pony-agent-core/tests/tui_dialog_wave39.rs`（A12 函数体）；`tools/tui-audit/checks.py` 仅注释同步（无断言/逻辑变化）
- 结果：**`tui_dialog_wave39` → 12 passed / Exit 0**（当前 v4 实现，呼吸行 content 宽）；A11 4 passed / Exit 0；`py_compile` Exit 0

## 1. 缺陷根因

原 A12 对 y=28（呼吸行）遍历 `for x in 0..buf.area.width` 断言 `bg == SURFACE`，并把 y=29 指标行也按整行断言，等于把「色带/指标行的宽度」错误地钉成**终端整行**。Executor 为满足该断言，在色带 content 宽之外把呼吸行补满终端整行 SURFACE（其 v4 绿相收据 `.dev-team/wave39-v4-green-receipts.md` L18-19、L87-91 有记录），导致对话框底部出现比对话框更宽的 SURFACE 横条，违反 ADR 0038「沟槽列不得有内容/底色」纪律。

实证时序（本工作区）：
- 21:21 读取实现：呼吸行补块为 `Rect { x: area.x, …, width: area.width }`（终端整行）；
- 21:22:21 Executor 收回为 content 宽：`Rect { x: content.x, width: content.width }`（注释同步为「左右沟槽列保持 CANVAS」）；
- 本次收敛后的 A12 若再遇整行铺色，会在新增的沟槽断言处立即失败。

## 2. 修法（断言语义不变，不加豁免）

A12 三个 Assert 全部收敛到内容列，并显式锁死沟槽：

1. **Assert 1（指标行 y=29）**：循环 `for x in content.x..content.x+content.width`，非续格 `bg == CANVAS`、字形 `fg == GHOST(104,98,90)`；宽字符续格跳过逻辑保留（ratatui `Buffer::diff` 首帧 `to_skip` 语义，见 v3 test-fix）。
2. **Assert 2（呼吸行 y=28）**：同一内容列区间逐格断言「无字符 + `bg == SURFACE`」——呼吸行仍属色带（R13/R14 语义不变）。
3. **Assert 3（新增 · ADR 0038 沟槽纪律）**：沟槽列 `0 / 1 / W-2 / W-1` 在 y=28 与 y=29 上必须「无字形 + `bg == CANVAS`」；并断言 `content.x == 3`（110×30 的 GUTTER=3）以固定列区间前提。

未删除任何语义：GHOST/CANVAS 契约、呼吸行 SURFACE、指标行为最后一行、无值隐藏（A8/A13）全部保持。

## 3. checks.py 检查结论

全文检索 `bg`/`SURFACE`/`CANVAS`：**不存在同类「整行 SURFACE」假设**——唯一的背景断言是 `composer_cursor_checks` 对占位符首字 cell 的反色块检查（单 cell，与行宽无关）。仅把顶部说明注释从 v3（「独立空白间距行」）同步为 v4（「呼吸行属色带、指标行 CANVAS/GHOST、content 列宽、沟槽保持 CANVAS」），无代码/断言变化。`bottom_chrome_start()` 的 v4 边界逻辑（只排除指标行）保持 `task-10` 交付状态。

## 4. 门禁命令与真实退出码

```
$ cd /home/dm/pony-agent && cargo test -p pony-agent-core --test tui_dialog_wave39 2>&1 | tail -4
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s
EXIT_WAVE39=0

$ cargo test -p pony-agent-core --test tui_usage_wave39_delta        → EXIT_A11=0（4 passed）
$ python3 -m py_compile tools/tui-audit/checks.py                     → EXIT_PYCOMPILE=0
$ bash -n tools/tui-audit/run_audit.sh                                → EXIT_BASH_N=0
```

`git diff --stat crates/pony-agent-core/tests/tui_dialog_wave39.rs`：+34 / −14（含 v4 红相迁移与本次 A12 收敛）。

## 5. 结论与交接

- A12 现在只约束「内容列内的呼吸行 SURFACE / 指标行 CANVAS+GHOST」，并把「沟槽列保持 CANVAS」写成可执行断言 —— 实现无需为测试把色带铺满整行，ADR 0038 回归测试化。
- 测试冻结语义未放宽；后续 v4 实现变更若越出内容列（呼吸行/指标行铺满）会被 A12 Assert 3 立即拦截。
- E2E（`checks.py`）无需改动即可复跑；建议下一轮 E2E 顺带截图确认对话框底部无整宽 SURFACE 横条。
