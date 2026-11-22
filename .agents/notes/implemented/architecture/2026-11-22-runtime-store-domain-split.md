# Agent Note: runtime store 按域拆解为模块并保留兼容门面

Status: implemented

## Problem

`src/stores/runtime.ts` 是桌面端唯一的前端状态机：6075 行、84 个 state 键、18 个 getters、97 个 actions、
23 处 `invoke`、14 个消费方文件。它同时承载会话目录、工作区树、轮次执行与流式缓冲、消息转录、遥测指标、
历史分支、队列授权、健康与能力目录八类关注点；token 级 delta 这类高频路径与会话列表、健康探测这类
低频状态共享同一响应式对象。

三个具体后果：

1. **没有可声明的接口边界**：任何新增视图或面板只能整体依赖这个 store，无法表达「我只用 5 个字段」，
   这直接卡住后续「桌面工作台 / 可替换视图」的推进；
2. **订阅与卸载无法成对**：store 是常驻单例，turn 事件订阅集中在文件内部注册，视图卸载不释放订阅；
3. **协作与测试成本**：6075 行单文件是多会话并行的天然冲突源；97 个 action 无法分片测试，
   前端 107 个既有用例里没有一个直接覆盖这个 store 的状态机。

## Decision

1. **保留单一 Pinia store**（id `runtime`、导出名 `useRuntimeStore` 不变），把它退化为 78 行装配壳：
   `state` 来自 `src/stores/runtime/state.ts`，`getters`/`actions` 按域 spread 自 10 个模块；
   14 个消费方在本决策内**零改动**。
2. **8 个域模块按 `fact-map` 归属切分**：`session-catalog`(D1) / `workspace`(D2) / `turn-execution`(D3) /
   `messages`(D4) / `telemetry`(D5) / `history`(D6) / `queue`(D7) / `catalog`(D8)；跨域事务入口集中在
   `orchestration.ts`（21 条，含 `resetSessionRuntimeState`/`applySessionSnapshot`/`submitTurn`/`persistHistory`）；
   turn 事件注册体集中在 `turn-events.ts`（1785 行、14 个监听器）。
3. **搬迁方式为逐字搬运**：函数体、注释、wave/issue 编号、豁免标记、节流与计时常量一律不动；
   唯一允许的改动是 import 路径、`this: RuntimeStoreThis` 形参与两处跨模块可见性调整。
4. **行为等价由重构前冻结的 golden fixtures 证明**：15 个 fixtures / 23 条用例在旧实现上捕获，
   拆解后逐字节比对；另有 3 处变异自检证明用例能由绿转红；补一条独立核验：原 6075 行里
   115 个成员（97 actions + 18 getters）的成员体经两路独立抽取器比对全部 token 级一致
   （Lead 脚本覆盖 108 个方法式成员、0 缺失 0 mismatch 0 跨文件重复；6 个箭头式 getter 与 1 个泛型
   action 由 L3-A/L3-B 的独立抽取器覆盖，合计 115/115）。
5. **视图窄契约以类型形式就位**：`RuntimeStore` 门面类型拆成 8 个 slice 接口（`types.ts`，84 键 / 18 getters /
   97 actions，含键集冻结断言），未来视图或插件贡献点可只依赖所需切片；本决策不改运行时语义、不引入多 store。
6. **非目标**：不碰 14 个消费方、不改既有 107 个前端用例、不改 `src/lib/**`、不改 Rust；
   不顺手清理死分支（`backendGovernanceSummary` 读法）、只写不读键（`runningTurnId`）、
   三个恢复死路径（`loadPersistedRuntimeCache` 丢运行态映射、`buildRuntimeViewFromPersistedState` 恒
   `checkpoint:null`、`checkoutHistoryBranch` 不返回 `cursorVersion`）——它们原样保留并登记为独立后续事项。

## Alternatives considered

- **3 个 store（session+workspace+queue / turn / catalog）**：能立刻得到跨视图隔离，但 Pinia 语义从单 Proxy
  变为三 Proxy：`HomeWorkspace.vue` 的 15 个键跨两个 store、`useTraceProjection.ts` 的 20 个键必须全部落在
  turn 侧才不破，`lib/tauri.ts` 读 `activeTraceId` 会形成 `lib → store → lib` 环。落选原因：本波目标是
  「先有边界、行为不变」，把响应式语义变更塞进同一波会让等价证明成本翻倍。
- **8 个 per-domain store**：编译期隔离最彻底，但 97 个 action 全搬、21 个跨域入口需要显式事务编排，
  且 `conversationCheckpointEntries`/`agentRuntimeStatus`/`activeTraceId` 三个跨域 getter 要重组 computed。
  落选原因：真正缺的是「可声明的窄契约」，这个用类型层就能满足，不必付多 store 的语义代价。
- **保持单文件，只补窄类型**：改动最小，但 6075 行的定位成本、冲突面与测试不可分片问题原样保留，
  后续工作台化时仍要重做本次拆解。
- **改用 setup-store（composition API）重写**：更贴近组合式视图，但会改变 store 形态、HMR 行为与 `this`
  语义，14 个消费方与 107 个既有用例的等价证明面过大，且与「逐字搬运」纪律冲突。
- **边拆边清理死代码/只写不读键**：违反仓内「严禁夹带重构」；清理需要各自的影响面证据，
  已作为独立后续事项登记，不在本 ADR 内处理。

## Consequences

- 单文件 6075 行降为装配壳 78 行 + 契约 650 行 + 状态 165 行 + 10 个域模块 6150 行；域内改动可定位到具体文件。
- 窄契约（8 个 slice 接口）就位：未来视图可只依赖 `TurnExecutionSlice` 或 `MessagesSlice`，不再被迫整体依赖 84 键。
- 真耦合被显式化：`orchestration.ts`（2034 行）与 `turn-events.ts`（1785 行）承载全部跨域事务与事件注册，
  是 Wave 2（多 store 化 / 工作台贡献点）唯一需要重写的面；在此之前它们只是换了位置、语义未变。
- 已知契约缺口：`RuntimeStoreThis` 不含 Pinia 内建 `$patch`，`turn-events.ts` 用局部别名 `TurnEventsThis`
  补位；Wave 2 需把 `$patch/$reset/$subscribe` 纳入契约或明令禁止。
- 域归属依赖语义判断（`messageRevision` 归 D4 等）；误归不会在本波暴露，但会在 Wave 2 重写跨域事务时显形，
  故归属真源固化为 `.dev-team/runtime-split/fact-map.md` 并随代码修订。
- 装配壳依赖 spread + `ThisType` 推断，类型错误成本上升，`vue-tsc --noEmit` 必须常绿（CI 门禁保障）；
  且 10 个域对象以 `as XActions` 断言接入，只保证「齐全」不保证「唯一」——重名会 last-write-win，
  今日实测 0 重名，属潜在风险而非现存缺陷。
- 本波识别但**不修**的后续事项已入账本：B277（40 个未覆盖 action 的等价测试补强）、
  B278（`$patch/$reset/$subscribe` 纳入契约或明令禁止）、B279（三处定时器跨会话守卫缺口：
  trace 节流写入无 session 守卫、`hasCachedState` 切换不 `cancelStreamFlush`、`streamFlushFrameId` 为死码）。
