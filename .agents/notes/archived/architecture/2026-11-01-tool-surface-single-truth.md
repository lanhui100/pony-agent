# Agent Note: 工具面单一真值闭合与外部工具通道（T1）

Status: implemented

> **归档冻结说明**：本文档为原历史决策 `docs/archive/decisions/0065-tool-surface-single-truth.md` 之归档快照，仅供溯源。

## 背景

PA-076（`docs/architecture/tool-runtime-descriptor-registry.md`）已建立 `ToolDescriptor`、`ToolRegistrySnapshot` 与 `GovernedDispatcher`，并让治理分派成为唯一执行入口。但单一真值只覆盖了内建工具的一半：

1. **registry 只服务内建**：`builtin_tools()`（`tools.rs:5713`）产出 45 条定义，`GovernedDispatcher::register_handler`（`dispatcher.rs:513`）按 `descriptor_id` 注册 handler，但生产装配只注册了内建（`governed_executor.rs:450-570`），外部 descriptor 没有执行体。
2. **名推导与分派仍是手写 match**：`canonical_tool_name`（`tools.rs:7352`）做别名归一化、`execute_internal`（`tools.rs:1524`）做 45 臂分派。外部名称（MCP / 插件）落进这里只会得到 `unsupported_tool`。
3. **能力桥直连名称**：`capability_bridge.rs:568` 把 capability 的 `label` 直接当工具名，再回落到静态 match —— 这是"外部能力接不进来"的具体断点。

## 决策

1. **元数据单一真值**：工具的身份（`descriptor_id`）、模型名、别名、kind、exposure、权限事实、schema、顺序一律由 descriptor registry 提供。`*_for_name` 系列退化为查表或删除，保留者只作兼容壳，不得新增派生逻辑。
2. **执行按 `descriptor_id` 查表**：`GovernedDispatcher` 的 handler 表是唯一分派入口。内建 descriptor 绑定 `RouterPrimitiveHandler`（legacy `ToolRouter` 降级为**实现细节**，不再是元数据来源）；外部 descriptor 绑定 `ExternalToolHandler`。`execute_internal` 的 72 行 match（`tools.rs:1517-1596`）改为 `&'static str → fn(&ToolRouter, …)` 的 **fn 指针表**（零 `dyn`，保留 `allow_batch` 与 `deferred_to_registry` 两臂），配 45 primitive 穷尽性遍历断言补回编译期穷尽检查；`execute_internal` 仅 5 处调用点且全在本文件。
3. **新增"工具面原子换入"接缝**（本 ADR 的关键交付，T2 的前置）：`GovernedDispatcher` 的 registry 在构造期固定（`dispatcher.rs:471,489`），全仓无换表 API。新增 `swap_tool_surface(snapshot_id, registry, handlers)`，把 **快照 + handler 表作为一个原子单元**换入，保证任一时刻不存在"descriptor 已可见但 handler 未就绪"的中间态。

   **对外契约（调用方必须知道）**：换入面自洽性由实现强制 —— 只要新快照里**任一可见 descriptor 没有对应 handler，整个换入被拒绝**并返回缺失清单（不是部分生效、不是合并语义）。因此生产换面必须一次性重供全部可见 descriptor 的 handler（含 45 个内建），换入结果就是**整面替换**：新快照里没有的旧 descriptor 立即不可见。
4. **拆除"按数组长度推断集合身份"**：`is_builtin_tool_definition_set`（`provider/mod.rs:3541`，6 处调用）按工具数组长度与内容判断是否走 registry 投影，一旦外部 descriptor 进入数组即静默翻转到 fallback，45 个工具同时丢 descriptor 事实与排序 —— 这是 T1 与 T2 共有的地雷，也是 KV-cache 前缀的隐形开关。改为按 snapshot 事实判断。
5. **修只读模式漏洞**：`governed_executor.rs:320-333` 按**工具名嗅探**（read/edit/bash 字面量）判定 `read_only`，外部写工具名不在集合内 → 只读模式会放行外部写操作。改为读 descriptor 声明的权限事实（`ToolPermissionFacts`），名称不参与权限判定。
6. **新增 `ToolProvider` 进程内抽象**（L1 接缝）：
   ```rust
   pub trait ToolProvider: Send + Sync {
       fn source_id(&self) -> &str;
       fn list_tools(&self) -> Vec<ToolDescriptor>;
       fn call(&self, descriptor_id: &str, arguments: Value) -> Result<Value, ToolProviderError>;
   }
   ```
   MCP（0066）与插件宿主（0067）都是它的实现 —— **只允许一条外部通道**，禁止各自旁路治理管线。
7. **命名空间与别名登记口径**：外部工具 model_name 固定 `mcp__<server>__<tool>` / `plugin__<name>__<tool>`；与内建名或保留别名冲突时 fail-closed 并给出可读错误（registry 已有保留别名校验：`tools.rs:802-836`）。**所有外部 descriptor 必须把自己的 `model_name` 登记为 alias**，否则 `ToolRegistrySnapshot::resolve(model_name)` 取不到该工具，按名解析的前缀投影/回放/审批回显会出现来源间不一致（MCP 与插件两种 provider 必须同一口径）。
8. **工具面快照会话内冻结**：不 mid-turn 增删；按 `(source_id, model_name)` 稳定排序；条目上限默认 128，超限产出 `truncated` 证据。会话内新接入的外部 source 在下一轮生效。
9. **错误码分源**：`mcp_server_unavailable` / `mcp_tool_not_found` / `mcp_timeout` / `plugin_unavailable` / `capability_denied` 与 `unsupported_tool`（仅表示名字确实未知）区分；`unsupported_tool` 不再承担"外部能力未接线"的语义。

## 候选方案

- **A 在 `execute_internal` 加外部兜底分支**：执行路径出现两个真值源，且外部工具绕过 descriptor 校验。否决。
- **B 独立 `ExternalToolDispatch` 通道旁路 `GovernedDispatcher`**：丢授权、预算预约、生命周期证据（PA-076 的三项硬约束）。否决。
- **C 给 descriptor 加 `handler_id` 并把 45 臂 match 全量改成表**：形式最统一，但爆炸半径覆盖 7000 行 `tools.rs` 与全部工具回归用例，收益仅是写法收敛。降级为"分派按 `descriptor_id` 查表、内建 handler 复用 legacy router"（采纳）。
- **D 让 MCP 直接注册 handler 到 dispatcher**：可行但把 MCP 变成特例，插件宿主会复制第二套。改为 `ToolProvider` 抽象（采纳）。

## 后果

- 代码面：`tools.rs`（registry 快照对外暴露外部 descriptor 的构造/校验）、`governed_executor.rs`（装配外部 handler）、新增 `ToolProvider` 与 `ExternalToolHandler`、`capability_bridge.rs`（不再把 label 当工具名）、`error_code.rs`（新增分源错误码）。
- **不变量**：45 个内建工具的名称、顺序、schema、错误码、权限事实逐字节不变；现有 `wave37_tool_surface` 等冻结用例不迁移、不改断言。
- 新验收：外部 provider 注册一个合成 descriptor 后，能经 `GovernedDispatcher` 走完授权/预算/生命周期并返回结果；未注册 handler 的 descriptor 仍 fail-closed。
- 风险：T1 单独交付对用户零可见，验收**不得**用"45 工具行为不变"自证外部通道可用 —— 必须由 0066 的 MCP fixture 做端到端反向验证。

