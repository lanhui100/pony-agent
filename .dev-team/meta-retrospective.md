# Wave-12 元复盘（PA-SESSION-FALLBACK 删除/归档当前对话后的落点）

> 复盘对象：本次 Agent Team 的**协作元框架与组织协议**。账本：`.dev-team/state.json`（wave 12）、契约 `.dev-team/contracts/wave-12-session-fallback.md`、收据 `.dev-team/evidence/{red,green}-phase-wave12*.log`。

## 1. 通信拓扑与信噪比

| 要素 | 实际 | 评价 |
| :-- | :-- | :-- |
| 名册 | Lead + Test Agent + Executor（3/8） | 合适：单模块状态机改动 + 既有冻结测试改写 |
| 看板 | T5（红相）/T6（落点实现）/T7（冷启动红相）/T8（冷启动实现） | 干净：全部 completed，无 in_progress 残留 |
| 独立审查 | 2 轮（wave-12 增量复审 PASS；wave-12.1 增量复审 PASS + 2 条非致命观察） | **由 Test Agent 承担**（见 §4-①） |
| 关键增益 | 复审报告「同族残留」→ Lead 真实浏览器复现 → 契约演进 wave-12.1 | 本波最大价值：避免只修一半（重启后又落回已归档会话） |
| 信噪比 | 两次「处方不充分」均由执行方主动上报并附 A/B 机械证明（Executor 的第三处早退分支、Test Agent 的 T-g 红绿证明） | 好：未出现「跑通即放行」 |

## 2. 门禁穿透与误杀率

- **红相真实性**：wave-12 红相 9 failed（含 4 个旧契约用例按新契约转红）；wave-12.1 红相 5 failed + e2e 1 failed。两次红相均锚定 commit 后执行方才写 `src/**`（除 wave-11 遗留的抢跑问题本波未复现）。
- **假测试拦截**：T-g 加固证明「all-archived 早退分支」在 store 级原本无判别力（移除该分支 A9 仍绿，仅 e2e 能红）；而仓库 CI 的 `verify` 作业**不跑 e2e** → 该分支在 CI 里等于无门禁。加固后 store 级可判别（移除分支 → EXIT=1，已留痕）。
- **误杀率**：2 轮复审共 0 findings（无误杀）；2 条非致命观察 1 条被采纳加固、1 条（首启无会话 vs 全归档的侧栏行不对称）判为既有 UX 不对称、登记为后续项。
- **回退/篡改防护**：Test Agent 为红绿证明临时改 `src` 时，因 `src` 未提交而不能用 `git checkout --`（会抹掉 Executor 实现），改用备份 + `sha256sum -c` 字节级还原校验 —— 该纪律值得写入 skill 模板。

## 3. 分工契约与隔离有效性

- `src/**` 与 `tests/**` 硬分权保持：整波 8 次往返无越界写入（`git diff --name-only` 逐次核对）。
- `blocked_by` 正确串行化：T6 依赖 T5、T8 依赖 T7；`write_scopes` 对 `runtime.ts` 的跨任务重叠被看板显式提示（T6 完成→释放→T8），未发生并发写冲突。
- Lead 仅做：侦察与复现（浏览器脚本）、契约/裁定落盘、git 锚定与合并、最终门禁与文档（ADR/复盘）；未写业务逻辑。

## 4. 元协议迭代建议

1. **subagent 计算池在本环境连续失败**：wave-12 的 2 路 L3 审查子代理全部「failed before it finished」（无 closing message），wave-11 的增量复审子代理也失败过 4 次。应急做法是改由常驻 Test Agent 承担独立复审（其未编写 src，独立性成立）。建议 skill 增加**降级路径**：当 subagent 池不可用时，允许由「未参与该模块实现的常驻 teammate」承担 L3，并在账本中显式记录降级原因与独立性依据。
2. **「处方充分性」应作为 Executor 的强制回报项**：本波 Executor 两次主动证明「按任务书处方改完仍不达标」（wave-11 的 M3 度量、wave-12.1 的第三处早退分支），避免了假绿。建议 skill 在 Executor 回报模板中固定一行：`处方充分性：是/否 + A/B 机械证据`。
3. **红绿证明的 src 还原纪律**：任务书要求「临时改 src 做红绿证明」时必须规定还原校验方式（本波用 sha256），否则在未提交工作区上极易误伤实现。建议写入 skill 的测试加固章节。
4. **CI 门禁盲区核查应前置**：本波发现仓库 CI 不跑 e2e，导致「只有 e2e 能红」的分支在 CI 中无门禁。建议 skill 在 Phase 1 契约冻结时增加一步「CI 覆盖核查」：对每条新断言标注其运行层级，并确认该层级在 CI 中被执行。
5. **同族缺陷排查应成为契约冻结的固定动作**：wave-12 的复审主动 grep 同族调用点，直接发现冷启动路径。建议 skill 在契约模板中加入「同族入口枚举」小节（列出同类触发路径并逐条判定是否在范围内）。

