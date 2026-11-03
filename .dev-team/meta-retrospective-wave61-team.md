# 元框架复盘（Wave-61-team：Agent Team 激活）

> 对象：wave-61-team —— trace 显示 dev-team 技能无法触发 agent team / 子智能体。
> 交付：`.dev-team/wave61-team-delivery.md`；契约：`.dev-team/contracts/wave-61-team-activation.md`（FROZEN v1.4）。
> 团队：Lead + protocol-agent + test-agent + executor（常驻 4）+ 6 次 Subagent（2 Explorer、方案审查×2、L3-A、L3-P、recheck）。
> 交付区间：`52a7a21`（红相锚定）→ `922e960`（v1.3 绿相）→ v1.4 收口提交。
> 审查收据：`review-wave61-a.json`、`review-wave61-l3p.json`、`review-wave61-recheck.json`、`review-plan-wave61-team.json`（均通过 `dev-review-lint.py` 结构门）。

## 1. 通信拓扑与信噪比（Topology & Noise）

- 拓扑判定：Protocol/Test/Executor 是有状态连续体（契约→红相→实施→复审），落常驻 Teammate；
  两个 Explorer、方案审查、L3-A、L3-P、recheck 都是无状态投影，落 Subagent，未挤占名册（峰值 4 ≤ 8）。
- Explorer 价值实证：两路侦察提前钉死「团队工具全落 `Internal`」「`set_subagent_engine` 生产零调用」
  「`workflow` 是桩」「BASE_SYSTEM_PROMPT 有 3790 字节逐字冻结断言」四条反直觉事实，直接决定 C1/C2/C3 口径。
- Token 浪费点：① Lead 三次 `sleep` 等 Subagent（`wait_agent` 对 Subagent 报 no-active-peer）；
  ② 契约 v1.0→v1.4 演进导致冻结测试迁移做了三轮；③ 审查员把结果写进历史波次文件名，Lead 需回滚改名。
- 信噪比：回执均为「结论 + file:line + Exit Code」；executor 主动上报「契约 C3.3 与 Lead 指令冲突」并索裁决属正面行为。

## 2. 门禁穿透与误杀率（Gate Penetration / False Negatives & Positives）

- 真实拦截：四轮审查抓出 7 条机器门禁抓不到的缺陷 —— ①`workflow` 伪造 `status:success`；
  ②9 个团队工具同为桩（spawn 丢弃 prompt / send 无消费方）；③桌面宿主不传 workspace_root ⇒ 子代理跑错目录；
  ④双重扣并发许可 ⇒ 有效并发 2→1；⑤超时分支提前归还许可 ⇒ 孤儿线程突破上限；
  ⑥策略段引用不存在的工具/错误码；⑦drain-then-run 失败丢消息却仍报 `delivered:true`。全部修复且有对应用例。
- 门禁穿透（Lead 侧）：① v1.1 契约里「semiStable 层」与 C3.3「第二条 System 消息」自相矛盾，由 executor 阻塞上报才纠正；
  ② 红相首版把 39 项当定稿，第二轮审查把 `workflow` 退回后触发 39→38 全链迁移；
  ③ AC-05 迁移第三次才被方案审查指出未登记进 C4 —— 契约演进面清单人工维护必漏。
- 误杀/降级：「策略门控 + reserved_extra_count + C9 在生产恒真属死复杂度」判 ADVISORY（库/测试路径确有未接线态）；
  `--lib` 多次因环境 `EAGAIN` 失败已如实归因为环境而非断言失败。

## 3. 分工契约与隔离有效性（Contract Isolation）

- 测试分权守住：三轮冻结迁移全部由 Test Agent 执行，Executor 未碰 `tests/**`；每次改写要求「旧断言→新断言+理由+契约行号」，
  未放宽（禁 `>=`、禁 `#[ignore]`、禁 sleep 掩盖竞态）。
- 写域隔离：工作区同时存在并发会话（wave-60 收尾、wave-61-status-panel），全程未用 `git add .` / `reset --hard` / `clean`；
  `provider/mod.rs` 与 `control_plane/mod.rs` 两处共享文件的 hunk 归属逐次核对。
- 滚动契约账本：v1.0→v1.4 全部原地改写对应章节（C1.6/C1.7、C3、C6~C11），未追加平行章节。
- 环境代价：并发编译 `fork: EAGAIN` 反复出现，全量 `cargo test --tests` 无法稳定跑通，改以「定向 8 套件 + lib（低载 1210/0）」
  作为门禁并显式声明适用范围 —— 环境事实，不是验收降级。

## 4. 元协议迭代建议（Self-Evolving Protocol）

1. **能力诚实性前置门禁**：契约冻结前对每个「要暴露给模型的工具」做 handler 真伪核对（真实副作用执行路径 +
   失败可观测），产出 capability-honesty 表；桩工具一律不进模型面。本波因此返工两轮。
2. **审查产物唯一命名**：给审查 Subagent 的提示词必须写死「只写 `.dev-team/review-<wave>-<id>.json`，
   不得复用历史文件名」——本波两名审查员覆盖了 wave-60 的 `review-join-a.json` / `review-l3p.json`。
3. **`wait_agent` 对 Subagent 不可用**：skill 应明确「等 Subagent 用文件 mtime / job 事件驱动」并禁止 `sleep` 轮询。
4. **契约演进面清单自动化**：C4 的冻结测试迁移面应由 `git diff --name-only <red>..<green> -- tests/` 自动推导。
5. **环境资源门禁**：并发会话共享 `target/` 时全量测试会出现 `EAGAIN` 假红，建议内置「并发检测 → 降级定向套件 + 显式声明」模板。
6. **中断类语义三分同源**：凡「中断」工具必须在契约里区分「立即中止」与「边界停止」，并让工具描述、系统提示、验收断言三处同源。
