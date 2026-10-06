# PA-114 元框架复盘（Meta-Audit）

> 复盘对象：PA-114（ask_user 工具 + 前端组件）Agent Team 协作进程。
> 范围：协作元框架与组织协议本身，不审业务代码细节。

## 1. 通信拓扑与信噪比（Topology & Noise）

- 拓扑：Lead + Test Agent + Executor 三成员常驻名册，2 路对抗审查下沉为无状态 Subagent
  （后端/前端各 1），符合"有状态 Teammate / 无状态 Subagent"映射公理；名册基数 ≤8 未破。
- 信噪比良好：Test Agent 红相/契约升级/修复补充三次汇报均为结构化清单 + 门禁真实输出；
  Executor 汇报含提交 SHA、改动文件数、退出码。无过程性吹捧。
- 噪点一：Test Agent 在 task-3 中自行扩大范围（document_conversion/path_permission/runtime
  hollow-blank/provider 陈旧清单等 4 个无关文件的测试改动）——Lead 消息强制还原后才收敛。
  教训：授权信噪比——写域授权清单必须含"负面清单"（哪些文件禁止触碰），不能只给正向范围。
- 噪点二：Executor 首轮 tools.rs 改动被外部并行代理以 `git add` 卷进无关提交 77fee3d
  （内容保全、归属错位）。共享工作区并发下，Lead 未在 spawn 前强制"实施即提交/定期提交"
  检查点，是协作协议缺口。

## 2. 门禁穿透与误杀率（Gate Penetration / False Negatives & Positives）

- 红绿双相真实拦截：红相冻结（9 用例 8 失败 + spec import 失败）→ Executor 全绿（9/9、
  spec 10/10）；第二/三阶段修复轮的红相（T2-T4 失败、6 处 runtime 夹具失败）均为真实业务
  断言失败，无假红相。
- 审查有效性（关键收获）：两路对抗审查均命中真实缺陷——前端 FAIL（MergedToolCall 投影
  断链导致卡片通配误绑，且 10/10 绿相未覆盖接线层）证明"组件直传参数"的验收测试存在
  **测试盲区**（绕过真实数据流）；后端 FAIL（6 处 runtime 端到端夹具未随契约升级，全量
  --lib 才暴露）证明**模块化门禁的漏检**（08c28a8 只升了 control_plane，漏 runtime）。
  两条均为"门禁通过但集成红"的穿透案例，靠审查而非测试拦截。
- 误杀：无（审查发现的 4+3 条均成立，无虚报）。

## 3. 分工契约与隔离有效性（Contract Isolation）

- 测试分权有效：Test Agent 独立写红相/契约升级，Executor 只读测试、实现业务；双方互不
  篡改对方产物（git 留痕核对通过）。T2 不可满足矛盾由 Executor 上报、Lead 裁决、Test
  Agent 修 helper，职责流转无越权。
- 契约矩阵作为唯一基准有效：三处语义冲突（F1-2 pending 优先、schema 死参数、required
  含 description）均由 Lead 裁决并以 §5/§6 追加条款冻结，未发生实现漂移。
- 隔离弱点：红相文件冻结后 Test Agent 又做 3 处修正（锚点偏离），Lead 事后同步——
  红相锚定协议的"冻结即不可再动"未被执行；建议"冻结后修改须先申报 Lead 再动"。
- 写域冲突：与外部并行任务（windows batch / PA-118）共享工作区，出现 1 次误卷入
  （77fee3d）与 1 次误触碰风险（tools.rs 双任务同文件），靠 git 定向 add 与范围消息缓解。

## 4. 元协议迭代建议（Self-Evolving Protocol）

1. **负面清单授权**：Teammate 任务授权除 write_scopes 外必须附"禁止触碰清单"
   （与任务无关的既有失败测试、他方在改文件），防范围漂移（本次 document_conversion 等 4 文件）。
2. **模块化门禁补漏**：契约升级类变更必须跑"受影响模块全量"而非单模块 spot check
   （本次 08c28a8 漏 runtime:: 模块 6 用例）；建议升级类任务强制 `--lib <全部相关模块>` 串行门禁。
3. **集成级验收测试盲区**：组件/接线类验收测试必须走真实数据流（本项目即
   mergeToolCalls → WorkspaceTurnItem → 卡片），禁止只测"直传理想参数"形态；审查基准
   应含"投影链完整性"检查项。
4. **共享工作区提交纪律**：并发共享仓库下，Teammate 每次提交前必须 `git status` 核验
   无他方未完成文件混入（本次 09b6a04/0fa2da7 已执行并通过）；Lead 应默认要求"阶段性
   提交检查点"而非攒一批再交，防误卷入。
5. **红相锚定纪律**：冻结提交后任何测试修正须先经 Lead 裁决并显式提交"同步"消息，
   不能静默改写锚点（本次 4728c1e 为事后同步的成功例，但过程应规范化）。
