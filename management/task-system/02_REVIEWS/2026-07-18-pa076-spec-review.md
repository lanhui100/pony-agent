# PA-076 Spec Review

## 审核对象

- [PA-076 任务卡](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-076-harden-and-expand-agent-tool-runtime.md)
- [proposal.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/harden-and-expand-agent-tool-runtime/proposal.md)
- [design.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/harden-and-expand-agent-tool-runtime/design.md)
- [tasks.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/harden-and-expand-agent-tool-runtime/tasks.md)
- [delta specs](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/harden-and-expand-agent-tool-runtime/specs/)

## 审核阵容与结论

| 角色 | 结论 | 关键发现 | 处置 |
| --- | --- | --- | --- |
| `@architect` | 不通过 | hook 后授权错配；Ask 不能复用普通 `wait_user`；ToolSearch/MCP 缺少真实运行时边界；Run 缺 sandbox | 采纳，纳入 dispatcher、PendingControlRequest、TurnToolView、McpTransport 与 SandboxBackend 合同 |
| `@security-reviewer` | 不通过 | 攻击者可猜测 internal/deferred 名称；DNS rebinding/ambient proxy；MCP 覆盖 builtin；process containment 被误作 sandbox | 采纳，增加 invocation origin、pinned connector、reserved namespace、sandbox support matrix |
| `@test-engineer` | 不通过 | characterization 会冻结错误映射；Ask/MCP/Process/Web 缺可重复验收；平台/本地 fixture 矩阵不足 | 采纳，改为迁移删除测试，增加 fake harness、hermetic resolver/MCP fixture、平台 canary |
| `@consultant` | 有条件通过 | 控制 outcome、请求绑定、Run sandbox、Web pinned connector 是进入实现的 P0 门槛 | 采纳；门槛已写入 artifacts，完成 strict validation 后只允许从 characterization tests 开始 |

## 发现与采纳记录

### P0-1：hook 改写后的权限 TOCTOU

- 来源：架构、安全
- 决定：采纳
- 修订：dispatcher 固定为“原始校验 -> mutable hook -> 最终校验/规范化 -> policy/sandbox decision”；approval/pending request 绑定最终参数摘要。

### P0-2：Ask/approval 被编码为普通 ToolResult

- 来源：架构、测试、安全、consultant
- 决定：采纳
- 修订：定义 `ToolOutcome.execution_status` 与 `control_outcome` 正交；引入带 CAS、expiry、nonce 和调用事实绑定的 `PendingControlRequest`；恢复只以原 tool call id 生成唯一终态结果。

### P0-3：Run 将 containment 误作 sandbox

- 来源：架构、安全、测试、consultant
- 决定：采纳
- 修订：引入 `SandboxBackend` 与平台支持矩阵；无人值守 Run 无 sandbox 时 fail closed；Job Object/process group 只描述 containment，Unix best-effort 不承诺防主动逃逸。

### P0-4：WebFetch DNS/代理边界不可兑现

- 来源：架构、安全、测试、consultant
- 决定：采纳
- 修订：任意 URL 抓取在 injected resolver/pinned connector/peer-IP 校验完成前 fail closed；禁用 ambient proxy；redirect 每跳重新解析和 pin。

### P1-1：internal/deferred 可被猜名调用

- 来源：安全
- 决定：采纳
- 修订：`InvocationOrigin` 成为 dispatcher 授权输入；模型仅能调用 `TurnToolView` 中 direct/elevated descriptor，internal 仅允许受限 ChildDispatch。

### P1-2：MCP registry 仅有元数据且可碰撞

- 来源：架构、安全、测试
- 决定：采纳
- 修订：增加 versioned snapshot、reserved namespace、原子拒绝冲突、source-bound `McpTransport` 与独立 `ResourceTemplate`；当前 arguments 回显路径列入迁移删除范围。

### P1-3：ToolSearch 无选择协议与 next-hop 时序

- 来源：架构、测试
- 决定：采纳
- 修订：定义 `select`、descriptor/source revision、`TurnToolView`、同一 user turn 下一 provider hop 生效、source replacement/retry/reload 失效。

### P1-4：characterization 冻结已知错误

- 来源：测试
- 决定：采纳
- 修订：只冻结产品名、无争议 schema/输出、alias 和 payload；`Ask -> echo_input`、`Plan -> workspace_batch` 明确作为迁移删除测试。

### P2/P3 汇总

- 采纳：原子预算预留、exactly-once hook、敏感 trace 脱敏、Search/Glob/image/Plan 边界测试、平台与 hermetic network 测试矩阵。
- 部分采纳：本 change 不承诺立即实现 Linux cgroup/container 或所有 MCP transport；对应能力未就绪时按 fail-closed 处理，并以明确的 transport/sandbox port 保留后续扩展点。
- 不采纳：无。所有 P0/P1 已转为 requirement、design decision 或具体验收任务。

## 验证门槛

1. 严格 OpenSpec 校验必须通过。
2. 在上述 P0 contract 与测试 harness 之前，不迁移生产 dispatcher、不启用 Ask/approval、不替换 Run、不开放任意 URL WebFetch。
3. Rust 测试仍受本机缺少 MSVC `link.exe` 阻塞；只能通过项目 `npm run cargo:*:shared/exact*` 脚本尝试，不能绕过 target 槽位直接运行 Cargo。

## 当前结论

规格审核在修订后可有条件进入基线 characterization 阶段；实现阶段必须按 `tasks.md` 顺序逐项完成，并在 descriptor/dispatcher、control、process、web、MCP/ToolSearch 各阶段重新专项审核。
