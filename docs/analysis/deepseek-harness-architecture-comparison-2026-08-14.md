# 架构对比：Pony Agent vs DeepSeek Harness

> 更新时间：2026-08-14
> 目的：对比 Pony Agent 与 DeepSeek Harness（dsh）的架构设计思路，识别二者差异与各自优势，提炼 Pony Agent 可借鉴的具体机制与落地建议。
> 数据来源：`deepseek-harness/`（commit 47f9438，浅克隆）与 `crates/pony-agent-core/`、`docs/architecture/` 当前源码与文档。

## 1. 项目定位

| 维度 | DeepSeek Harness | Pony Agent |
|---|---|---|
| 技术栈 | TypeScript / Node.js pnpm monorepo（54 packages） | Rust core + Tauri 桌面壳 + Vue 3 前端 |
| 底层框架 | vendored Cordis 插件框架 | 自研分层 runtime |
| 阶段 | 开发者预览（无兼容承诺，`SESSION_FORMAT_VERSION = 0`） | 渐进式落地（PA-010 ~ PA-025 系列） |
| 宿主形态 | Web UI / headless / ACP / JSON-RPC SDK | Tauri 桌面（first host）/ SSE adapter / 规划中 CLI |

## 2. 架构总览

### 2.1 DeepSeek Harness：一切皆插件

- 核心是**空壳**：模型适配器、工具注册表、会话日志、agent loop 本身都是插件，无特权核心。
- 组合方式为**配置驱动**：profile（命名组合）→ bundle（分发格式）→ `cordis.patch.yml` 分层覆盖，`dsh --dump-config` 可查看实际启动的插件树。
- 事实源为**事件溯源**：append-only `SessionEvent` 日志，模型历史由 `deriveMessages()` 从日志投影；"模型可见 ⟺ 已记录"是运行时不变量。
- 能力抽象为 **Capability Seam 三件套**：Service Definition（接口）/ Service Provider（实现）/ Consumer（模型工具），换 provider 即换整个产品行为。
- 扩展机制为**四类事件分发**：`emit`（观察）/ `waterfall`（中间件链，可短路）/ `parallel` / `serial`。
- 生态兼容：内置 Claude Code / Codex hooks 桥，可直接消费用户已有 hook 配置。

### 2.2 Pony Agent：严格分层 + 状态机

- 目标为 8 层分层：宿主层 → Core 基础设施层 → 宿主控制面 → Graph 编排层 → Runtime 执行层 → 能力接入层 → 状态/上下文/记忆层 → 基础设施层，每层职责固定。
- 组合方式为**代码驱动**：`AgentRuntimeBuilder` / `HostControlPlaneBuilder` 显式注入 session backend、graph store、provider resolver、tool executor。
- 事实源为**状态快照 + 分层 checkpoint**：`ExecutionCheckpoint`（turn 级）/ `GraphRunCheckpoint`（run 级），恢复语义显式。
- 循环模型为**双 loop 硬分离**：turn loop 属 runtime（单轮内 model→tool→…→final answer），graph loop 属编排层（跨 turn 目标推进，只消费完整 `TurnResult`）。
- 扩展机制为 **hooks 管线**：`TurnHookPoint` 18 个点 + `RunHookPoint` + `MemoryWriteHookPoint` + `HistoryStateHookPoint`，带 `HookFailurePolicy` / `HookStructuredResult`。

## 3. 核心设计思路差异

| 维度 | DeepSeek Harness | Pony Agent |
|---|---|---|
| 扩展模型 | 运行时动态：插件挂载/卸载（HMR），注册即 effect、卸载即回滚 | 编译期静态：hooks 枚举 + trait + Builder 注入 |
| 组合方式 | 配置层叠：patch 按 id 覆盖行配置，用户不动代码 | 代码装配：preset 在代码中构造，组合逻辑编译期确定 |
| 事实源 | 事件溯源：session log 唯一事实源，可回放/派生 | 状态快照：SessionStore + 显式 checkpoint，可恢复 |
| 循环模型 | 事件流驱动：turn/step 由事件驱动，loop 本身可替换 | 显式状态机：turn loop 与 graph loop 硬边界，stop/checkpoint 按层拆 |
| 宿主策略 | 多宿主平铺：web / headless / ACP / SDK 平级 | 桌面优先：Tauri 是 first host，SSE/CLI 规划中，统一走 `HostControlPlane` |
| 拦截机制 | 接力式 waterfall：监听者可改、可拦、可短路 | 固定安检门：hook 点位置写死，行为可控 |

## 4. 各自优势

### 4.1 DeepSeek Harness

1. **可组合性**：一切皆插件 + 配置层叠，部署方可零代码重组产品；`--dump-config` 让组合透明可审计。
2. **生态兼容**：Claude Code / Codex hooks 桥直接消费已有配置，迁移成本低。
3. **记录可信**："模型可见 ⟺ 已记录"运行时断言 + 100% 覆盖率门禁 + keyless 快照测试，重放/审计/调试能力强。
4. **动态自修改**：agent 可查看并挂载自己的插件（`tool-cordis`）。
5. **跨进程 SDK**：JSON-RPC over stdio，外部进程可驱动 runtime，天然支持 subagent 委派。
6. **文档工程化**：生成式目录（tool-catalog / config-catalog / capability graph）、Agent Notes 决策记录、文档预算门禁。

### 4.2 Pony Agent

