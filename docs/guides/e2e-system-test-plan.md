# Pony Agent 端到端系统测试方案

## 1. 文档目标

本文档定义 Pony Agent 当前阶段的全面端到端系统测试方案，覆盖：

- `Vue + Pinia` 前端工作台
- `Tauri command / event` 边界
- `Rust runtime / session / tool router / provider registry`
- 关键会话控制与审计面能力
- 构建、回归、发布前准入与失败处理机制

本文档不是单测清单，也不是某一个功能的验收用例，而是面向“系统级稳定交付”的统一测试方案。

本文档已基于 `opencode/minimax-m3-free` 独立审核意见完成一轮收敛，当前版本按“必须执行的系统测试规约”编写，而不是开放式建议列表。

## 2. 当前测试基线

当前仓库已经具备两层测试基线：

- 前端单元/组件测试：`tests/*.spec.ts`
- Rust 回归测试：`src-tauri/tests/session_regression.rs`、`provider_registry_regression.rs`、`tool_router_regression.rs`

当前可直接复用的校验命令：

- `npm run test:unit`
- `npm run test:ui-guard`
- `npm run build`
- `npm run cargo:check:shared`
- `npm run cargo:test:shared`
- `npm run cargo:test:regression`
- `npm run verify`

现状判断：

- 已有“前端局部回归”与“Rust 子系统回归”。
- 仍缺“从用户操作到 Tauri 边界到 Rust runtime 再回到 UI”的系统级闭环验证。
- 仍缺“真实流式、真实持久化、真实恢复、真实故障降级”的跨层场景矩阵。

## 3. 测试目标与非目标

### 3.1 测试目标

本方案要证明以下四件事：

1. Pony Agent 的核心主链路可稳定跑通。
2. 跨层状态在正常、异常、恢复三类场景下保持一致。
3. 关键审计面信息可被正确生成、展示、持久化和恢复。
4. 发布前存在可重复执行、可追责、可扩展的系统测试流程。

### 3.2 非目标

当前阶段不把以下内容纳入端到端系统测试主范围：

- 大规模性能压测
- 长时间 soak test
- 多操作系统并行认证
- 全量 provider 真连接的商业级联调
- 视觉像素级截图回归体系

这些能力可以作为后续扩展轨道，但不应阻塞当前系统测试主线落地。

## 4. 系统测试范围分层

### 4.1 L0：静态与快速回归层

目标：以最低成本快速挡住明显回归。

- `npm run test:unit`
- `npm run test:ui-guard`
- `npm run build`
- `npm run cargo:check:shared`
- `npm run cargo:test:regression`

定位：PR 级、提交级、快速预检。

### 4.2 L1：子系统集成层

目标：验证前端状态模型、Tauri 适配层、Rust runtime 子系统之间的契约一致性。

- 前端 store 与 Tauri mock 交互
- Rust session / provider / tool router 回归
- schema、payload、事件顺序、错误映射

定位：主分支合并前必须通过。

### 4.3 L2：桌面端到端系统层

目标：从用户视角验证真实桌面应用闭环。

- 启动应用
- 发起用户操作
- 触发 runtime
- 接收流式事件
- 展示消息/状态/审计面
- 持久化后重启恢复
- 异常后可解释降级
- 验证 IPC 前后事件序列一致

定位：本方案核心。

### 4.4 L3：发布候选验收层

目标：对发布候选版本做较慢但可信的发布前确认。

- 完整系统冒烟
- 关键链路矩阵
- 历史回放与恢复验证
- 手动探索性补充

定位：release candidate 准入。

## 5. 测试环境策略

### 5.1 环境分级

定义三套系统测试环境：

1. `E1 Mock Provider 环境`
2. `E2 Fake Stream / Fault Injection 环境`
3. `E3 Real Provider 验证环境`

三套环境必须使用独立的数据目录、独立日志目录和独立 provider profile，禁止共享会话持久化路径。

### 5.2 E1 Mock Provider 环境

用途：

- 主跑 CI
- 主跑稳定用例
- 保证可重复性

要求：

- 模型响应固定
- 工具返回固定
- 错误码固定
- 时间线可控

适用场景：

- 首轮对话
- 流式增量
- 工具成功/失败
- 会话持久化与恢复
- 审计面展示

### 5.3 E2 Fake Stream / Fault Injection 环境

用途：

- 验证异常、超时、乱序、半完成、恢复路径

要求：

- 可人为插入延迟
- 可人为中断 chunk
- 可人为返回 malformed payload
- 可模拟 provider 切换失败
- 可模拟 session snapshot 缺字段或降级恢复

适用场景：

- stream 中断
- tool 返回异常
- run restart / replay
- snapshot roundtrip
- degraded summary

### 5.4 E3 Real Provider 验证环境

用途：