## 5. 终态

- HEAD `cb12358`（+ `9bd2964` 文档）：`typecheck` / `test:unit`(78 files·828 passed·10 skipped) / `build` / 两条 e2e（session-fallback、session-fallback-cold-start）全部 **EXIT=0**。
- 交付：`src/stores/runtime.ts`（`enterFreshSession` + 冷启动落点）、`src/lib/runtime/sessions.ts`（`pickPreferredSessionId` / `hasSelectableSession` / `deriveInitStrategy`）、ADR [0034](../../docs/decisions/0034-fresh-session-after-delete-or-archive.md)、契约 `.dev-team/contracts/wave-12-session-fallback.md`。

---

# Wave-16 元复盘（Ask 控制请求消费冲突与终态重复列出修复）

> 复盘对象：本次 Agent Team 的**协作元框架与组织协议**。账本：`.dev-team/state-wave16-ask-cas.json`、契约 `.dev-team/contracts/wave-16-ask-cas.md`、收据 `.dev-team/wave16-{red,green}-receipts.md`。

## 1. 通信拓扑与信噪比

| 要素 | 实际 | 评价 |
| :-- | :-- | :-- |
| 名册 | Lead + test-agent + executor-core + executor-ui(已退役)/executor-ui2（5/8） | 合适：Rust 列表谓词收窄 + 前端 store 提交时钟与自动对账，责任清晰 |
| 看板 | task-1（红相验收）/task-2（core 修复）/task-3（ui 修复） | 干净：全部 completed，无 in_progress 残留 |
| 独立审查 | 2 轮对抗审查（首轮 2 视角均 FAIL 暴露出 4 条实质缺陷；二轮针对单飞断言与早退提示收敛并通过） | 极高价值：精准拦截了「过期无法对账卡死」与「断言未约束 SUT」 |
| 关键增益 | 审查 A/B 发现「生产卡片无过期触发入口」「快照时钟导致过期漏判」→ 推动 C2-6~C2-10 契约完善 | 避免只修一半（消除终态重列后仍会因 TTL 报同一错误） |
| 信噪比 | executor-ui 因上下文耗尽未交付，executor-ui2 接手后高精度落地；test-agent 精确配合四轮断言加固 | 高：无互相扯皮推诿，数据与收据机器可核验 |

## 2. 门禁穿透与误杀率

- **红相真实性**：首轮红相 Rust 6 failed / L2-AT 4 failed / vitest 4 failed。红相 commit `fc15414` 物理锚定后 Executor 始写代码。
- **假测试拦截**：审查 B2 成功揪出 C2-9 用例中因 `mockExpiredAskHost` 提前清空快照导致测试虽绿却未覆盖 SUT 单飞守卫的假断言问题（变异检验确认删除守卫仍绿），经改用快照直注后断言恢复真实验证力。
- **误杀率**：4 条裁决前失败经 Lead 复核确认为「用例机制与后置新增契约的夹具冲突」，判定为用例缺陷重写而非实现缺陷，免于熔断扣分。
- **回退/篡改防护**：验收测试冻结后实施者只读无写入权；test-agent 修订测试经 `git diff fc15414` 严格审计证实删除 expect 行为 0，断言强度只增不减。

## 3. 分工契约与隔离有效性

- **跨会话工作区 Hunk 定向隔离**：面对工作区同时存在并行 wave-15（token 指标）未提交 WIP（`WorkspaceComposer.vue` 等），严格杜绝 `git add .`，全流程定向暂存。
- **写域正交**：executor-core（`ask_control.rs`）与 executor-ui2（`ask.ts`/`AskPanel.vue`）写域互不干扰，零并发冲突。
- **遗留项规范收口**：发现 `WorkspaceComposer.vue` 生产卡片无法本波次修改后，坚守抗熵增底线不强行跨会话篡改，规范登记为 `PA-120`。

