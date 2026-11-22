# 元复盘 · `runtime-store-split`（前端 runtime store 按域拆解）

> 范围：2026-11-22 单会话交付（B 级；worktree 强隔离 `~/pony-agent-worktrees/runtime-split`，
> 因主工作区被另一会话 `fix/tui-panel-visibility` 占用）。名册峰值 5 常驻 + 4 子代理（L3-A/L3-B/L3-P/Gate-1）。

## 1. 通信拓扑与信噪比

**事实**：常驻 5 名（Lead / Protocol / Test / Executor A / Executor B）全部按「文件数 + 行数 + 命令 + exit code + 未决项」
回报，没有过程性吹捧。唯一一次信噪比事故是 Lead 自己用 `ls .dev-team/*.json | head` 把 300 条历史审查文件全量打屏（约 13KB）。

**重复命令率**：`npx vue-tsc --noEmit` 全程被 5 个角色各跑 2–4 次（合计 ≈14 次，单次 20–60s），其中至少 4 次是同结论复跑
（Protocol 基线、Executor A 前后、Executor B、Lead 独立）。这是「独立收据 vs 重复计算」的取舍：用 5–8 分钟墙钟换跨角色
独立取证。可优化：把 tsc 结论写 `.dev-team/…/tsc-receipt.json`（HEAD + cmd + exit + 输出 hash），下游只验 hash 不重跑。

**Token 效率**：一次 Explorer 派发产出 90 行事实地图，替 Lead 省掉读 6075 行的注意力消耗，是本波最划算的一笔。
反面：Lead 在等子代理期间做了 3 次低价值轮询（2 次 `wait_agent` 空转 + 1 次巨量 `ls`），应改为以任务板 revision / 产物 mtime 为唤醒信号。

## 2. 门禁穿透与误杀率

**true positive**：Test Agent 变异自检 3/3 全部按预期由绿转红并回滚复绿（`persistHistory` 少写 `cursorVersion`、
delta 不推进 `messageRevision`、completed 不清 `phase`），证明 23 条等价用例不是「跑起来不报错」的假测试。

**false positive**：Lead 自写的字节级等价脚本前两版误报 80/108、10/108，根因是**去缩进基准取错**（把首行 `{` 的 0 缩进算进 min）
与只归一 `RuntimeStoreThis` 漏掉 `TurnEventsThis`；修正后 108/108 全过。教训：等价核验脚本必须先做负样本再用于判绿。

**真实穿透风险**：23 条用例只覆盖 57/97 actions、13/18 getters；40 个未覆盖 action 的行为漂移不会被任何门禁拦住，
只能靠 Lead 的 token 级函数体比对兜底。已登记为 Wave 2 测试补强项。

**Gate-1 时序偏差（自省）**：`dev-team` 要求方案对抗审查先于编码，本轮实际是编码完成后补做 Gate-1。
补救是真做了对抗审查（靶点全部落 ADVISORY 并进 Wave 2 清单），但时序属流程穿透，记入 §4。

**共享文件名竞态（CI 抓到，已修）**：本波 Protocol 直接把 `dev-team` 要求的 NFR 基准写成 `.dev-team/nfr-baseline.json`，
覆盖了仓库共享的既有文件 —— 该文件同时是 `acp_p1_blockers` / `acp_realtime_permission` / `acp_tool_policy` 等
Rust 验收用例的阈值真源（`acp.request_timeout_ms` 等 5 个键）。本地前端门禁全绿，直到 PR CI 跑到 Rust 用例才炸
（6 条 panicked: 「nfr-baseline.json 缺 acp.request_timeout_ms」）。修正：恢复共享文件为 base 内容，本波基准改为
`.dev-team/nfr-baseline-runtime-store-split.json`（wave 作用域，与 `nfr-baseline-wave45.json` 同惯例），两份都过 `dev-nfr-lint`。
教训：`dev-team` 里凡以通用名落盘的文件（`nfr-baseline.json`、`state.json`、`review-*.json`、`meta-retrospective.md`）
都是跨波共享面，写入前必须 `git ls-files` 确认归属；CI 的 Rust 用例是这条的唯一机械门禁，前端绿灯不能替代。

## 3. 分工契约与隔离有效性

**写域隔离有效**：Executor A（4 文件 / 28 actions）与 B（6 文件 / 69 actions）零重叠，最终 `git diff --name-only`
恰为 10 个域模块，机械写域审计 `ALL_IN_SCOPE`。

**事实耦合两处**：① Protocol 冻结的 `types.ts` 与任务描述出现 3 处归属冲突（`getAttachmentAssets` /
`persistHistory`+`updatePersistedBackgroundSession` / `recallLastUserMessage`），由 Protocol 落盘前主动上报纠正 ——
说明「契约文件 vs 任务描述」双真源本身就是风险，任务描述应引用接口名+行号而非复述清单。
② 两位 Executor 共享同一工作树同一分支，靠 write_scopes 约定而非物理隔离；本波文件正交所以安全，
若下次需多人改同一文件必须升级 worktree 强隔离。

**测试分权有效**：Test Agent 独立写用例并冻结（2f11dc2f），Executor 全程无测试写权限；两次测试侧缺陷
（`$patch` 类型错、`cursorVersion` 陈旧断言）均由 Test Agent 自己发现并以「断言注释 + 依据行号」修复，
未出现「Executor 改测试让自己变绿」的路径。

## 4. 元协议迭代建议

1. **门禁收据去重**：tsc/vitest 结论写 `receipt.json`（HEAD + cmd + exit + hash），下游只验 hash；本波 14 次 tsc 至少 4 次可省。
2. **Gate-1 前置硬约束**：把「`review-plan.json` 存在」设为开波前置门禁（本轮是后补），缺则不得 claim 任何编码任务。
3. **等价核验脚本入 skill**：把修好的「token 归一化函数体比对」脚本附负样本自检，作为 dev-team 复用资产，避免每次重构重踩去缩进基准的坑。
4. **契约单真源**：任务板描述只允许引用 `types.ts` 接口名 + 行号，禁止复述键/方法清单（本波 3 处冲突全部源于复述）。
5. **等待策略**：peer 全 inactive、只有 subagent 在跑时 `wait_agent` 必然空转；改为轮询产物 mtime / 任务板 revision，
   并把子代理产物落盘路径写进任务描述。
