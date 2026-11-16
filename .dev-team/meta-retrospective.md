# 元复盘：B218–B223 状态行/降级终态/会话幂等波次（2026-11-17）

> 复盘对象：本次 Agent Team 的协作元框架与组织协议，不是业务代码细节。
> 波次：B218–B223（tier B，基线 0489b4c6 之后的实现窗口）。成员：Lead + test-agent + exec-tui + exec-core；审查以 fresh-context subagent 挂载（方案对抗 / Join 代码审查）。

## 1. 通信拓扑与信噪比（Topology & Noise）

- **拓扑**：Lead（编排/冻结/门禁/裁决）+ test-agent（独立测试者，3 波红相）+ 2 条常驻 Executor lane（exec-tui：B218/B220/B223；exec-core：B219/B221/B222）+ 2 个 Subagent 审查者，常驻名册 4 人（≤8），审查不占槽位。
- **推进节奏**：每波「红相冻结提交 → 实现 → 定向测试」。3 次红相提交（6dae287b/fe3c647d/0489b4c6）均早于对应实现提交（13c9a35e/b7ef1da7/18394ecd），git 写主唯一（Lead），Executor 只留工作树、零 commit。
- **信噪比**：Executor 报告统一为「符号 + 行号 + 真实 exit code + 未接线项」，无吹捧；Lead 把三次只读侦察（provider/tui/session+sandbox）下沉为 Subagent 执行，主代理只保留裁决所需事实。
- **真实损耗**：`#[path]` 引入 12.9k 行 `bin/tui.rs` 使 5 个 TUI 集成 target 各耗 200–400s；全量 `--lib` 单次 210s、前端全量单测 15min。这是既有成本结构，对策是「定向测试由 Executor 跑、全量由 Lead 在栅栏跑一次」，本轮未出现重复全量跑。

## 2. 门禁穿透与误杀率（Gate Penetration）

- **有效拦截**：①红相阶段 10 条编译错误精确指向缺失契约符号；B222 红相为**断言红**（user 2≠1），拒绝「编译失败冒充红相」；②`file-size` 门在实施中途拦下 4 条新增越界（provider/mod.rs 7116>6975、store.rs fn 210>206、control_plane fn 267>265、runtime/tests.rs 13898>13894），迫使把 B221 家族拆成独立模块 `provider/fallback_body.rs`，属正向收敛；③`contract:check` 复核 TurnStreamEvent Rust/TS 39/39 键无漂移；④`audit-diff` D1 拦住「改声明未显式批准」，提交带 `[decl-change-approved]` 后复跑 PASS。
- **误杀/环境假红**：①`npm run verify` 的 T7-PONYGO-OB2-GATE 在全量并发下偶发 exit=3（audit-diff 判「证据不可得」），单跑稳定 PASS；②V10-3/V10-4 是「全量套件里再跑一遍全量套件」，嵌套 2 worker 实测 608s 撞 600s 上限而失败，属既有环境约束（改前 verify 同样失败）；③`runtime/tests.rs:886` mock server join 曾在一次并行跑中 flaky（单跑 PASS；Lead 全量跑 1592 passed / 0 failed）。三项均在交付说明点名，未当作绿相。
- **基线诚实性**：`scripts/file-size-baseline.json` 同步提交（7582e3c6）只下调本波条目（6975→6963、213→206、388→378、10828→10675），仅 `process.rs 275→308` 一条上调，已用 `7582e3c6^` 基线复跑证明来自 B206（2026-11-16），非本波夹带。

## 3. 分工契约与隔离有效性（Contract Isolation）

