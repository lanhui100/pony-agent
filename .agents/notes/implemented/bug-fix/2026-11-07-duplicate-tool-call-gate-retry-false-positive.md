# Agent Note: 重复工具调用闸从「集合判重」改为「同签名计数闸」，并把提示文案改为事实陈述

Status: implemented

## Context

`turn_stream.rs` / `turn_sync.rs` 的同签名重复调用闸用**永不清理的 `BTreeSet`** 判定：turn 起始以首批
调用签名初始化，每执行一次工具再 `insert`，新 hop 的首个调用在**执行之前**做 `!insert(...)` 判定。
后果有两层：

1. **行为**：同一 turn 内第 2 次同参调用直接命中闸，命中不是「拒绝这一次」，而是**整轮转恢复式收尾**
   （`recover_tool_followup_completion(_stream)` 回灌合成 tool result 后收尾，不再有后续 hop）——
   语义等价于「第 2 次同参调用 = 本 turn 工具能力熔断」。线上 147MB 运行日志里 `tool_status=blocked`
   91 条，其中文案含「近似」的 73 条，按工具分布 `Run` 26 / `Edit` 15 / `team_task_list` 14 /
   `list_agents` 8 / `Read` 4 / `Search` 3 / `JobOutput` 2 / `BatchExecute` 1 —— 主体是**正常工程动作**：
   工具失败后原样重试（`Edit` 的 `fs_not_observed`、`Run` 的临时网络/编译错误）、改代码后重跑同一条
   测试命令、写后回读校验、团队/子代理状态轮询、幂等写重试。真阳性只剩「没有任何世界状态变化地反复
   申请同一次检索」，而真死循环已由既有三道闸（同指纹 3 次 / 同工具连续失败 4 次 / 单轮墙钟 900s，
   ADR 0060 决策 5）兜底。
2. **文案**：提示语写「重复调用了**近似相同**的参数」，实际判定是工具名 + `canonicalize_tool_argument_value`
   后的 JSON 串**逐字段精确相等**（仅对象 key 排序、剔除自由文本 `description`），无任何相似度匹配。
   把确定性拦截描述成概率性匹配，会诱发「换个措辞再试」的无效重试并再次撞闸。

## Decision

1. **集合 → 计数**：新增 `SameSignatureTracker`（`agent/runtime/stream_support.rs`，`BTreeMap<String, usize>`），
   三条不变量：① **闸点只读**（`is_exhausted` / `occurrence_count` 不改状态，被拒的调用不记账）；
   ② **执行点唯一记账**（`record_executed` 在工具真正执行后调用）；③ **不预置首批计数**
   （turn 起始一律 `new()`）——提案原设计的 `from_batch` 预置 1 次 + 执行点再记 1 次构成双重计数，
   默认上限 2 时第 2 次即熔断，直接违反「前 2 次放行」的验收口径，故不采纳该 API。
2. **闸点由「第 2 次即拦」改为「第 `limit + 1` 次才拦」**：`turn_stream.rs:157/748/1330` 与
   `turn_sync.rs:42/292/552` 六处对称改写，两路径逐字同构；文案带出实际次数与上限
   （`occurrences = 已执行次数 + 1`）。
3. **阈值三件套 + 环境变量**（`agent/runtime/limits.rs`，与既有 `DEFAULT_/MAX_ALLOWED_/_ENV` 同风格）：
   `DEFAULT_MAX_SAME_SIGNATURE_OCCURRENCES_PER_TURN = 2`、
   `MAX_ALLOWED_SAME_SIGNATURE_OCCURRENCES_PER_TURN = 8`、
   `PONY_AGENT_MAX_SAME_TOOL_SIGNATURE_OCCURRENCES_PER_TURN`；非法/越界/`0` 一律**回退默认**，
   禁止静默取 0（0 等于每轮第一次调用后立即熔断）。另补 `#[cfg(test)]` thread_local 覆写槽。
4. **写类成功清空其它签名计数**（世界变化信号）：`tool_execution_changed_world` 判定
   「执行未失败/未取消（`status ∉ {"error","aborted","cancelled"}`）且 `tool_kind_for_name(name)` ==
   `ToolKind::Write`」，命中时 `record_executed(.., true)` 先 `retain` 掉**其它**签名再给自身 +1。
   工具分类的唯一真值源是工具注册表（`agent/tools.rs:7944 tool_kind_for_name`，内部已做别名归一），
   闸内**不建**任何工具名白名单。于是 `Read → Edit → Read`、`Run test → Edit → Run test` 天然放行，
   纯重复检索仍被计数拦。**只认 `Write` 而不含 `Execute`** 是 Gate-1 审查后的收窄：`Job*` 全部是
   `Execute`（`tools.rs:7980`），若 `Execute` 也清空，`JobOutput(A)` 与 `Run(X)` 成功交替会互相清空、
   两签名计数恒为 1 而永不熔断（旧 `BTreeSet` 第 2 次即拦）——那是本方案新引入的绕闸面；且 `retain`
   恒保留自身，`Execute` 清空对「同参重试」零收益，「改完重跑同一条命令」的受益路径本来就是中间的 `Edit`。
5. **文案改为事实陈述 + 可执行出路**：`build_duplicate_tool_call_error(&call, occurrences, limit)` 产出
   「已第 N 次以**完全相同**的参数被调用（同名 + 同参数，仅忽略 `description` 与 key 顺序；同一 turn 内
   最多允许 L 次）……若上次调用失败或结果已失效，请改变参数、先确认当前状态与已有证据，或直接基于
   已有证据作答」；删掉「近似」，把比较口径与出路一次讲清。