## 4. 元协议迭代建议

1. **并行多会话波次编号预检**：本波次初始编号为 wave-15，与并行会话发生重合，后紧急自愈更名为 wave-16。建议 skill 规定：每次启动波次前必须以 `git log` 与 `state.json` 的最大波次编号 +1。
2. **上下文容量熔断时一键接手模板**：executor-ui 在长思考中消耗完 context 后，生成具体清晰的规格 prompt 交给 executor-ui2，executor-ui2 一轮即达成全部代码落地。建议在 dev-team skill 中标准化「Executor 耗尽替换」操作指引。
3. **复合契约下夹具前置隔离律**：当后续契约（如 C2-6 自动对账）会改变前置状态时，早期动作测试（如 C2-1 点击测试）应优先使用直接状态注入而非依赖全链路 refresh，避免夹具副作用冲突。

## 5. 终态

- 门禁全绿：`ask_pending_contract` (6 passed) / `ask_pending_adversarial` (4 passed) / `ask_user_acceptance` (9 passed) / `ask_control lib` (9 passed) / `vitest 5 spec files` (49 passed) / `typecheck` (0 error) / `15 次防 flaky 连跑` (0 failures)。
- 交付成果：
  - `crates/pony-agent-core/src/agent/ask_control.rs`（收窄 `filter_interaction_requests` 仅返回 `Pending` 状态）
  - `src/stores/ask.ts`（提交时刻过期判定、单飞无递归自动对账、轮询自愈三态管理、在飞乐观状态保护）
  - `src/components/AskPanel.vue`（实时渲染时钟与过期锁定）
  - 契约与收据：`.dev-team/contracts/wave-16-ask-cas.md`、`.dev-team/wave16-green-receipts.md`、`PA-120 / PA-121` Backlog 登记。

---

# Wave-15 元复盘（Token 指标口径：输入上下文单次调用化 + 生成速度调用级即时化）

> 完整版：`.dev-team/meta-retrospective-wave15.md`。账本 `.dev-team/state.json`（wave 15）、契约 `.dev-team/contracts/wave-15-token-metrics.md`、收据 `.dev-team/wave15-green-receipts.md`。

## 1. 通信拓扑与信噪比

- 名册 Lead + Protocol + Test + Executor-Rust + Executor-Frontend（5 常驻 ≤ 8）；侦察 3 路与审查 3 路（L3-A/L3-B/L3-P）全部走 Subagent 计算池，未占常驻槽位。
- 看板 7 任务全部 completed。3 路只读侦察在**任何代码写入前**以 `file:line + 原文引用` 锁定 5 条根因（RC1–RC5），是本波最大增益。
- 噪声：Lead 先自行深读约 10 次才派生侦察，存在重复侦察；自动续轮 + 3 次模型切换拉长会话（改进项见 §4-⑤）。

## 2. 门禁穿透与误杀率

- 红相真实：`cargo test --lib wave15_multihop` Exit 101、`vitest run tests/acceptance/wave15` Exit 1（17 failed/3 passed），纯度分类 13 例 SUT 空桩 + 4 例 SUT 断言、基建失败 0。
- **本波最高价值事件（假绿拦截）**：executor-rust 首版 hop_complete 写成 `records[..len-1]`（滞后一跳），与当时冻结用例的错误映射同向 → 套件 20/20 全绿。Test Agent 主动上报 P2 异常，Lead 裁定不变量 I1（`Σrecords == payload 累加`）并重锚红相，再修实现转绿。**结论：当测试与实现共享同一误读时，绿相会穿透门禁**，须加阴性对照（§4-①）。
- 用例自缺陷 2 处（A4 夹具互斥、wire R1 基线自指）由 Executor 上报、Lead 复核、Test Agent 以 TEST-DEFECT 修正，pre-fix 复跑证明红相纯度未损（14 failed/14）。
- 审查 3 路（含 L3-P 五问否决制）全 PASS、0 findings；误杀 0，但假阴性率未测。NFR 欠账 0；债务 `wave15-D1` / `wave15-P3` 显式登记。

## 3. 分工契约与隔离有效性

