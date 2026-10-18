# wave-35 元复盘：Agent Team 协作元框架诊断

对象：wave-35「修复 11 个恒定失败用例 + 2 个 `-D warnings` 编译阻断 + 全量套件真死锁」。
Roster：Lead + 3 Executor + 1 Test Agent（含 Lead 共 5，≤8）；分析/审查全部下沉为无状态 Subagent（3 名分析 + 2 路 L3 + 1 路 L3-P + 1 路 L3 增量复审），不占常驻槽位。

## 一、通信拓扑与信噪比

**有效部分**：任务板 + `write_scopes` 把 6 个任务切成互不相交写域，全波**零次写域冲突**（9 个文件分属 4 个 Executor 且两次接手都发生在前任已停止之后）。把「归因」与「修复」彻底分离是本波最大收益：3 名只读分析 Subagent 给出带 `path:line` 的机械结论后，Executor 只做改码，不再重复侦察——Executor 平均单轮即交付，无一次因「先想清楚再动手」返工。

**信噪比劣化点**：
1. **Lead 侧的侦察仍偏重**。Lead 亲自跑了 20+ 次 `grep`/`sed` 与多次 `cargo test` 复核，其中相当一部分（如 `if cfg!(windows)` 全量扫描、`rfind`/字节切片全量扫描）本可外派 E1 档 Subagent。Lead 上下文被读取动作吃掉，是本次 session 变长的主因。
2. **两个任务描述里的数值口径错误**由 Executor 反向指出（task-3 写「6 个 TEST-DEFECT」实为 5 个）。Lead 出题时的自检不足，靠 Executor 纠错属运气。
3. **两次严重串行化失误**：Lead 在 Executor 仍在写 `runtime/tests.rs`/`retry.rs` 时两次授权 Test Agent 跑全量，导致其验收作废重跑（freeze-1 → freeze-2）。根因是 Lead 把「验收」当成一次事件而非「写者全停」的全局状态判断。教训：**freeze 必须由 Lead 用 `git rev-parse` + `git status` 机械确认「无写者在飞」后才发布**，不能凭记忆。

## 二、门禁穿透与误杀率

**真实拦截（有物理证据）**：
- **L3-B（安全/协议视角）拦下了本波自己引入的 P0**：C3 首版 `strip_endpoint_fragments` 用 `str::rfind(...)+1` 的字节索引做切片 ⇒ 中文/全角冒号/emoji 紧贴 URL 时 panic，且入参是**上游可控文本**（会打掉整次 turn）。若只有机器门禁，这个缺陷会随「全绿」放行。
- **Test Agent 的独立复现补上了 L3 的盲区**：L3-B 只给出一个反例，Test Agent 用独立探针 crate 实测 4 例（CJK/全角/emoji）并指出**首版自带回归测试根本捕获不到它**（输入在 URL 前总有 ASCII 分隔符）⇒ 促成「多字节紧邻 `://` 必测」的硬要求与门禁新判据 `UNICODE_URL_BOUNDARY_UNCOVERED`。
- **反作弊门禁 17/17 类注入作弊全部捕获**，且 Test Agent 主动加固了 Lead 原始 G1 口径的 7 类绕过漏洞（断言数守恒式替换、恒真操作数、断言迁移、空壳断言助手等）。
- **L3-A（并发/状态视角）**判 PASS 但顺手给出两条高价值事实：`OverrideGuard::set(` 全仓仅 1 处（证明 C1 修复的完备性）、以及门禁 session 解析本就 fail-open（影响安全结论的解释）。

**误杀与门禁成本**：
- L3-P 判 PASS 0 findings，但其要求的「test-only 偏差登记」是真实成本：`MOCK_SERVER_FINISH_TIMEOUT` 初值 10s 命中 `deadlock_watchdog_timeout_ms=5000` 基准，Lead 复核后下调为 5000ms。**这说明数值型基准门禁对「新引入常量」是有效且廉价的**——它逮住了一个纯靠 review 容易放过的不合规。
- 误报：Test Agent 自己的仪器出过两类假信号（`UNICODE_URL_BOUNDARY_UNCOVERED` 假阳性因 TSV 生成顺序错；边界探针 v1 丢落盘写入却打印 `MUTATION_APPLIED` 导致假阴性 `INSENSITIVE`）。两者都由 Test Agent 自证并留痕隔离（`INVALID_*.log`），**这是本波最重要的方法论收获：验收仪器自身必须自证（替换→落盘→读盘校验），否则「看起来跑了」会直接污染裁决。**

