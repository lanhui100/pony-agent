# PA-114: ask_user 工具与前端对应组件

## 1. 状态
- **阶段**: Review（实现与机器门禁全绿，对抗审查进行中）
- **优先级**: P1
- **复杂度**: B（跨模块：core 工具注册 / 受管执行判定 / 前端组件 + 测试）

## 2. 目标
补齐模型可见的 `ask_user` 工具（一等公民 builtin）与聊天/轨迹内的专属交互卡片：
- 后端：`ask_user` 注册进 builtin catalog（schema: question required + options/defaultAnswer/timeoutMs 可选 + description），产品名 "Ask" 的模型面 winner 从 legacy `echo_input` 占位符移交给 `ask_user`；`LegacyCompatiblePolicyEvaluator` 对 ask_user 返回 WaitingHost，复用既有 Interaction pending-request 控制平面（PA-076 phase-4）。
- 前端：`AskUserToolCallCard` 在 WorkspaceTurnItem 的 tools 行按 `isAskToolName` 路由渲染（问题 + options + 输入 + 回答/取消 + waiting 态），复用 `useAskStore` 的 answer/cancel/resumeGraphAsk 闭环。

## 3. 调研结论（实施前快照）
- 后端 Ask 控制平面已完整存在（dispatcher WaitingHost→Interaction PendingControlRequest、ask_control.rs、Tauri ask_* 命令、graph bind/resume、turn:suspended），缺口在工具本体：全项目 `ask_user` 0 命中，模型可见工具为 `echo_input` 占位符（schema 仅 {text}）。
- 前端 AskPanel（宿主侧待确认面板）已存在；缺聊天/轨迹内 ask 工具调用卡片。

## 4. 输出
- 后端：`crates/pony-agent-core/src/agent/tools.rs`（TOOL_ASK_USER 常量/定义/映射/权限声明，12 处编辑）、`governed_executor.rs`（WaitingHost 判定扩展）
- 前端：`src/lib/runtime/ask-tools.ts`、`src/components/ask/AskUserToolCallCard.vue`、`src/components/chat/WorkspaceTurnItem.vue`（tools 行路由）
- 测试：`crates/pony-agent-core/tests/ask_user_acceptance.rs`（B1~B3，9 用例）、`tests/acceptance/stage-1-ask-user.spec.ts`（F1，9 用例，红相冻结）、旧 characterization 测试契约升级（tools/governed_executor/control_plane/dispatcher_matrix）

## 5. 验收标准
- B1-1~B3-4 / F1-1~F1-4（见 `.dev-team/contract-matrix-ask-user.md`，§5 Lead 裁决：pending prompt 优先）
- 机器门禁：`cargo test -p pony-agent-core --test ask_user_acceptance` 9/9、`vitest run tests/acceptance/stage-1-ask-user.spec.ts` 10/10、`npm run build`、`cargo check --workspace`、`npm test` 全量 648 passed、相关 Rust 模块 0 failed

## 6. 当前进展
- ✅ 红相测试冻结（534bf94 + 同步 4728c1e）
- ✅ Executor 实施（tools.rs 内容随外部提交 77fee3d 保全；466d028 含 governed_executor + 3 前端文件）
- ✅ Test Agent 契约升级旧测试（08c28a8）
- ✅ 机器门禁全绿
- 🔄 对抗审查（2 路）进行中

## 7. 下一步
- 收集 2 路审查 verdict；致命发现则限 2 轮对抗调优
- 物理检查点提交 + `.dev-team/state.json` 更新
- 冒烟 + 收口（看板、归档、meta-retrospective）

## 8. 当前卡点
- 无功能阻塞。知悉事项：tools.rs 的 ask_user 内容被外部代理提交 77fee3d（windows batch 任务）以 git add 一并卷入（内容完整、归属错位，不重写共享历史）；全量 --lib 串行跑存在 7 个既存环境失败（path_permission×3、document_conversion×1、runtime hollow blank×2、sandbox 装配×1），基线 a8e9fd9 已复现、属 PA-080/46040ae/b8cba07 域，不在 PA-114 范围（契约矩阵 §5 裁决），收口报告如实记载。

## 9. 断点续跑提示
- 红相锚点：4728c1e；实施提交：466d028；契约升级：08c28a8
- 从"对抗审查收集 → 冒烟 → 收口"续跑即可。
