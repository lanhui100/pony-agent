# 元框架复盘（Wave-54：token / 速度口径真值契约）

> 本轮对象：wave-54（桌面端右栏 token 与速度「不准」的根因修复：span 真值属性 + 单一加权速度口径 +
> 显示纪律 + golden replay 门禁）。历史复盘保留于 `meta-retrospective-wave44.md` 等分波文件。

## 1. 通信拓扑与信噪比（Topology & Noise）

本波是**首次由 Lead 先做三路只读审计、再立项实施**的波次：审计阶段 3 个 Teammate（Tempo 实测 / 前端口径链 /
Rust 物理口径）并行产出带 `file:line` 与真实 stdout 的证据；实施阶段切成 4 个正交分支
（protocol → test → rust / ui），最终由 2 路互补 L3 + 1 路 L3-P 收口。
信噪比问题两处：① 首位 Test Agent 写完 Rust 测试（1301 行）后耗尽上下文，被迫派生 `test-ts` 接手前端测试，
属**任务切片过粗**而非模型能力问题；② L3 审查把 `--scopes` 的逗号串当成单个 scope → lint 误报「范围未闭合」，
两次浪费在工具参数形状上。Token 侧最大浪费是「实施者并行改同一批结构体字面量」引发的返工（见第 3 段）。

## 2. 门禁穿透与误杀率（Gate Penetration / False Negatives & Positives）

机械门禁表现良好：`cargo test --lib` 基线 1175 passed 全程未回归；`vitest` 1084 passed / 0 failed；
`audit-diff --decl-approved` = 0；`check-declaration` PASS。**真实拦截**（本波最有价值的部分）：
- 方案审查（review-plan，FAIL/7 BLOCKING）抓到 3 条真实缺陷：根 span 速度**无生产调用点**（只有测试在调）、
  projection 旧断言与 C5 互斥、anthropic 归一漏 `cache_creation`；
- L3-A 抓到「28 个终态出口只有 4 个落真值」，直接促成架构级修正（收口到 `TurnTraceGuard::drop` 累加结算），
  把「逐点接线」变成「单点收口」；
- L3-B 独立复算 fixture（Σout=4542 / Σgen=47579 ⇒ 95.462）证明两侧公式同源，同时抓到 Rust 侧缺有效调用过滤。
**误杀与噪声**：`--scopes` 逗号问题两次误报；对「已知限制」的重复标记靠人工剔除；一次把「无测试锚定的显示
取整」当缺陷上报（实为既有空白）。结论：门禁的**参数契约**与**已知限制清单**应前移到任务板，而非靠审查员记忆。

## 3. 分工契约与隔离有效性（Contract Isolation）

测试分权有效：9 例旧口径断言全部由独立 Test Agent 迁移，实现者零改测试；冻结的 wave54 测试在实现期只被
**追加**（T4-4 `call_only`、T7 生产接线），无删例、无放宽。
写域正交基本成功，但 Rust 侧「新增结构体字段」触发**编译闭包爆炸**（20 个文件 / 13 个衍生站点），说明凡动共享
DTO，仅靠 `write_scopes` 不足以描述真实爆炸半径 —— 应在 Protocol 阶段预登记 `derivative_scopes`（本波事后补
§3.1 追认）。
另一处摩擦：`call_only` 分支不可达，是 Test Agent 写断言时才暴露的契约空洞（契约只写了模式名、没写可达条件）⇒
**每个枚举值都必须配一条可达性断言**，否则等于没定义。

## 4. 元协议迭代建议（Self-Evolving Protocol）

1. **契约模板增补两节**：`可达性断言`（每个枚举/分支至少一条测试落点）与`已知限制清单`（前置声明，审查员
   不得重复计 BLOCKING）——本波两项都靠事后补丁。
2. **写域声明增补 `derivative_scopes`**：动共享 DTO 时由 Protocol 预登记衍生文件清单，避免实施中途追认。
3. **审查工具参数契约化**：固定 `--scopes` 为空格分隔（或脚本兼容逗号），消除误报。
4. **Test Agent 切片上限**：单 Agent ≤ 2 个测试文件 / ≤ 600 行；超限拆 rust-test / ui-test 两角色。
5. **闭环证据结构化**：`review-join.json` 增设 `closed_findings` 字段，要求逐条给出复核命令，替代自由文本。
