# PA-044 Spec Review

## 审核对象

- [PA-044 任务卡](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-044-harden-agent-core-infrastructure-boundary.md>)
- [proposal.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/harden-agent-core-infrastructure-boundary/proposal.md>)
- [design.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/harden-agent-core-infrastructure-boundary/design.md>)
- [tasks.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/harden-agent-core-infrastructure-boundary/tasks.md>)
- [delta spec](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/harden-agent-core-infrastructure-boundary/specs/agent-core-infrastructure-boundary/spec.md>)
- 相关实现现状：
  - [runtime.rs](</C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/runtime.rs>)
  - [control_plane.rs](</C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/control_plane.rs>)
  - [tools.rs](</C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/tools.rs>)
  - [session.rs](</C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/session.rs>)
  - [graph.rs](</C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/graph.rs>)
  - [tauri_adapter.rs](</C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/tauri_adapter.rs>)
  - [sse_adapter.rs](</C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/sse_adapter.rs>)

## 审核方式

- 只读审阅 OpenSpec proposal / design / tasks / delta spec
- 对照本轮 agent core 审核发现，重点检查：
  1. 是否把实现步骤误写成过硬的长期产品 contract
  2. 是否足以防止 core package / constructor / desktop preset 回粘 Tauri
  3. non-Tauri harness 验证矩阵是否能真实证明多端基础设施方向
  4. 是否保留现有 Tauri desktop command/event 行为作为迁移保护线

## 主要结论

未发现阻塞 PA-044 spec 交付的问题。

当前 spec 已经把核心风险拆成可验收边界：

1. `agent core` 必须能作为 Tauri-free 或等价 core target 被非 Tauri 宿主依赖
2. runtime/control-plane 必须提供非测试可用的 host-injectable construction
3. 桌面本地路径、keyring、`current_dir` workspace 与 Tauri async runtime 必须降级为 host preset
4. adapter 只能翻译宿主协议与投递事件，不得复制 provider/session/tool/graph 语义
5. 至少一个 non-Tauri harness 必须证明 sync turn、stream turn、injected workspace root、injected persistence 与 non-Tauri event sink
6. provider transport 被定义为可替换实现细节，避免 blocking desktop transport 变成服务端唯一策略
7. 现有 Tauri desktop command/event contract 被明确列入迁移 non-regression

## 非阻塞观察

1. `core package 或等价 Tauri-free target` 的表述足够给实现留余地，不会强迫第一步必须完成完整 crate migration。
2. builder/preset 的要求偏基础设施 contract，但没有固定具体类型名为唯一实现路径；`AgentRuntimeBuilder / HostControlPlaneBuilder` 被 design 标为推荐，delta spec 使用的是 host-injectable construction。
3. provider transport seam 在 spec 中只要求可替换 policy seam，没有要求本轮立即重写为 async provider，因此范围可控。
4. `tasks.md` 中实现任务仍保持未勾选是正确状态；本次只完成 spec 阶段，不应把 implementation / verification / acceptance audit 伪标完成。

## 验证证据

已通过 OpenSpec 严格校验：

```powershell
npm run openspec -- validate harden-agent-core-infrastructure-boundary --type change --strict --no-interactive
```

结果：

- `Change 'harden-agent-core-infrastructure-boundary' is valid`

已通过 artifact 状态检查：

```powershell
npm run openspec -- status --change harden-agent-core-infrastructure-boundary --json
```

结果：

- `isComplete: true`
- `proposal: done`
- `design: done`
- `specs: done`
- `tasks: done`

## 交付判断

`PA-044 / harden-agent-core-infrastructure-boundary` 的 spec 阶段可以交付，并可作为后续实现的执行基线。

下一步进入实现时，应从 `tasks.md` 的 `2. Construction Boundary` 开始，优先实现 host-injectable construction，而不是直接大规模搬 crate。