- 证明系统与真实模型供应商的集成没有被 mock 掩盖

要求：

- 只覆盖最少关键主链路
- 控制成本与频率
- 不作为高频 CI 阻塞项

适用场景：

- 真实请求成功
- 真实流式展示
- provider 凭证读取
- provider 切换最小冒烟

## 6. 测试数据与夹具策略

系统测试必须把“输入、过程、结果、证据”都标准化。

## 6A. 跨层可观测性契约

所有 P0/P1 系统测试必须具备统一追踪骨架：

- `run_id`：一次运行的全局唯一标识
- `trace_id`：一次端到端测试实例的全局唯一标识
- `event_seq`：单次运行内的严格递增事件序号

以上 3 个字段必须同时出现在：

- 前端触发 command 的入参或日志中
- Tauri IPC 边界日志中
- Rust runtime 事件记录中
- 前端消费 event 的日志或断言产物中

P0/P1 用例不允许只断言 UI 最终态，必须同时断言一条最小事件链闭环。例如：

- `command_in -> stream_open -> chunk_n -> tool_call -> tool_result -> done`
- `command_in -> stream_open -> interrupted -> degraded_summary`

## 6B. 数据隔离与清理契约

每条系统测试必须显式声明：

1. 使用的环境级别：`E1 / E2 / E3`
2. 使用的数据命名空间：会话目录、日志目录、snapshot 目录
3. `setup` 动作：准备哪些 fixture、注入哪些 provider/tool 响应
4. `teardown` 动作：清理哪些本地状态、保留哪些失败证据

并发执行约束：

- 共享同一 snapshot 读写路径的用例必须串行执行
- 涉及 provider 切换的用例必须串行执行
- 只读场景、独立 profile 场景、独立会话目录场景可以并行执行

### 6.1 固定测试资产

- 固定 prompt 集
- 固定工具调用夹具
- 固定 provider 响应片段
- 固定 session snapshot 样本
- 固定异常注入脚本

### 6.2 样本分类

- `happy-path`
- `tool-success`
- `tool-failure`
- `stream-interrupted`
- `restore-after-restart`
- `audit-summary-missing-evidence`
- `provider-misconfigured`

### 6.3 证据输出

每条系统测试必须输出按场景类型定义的必选证据集：

- `P0 主链路`：UI 关键状态断言 + IPC/Rust 事件序列 + 运行日志
- `恢复类 P1`：UI 关键状态断言 + IPC/Rust 事件序列 + snapshot/持久化快照 + 运行日志
- `异常类 P1`：UI 关键状态断言 + IPC/Rust 事件序列 + 错误日志/失败截图
- `E3 真实 provider`：UI 关键状态断言 + 运行日志 + provider 请求结果摘要

除上述必选项外，可按需补充：

- 会话持久化文件快照
- 失败截图
- 视频或 trace 文件

## 7. 关键端到端场景矩阵

### 7.1 P0 主链路场景

这些用例必须进入每日或每次主分支合并前的系统测试集合。

1. `P0-E2E-001` 应用首次启动成功，基础工作台渲染正常。
2. `P0-E2E-002` 用户发起首轮消息，UI 进入运行中状态，最终收到完整 assistant 响应。
3. `P0-E2E-003` 流式响应期间，正文逐步追加，最终状态从运行中切换为完成。
4. `P0-E2E-004` 会话结束后刷新或重启应用，会话列表、消息内容与关键状态可恢复。
5. `P0-E2E-005` 工具调用成功时，tool call、tool result、最终总结三段链路一致。
6. `P0-E2E-006` 工具调用失败时，UI 能展示失败结果，runtime 不崩溃，会话仍可继续。
7. `P0-E2E-007` provider 未配置或配置错误时，前端提示、Rust 错误、控制状态三者一致。
8. `P0-E2E-008` run-control / history-control 审计摘要在应出现时出现，不应出现时不误出现。

### 7.2 P1 高风险恢复与降级场景

这些用例至少在每日一次完整系统回归中执行。

1. `P1-E2E-001` 流式过程中应用关闭，再次启动后会话处于可解释状态，不出现伪完成。
2. `P1-E2E-002` snapshot roundtrip 后，历史 action evidence 保持稳定，`current_context_projection` 重新投影而非伪造历史。
3. `P1-E2E-003` replay from checkpoint 与 restart from checkpoint 能正确区分开始原因与摘要文案。
4. `P1-E2E-004` stream 中断后，trace、控制状态、消息区不会互相矛盾。
5. `P1-E2E-005` 半完成工具链路不会把缺失证据伪装成成功结果。
6. `P1-E2E-006` provider 切换后，新旧 provider 状态不串线。

### 7.3 P2 可用性与防回归场景

这些用例可进入夜间或发布前批次。