- 双 Executor 写域完全正交（Rust runtime vs 前端投影/store/组件）；`tests.rs` 由 Test Agent 独占。
- R2 采用「保留旧签名 + 新增 `*_with_provider_calls` 薄委托」，零跨域编译阻塞、legacy 行为与既有断言零变化。
- 唯一越界 `src/lib/runtime/types.ts`（仅类型字段）经 Lead 核准为衍生豁免；根因是 Lead 任务卡路径笔误。
- 并发多会话共享工作区（wave-16 ask-cas 同树写入）：坚持定向 `git add`、零全局 git 写操作；全量套件唯一失败被准确归因到对方未提交的 `HomeWorkspace.vue`。
- 契约 3 次演进（§7.6 / §7.10 / ADR 0035）全部由实证驱动，无漂移式改约。

## 4. 元协议迭代建议

1. **绿相阴性对照（mutation negative control）**：Join Barrier 前对 SUT 施加一次单行语义突变，冻结套件必须转红，否则判定门禁失效。可机械化，成本 ≈1 次测试运行。
2. **指标/事件载荷契约必填「集合一致性不变量」**：如 `Σ per-item == aggregate`，仅写"应携带"无法约束实现顺序。
3. **`TEST-DEFECT` 上报通道正式化**：Executor 发现冻结用例矛盾必须停手上报，严禁改测或降级断言。
4. **波次编号与写域预检前置**：开局以 git log + state.json 最大编号 +1，并扫描未提交改动识别并发写域。
5. **Lead 侦察预算纪律**：先派生只读侦察拿全景，Lead 只复核 crux 行。
6. **审查灵敏度抽检**：0 findings 时做一次低成本阴性对照，区分"真干净"与"审查过松"。

## 5. 终态

- 红相锚定 `f38b6b9`（tag `wave-15-red`）→ 重锚 `3475e53`；绿相 `5901ef4`（tag `wave-15`）。
- 门禁：wave15 Rust 0 / 既有 Rust 回归 0 / `cargo check` 0 / `vue-tsc --noEmit` 0 / wave15 vitest 0（20 passed）/ 目标回归 0（46 passed,2 skipped）/ 全量 vitest 865 passed（唯一失败归因并发波次）/ 真实 Chrome 烟测 Console Error=0 + 2 截图。
- 交付：`turn_stream.rs`(R1×2) / `trace_timeline.rs`(R2) / `turn_persist.rs` / `turn_sync.rs` / `token-metrics.ts`(S1/S3/S4) / `useTraceProjection.ts`(R3) / `stores/runtime.ts`(R4) / `HomeSidebar.vue` / `HomeStatusPanel.vue` / ADR 0035 / `docs/release/2026-10-16-wave15-token-metrics/`。

---

# Wave-17 元复盘（IME 组合态回车守卫 + 动态数字位宽稳定）

> 复盘对象：本次 Agent Team 的**协作元框架与组织协议**。账本：`.dev-team/state-wave17.json`、契约 `.dev-team/contracts/wave-17-ime-and-numeric-jitter.md`、收据 `.dev-team/wave17-{red,green}-receipts.md` 与 `.dev-team/wave17-red-receipts-l2at.md`。
> 名册：Lead + protocol-agent + test-agent + executor-a + executor-b（5/8）；Subagent 池派发 7 次（方案对抗 3 + L2-AT 1 + L3 互补 2 + L3-P 1）。
> 熔断事件 0；NFR-DEGRADED 欠账 0。

## 1. 通信拓扑与信噪比

| 要素 | 实际 | 评价 |
| :-- | :-- | :-- |
| 拓扑公理映射 | 有状态（Protocol/Test/2×Executor）→ Teammate 上板；无状态封闭求值（方案对抗 3 + L2-AT 1 + L3 2 + L3-P 1）→ Subagent | **零失误**：7 路 Subagent 未占常驻槽位，名册 5/8 |
| 零废话协议 | 7 路 Subagent 全部返回严格 JSON，机器 Schema 校验 `SCHEMA_ALL_OK=true` | **优**：零句吹捧；L3-B 每条 finding 附可复现探针（如 `PROBE-B {"inlineGuard":true,"extractedGuard":false}`），Lead 无需追问 |
| 方案评审前置收益 | 3 路对抗评审把 6 条建议砍到 2 类缺陷，**否决 9 个子项**（4 项已实现 / 1 项跨层 / 2 项大范围重构 / 1 项非功能） | **本波最大收益**：避免约 19 文件 / 96 处颜色字面量的无收益重构 |
| 结构性浪费① | 方案评审 C 路（状态机与风险）与 A 路结论高度重叠（都判「已实现」） | 3 路 → 有效独立结论 2.3 路；视角按「建议条目」而非「技术面」切分导致重叠 |
| 结构性浪费② | L3-B 3 条 finding 全部裁决「接受不改」 | 3 findings / 0 修复；但 2 条转化为 backlog PA-135/136，属债务显式化而非纯噪声 |
| 结构性浪费③ | L2-AT Subagent **无 `team_task_*` 工具**，无法 claim/complete | Lead 代收口 1 次（见 §4.3） |
| Token 产出比 | `src/` 净产出 101 行；过程产物 ≈ 20 倍 | 对「跨模块契约 + 用户可见行为」可接受（产出 1 篇 ADR + 7 条 backlog）；同类缺陷反复出现应走 A 级快速路径 |