1. **性能与安全**：Rust 原生 + 内存安全 + 单二进制分发，无运行时依赖；系统密钥存储（Windows Credential Manager / macOS Keychain / Linux Secret Service）。
2. **分层纪律**：turn/graph 硬边界 + stop/checkpoint 按层拆，防止架构腐化。
3. **编译期类型安全**：Rust 强类型 + serde，`ProviderDecision` / `ToolPlan` / `GraphRunCheckpoint` 等契约编译期保证。
4. **显式恢复语义**：turn 级 vs graph 级 checkpoint 分层恢复，长任务中断续跑有明确状态机支撑。
5. **渐进式落地**：PA 系列任务每步有验证（`npm run verify` + probe 二进制），架构演进可追溯。
6. **可观测性**：turn trace / tool activity / `ToolPlan` 显式计划边界，前端可展示 Graph 轨迹。

## 5. Pony Agent 可借鉴点（按性价比排序）

### 5.1 配置层叠（能力清单）

- **dsh 做法**：用户改 `cordis.patch.yml` 即可换模型、换工具集、换沙箱策略，不动代码。
- **Pony 现状**：`DesktopRuntimePreset` 代码写死装配，用户无法定制。
- **借鉴价值**：桌面产品用户非程序员。引入声明式"能力清单"配置（如 `pony-agent.yml`），声明启用哪些工具、默认模型、沙箱策略，代码只负责执行清单。
- **落地**：扩展 `config.rs`（providers.json），将工具集、hook 开关、graph 策略纳入配置面。

### 5.2 "模型可见 ⟺ 已记录"断言

- **dsh 做法**：运行时断言——凡是模型看到的内容必须能从 session 日志重建。
- **Pony 现状**：`sqlite_session` 快照式存储，模型请求与持久化记录之间无强制绑定。
- **借鉴价值**：无需全量事件溯源，先加审计不变量：构造模型请求时断言请求内容可从 session 记录重建，提前抓住上下文泄漏/丢失类 bug。
- **落地**：在 `turn_prep.rs` 构造请求处加校验点，复用 `trace_persistence` 与 provider-native transcript。

### 5.3 无 key 快照测试

- **dsh 做法**：录一遍真实对话，之后无 API key 也可重放验证，CI 不花钱、不依赖网络。
- **Pony 现状**：测试依赖真实 provider（`npm run cargo:test:shared` + probe），CI 有成本且受网络/限流影响。
- **借鉴价值**：`provider-native transcript` 已提供录制素材，加"重放模式"即可离线回归。
- **落地**：在 `provider/mod.rs` 增加 `ReplayProvider`，读取 transcript 文件作为响应（移植 dsh `llm-replay` 思路）。

### 5.4 hooks 桥接（生态兼容）

- **dsh 做法**：内置 Claude Code / Codex hooks 桥，读取用户已有 `hooks.json` 映射到自身拦截点。
- **Pony 现状**：`hooks.rs` 已有 18 个 hook 点，但只认自有格式。
- **借鉴价值**：加"兼容读取器"解析 `.claude/hooks.json` 或 codex 配置，映射到 `TurnHookPoint`，降低迁移成本。
- **落地**：`HookStructuredResult` / `HookFailurePolicy` 与 dsh Decision 模型同构，映射层为纯机械工作。

### 5.5 自省暴露给模型（安全版 self-modification）

- **dsh 做法**：agent 有工具可查看/挂载自己的插件（self-modification，功能强但风险高）。
- **Pony 现状**：`control_plane` 已有 `CapabilityInspectionQuery` / `SkillInspectionQuery` 自省查询面，但只给前端用，未暴露给模型。
- **借鉴价值**：只学"自我查看"不学"自我修改"：给模型 `inspect_capabilities` 工具，查询当前工具集、hook 生效状态、会话状态，减少模型对自身能力的幻觉。
- **落地**：inspection 面现成，包一层工具注册进 `ToolExecutor` 即可。

### 5.6 统一 wire 协议（多宿主铺路）

- **dsh 做法**：SDK 用 JSON-RPC over stdio，任何外部进程可驱动 harness，是 subagent 委派的基础。
- **Pony 现状**：`HostControlPlane` 已是统一命令面（`RunTurnCommand` / `StartGraphRunCommand` / `StopTurnCommand`…），但只经 Tauri 命令暴露，无稳定 wire 协议。
- **借鉴价值**：规划 HTTP-SSE/CLI 宿主时，将控制面命令序列化为稳定协议（JSON-RPC 或 SSE 命令流），避免各宿主各写各的协议。
- **落地**：`control_plane` 命令已是结构化类型，加序列化 + 传输层即可，`sse_adapter.rs` 已有雏形。

## 6. 结论

dsh 与 Pony Agent 是两条互补的路线：dsh 以"运行时组合的插件生态"换取极致的可扩展性与生态兼容，代价是运行时复杂度和动态类型；Pony Agent 以"编译期分层的状态机"换取性能、安全与架构纪律，代价是扩展需改代码。

Pony Agent 不应照搬"一切皆插件"，但可借鉴其插件化带来的三个副产品：**配置可定制**（5.1）、**记录可重放**（5.2 / 5.3）、**生态可兼容**（5.4）。建议按 5.1 → 5.2/5.3 → 5.4 → 5.5 → 5.6 的顺序推进，前四项成本低、收益直接，是 Pony Agent 从"开发者工具"走向"成熟产品"的关键拼图。