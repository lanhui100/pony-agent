# 2026-06-04 Session 74

## 主题

- 启动并收口 `PA-037` 的前端 session 控制交互面
- 把既有 session/runtime/history 合同消费成可见、可验收的 UX

## 本轮完成

1. 已把 `HomeWorkspace.vue` 的主 CTA 收口为显式 session 控制入口：
   - 运行中显示 `停止`
   - paused recovery 显示 `恢复`
   - continue plan 显示 `继续`
   - replay-required / lifecycle-boundary 显示 `重新开始`
2. 已在 `HomeWorkspace.vue` 增加控制反馈：
   - stop 请求发出后显示“等待安全边界暂停”
   - cancelled / resume / replay 场景显示明确控制提示
3. 已把 `HomeSessionSidebar.vue` 收口为统一状态语言：
   - `live / historical / historical_dirty` 不再裸露给用户
   - 新增 session control 状态卡，对 paused / recovery-capable / replay-required 给出用户可读标签
   - 新增 history action disabled reason
4. 已补 history result feedback：
   - checkout degrade -> “仅恢复对话，未恢复工作区”
   - restore / fork / branch switch -> 明确反馈 branch / visible node / mode 变化
5. 已补前端验收测试：
   - `HomeSessionSidebar.spec.ts`
     - 覆盖状态语言、disabled reason、degrade feedback、restore/fork/switch 成功反馈
   - `HomeWorkspace.spec.ts`
     - 覆盖 stop / resume / replay CTA 展示与派发
   - `runtime-store.spec.ts`
     - 复跑底层 `stop / resume / continue / replay` 派发合同，确认未回归
6. 已完成 `PA-037` acceptance audit：
   - [2026-06-04-pa037-acceptance-audit.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-04-pa037-acceptance-audit.md)
7. 已回写：
   - `PA-037` 任务卡
   - `PA-037` OpenSpec tasks
   - 任务板状态

## 验证

- `npx vitest run tests/HomeSessionSidebar.spec.ts --pool=forks --maxWorkers=1`
  - 结果：`24 passed`
- `npx vitest run tests/HomeWorkspace.spec.ts --pool=forks --maxWorkers=1`
  - 结果：`18 passed`
- `npx vitest run tests/runtime-store.spec.ts --pool=forks --maxWorkers=1`
  - 结果：`55 passed`

## 当前判断

- `PA-037` 已从“能力已存在但前端不可验收”推进到“控制面、状态语言、结果反馈与验收测试齐备”
- 本卡可以进入 `Review`
- 下一条主线应切回 `PA-035`，继续推进 hooks stable-boundary integration，而不是在 `PA-037` 内继续堆前端补丁
