# 元框架复盘：submission_watchdog_timeout 根因修复（wave-2）

## 1. 通信拓扑与信噪比

- 团队拓扑：Lead（侦察/裁决/验证） + Protocol Agent（契约冻结） + Test Agent（红相验收） + 2 路 Executor（前端/后端正交写域） + 1 路 L3 对抗审查（视角 A：并发）。
- 契约先行：`.dev-team/contract-watchdog-fix.json` 冻结三根因（RC1 watchdog 不续期误杀、RC2 前端不协同后端落盘、RC3 assistant 缺失丢错误）与前后端契约（rearm 语义、timeout 240s、新命令 fail_turn_for_watchdog 行为），有效避免 Executor 实现漂移。
- 信噪比良好：teammate 输出均为结构化结论 + 行号证据；L3 输出符合 JSON verdict/findings schema。

## 2. 门禁穿透与误杀率

- 红相门禁真实拦截：T1（续期不误杀）/T3（assistant 缺失兜底）/T4（后端命令调用）三用例在一次性计时器实现下红相失败，与三根因一一对应；T2 作为静默超时回归护栏保持绿，未制造借口性红。
- L3 对抗审查捕获 2 项合入前必须修复项（O1 终态覆盖竞态、O2 session 权威优先），已在合入前修复并补 2 个单测；另 3 项观察项（历史模式 rearm 盲区、锁竞争延迟、stop 幂等冗余）评估为已知边界。
- 机器门禁：cargo check（core+src-tauri）、cargo test tauri 全量 exit 0、core lib control_plane 66 passed、前端 vitest 661 passed / runtime-store 117 passed、tsc 无接触文件新增错误。

## 3. 分工契约与隔离有效性

- 写域正交：前端 Executor 限 src/stores/runtime.ts；后端 Executor 限 crates/pony-agent-core + src-tauri；Test Agent 限 tests/；Protocol 仅写契约 JSON。diff 审计无越界（Cargo.lock 仅版本号同步，属编译衍生豁免）。
- Test Agent 只读不写业务代码，红相用例落盘于 `tests/runtime-store.spec.ts` red-phase describe 块，符合独立分权。
- L3 审查基于真实 diff 逐项验证，未发现死锁/永不触发路径；O1/O2 修复后复测通过。

## 4. 元协议迭代建议

- 新增跨前后端契约类修复时，Protocol Agent 应同时冻结"边界竞态"防回归清单（如 O1 终态覆盖、O2 session 权威），使 L3 审查聚焦最高风险点。
- rearm/watchdog 类静默超时语义应与后端最大阻塞窗口（provider 180s+连接 15s）显式对齐并在契约中记录推导，防止再次出现阈值错配。
- L3 输出项应分级（合入前必须修 / 已知边界），当前已实践，建议固化为模板字段。