- **有效项**：B218（`bin/tui/*`）与 B219（`agent/*`）文件集合不相交；并行期间仅一次跨 lane 编译红（exec-core 加字段导致 `--lib` 红），经 Lead 授权「编译兼容衍生写入豁免」在 12 处测试字面量补 `provider_fallback: None` 后解除，未发生互相覆盖。
- **被击穿项（方案审查 BLOCKING #2）**：实现提交 b7ef1da7 实际改 14 个文件，超出 B219(6)+B221(1) 的 `touched_paths` 声明——扩域包含 Lead 中途显式授权的 `turn_stream.rs`/`turn_sync.rs`（为满足「历史含检查点」验收）、尺寸门禁强制拆出的 `provider/fallback_body.rs`、以及衍生测试模块。写域是并发租约唯一判据，声明滞后意味着「正交性在错误文件集合上被验证」。纠正：本报复盘显式登记实际文件集合；后续波次在 `start` 时即登记 `allow_derivative_scopes`，中途扩域须先更新 `touched_paths` 再动文件。
- **被击穿项（BLOCKING #1）**：同缺陷未判重——B199 与 B219、B200 与 B222 是同一现场同一代码面，新卡落地后未同步闭环旧卡，形成「一边 done 一边 todo」的双真值。已在复盘时闭环（B199→B219、B200→B222，证据含 commit + 测试）。B215→B218 的同波去重（b3dc49c6）是正例；规则应固化为「实现卡 resolve 前必须 grep 同 fingerprint/同现场日志的活跃卡」。
- **测试分权**：5 份验收文件无 `#[ignore]`/`todo!()`/`assert!(true)`；实现提交零改动冻结测试（`git show --name-only` 交叉）。残余弱点：B218/B220 共用同一物理测试文件（fe3c647d +168/−0，实测无篡改，但按卡冻结在文件粒度不可分离）——后续同主题两卡应各自新建文件。

## 4. 元协议迭代建议（Self-Evolving Protocol）

1. **判重前移**：`record` 阶段强制填 `metadata.duplicate_of` 候选（同 fingerprint 前缀/同现场会话 id），`resolve` 时 lint 拒绝「同 fingerprint 前缀仍有活跃卡」。
2. **写域声明与衍生豁免显式化**：`start` 增加 `--allow-derivative`；波次内扩域走 `backlog.mjs edit <ID> --paths ... --reason ...`，让 `git show --name-only` 与 `touched_paths` 可机械对齐（本轮缺口正是缺这条命令）。
3. **重测试隔离**：把「全量套件里再跑全量套件」的 V10-3/V10-4 迁到 `npm run verify:slow` 独立入口，或把嵌套超时提到实测上限 2×；否则每次 verify 都以 ~10 分钟换一个环境超时假红。
4. **TUI 集成测试接缝分片**：47 个 `tests/tui_*.rs` 各自 `#[path]` 引入 12.9k 行 bin；建议抽共享 `tui_test_harness` 编译单元，把每 target 成本从「全量 bin」降到「harness + 用例」。

## 5. 审查收口记录（L3 Join）

- 首轮 L3（reviewed_commit c912def2）verdict=FAIL：1 BLOCKING（store 层幂等早退击穿冻结用例 ADR0123 T-A1b）+ 3 ADVISORY（turn_started_at 累计、事件物化覆盖检查点正文、兜底正文消毒可绕过）。
- 处置：BLOCKING 走「显式契约变更（Owner 批准）」——在 ADR 2026-11-16-session-history-turn-id-idempotency 增补「契约变更登记」章节并把 T-A1b 断言迁移到新语义（保留首写、同轮两条）；3 条 ADVISORY 全部落地（TurnStarted 无条件重置锚点、`close_out_assistant_contents` 让 kind=Some 走调用方检查点、零宽/拆分/裸 host 消毒）。
- 复审（reviewed_commit 7f3ebd2f）verdict=PASS，`dev-review-lint.py` Exit 0；新增 1 条不阻断 ADVISORY（消毒判据误伤含点/斜杠的工具名）已在本轮收口。
- 教训：把「冻结用例被新契约击穿」当作**契约演进**处理（Owner 批准 + ADR 登记 + 用例迁移），而不是回退实现——回退等于保留一个已被现场证伪的契约。