## Alternatives considered

- **保持现状（把闸当安全网）**：否决。73 次命中里约 30% 是纯只读轮询，26 例 `Run` + 15 例 `Edit` 高度
  集中在「失败重试 / 改后重跑」，误杀代价是整轮工作丢失；真死循环已有三道闸兜底，收益与代价极不对称。
- **只改文案不动行为**：保留为并行子项（第 5 条），但单独实施不解决「整轮熔断」这一缺陷本体。
- **直接删掉这道闸**：否决。「原地打转但每次成功」的检索风暴只能靠要求失败的连续失败闸与 900s 墙钟
  兜住，等于放任烧预算。
- **参数做模糊/相似度匹配**：否决。判定面不可解释（阈值调参后无法回答「为什么这次过了」），既放过
  语义不同的调用、又拦不住换措辞的重复。
- **把 `description` 纳入签名以便「换说辞即放行」**：否决。`description` 是自由文本，纳入即给模型留
  逃生舱（`stream_support.rs` 早已明确否决）。
- **只把环境变量阈值调到极大**：否决。等于默认关闭却不改语义，且默认值仍然错——新装用户照旧被误杀。
- **把「只读轮询类」阈值单独提高（提案第 3 项后半）/ 命中后降级为普通 tool result 软拦截（提案第 4 项）**：
  本轮不做，登记后续条。前者需先在工具注册表补「轮询/poll」语义标记（现有 `ToolKind` 无法区分
  `team_task_list`(Read) 与 `Read` 检索），按「先补标记再改闸、禁止在闸内建字符串白名单」的既有纪律
  属独立变更；后者要新增 error code 与 telemetry 归属面，收益（体验）与成本（事件面膨胀）需单独立项。

## Consequences

- **用户可见行为**：同一 turn 内同参调用从「第 2 次即整轮熔断」放宽为「前 2 次正常执行、第 3 次才收尾」；
  写类工具成功执行后，其它签名的历史计数清零，故「写后回读」「改完重跑」不再被误杀；失败重试的文案
  不再诱导「换个措辞再试」，而是给出改参数 / 先确认状态 / 直接作答三条出路。
- **可调**：`PONY_AGENT_MAX_SAME_TOOL_SIGNATURE_OCCURRENCES_PER_TURN`（1~8，默认 2，非法回退默认）。
- **验收（物理收据）**：`cargo test -p pony-agent-core --lib` 1347 passed / 0 failed（基线 1336 + 本任务
  新增 8 例 + 并行会话 `retry.rs` 新增 3 例）；本任务 11 个新旧用例在**红相提交 `2e2a8d22`** 内同时冻结
  （新增 8 + 改写 3，含
  `run_turn_stops_followup_after_consecutive_identical_tool_failures`、`start_turn_stream_streams_duplicate_tool_recovery_answer`、
  `duplicate_tool_recovery_request_keeps_full_context`），绿相提交 `4ed85bdd` 不改测试文件，
  从而杜绝「代码改了但测试仍绿」的假绿；
  另跑 `pa109_goal_acceptance` / `pa110_teams_acceptance` / `pa112_todo_acceptance` /
  `tool_surface_singularity` / `wave61_team_activation` / `harness_optimizations_test` 全绿。
  契约矩阵与红相收据：`.dev-team/contract-matrix-duplicate-gate.md`、`.dev-team/red-phase-duplicate-gate.txt`。
- **已知残余风险**：① 上限 2 只多给一次机会，超长真循环仍靠连续失败闸（4 次）与墙钟（900s）兜底；
  ② 写类清空会让「读/写交替」这类交替式打转走得更远（受 hop 上限 1024 与墙钟约束），若线上出现
  「3 次同参全 ok 但零进展」应先看墙钟与任务进度，而不是把阈值降回 1；
  ③ 批内（`PONY_AGENT_PARALLEL_TOOLS`）自身重复**不拦**的口径逐字保留——闸点只在 hop 边界，批内并发
  记账顺序不影响计数正确性（只增不减）；
  ④ 纯只读轮询（`team_task_list` / `list_agents` / `JobOutput`）的 30% 误杀只被「上限 2」缓解、未被根治，
  根治需先补注册表标记（见 Alternatives 末条）；
  ⑤ 清空触发集只含 `ToolKind::Write`：`Run`（Execute）与 `workspace_batch`（Composite）成功执行不清空，
  方向是**偏多拦**（保守）而不是漏放行；若要覆盖批内写，需先给注册表补 `Composite` 细分；
  ⑥ 「闸点只读」这条不变量目前**没有机器锁**——命中即收尾，被拒调用即使被误记账在结构上也观测不到（审查实测：
  在闸点插一行 `record_executed` 后 10 个相关用例全绿），只能靠 code review 守；
  ⑦ 批内（同 hop 多个同参调用）记账可越过上限，此时文案「最多允许 L 次」与实际执行次数不符——属既有
  「闸点只判批首」口径的局限，未在本轮改动。
- **审查**：Gate-1 方案对抗 1 路（FAIL→修 AMEND-2 后放行）+ 实施后 L3 两路互补（视角 A 调度/状态流转、
  视角 B 契约/回归面，均 PASS）。收据：`.dev-team/review-plan-dup-gate.json`、`review-l3a-dup-gate.json`、`review-l3b-dup-gate.json`。
- **文档链**：ADR 0060 决策 5「止损三道」已补入第四道（同签名重复计数闸）。
