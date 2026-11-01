# 元框架复盘（Wave-58：上下文构建与 Skills / AGENTS.md 加载修复）

> 本轮对象：wave-58——按审查报告 `management/task-system/02_REVIEWS/2026-11-01-context-skills-loading-audit.md`
> 修复上下文构建、技能清单注入、指令根解析、触达去重、frontmatter 解析与扫描/加载信任一致性。
> 交付：`.dev-team/wave58-delivery.md`；契约：`.dev-team/contract-matrix-wave58.md`。
> 团队：Lead + protocol-agent + test-agent + executor-alpha + executor-beta（常驻 5）+ 6 次 Subagent 池审查。

## 1. 通信拓扑与信噪比（Topology & Noise）

- **拓扑选择正确**：Protocol/Test/Executor 是跨阶段有状态连续体，落为常驻 Teammate；L3 审查、方案对齐属无状态投影，落为 Subagent（2 轮 × 2 视角 + 方案对齐重试）。名册峰值 5 ≤ 8，符合容量引理。
- **写域正交有效**：`context.rs/subagent_runtime.rs` ‖ `workspace_hotloader.rs/skill_body.rs/path_permission.rs` ‖ `tests/` 三块互不相交，两个 Executor 全程零文件冲突；`tools.rs/instruction_touch.rs` 作为第二波次串行接续。机械写域审计一次性通过，无越界。
- **信噪比问题（真实）**：跨写域新 API 产生 3 轮签名协商（`SkillCatalogBudget` / `build_cached_prompt_prefix_with_budget` / `resolve_session_prompt_context_with_budget_and_catalog`）。根因是 Lead 先要求 alpha 在 context.rs 自行裁剪，随后 beta 又提供裁剪入口，导致 alpha 写了一份裁剪再被要求删除。改进：新 API 一律先由 Protocol Agent 冻结签名再分发。
- **重复开销**：全量 `cargo test -p pony-agent-core` 跑了 5 次（1777 基线 → 1798 → 1802），每次都有新信息（修复后复验），不构成反重试风暴，但构建墙钟是主要成本项。

## 2. 门禁穿透与误杀率（Gate Penetration / False Negatives & Positives）

- **真实拦截**：L3 两轮共抓到 5 个真实缺陷，其中 4 个是本波**新引入**的：
  1. `skill_files` 缓存键缺 workspace + 进程单例化 ⇒ 跨工作区技能描述泄露（BLOCKING，两路独立命中，安全视角做了物理复现 `POISON=true`）；
  2. 会话基线 LRU 淘汰静默重建前缀（BLOCKING，破坏 F1-3 字节稳定）；
  3. 整段前缀回放忽略 `catalog_budget`（由 executor-alpha 自测捕获，非门禁捕获）；
  4. 清单 token 未转义 ⇒ 可伪造条目行/段落标题；
  5. 会话基线只按 session_id 分桶 ⇒ 换绑工作区回放旧前缀。
- **门禁穿透（重大）**：**红相门禁被流程偏离击穿**。DEV-58-1 允许实现在测试冻结前并行启动，导致 F1-* 与 F2-* 共 21 条中大部分用例的「空桩态失败」证据永久缺失（落地即真绿）。补偿（契约矩阵先冻结 + 独立 Test Agent + F3-1/F3-3 保留真红相 + L3 以 diff↔F 编号对照）**部分有效但不等价**：它证明「断言可实现」，没有证明「断言能捕获错误实现」。本波若没有 L3，两个 BLOCKING 会直接进入主干。
- **误杀**：0 条 BLOCKING 误判；2 条 ADVISORY 被正确降级（LRU 淘汰语义实质正确、种子「并集方案」无收益）。
- **门禁自身假绿**：`.dev-team/review-plan.json` 为历史波次残留，元复盘 lint 仅 glob 判存在 → 假绿。本波已补 `review-plan-wave58.json` 并在本文点名。

## 3. 分工契约与隔离有效性（Contract Isolation）

- **测试分权成立**：Test Agent 独立依契约矩阵写 25 条用例（12+9+4）；Executor 全程只读测试。executor-alpha 发现新红后**主动上报而非改测试**，是分权有效的正面证据。
- **契约冻结有效**：F3-1 与既有 wave42 C4-5 逻辑互斥时，唯一改动由 Lead 执行（裁决 D6'），并留有 `git diff 9af49dd -- tests/` 漂移收据（仅 13+/3-）。全波零「实现者改测试换绿」。
- **信息下发两侧失衡**：过度——给 beta 的 D5' 指令夹带 Lead 自拟的排序口径 `(rank, 项目优先, name)`，该隐性契约直接导致 F1-6 先红；不足——未在立项时登记「`HostTurnEngine` 生产零构造」，直到 F1-5 报告才暴露「能力就绪但整体不可达」。
- **Subagent 稳定性**：方案对齐审查首次与重试各失败 1 次（无收尾消息）。长 prompt + 多文件读取显著降低成功率。改进：Subagent 审查限「只读 3~5 文件、只回答 3 问」。

## 4. 元协议迭代建议（Self-Evolving Protocol）

1. **红相先行不得用「契约矩阵先行」替代**：并行唯一条件是「测试文件已落盘且编译通过」；否则 Lead 必须串行等待 Test Agent。本波省 1 轮等待，代价是 21 条用例失去红相证据。
2. **跨写域新 API 先经 Protocol Agent 冻结签名**，禁止在单 Executor 指令里临时定义跨写域契约。
3. **单例化改造强制三件套**：改缓存键含作用域 + 加跨作用域隔离回归用例 + 检查「进程级单例 × 每工作区缓存」矩阵。本波 R1/R2 属事后补加。
4. **元复盘门禁改为「本波次生成的 review 文件」**（按 commit 范围或 mtime 过滤），消除历史残留假绿。
5. **B 级也强制跑「六问」的并发/缓存两项**：本波未触外部网络（`audit-diff.sh` Exit 0），但「并发与幂等」「资源泄漏」与单例缓存 + LRU 实质相关，恰好是缺陷高发区。

## 5. 逃逸与残留

- 无交付后逃逸（无 revert/hotfix）。
- 已登记残留（见交付说明 §3）：F1-5 生产未接线、A2 边缘缺口、D6 同目录共存语义未改、`.agent/` 与 `~/.dsh` 未支持。