## 2. 门禁穿透与误杀率

**核心结论：独立测试分权制真实生效——3 次拦截全部指向 Lead 或 Test Agent 自己的产出，0 条实现缺陷。**

| # | 拦截者 | 被拦截对象 | 机械证据 | 性质 |
| :-- | :-- | :-- | :-- | :-- |
| 1 | **Test Agent** | **Lead 编写的契约 C2-2** | 原口径「9→12→1234 宽度变化 ≤0.5px」物理不可满足：基线 Δ=17px，注入 `tabular-nums` 后 Δ=21px；等位数异字形才可归零（Δ=4px→0） | 契约缺陷 **REV-1** |
| 2 | **Executor A** | **Test Agent 冻结的 L2-AT 用例** | `toBe(rawObject)` 对 Pinia reactive Proxy 恒假；仓库外探针 `{"activeAskIsProxy":true,"toRawActiveAskIsRaw":true,"deepEqual":true}` | 测试缺陷 **REV-2** |
| 3 | **Lead（目视）** | **Test Agent 的 C3-3 物理凭据** | 截图实为启动闪屏；原断言仅「文件存在且 >10KB」，**无法证伪拍到闪屏** | 测试缺陷 **REV-3** |

- **假绿风险与被堵住的具体形态**：
  - REV-3 是方法论级发现——`existsSync + size>10KB` 形式的「物理凭据」可被**完全无关的画面**满足；Playwright `toBeVisible()` 对**被全屏遮罩覆盖**的元素仍判可见。修复不是加断言数量，而是引入**遮挡命中证伪器**（`document.elementFromPoint()`）：闪屏在场时命中 `div#app-loading-screen`（会真红），移除后命中 `textarea`。
  - REV-2 的隐蔽性——该断言在红相时被前置的 `toHaveBeenCalledTimes(100)` 失败掩盖，**从未执行**。红相收据「12 条断言失败」看似健康，实则掩盖一条恒假断言。→ **红相失败会掩盖同用例后续断言缺陷**，绿相必须重审可证伪性，而非只看「全绿」。
- **红相未被测试修订污染的机械证明**：Lead 用 `git worktree add --detach /tmp/w17-redverify wave-17-red-anchor` 拉出红相工作树，放入 REV-2 修订后的 spec 复跑 → `12 failed / 2 passed, EXIT=1`。**把「测试修订是否削弱红相」从主观判断升级为机械事实**，是本波最关键的防腐化动作；工作树已 `remove --force` 回收。
- **假红/误杀率**：L3-A 0 findings、L3-P 0 findings、L3-B 3 条全 NON-BLOCKING 且裁决「接受不改」→ 误杀率 0，零修复产出率 3/3。
- **blast-radius 触发分析**：union diff 101 行（超阈值 1 行）即触发「≥2 路互补 L3 + 1 路 L3-P」。对纯前端 class/事件绑定成本偏高，但 L3-A 的真实 Chromium 探针（`@keydown.enter.prevent` → `{enter:1, click:0}`）是排除「双提交」风险的**唯一机械证据**，不可取消 → **阈值保持不动**，改为允许审查强度分级（§4.6）。
- **存量基线缺陷暴露**：`npm run test:e2e` 在基线即 **8 failed / 12 passed**（全新构建后复现，`session-sidebar-session-list` 漂移）。规范 e2e 门禁长期红灯，任何依赖它的波次被迫降级。登记 PA-134。→ 教训：**基线必须先跑一遍规范门禁并登记既有失败**。