1. `P2-E2E-001` 长消息与 Markdown 渲染不破坏主工作区。
2. `P2-E2E-002` 侧栏、工作区、状态区在窄宽度下不出现明显遮挡。
3. `P2-E2E-003` 多会话切换不会导致当前会话状态泄漏到其他会话。
4. `P2-E2E-004` 审计面和主消息区文案语义保持一致。
5. `P2-E2E-005` 图标、品牌组件、基础交互控件不出现明显回归。

每条用例在落地时必须附带被测模块映射：

- 前端组件或 store
- Tauri command / event
- Rust 模块或测试夹具

## 8. 审计与控制面专项验证

Pony Agent 当前阶段的高风险点不只是“能不能跑通”，而是“能不能把系统真实状态解释对”。

因此端到端方案必须单列以下专项：

### 8.1 Truth-source guardrail

验证重点：

- `action_evidence_summary` 只反映真实动作证据
- `current_context_projection` 只反映读取时上下文投影
- 历史动作证据不被 read-time 状态污染

可执行断言：

1. 恢复前后 `action_evidence_summary` 的 required fields 不发生与读时上下文相关的漂移。
2. `current_context_projection` 的变化不会改写历史 tool/result 证据。
3. 当 read-time context 变化时，只允许 projection 变化，不允许历史 action evidence 被重写。

### 8.2 Start reason guardrail

验证重点：

- `initial_turn` 不误进入 run-control summary
- `replay_from_checkpoint` 正确进入专项摘要
- `restart_from_checkpoint` 与 replay 可区分

可执行断言：

1. 普通首轮启动不产生 run-control summary。
2. `replay_from_checkpoint` 必须带正确的 `start_reason`。
3. `restart_from_checkpoint` 的摘要字段、文案或状态必须与 replay 可区分。

### 8.3 Missing / degraded guardrail

验证重点：

- 缺失证据必须显式标记 `missing` 或 `degraded`
- UI 不得把缺失态渲染成成功态
- 失败链路保留可诊断线索

可执行断言：

1. 当证据缺失时，summary 必须包含缺失态标识。
2. 缺失态下，UI 不出现成功态文案或成功样式。
3. 异常链路至少保留一条可回溯日志与一个对应状态标识。

## 9. 自动化落地方案

### 9.1 工具栈

系统测试 `v1` 强制使用以下工具栈：

- 前端和 WebView 交互：`Playwright`
- 桌面运行形态：`tauri dev` 开发态启动的 WebView
- 跨层观测：Rust 事件日志 tap + Tauri IPC 日志 + 前端测试日志
- 运行日志收集：PowerShell + 定向日志文件
- 快照与夹具：仓库内固定样本

`v1` 不引入第二套并行桌面自动化框架，避免工具栈长期分叉。

### 9.2 分阶段自动化策略

第一阶段：

- 先把现有 `vitest + cargo` 轨道纳入系统测试前置门禁
- 增加可脚本化的 mock provider 与 fault injection 入口
- 打通 6 到 8 条 P0 系统冒烟

第二阶段：

- 补齐持久化、恢复、异常注入、审计面专项
- 建立 nightly 完整矩阵

第三阶段：

- 加入少量 real provider 冒烟
- 形成发布候选准入模板

凡未进入上述三阶段范围的内容，统一视为 `v1 不做`，不得在实施中隐式扩展范围。

### 9.3 目录建议

建议新增如下结构：

- `tests/e2e/`：桌面或 WebView 端到端用例
- `tests/fixtures/`：provider、tool、snapshot、error 样本
- `tests/helpers/`：启动、日志、断言、清理工具
- `scripts/system-test/`：执行入口、日志汇总、环境准备
- `artifacts/system-test/`：运行产物

## 10. 执行节奏

### 10.1 提交级

- `npm run test:unit`
- `npm run cargo:test:regression`

目标：开发者本地快速挡回归。

### 10.2 PR 级

- `npm run test:ui-guard`
- `npm run build`
- `npm run cargo:check:shared`
- P0 系统冒烟子集

目标：阻止明显跨层回归进入主分支。

### 10.3 每日级

- 全量 P0
- 核心 P1
- 关键日志与失败截图归档

目标：尽早发现恢复链路、状态链路和审计链路退化。

### 10.4 发布候选级

- 全量 P0
- 全量 P1
- 选定 P2
- E3 real provider 最小冒烟
- 手动探索性验证

目标：给发布提供可信的系统质量结论。

## 10A. Flaky 治理机制

桌面端端到端测试默认存在波动风险，因此必须建立显式治理机制：

