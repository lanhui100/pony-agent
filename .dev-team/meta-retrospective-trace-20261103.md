# 元框架复盘（Trace 诊断评估：pa_01a1a134 · 4 turn）

> 本轮对象：按用户要求「找到桌面端最近一次 trace 上传，基于该 trace 诊断桌面端优化点，由 dev team 给出评估」。
> 任务性质：**只读诊断 + 文档交付**（不写业务代码，故红相/契约冻结/NFR 六问等编码门禁不适用，下文记 N-A 理由）。
> 交付：`docs/analysis/trace-analysis-pa-01a1a134.md`（+ `docs/INDEX.md` §2.5 索引）；证据：`.dev-team/evidence/trace-20261103/{forensics,core,desktop,verify}.json`。
> 注：本文件独立命名（非 `meta-retrospective.md`），因该路径正被另一波次（wave-59）占用且工作区存在未提交改动，共享工作区内覆盖他人文件属脏写。

## 1. 通信拓扑与信噪比（Topology & Noise）

- **拓扑选择**：三个取证/映射角色均为「单次交付即解耦」的无状态工作，按公理本应下沉 Subagent；
  本轮仍用 Teammate（3 个）承载，理由是三者写域互斥（三个不同证据 JSON）、且需要并行长时取证。
  实际回报：三者均一次性产出、无跨阶段状态继承，**Teammate 槽位属过度配置**（成本无差异，但违反了公理）。
- **并行收益真实**：trace 取证（ClickHouse/Tempo 查询）与两侧代码映射（Rust / Vue）几乎零依赖，并行消除了等待。
- **信噪比**：三份回执均为「结论 + path:line + 数字」，无套话；desktop-optimizer 主动上报自身未直连 ClickHouse 的
  证据边界并请 Lead 复核（正面行为，避免了把推断当事实）。
- **Lead 侧的浪费**：写报告前 Lead 自查了 8 处 path:line（必要，属裁决性复核）；但对 Tempo 的
  `pony.otel.forced` 反查与 `agent.turn` 全量检索（3 次 curl）属可用一次查询合并的重复动作。

## 2. 门禁穿透与误杀率（Gate Penetration）

- **命中（真实拦截）**：trace-analyst 的 F4/F5 自查把「零 payload 是桌面端独有」修正为「桌面端 trace 级 + 全局 observation 级两处缺口叠加」；
  若不做对照 producer，会把全局问题误判成单端 bug。
- **命中（Lead 纠偏）**：desktop-optimizer 断言 degraded「口径不一致」，Lead 复核 `otel/mod.rs:1986` 后确认
  `degraded` 仅在 `phase=completed` 输出，属**设计分流**而非缺陷——报告已按「设计分流 + UI 未呈现」改写，避免误杀。
- **风险（未闭合）**：三个 Teammate 均由低成本快模型承载，其结论属 E2 判断型；虽有 8 处 Lead 抽验，
  但抽验覆盖不等于全量复核，故报告对推断项显式标 `[UNVERIFIED]`/`inferred`，并把决定结论走向的关键项
  （payload 缺失根因、parallel_tools 未接线、上传反馈文案）交给独立 verifier 全量复核。
- **N-A（编码门禁）**：本轮零源码改动、零契约变更、零网络/IO 行为变更，故
  NFR 基准表、L2-T/L2-AT 红相、L3-P 六问、observability `audit-diff` 均不适用；理由：交付面为只读诊断文档。

## 3. 分工契约与隔离有效性（Contract Isolation）

- **写域正交成立**：`forensics.json` / `core.json` / `desktop.json` 三文件互斥，全轮零冲突；Lead 只写报告与索引。
- **信息不过度下发**：给三者的 prompt 只含「trace 事实 + 自己那一面 + 交付格式」，未互相下发改写域外的实现细节。
- **隔离缺口**：Lead 把「已知 trace 事实」以文字粘贴给两侧代码映射者，等于把 trace-analyst 的待验证结论
  提前当事实下发——若该结论有误，两路分析会同时建立在同一误判上（同源污染）。
  本轮缓解：要求三者在结论标 confidence，并由 Lead 抽验 + verifier 复核。

## 4. 元协议迭代建议（Self-Evolving Protocol）

1. **诊断型任务需要独立分支**：`dev-team` 全文以「编码交付」为假设（红相/契约/NFR/栅栏），对「只读诊断」类任务缺少
   最小路径定义（本次靠 Lead 临场裁剪，过程记 N-A）。建议 SKILL.md 增「R 级（Research/诊断）」：允许无红相、无 NFR，
   但强制「事实层（取证）/ 解释层（映射）/ 裁决层（Lead）」三段分离 + 独立 verifier。
2. **证据 JSON 契约应内建 `confidence` 与 `coverage` 字段**（本轮三份都自发采用，说明是刚需），并规定 E2 型结论
   必须由非同一模型复核——否则「低成本快模型 + 并行」会放大同源误判。
3. **共享工作区多波次并发**：`.dev-team/meta-retrospective.md`、`state.json` 这类单文件账本在多 shell 并行时必然冲突，
   建议按波次分文件（`meta-retrospective-<wave>.md`）并由 lint 接受目录级聚合。
4. **trace 取证工具化**：本轮「找最近一次上传 trace」共 6 次 kubectl/curl 手工拼装（ClickHouse 表名、Map 列函数坑、
   Tempo TraceQL 的 `client.service.name` 归因），应收敛为 `resolve-issues` 的一个 verb（`rca.py latest --producer desktop`）。