## 3. 分工契约与隔离有效性

- **写域隔离机械审计通过**：越界文件 **0**。task-3（C1，4 文件）与 task-4（C2，3 文件）文件集交集为空，k=2 并行零冲突。
- **`src/lib/ime.ts` 跨任务共享被显式串行化**：task-1（Protocol 落空桩 + 冻结签名）→ task-3（Executor 实现）。看板自动报 `writeScopeWarnings: ["write scopes overlap with task-1"]`，Lead 据此把 task-1 补入 task-3 的 `blocked_by`，**未静默降级**——「advisory 写域 + 显式串行化声明」的正确用法。
- **`tests/**` 与 `src/**` 硬分权保持**：Test Agent 全程未写 `src/`，Executor 全程未写 `tests/`。
- **并行 ROI**：收益（两正交缺陷类窗口重叠，省约一个执行周期）略大于成本（两份全量 vitest 并发、`dist/` 共享产物竞争）。若缺陷类只有 1 个或共享文件，应降级 WIP=1。
- **跨会话共享工作区风险真实发生（3 类）**：
  1. **我的 `.gitignore` 编辑被吸入他人提交**——`git log -S"open-claude-ui/" -- .gitignore` 证实该改动最终出现在并行会话提交 `5888d36` 中。结果无害但说明「未上板而行文件写入 = 非法竞争」在共享工作区下判据过松。
  2. **8 个 `.dev-team/evidence/*` 被并行会话持续改写**，Phase 4「`git status --porcelain` 必须为空」**物理不可满足**。处置：全程定向 `git add <显式路径>` + 每次提交前 `git diff --cached --name-only` 机械核验「并行会话文件数 = 0」（2 次提交实测均为 0），门禁降级记录 `[HYGIENE DEGRADED: PARALLEL-SESSION ARTIFACTS]`。
  3. **`dist/` 是共享产物**：`playwright.config.ts` 的 `webServer.command = npm run preview:e2e` **只 preview 不构建**，裸跑 `npx playwright test` 会测到过期产物 → **假绿**。Lead 向 Executor B 发出更正，统一为 `npm run build && npx playwright test <file>`。
- **契约修订全部走同一入口**：REV-1（锚定前澄清）/ REV-2、REV-3（锚定后测试缺陷修订）均记入契约 §6，含触发、根因、裁决、影响面、熔断计数归属，**无「改测试不改记录」的暗改**。
- **信息过度下发控制**：Executor 只拿到自身 3~4 个文件路径 + 契约对应小节 + **测试的结构选择器约束**（后者至关重要，见 §4.4），未拿到契约全文与方案评审过程。

## 4. 元协议迭代建议

