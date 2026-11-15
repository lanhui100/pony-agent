# Agent Note: TUI 速度段不再行内自描述口径与样本量（去掉 `decode` / `n=`）

Status: implemented

## Problem

TUI header 右上角的速度段此前按 F1 I4 的「自描述三件套」渲染：`120 t/s decode n=1`，
突发出清时再追加 ` 剔突发N`。实测在常规终端（96~110 列）里，这段文字带来的可读性收益
远小于代价：

- `decode` 是**默认口径**（绝大多数轮次都走 `stream_decode`），每天出现的后缀不携带
  区分信息，只贡献 7 个字符的噪声；
- `n=1` 在单调用轮次几乎恒为 1，同样不改变用户判断，却把速度段撑长到与 `首token`
  同级——ADR 2026-11-14-tui-header-metrics-overflow 记录的「速度段变长后首 token 被
  让位」正是这条后缀推高的宽度压力；
- 用户读到 `decode n=1` 的第一反应是「这是不是解析残留」，即需要解释的文本＝失败的文本。

结论：口径与样本量的可读性要求放在**交互式取证面**（桌面 tooltip / trace）成立即可，
不该用 header 的常驻宽度买单。

## Decision

`crates/pony-agent-core/src/bin/tui.rs::format_speed_segment` 只输出「值 + 单位」，
删掉口径与样本量后缀，`turn_calls` 形参一并移除（不再被 header 消费）：

| 快照状态 | 旧（F1 I4 原判） | 新 |
|---|---|---|
| `stream_decode` | `120 t/s decode n=1` | `120 t/s` |
| `end_to_end` | `≈120 t/s e2e n=1` | `≈120 t/s e2e` |
| `basis` 缺席 | `120 t/s` | `120 t/s`（不变） |
| 突发出清 `> 0` | 追加 ` 剔突发N` | 追加 ` 剔突发N`（**保留**） |

保留的三项不是遗漏，各有理由：

1. `≈` + `e2e` 保留 —— 它是**异常口径**的标记（含排队/prefill，跨调用不可比）。默认口径
   静默、异常口径显式，才符合「沉默即正常」的读图习惯；去掉后 `≈` 会变成无解释的符号。
2. ` 剔突发N` 保留 —— 只在 `turnBurstExcluded > 0` 时出现（罕见），且它解释的是「本次速度
   为什么偏低」，属不可省的异常说明。
3. `turn_calls` / `turn_speed_basis` 字段本身**不动** —— 快照仍是唯一真值源，桌面端
   tooltip（`src/lib/runtime/token-metrics.ts::formatSpeedSampleLabel`）与 trace 照旧可取证。
   本次只改 TUI 的**行内呈现**，不削弱数据面。

契约修订边界：token-speed v2 / F1 I4 的「口径与样本量可辨识」在 **TUI header 这一呈现位点**
上被本条改判（改由桌面 tooltip + trace 承担），计算与快照侧判据不受影响。回归锁定
`crates/pony-agent-core/tests/tui_header_metrics_overflow.rs`（C2 字面量同步改为
`120 t/s 剔突发2`），旧条
`.agents/notes/implemented/bug-fix/2026-11-14-tui-header-metrics-overflow.md` 已链回本条
（其「方案 A：缩短速度标签」当时因违反 I4 被否决，本次以「删后缀 + 明确改判契约」重新落地）。

## Alternatives considered

- **A. 保留原样（`decode n=1`）**：I4 原判最省事，但把「需要解释的文本」留在常驻位置，
  且持续推高让位压力；用户已明确要求去噪；否决。
- **B. 缩写（`decode n=1` → `d/1`）**：既没消除噪声（仍需图例才能读懂），又照样占用宽度；
  旧条已按「治标不治本」否决，本次同样否决。
- **C. 只在宽度不足时才隐藏后缀**：可在宽屏保留取证能力，但同一读数在不同终端宽度下形态不同，
  截图/日志无法互认，且让位策略会因此再长一档；否决。
- **D. 移到状态行 / 第二行**：破坏 ADR 0042 / ADR 0078 的单行 header 几何；否决。
- **E. 采用方案：彻底删掉 `decode` 与 `n=`，保留 `≈e2e` 与 ` 剔突发N`**（本 ADR）。

## Consequences

- 正面：速度段从 18~20 列压回 8~12 列，`首token` 在中等宽度下的让位压力显著下降；
  header 只呈现「这一轮多快」，口径溯源交给 tooltip/trace，职责更清。
- 负面/已知限制：**TUI 内不再能一眼看出速度口径与样本量**——`stream_decode` 与
  `basis` 缺席（旧快照）在 header 上不可区分（两者都渲染裸值）。需要区分时走桌面 tooltip
  或 trace；这是本次明确接受的取舍。
- 测试面：`tui_header_metrics_overflow::c2` 字面量同步更新（断言不再是契约反向钉子，
  而是新形态的钉子）；桌面侧 `f1-token-metrics-sample.spec.ts` 不受影响（走 tooltip）。
- §3 观测性：结构性 N-A（纯呈现层文本裁剪：无新增对外接口 / 网络 IO / 后台任务 / LLM 调用，
  不新增 metrics 归属与收口点；本变更本身即「减少展示字段」）。
