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

## 下一步动作
- 先枚举稳定可承诺的 lifecycle 节点
- 再决定 hooks 的注册、执行次序与失败策略
- 最后为未来扩展能力提供统一挂接点

## 当前卡点
- hooks 属于横切层，若在 graph/runtime/memory/planner 尚未稳定时过早引入，只会把原本应收束的主流程再次打散

## 断点续跑提示
继续前先看：
- `management/task-system/03_TASKS/PA-014-add-graph-stop-resume-and-checkpoint-matrix.md`
- `management/task-system/03_TASKS/PA-018-build-context-state-subsystem-and-retrieval-boundary.md`
- `management/task-system/03_TASKS/PA-019-build-graph-planner-and-decision-policy.md`
- `management/task-system/03_TASKS/PA-020-build-mcp-capability-bridge.md`
- `management/task-system/03_TASKS/PA-021-build-skills-registry-and-bridge.md`
- `docs/architecture/overview.md`