1. 单用例最大自动重试次数为 `1` 次。
2. 超过 `1` 次仍失败的用例，按真实失败处理，不允许无限重跑直到通过。
3. 可识别为脆弱用例时，必须进入 `quarantine` 清单，并在限定周期内修复。
4. 7 日滚动 flaky 率目标低于 `3%`。
5. 每周产出一次稳定度报告，至少包含 `PASS / FAIL / RETRY` 三态分布。
6. 系统测试必须定义用例级超时与批次级超时，避免单例拖死整批执行。

## 11. 准入与退出标准

### 11.1 准入标准

系统测试轨道启用前，必须先满足：

1. 核心命令可稳定执行。
2. mock provider 与 fault injection 入口可调用。
3. 会话与审计相关关键数据有可断言输出。
4. 失败产物可自动归档。

### 11.2 通过标准

一个版本可视为通过系统测试，至少需要：

1. L0 全通过。
2. L2 P0 全通过。
3. P1 不存在未解释失败。
4. 审计面专项 guardrail 无破坏性偏差。
5. 所有失败项都有明确归因、风险评估和处置建议。

### 11.3 阻塞发布的红线

- 首轮对话主链路失败
- 重启恢复破坏会话一致性
- tool 成功/失败状态错判
- 审计摘要把缺失态误报为成功态
- provider 错误导致 UI 和 runtime 状态冲突

## 11A. 发布签核与回滚

发布候选测试必须附带以下产物：

1. 版本号与平台信息
2. Tauri 安装包或候选构建标识
3. 关键依赖版本锁定信息
4. 系统测试报告
5. 失败项与风险接受记录

发布签核至少包含以下角色：

1. 前端 owner
2. Rust runtime owner
3. 测试 owner

每个角色必须基于同一份系统测试报告完成 sign-off，不允许口头放行。

发布回滚预案至少包含：

1. 触发回滚的失败条件
2. 回滚到哪个版本或构建
3. 用户侧降级路径
4. 紧急 hotfix 入口
5. 发布后 `N` 小时冒烟观察窗口与阈值

## 12. 失败分级与响应机制

### 12.1 分级

- `S0`：阻塞主链路或发布
- `S1`：高风险恢复/审计错误
- `S2`：局部功能异常但有降级
- `S3`：文案、样式、非关键交互问题

### 12.2 响应

- `S0/S1`：立即阻塞合并或发布，要求修复后重跑
- `S2`：允许带风险记录进入后续队列，但不得掩盖真实状态
- `S3`：记录并纳入常规修复

## 13. 首批落地路线

按三周节奏推进首批系统测试建设。

### 第 1 周

- 责任：测试 owner 主导，前端/Rust owner 配合
- 交付物：`tests/e2e/`、`tests/fixtures/`、`scripts/system-test/`
- 交付物：应用启动、首轮消息、流式完成、重启恢复 4 条 P0
- 交付物：统一日志、截图、snapshot 产物归档
- Go/No-Go：若跨层 `run_id / trace_id / event_seq` 未打通，则不得进入第 2 周

### 第 2 周

- 责任：Rust runtime owner 主导，测试 owner 配合
- 交付物：工具成功/失败、provider 配置错误、审计面显示专项
- 交付物：fault injection 基础能力
- 交付物：replay / restart / degraded 场景
- Go/No-Go：若 P1 恢复类证据集不完整，则不得进入第 3 周

### 第 3 周

- 责任：测试 owner 与发布 owner 联合主导
- 交付物：nightly 批次
- 交付物：发布候选清单
- 交付物：real provider 最小冒烟
- Go/No-Go：若 flaky 率超过阈值或发布签核模板未建立，则不得作为正式发布准入

## 14. 推荐交付物

为了让这套方案真正可执行，建议同时交付：

1. 系统测试用例清单
2. 环境准备脚本
3. mock provider / fault injection 夹具
4. 失败证据归档规范
5. 发布候选测试报告模板
6. `quarantine` 脆弱用例清单
7. 发布签核模板与回滚预案模板

## 15. 最终建议

Pony Agent 当前最需要的不是“再补几条单测”，而是建立一条以系统真实性、一致性、可恢复性为核心的跨层测试主线。

落地优先级建议如下：

1. 先用 `Mock Provider + Fault Injection` 打通可重复系统冒烟，并把跨层事件序列做成硬证据。
2. 再把会话恢复、审计摘要、降级路径和 flaky 治理纳入每日回归。
3. 最后用少量真实 provider 冒烟与发布签核闭环为发布兜底。

只有这样，现有的前端单测和 Rust 回归测试，才能真正被抬升为“可发布的系统质量能力”。

## 16. 当前阶段显式不覆盖项

为避免误读，以下内容不属于本文档 `v1` 覆盖范围：

- 安全渗透与权限对抗测试
- 国际化与多语言行为一致性
- 完整可访问性认证
- 多操作系统全矩阵一致性认证
- 长时间 soak test 与容量压测

这些主题需要在后续独立测试轨道中补齐，不应默认视为已覆盖。