1. **【高优先】skill 声明的机械校验脚本未随 skill 分发**：SKILL.md 要求运行 `dev-stub-lint.py` / `dev-nfr-lint.py` / `dev-red-purity.py` / `dev-docs-lint.py`，并规定「脚本缺失时严禁人工目检放行并先安装」，但 `/home/dm/.agents/skills/dev-team/` **只有 SKILL.md**，且无安装路径。本波 Protocol Agent 自建 `validate-nfr-baseline.mjs`（含 8 组反例 `--self-test`）与 `validate-stub-purity.mjs` 作等价门禁。建议：随 skill 分发脚本，**或**在 SKILL.md 明确「等价自建必须含反例自检以证明门禁非恒真」并强制标注 `[GATE DEGRADED: TOOL-MISSING → INLINE-EQUIVALENT]`。
2. **【高优先】NFR 基准必须默认落旁路文件，禁止覆盖既有基准**：本项目 `.dev-team/nfr-baseline.json` 是 PA-118 冻结的 16 键基准，被项目自有 `nfr-lint.py`、`docs/release/2026-10-14-pa118-deadlock-fix/`、ADR 0024 与两个 Rust 死锁测试注释共同引用。Protocol Agent 主动升级冲突，Lead 裁定落 `nfr-baseline-wave17.json` 并机械证明既有基准零字节改动 + 项目 16 键 lint 回归 `EXIT=0`。建议 SKILL.md 改为：**若基准已存在则默认落 `nfr-baseline-<wave>.json`**，收据须同时跑「项目既有 lint 回归」与「本波校验」。
3. **【中优先】Subagent 池成员可能无任务板工具，Lead 必须代收口**：L2-AT Subagent 回报「本会话无 `team_task_get` / 任务板工具」，无法 claim/complete。建议 SKILL.md 补充「凡下沉至 Subagent 池的任务，Lead 负责派发前 claim、收到回执后 complete」，并在派发 prompt 中显式告知。
4. **【高优先】UI 物理凭据必须含证伪断言，文件存在性不算凭据**：REV-3 教训。建议在「前端真实链路与环境验收门禁」增加硬性条款——截图凭据必须含至少一条**遮挡/视口证伪断言**（`document.elementFromPoint()` 命中目标自身或后代，或 `toBeInViewport()`）；**禁止**仅以「文件存在 + 大小阈值」作凭据；应用存在启动遮罩时必须显式等待其从 DOM 移除，且**先断言其曾经存在**（否则「等待消失」在遮罩从不出现的环境下是恒真空等待）。
5. **【高优先】红相失败会掩盖同用例后续断言缺陷 → 绿相必须重审可证伪性**：REV-2 教训。建议绿相门禁增加独立动作——「对每条红相失败用例，核查其后续未执行断言在绿相下是否**真能证伪**（至少 1 条负向/诱饵自检）；仅当全用例具备证伪能力时才允许绿相收口」。
6. **【中优先】blast-radius 触发后的审查强度可分级，但不得取消**：101 行纯前端 diff 触发 3 次 Subagent 审查（成本偏高），但 L3-A 探针不可取消。建议：diff **100% 为 CSS class / 纯谓词 / 模板事件绑定**且零状态机/IO/持久化时，L3-B 降级为固定 5 项 checklist，L3-A 与 L3-P 保持全量；**禁止**以任何理由裁剪 L3-P。
7. **【中优先】前端 e2e 门禁命令必须显式包含构建步骤**：`webServer.command` 可能只 `vite preview` 不构建，裸跑即假绿。建议 SKILL.md 增加条款：前端 e2e 门禁命令必须显式含构建步骤且显式指定待跑文件，避免存量失败污染 EXIT 码。
8. **【中优先】Phase 4「工作区整洁度归零」需增加并行会话豁免通道**：`git status --porcelain` 全空在多会话共享工作区下物理不可满足。建议改为「**属于本波 write_scopes 的文件必须为空**；非本波文件须逐条给出归属证据（mtime 早于本波首次写入 / 未被任何本波任务 write_scopes 覆盖）并标注 `[HYGIENE DEGRADED: PARALLEL-SESSION ARTIFACTS]`」，同时把「提交前核验 staged 清单中并行会话文件数 = 0」列为强制动作。
9. **【中优先】冻结测试的结构选择器约束必须随任务书下发给 Executor**：本波 `tabular-nums-contract.spec.ts` 依赖「恰好 3 个 / 恰好 4 个 `span.inline-flex`」等结构锚点，Lead 在 Executor B 任务书中显式列出这些约束（禁止新增同级 `span.inline-flex`、禁止改 `div.flex.items-center.justify-around` 的 class 组合），使 C2 一次通过。建议 SKILL.md 在「冻结测试 → Executor」交接清单中固定一行「测试的结构锚点与不可破坏的 class 组合」。

## 5. 终态

- 红相锚定 `f060c8d`（tag `wave-17-red-anchor`）→ 绿相检查点 `49fd118`（tag `wave-17`）。
- 门禁：`vue-tsc --noEmit` 0 / wave17 冻结验收 31 passed 0 / 全量单测 897 passed（85 files，基线 866）/ 真实 Chromium 门禁 4 passed 且 Console Error=0 / `version:check` 0 / 本波 NFR 校验 0 / 项目 16 键 NFR lint 回归 0 / flake 复跑 3×0。
- 交付：`src/lib/ime.ts`（新增接缝）+ `WorkspaceComposer.vue` / `HomeSessionSidebarRow.vue` / `HomeSessionSidebar.vue`（C1 六处入口）+ `HomeStatusPanel.vue` / `HomeTracePanel.vue` / `AskPanel.vue`（C2 八位点）；ADR [0036](../../docs/decisions/0036-ime-composition-enter-guard-and-numeric-width-stability.md)；`docs/release/2026-10-16-wave17-ime-guard-numeric-jitter/NFR-COMPLIANCE.md`。
- 债务显式化：PA-130 ~ PA-136（含存量 e2e 基线红灯 PA-134、存量守卫粒度缺陷 PA-135）。
