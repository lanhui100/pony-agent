# Design: Second Wave Tool Surface

## 背景

当前 Pony Agent 的工具系统已经具备较稳定的协议底座：

- `ToolDefinition / ToolCall / ToolResult / ToolFailureKind`
- `WorkspaceContext`
- `ToolPermissionFacts`
- 产品级 canonical tool name 与前端展示元数据

同时，`first-wave-tool-surface` 已经把首批产品级工具面冻结为 8 个工具。但真实实现仍停留在偏只读探索阶段：

- `Read / Search / List / Plan` 已有真实工作区 primitive 支撑
- `Ask` 有基础交互占位
- `Run` 仍是极轻量能力，不足以承担真实受控执行
- `Edit / Write` 尚未有真实内建 primitive

因此第二批工具面的核心不是“无节制继续加工具”，而是先把 v1 最小 coding agent 闭环补完整，再把最有价值的外部探索和桥接能力纳入正式路线。

## 设计目标

1. 优先补齐已冻结首批工具面里的实现缺口
2. 为近线最有价值的新增工具提供稳定边界
3. 避免把 Browser、Workflow、Thread、Automation 等更大产品面混入本卡
4. 明确阶段顺序，让后续实现按批推进而不是并行发散

## 非目标

- 本 change 不直接实现工具代码
- 本 change 不重新设计 `PA-045 ~ PA-049` 的母合同
- 本 change 不把所有 Codex / Claude Code 工具一次纳入 Pony 内建面
- 本 change 不把 MCP 工具桥接与 Browser 自动化的完整产品面一并做完

## 范围分层

### Layer 1：首批缺口补齐

这是当前最优先的一层，因为这些工具已经进入合同，但还未落地为真实能力。

#### 1. Edit

边界：

- 只负责“定向修改已有文本内容”
- 必须面向已有文件，不承担新建文件职责
- 修改方式可以是 patch、search/replace、range replace 或等价结构化编辑
- 必须保留 workspace 边界、权限事实和失败归一化
- 零匹配时必须返回结构化错误，而不是静默跳过
- 多处匹配且无法唯一定位时必须返回歧义错误，而不是默认选第一个命中
- patch 或编辑指令格式非法时必须返回结构化错误

#### 2. Write

边界：

- 负责新建文件或整体覆写
- 与 `Edit` 分离，避免“既可增量改、又可整文件覆写”的语义混乱
- 需要显式进入写入权限语义

#### 3. RunShell

虽然产品层 canonical name 仍可继续表现为 `Run`，但底层必须从“极轻量 primitive”提升为真正受控执行能力。

边界：

- 产品层 canonical name 保持 `Run` 不变，不新增 `RunShell` 作为独立对外工具名
- `RunShell` 仅作为实现内部能力层名称，不单独暴露到模型合同或前端主展示
- 支持在 workspace 内执行受控命令
- 至少具备 `cwd / timeout / exit_code / stdout / stderr` 基本结构
- 权限语义必须显式复用 `PA-047`
- 不是任意宿主控制总入口，不承担浏览器、人机提问或线程管理语义
- 必须有明确的高风险命令边界，禁止默认执行超出 workspace 的系统级破坏性命令

### Layer 2：代码库探索增强

这一层对应 Claude Code 中最成熟、且对 coding agent 提效明显的能力。

#### 4. Glob

边界：

- 用于按 pattern 搜索文件路径
- 区别于 `List` 的目录列举、区别于 `Search` 的文本搜索
- 应优先服务于大代码库文件发现

#### 5. Grep

边界：

- 用于 pattern / regex 文本检索
- 区别于当前较泛化的 `Search`
- 若 `Search` 继续承担全文检索，则需要在产品层明确 `Search` 与 `Grep-like` 子能力关系

设计倾向：

- 产品层继续保留 `Search` 作为唯一对外文本检索入口
- `Grep` 作为实现层 primitive 或增强检索能力，不新增独立对外 canonical tool name
- 实现层应把“文本模式检索”升级为更强 primitive，而不是长期停留在简化搜索

### Layer 3：外部读取与知识获取

这一层用于补齐 agent 与外部信息源的最小闭环，但优先级低于本地代码编辑执行能力。

#### 6. WebFetch

边界：

- 只负责读取指定 URL 内容
- 不承担搜索排序职责

#### 7. WebSearch

边界：

- 负责外部搜索
- 应和 `WebFetch` 分层，避免“搜索 + 抓取”混为一个工具

### Layer 4：桥接与大工具池治理

#### 8. MCP Resource Read

边界：

- 至少覆盖 `ListMcpResources / ReadMcpResource` 这种只读资源面
- 作为 capability bridge 的稳定上层入口，不直接把原始桥接细节暴露为产品合同

#### 9. ToolSearch

边界：

- 负责 deferred / dynamic / capability-backed 工具发现
- 不是普通 `Search` 的同义词
- 只在工具池足够丰富时成为近线高价值能力

## 分阶段建议

### Phase A：最小闭环补齐

优先顺序：

1. `Edit`
2. `Write`
3. `RunShell`

原因：

- 这是从“只读 agent”走向“可修改、可执行 coding agent”的最短路径
- 也能直接填平当前首批工具合同与真实实现之间的缺口

### Phase B：探索增强

优先顺序：

4. `Glob`
5. `Grep` 或增强后的 `Search`

### Phase C：外部读取

优先顺序：

6. `WebFetch`
7. `WebSearch`

### Phase D：桥接与治理

优先顺序：

8. `MCP Resource Read`
9. `ToolSearch`

说明：

- `MCP Resource Read` 在本卡中需要形成正式边界与进入条件
- `ToolSearch` 在本卡中只形成 deferred 进入条件，不展开实现级设计

## v1 不做

当前 change 不纳入以下能力：

- Browser 自动化
- Notebook 专项编辑
- Thread / Automation 管理
- 多代理任务分工工具
- LSP / IDE 深度语义工具

这些能力可能有价值，但不属于当前“第二批基础内置工具面”的最小收敛范围。

## 验证策略

本 change 的验证重点不是代码行为，而是 spec 是否足够约束后续实现顺序与边界：

- 任务卡、proposal、design、spec、tasks 应一致表达阶段顺序
- 必须显式说明 `Edit / Write / RunShell` 是当前近线优先项
- 必须显式说明哪些能力延后
- 必须完成一轮 `opencode / deepseek-v4-flash` 独立 spec review，并采纳合理意见
