# PA-022 建立 lifecycle hooks pipeline

## 状态
- Status: `Backlog`
- Priority: `P3`
- Owner: `Codex`

## 目标
在 graph run lifecycle、host control plane、memory/planner/capability bridge 都有稳定边界之后，引入 hooks pipeline 作为横切机制，让 turn、run、tool、checkpoint、memory write 等关键节点可被统一拦截、增强和审计，而不把这些逻辑散落进 runtime、graph 或宿主实现里。

## 输出
- hook lifecycle 第一版定义
- `before/after turn`、`before/after run`、`before/after tool`、`before/after checkpoint` 等 hook 点
- hook registration / ordering / failure policy
- hook 与 planner / memory / skills / MCP 的边界说明
- hook observability / audit 字段

## 验收标准
- hooks 被定义为横切机制层，而不是新的调度层
- hooks 不要求宿主、adapter 或 UI 理解内部细节
- hook failure policy 明确，不会破坏 runtime / graph 的核心收口语义
- 常见扩展点可通过统一 lifecycle 接入，而不是直接修改主流程
- 文档明确本卡不要求一次性实现复杂插件生态

## 当前进展
- 当前 turn / tool / session / checkpoint 生命周期已经有雏形，但还没有统一 hook pipeline
- `PA-014` 将先稳定 run/goal 级 lifecycle
- `PA-018 ~ PA-021` 将先把 context/state retrieval、planner、MCP、skills 的边界固定下来
- `PA-033` 已把其中 turn-lifecycle foundation 范围单独拆成可执行卡；本卡保留为 foundation 之后的更大 hooks 扩展入口
- 当前已进一步拆成下一轮可执行卡：
  - `PA-038`：run hooks 与 execution-control boundary
  - `PA-039`：memory-write hooks 与 persisted side-effect contract
  - `PA-040`：planner 与 capability-mediation hooks
- `PA-038 / PA-039 / PA-040` 现已全部完成 closeout，并已完成 OpenSpec spec 同步与 archive：
  - `openspec/specs/` 已新增对应 canonical spec
  - `openspec/changes/archive/2026-06-05-*` 已完成归档
- 当前 hooks 主线已完成第一轮 post-foundation 扩展收口：
  - turn foundation：`PA-033 / PA-035`
  - run / execution-control：`PA-038`
  - memory write / persisted side effect：`PA-039`
  - planner / capability mediation：`PA-040`

## 与 PA-033 的关系
- `PA-033` 只覆盖 turn lifecycle foundation
- 本卡仍保留 `run / memory write / planner / skills / MCP` 等 post-foundation 扩展范围
- `PA-033` 完成不自动关闭本卡；后续需根据 foundation 交付结果重写或拆分本卡

## 下一步动作
- 保持本卡作为 post-foundation hooks 总入口与分流说明，不直接承载实现
- 仅当出现新的稳定 lifecycle boundary 且未被 `PA-033 / PA-035 / PA-038 / PA-039 / PA-040` 覆盖时，再拆新的独立 change
- 在没有新稳定边界之前，不新增第四张近线 hooks 主卡，避免重新回到模糊大卡

## 当前卡点
- 当前近线底座已基本稳定；本卡的主要工作不再是补实现，而是继续防止后续需求把 hooks 范围重新混回一张大卡

## 断点续跑提示
继续前先看：
- `management/task-system/03_TASKS/PA-014-add-graph-stop-resume-and-checkpoint-matrix.md`
- `management/task-system/03_TASKS/PA-018-build-context-state-subsystem-and-retrieval-boundary.md`
- `management/task-system/03_TASKS/PA-019-build-graph-planner-and-decision-policy.md`
- `management/task-system/03_TASKS/PA-020-build-mcp-capability-bridge.md`
- `management/task-system/03_TASKS/PA-021-build-skills-registry-and-bridge.md`
- `docs/architecture/overview.md`
