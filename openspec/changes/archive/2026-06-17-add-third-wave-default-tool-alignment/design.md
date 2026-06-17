# Design: Add Third Wave Default Tool Alignment

## Context

第二波工具面已经解决了“基础能力有没有”的问题，但还没有完全解决“默认工具是不是已经像一个稳定产品面那样工作”的问题。

这一轮不是重新发明新的工具体系，而是把 5 个已经部分具备、但在默认工具层仍不够收口的工具统一拉平：

- `Plan`
- `Ask`
- `MCP Resource`
- `ToolSearch`
- `Run`

## Design Goals

1. 让 `Plan / Ask` 从展示层工具名收口为真实默认工具边界
2. 让 `MCP Resource / ToolSearch` 从“桥接能力入口”提升为“默认工具面对齐成员”
3. 让 `Run` 的默认工具语义继续保持为 `Run`，但在执行合同上更明确地对齐 shell execution
4. 不把这一轮扩成新的浏览器、线程、自动化或 marketplace 规划

## Run Coupling

`Run` 之所以与本波其他工具同批收口，不是因为它和它们同类，而是因为它与 `Plan / Ask` 存在直接的执行前编排与审批路径：

- `Plan` 可能生成后续执行意图
- `Ask` 可能补齐执行前缺失上下文
- `Run` 承接最终受控执行

因此 `Run` 在这一波里要收口的是默认工具合同升级，而不是单独的产品名变更。

## Tool-by-Tool Boundary

### 1. Plan

`Plan` 的职责是显式表达计划、更新计划，或驱动后续计划化执行编排。

这一轮需要收口的关键不是重新设计 `ToolPlan`，而是明确：

- `Plan` 可以只产出计划而不执行
- 若 `Plan` 触发后续执行，执行细节必须仍通过统一 `ToolPlan` / `child_results` 暴露
- `Plan` 不等于任意批量执行入口
- `Plan` 不替代底层 `workspace_batch` 或其他内部原语名

### 2. Ask

`Ask` 的职责是向用户或宿主请求：

- 澄清
- 确认
- 补充输入

这一轮需要收口：

- `Ask` 必须是受控的人机/宿主中介入口
- `Ask` 不能退化成任意字符串输出或泛化执行入口
- `Ask` 必须与 permission / host mediation 合同对齐
- `Ask` SHALL NOT produce side effects beyond message delivery
- 当 host mediation 不可用时，`Ask` SHALL 明确 fallback 行为，而不是静默丢弃

### 3. MCP Resource

`MCP Resource` 在第二波里已经有最小 bridge 入口，本轮不重做 capability bridge。

本轮要解决的是默认工具对齐语义：

- `MCP Resource` 必须被明确视为默认工具面中的正式只读资源入口
- 它与普通文件 `Read` 不同，读取目标是 capability-backed resource
- 它不能混入普通工具执行或写操作

### 4. ToolSearch

`ToolSearch` 在第二波里已经有最小 discovery 入口，但默认工具地位仍偏弱。

本轮收口：

- `ToolSearch` 负责 deferred / dynamic tool discovery
- 它不等于普通文本搜索
- 它不替代 `Search`
- 它必须返回结构化候选，而不是普通 grep/search 风格结果

最小结构化结果至少应包含：

- `tool_name`
- `description`
- `source`
- `confidence`

### 5. Run

`Run` 的 canonical product name 保持不变。

本轮只做默认工具合同升级，不新发明 `RunShell` 作为对外工具名。允许：

- `RunShell` 作为内部实现能力名
- `Run` 对外继续保持稳定默认工具名
- `Run` SHALL delegate to `RunShell` with contract validation

本轮要收紧：

- `Run` 至少稳定暴露 `cwd / timeout / exit_code / stdout / stderr`
- 继续遵守高风险命令默认拒绝或更高审批
- 不允许把 `Run` 扩成任意宿主控制平面

## Sequencing

这一轮按以下顺序收口：

1. `Plan / Ask`
   原因：它们属于默认工具面里最明显的合同缺口
2. `MCP Resource / ToolSearch`
   原因：实现入口已有，但默认工具定位需要正式提升
3. `Run`
   原因：基础 primitive 已具备，本轮主要做合同与体验对齐升级

## Non-goals

本轮不做：

- Browser 工具面
- Thread / Automation / Workflow 工具面
- LSP、IDE、marketplace 排序体系
- 新一轮 capability bridge 架构重写
- 替换第二波已经完成的 `Edit / Write / Glob / WebFetch / WebSearch` 合同

## Review Requirements

本轮文档必须完成至少一轮 `opencode / deepseek-v4-flash-free` 独立只读审核，并至少覆盖以下 3 个维度：

1. 范围与分组合理性
2. 工具边界与安全约束
3. proposal / design / spec / tasks / task card 一致性
