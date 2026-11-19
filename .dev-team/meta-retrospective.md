# 元复盘：wave-acp-delegate（ACP 委派结果与 trace 完整性，P0–P2）

## 通信拓扑与信噪比

本轮采用 Lead + Protocol Agent(1) + Test Agent(多轮替换) + Executor(2) 的拓扑，审查全部走 Subagent 池（Gate-1 计划审查 1 路、L3-A/L3-B 两路互补、L3-P 六问 1 路、L2-AT 独立池 1 路），名册峰值 8（含 Lead），符合容量上界。信噪比问题集中在 **Test Agent 反复外派子代理导致上下文耗尽**（test-agent、test-agent-2/3/4 与三个子代理均报 "ran out of room"），产出为空或半成品，直接拉长墙钟并迫使 Lead 三次换人；有效产出集中在窄任务（test-agent-4 的 C6 红相、test-agent-3 的断言迁移）。探索者两路（落盘链路、sink/step/ACP 链路）一次性给出高密度 path:line 事实，证明 E2 侦察外派是本轮最高 ROI 动作。重复命令率偏高来自构建锁排他：全局 flock 被其他会话长测试占用时，同一套件被重跑 3 次（均已记录），后续应以锁状态探测替代盲目重试。

## 门禁穿透与误杀率

红相门禁真实生效：契约套件 8 failed（C0.1/C0.2/C0.3/C2.1/C3.1/C4.1/C5.1×2）、对抗套件 4 failed、C6 套件 3 failed，全部为断言失败而非编译错误；W2/W3 落地后逐套件转绿（12/6/3），全量 lib 1614 passed/0 failed、ACP 19 套件全绿。对抗门禁的独立价值在 at_03b：它同时暴露 C0.1 与 C1.1/C1.5 两类违约，L2-AT 主动把早退断言改为违例聚合（只增断言，已按 test-defect rewrite 重锚）。L3 门禁的误杀/漏杀：L3-A/L3-B 均 PASS（0 BLOCKING，共 12 条 ADVISORY），L3-P 抓到真实 BLOCKING——`ponycli acp` 在子命令分派前未改道 stderr，jsonl/sse 下结构化日志污染 stdout，破坏 wave-59 C1（若只信 L3-A/B 会漏放）；Gate-1 计划审查补做时又抓到 truncated 成功轮仍不回传正文（C0.4 语义缺口）与 C0.5/C1.2 零断言，两条均修复并补测。误判方面无 BLOCKING 被误杀；C4.1 仅断言 step==0、C6.1 仅串行证据属真实覆盖缺口，已登记。Join 复审又抓到 1 条 BLOCKING：`schema.rs` 模型可见描述仍称「截断轮不返回 finalText」，与新实现矛盾，而 T-ACP-05 的逃逸子句恰好放过它——同一契约的「描述面」与「行为面」必须由同一用例双向锁定，这是本轮最有价值的门禁教训。残余 12 条 ADVISORY 已登记 backlog B239（flush 并发去重/eventTimes 解码）与 B240（projection 丢归因/日志脱敏/协议兜底），并在 state.json 记录 owner 与到期。

## 分工契约与隔离有效性

写域正交总体成立：Executor-A（acp/tools + bin/cli）与 Executor-B（runtime/turn_flow/telemetry/持久化）并行无文件冲突；turn_flow/runtime 在 W3 与 W2 存在同文件依赖，已显式串行并合并为单一绿色检查点（state.json 记录）。隔离层的关键决策是**独立 worktree**（主工作区有他人未提交改动 + 另一个 worktree 在跑），避免污染；代价是两次 frozen-test drift（L2-AT 改写对抗用例、Test Agent 压缩 C6 文件），均以 `git checkout` 恢复或重锚收口，并在 state.json 留痕。公共接线（bin/cli.rs 诊断、CLI 日志改道）由 Lead 裁决后单点授权，未出现多人同改。构建预算方面，全局 flock 被跨会话长测试占用（load 一度 32.9，超 1.5×16 上限），Lead 启用专用锁 `/tmp/pony-agent-build-acp-delegate.lock` 并记录覆盖理由；这是本轮最明显的共享设施争用成本。

## 元协议迭代建议

