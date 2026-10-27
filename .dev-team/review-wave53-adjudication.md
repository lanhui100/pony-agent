# Wave-53 审查裁决（Lead）

两路 L3 审查收据：`.dev-team/review-wave53-a.json`（视角 A：并发/状态流转/正确性）、
`.dev-team/review-wave53-b.json`（视角 B：契约一致性/可观测性语义/安全）。

## L3-A（verdict=PASS，1 ADVISORY）
| 条目 | 裁决 | 处置 |
| :--- | :--- | :--- |
| NFR_VIOLATION（`record_tool_call_span` 在 `spans_allowed()` 门禁前 `clone`，门禁关闭时每次工具调用多分配） | **接受** | 已修：`otel/mod.rs:2113` 先判门禁再克隆（零分配快路径），语义不变；executor 回执门禁 7/14/1175 全绿 |

## L3-B（verdict=FAIL，2 BLOCKING + 4 ADVISORY）
| 条目 | 裁决 | 理由与处置 |
| :--- | :--- | :--- |
| BLOCKING-1 生产入口 `begin_turn_trace(..., 0)` ⇒ trace 名恒 `pony-agent#turn-0`，可读但不可区分 | **成立（Lead 独立复核确认）** | 契约 C5 升 v2：`pony-agent#turn-<seq>@<turn_id 尾 6 字符>`（按字符截尾、非 ASCII 不 panic）；test-agent 重锚 T5 六个子断言（含同名反证 / 非 ASCII 反证），executor 以新纯函数 `turn_trace_name` 落地；7/14/1175 全绿 |
| BLOCKING-2 审计 §6.5 把手搓 OTLP 探针写成「端到端物理收据」，且当时实现未提交 | **成立** | 已改：§6.5 标题与证据等级说明改为「服务端映射探针收据」，显式声明「不回答客户端是否发出，客户端出口由 T1~T6 wire 级覆盖，真实会话复跑留待下一轮」；ADR 0052 同步改写 |
| ADVISORY-3 yaml「无价不再被显示成 0 元」夸大 | **成立** | 已改为「Tempo/原始属性面可区分 0 与未计价；Langfuse 展示面仍为 0（需产品侧补价）」 |
| ADVISORY-4 审计 §1 收据命令 `scripts/check-declaration.sh` 不存在 | **成立** | 已改为 skill 绝对路径（与 §7 一致） |
| ADVISORY-5 `audit-diff --base origin/main` 只覆盖已提交内容，对本波无判别力；声明变更提交需 `[decl-change-approved]` | **成立** | 接受：门禁在**收口提交后**复跑（收据记在本文件末尾），提交信息带 `[decl-change-approved]` 标记 |
| ADVISORY-6 ADR「端到端复核」与 BLOCKING-2 同源 | **成立** | 已与 §6.5 一并降级表述 |

## 门禁复跑（提交后）
- `audit-diff.sh --base origin/main`：见 `.dev-team/wave53-gates.log`
- `check-declaration.sh /home/dm/pony-agent`：同上