**未被门禁覆盖的盲区（靠 review，已显式声明）**：等值断言被换成更弱但成立的断言、平台门禁补偿规则被语义空洞的新测试利用。Test Agent 与 L3 均标注，未假装机器能判。

## 三、分工契约与隔离有效性

- **写域排他公理执行良好**：9 个改动文件的 `write_scopes` 全程无交集；`governed_executor.rs` 同时含「生产豁免」与「测试补 hash」两类改动时，Lead 把它们收在**同一个 Executor** 下，避免两写一文件。
- **测试分权真实生效**：Executor 只改测试以符合冻结契约（补 `expectedHash`），**验收口径（用例数不降、`#[ignore]`=0、断言数不降、变异敏感性）全部由 Test Agent 独立机械判**，Executor 无权改判。Executor 交付的「7/7 绿」被 Test Agent 用 `--list`/G1 普查二次确认，未出现「自证放行」。
- **共享工作区风险真实发生**：另一会话（TUI e2e 波）在本仓库执行了 `git add -A` 提交（`626f023`），把 Test Agent 的 wave-35 收据顺手卷进其提交。Lead 以定向 `git add <9 paths>` 保护了自己的改动。**教训：多会话共享工作区下，`write_scopes` 只保护「谁写文件」，不保护「谁提交」；Lead 必须在每个波次结束点用定向 add 抢占提交，不能等到收尾。**
- **滚动契约账本**：本波的关键契约（9 文件 + 基准 1058/1062 + `#[ignore]`=0 + 两个冻结回归测试）始终压缩在 10 行内向下游传递，未出现跨阶段依赖错乱。

## 四、元协议迭代建议（对 dev-team skill 自身）

1. **冻结必须机械化**（新增硬门禁）：`freeze` 动作应由脚本断言「无 Executor 在 running + `git status` 中无本波 write_scopes 内文件处于 dirty」，否则拒绝发布 freeze 哈希。本波两次因违反此条而作废重跑验收。
2. **「新增回归测试是否足以捕获被修缺陷」应成为强制项**：本波 C3 首版的回归测试在 URL 前总有 ASCII 分隔符 ⇒ 修法可退回而测试恒绿。建议 skill 在红相阶段强制要求对**每个修复**给出「退回测试（mutated revert）必须变红」的机械读数，并把「变异自证」写入 Test Agent 的交付契约（本波已实践，效果决定性）。
3. **数值型 NFR 门禁应扩展到「新引入常量」**：现有基准只比对既有实现常量，本波 10s vs 5s 是 Lead 主动比对才发现的。建议把 `deadlock_watchdog_timeout_ms`/`lock_acquisition_timeout_ms` 等做成「新增 `Duration`/超时常量的自动比对」门禁。
4. **修正遮蔽风险应作为独立检查项**：修复一个判据/门禁时，可能**解除对下游缺陷的遮蔽**（本波 C3 修复让长期被「端口数字伪重试」掩盖的 mock 启动竞态暴露）。建议 skill 增加「遮蔽分析」问句：本次修复是否移除了某个掩盖下游问题的偶然因素？并把暴露出的新失败**登记为已知残留**而非回滚修复。
5. **Test Agent 的仪器自证条款入册**：把「替换→落盘→读盘校验」与「假阳性/假阴性必须隔离留痕（INVALID_*）」写进 skill 的 Test Agent 契约。
6. **Lead 侦察阈值执行不严**：skill 已有「≥5 文件读 / ≥2 轮 grep 即必须外派 Explorer」的机械阈值，本波 Lead 有 3 次违反（`if cfg!(` 扫描、字节切片扫描、失败用例清单普查）。建议在 Lead 守则中加入「同一波内 Lead 连续 3 次 `grep/sed` 未产出裁决性结论即强制外派」。

## 附：本波未决残留（转入待办，见 ADR 0038）

F-1 门禁 session 解析 fail-open（预存）｜F-2 `path_permission.rs:764` Linux 零断言假绿（预存）｜F-3 CI 只在 windows 跑 core 测试，Linux CI 从不执行平台用例（预存）｜
F-4 `lifecycle_boundary_checkpoint` 全量并行 ~7.7% flake，机制未定位，建议确定性就绪握手（本波解除遮蔽）｜F-5 码表仍为子串匹配（`elapsed=40832ms` 可命中 `408`）｜F-6 `recovery.rs` 第二套裸 `contains` 判据（潜在漂移）｜
F-7 `plan_control` 豁免后的安全姿态变化（Draft 自解锁，本轮决策留档）｜F-8 scheme-less `host:port` 仍可被误判（P3）。