1) **Test Agent 禁用子代理派生**：在 `spawn_teammate` 初始 Prompt 中硬性禁止，并把测试任务限制为单文件、≤15 分钟、可独立验收；本轮三任 Test Agent 均因外派失败，此条为最高优先修正。
2) **冻结测试前要求"版式定稿"**：先由 Test Agent 报告最终行数与用例清单，Lead 再锚定；否则行号漂移导致收据与冻结件不一致（本轮 C6 红相收据来自 785 行草稿，冻结件为 673 行等价压缩，已在 state 登记）。
3) **评审产物按波次唯一命名**：`review-l3-<wave>.json`，避免覆盖历史波次收据（本轮已先备份再恢复，若直接提交会误删旧证据）。
4) **构建锁增加挂起检测/公平性**：跨会话 flock 被长测试占用时应有 stale 提示与等待上限，否则 lane 排队不可观测（本轮靠 ps/flock 手工诊断）。
5) **L3-P 前置到文档提交之前**：本轮 L3-P 在 docs/ADR 提交后发现 CLI stdout 契约冲突，导致二次改文档 + 二次测试迁移；六问应在代码绿色检查点后立即跑，文档最后写。
6) **把"轮内增量落盘"拆成独立波次**：C6 触及 control_plane/sqlite/log/fold 五层，与 C0–C5 的表情混合在一个绿色提交里，回滚粒度变粗；建议后续把持久化语义变更单列 DAG 节点。
7) **Gate-1 计划审查不得后补**：本轮因先开工后补 Gate-1，其 BLOCKING（truncated 正文、缺测）在实现后才被发现，返工成本高于前置评审；流程上应在 Protocol 冻结后立即开 Gate-1。

---

# B254 元复盘：tool_followup 断流不再中断回合（Agent Team 协作元框架）

波次：B254（流式唯一通道 + 断流可重试 + 去聚合终态 + 用户文案朴素化）
基线 `9b5b931c` ｜ 红相锚定 `e7f9bcad` ｜ 契约 `.dev-team/contract-matrix-b254.md`

## 通信拓扑与信噪比

Lead 只下发「契约 C1–C5 + 文件锚点 + 禁止项」，执行者回报「改动清单 + exit code」，无长篇设计复述；关键路径三轮内收敛。**本轮最大损失是 spawn 后端失败**：`test-agent`、`exec-core` 两次创建后无任何产物即转 inactive（只收到一条系统失败通知），Lead 在等待 10 分钟后接管红相测试与核心实现。重复命令率低（失败先读报错再改），但 lane 串行编译占墙钟约 40%。

## 门禁穿透与误杀率

红相真实失败：mock 回环计数抓到 1 次 `"stream":false` 同步请求（C1 反证），文案测试抓到「同步兜底第 2 次…」。L3 独立复审（fresh context）两次抓到真问题并判 FAIL：`fallback_body.rs:214/:244` 旧断言漏重锚（BLOCKING，已修，实测 9 passed）；重锚断言放宽（ADVISORY，已补「真实错误原因」断言）。最终门禁：`agent::` lib 1645 passed/0 failed、`b254_stream_only_followup` 1 passed、b221 5 passed、b219 4 passed、TUI `--test-threads=1` 240 passed、`rust:lint` PASS（新增/恶化 0）、`adr:check` / `backlog:gate` / `version:check` / observability `audit-diff` 全 exit 0。误杀：`b254_plain_copy` 未收裸「同步」属有意收窄（避免误伤「已同步最新状态」），复审判定 ADVISORY 合理。残余空白面：`--bin pony` 并行偶发 futex 死锁（既有测试并发 flake，串行 240 绿）。

## 分工契约与隔离有效性

写域正交成立：protocol 只写 `.dev-team/nfr-baseline.json`，exec-tui 只改 `status_dwell.rs`，核心实现集中 `provider/mod.rs`/`retry.rs`/`fallback_body.rs`。**违约事实（记入元账）**：`exec-core` 失败后 Lead 直接实施核心业务代码（>50 行），违反「Lead 不写业务代码」；补偿是验收测试由先冻结的独立红相文件驱动（`e7f9bcad` 先于实现），并由独立 subagent 复审，自证风险被吸收，但流程判为降级（state 记 `degraded: lead-implements-core`）。

## 元协议迭代建议

1) **spawn 失败自动降级**：spawn 后 2 分钟无产物且 inactive ⇒ 标记 failed、Lead 接管并写 `degraded` 登记，禁止静默越界。
2) **红相兜底可写**：显式允许「Test Agent 失败时 Lead 写红相，但绝不写实现」，并强制复审。
3) **断言等价替换机器化**：review JSON 增 `assertion_delta`（被删断言 → 等价新增断言），缺失判 FAIL。
4) **门禁批次脚本**：一次编译多 target 串跑，减少重复编译墙钟。
