# Agent Note: 五份契约版本化、conformance kit 与扩展点归属表（T4）

Status: implemented

> **归档冻结说明**：本文档为原历史决策 `docs/archive/decisions/0068-contract-versioning-and-conformance-kit.md` 之归档快照，仅供溯源。

## 背景

0064 的三层模型只有在契约可被判定的前提下才成立。当前事实：

- 已存在半成品契约资产：`tests/ts_contract_snapshot.rs`、`docs/contracts/ts-contract.snapshot.json`、`scripts/check-ts-contract.mjs`（前后端契约哈希门禁）。
- hooks 已有版本常量 `AGENT_HOOKS_CONTRACT_VERSION = "agent-hooks-v1"`（`hooks.rs:5`），但没有对应的 schema 文件与负例。
- tool descriptor 的字段语义只存在于 `tools.rs` 与架构文档，没有独立的 schema 与 golden。
- 错误码 `ErrorCode::as_str` 是线缆可见字符串，但没有 append-only 纪律。
- 缺「新行为该挂哪」的归属表：DSH 的 `docs/architecture` 归属表是其可扩展性的真正资产，本项目没有等价物。

## 决策

1. **五份契约 + 版本**（每份 = JSON Schema + Rust 类型 + golden 用例 + 负例）：
   | 契约 | 版本 | 载体 |
   |---|---|---|
   | 工具描述符 | `pony-tool-descriptor-v1` | `docs/contracts/tool-descriptor.v1.schema.json` + `tools.rs` |
   | Hook 协议 | `agent-hooks-v1`（已存在） | `docs/contracts/hooks.v1.schema.json` + `hook_commands.rs` |
   | MCP 配置 | `mcp-config-v1` | `docs/contracts/mcp-config.v1.schema.json` + `mcp_client.rs` |
   | Skill manifest | `pony-skill-manifest-v1` | `docs/contracts/skill-manifest.v1.schema.json` + `workspace_hotloader.rs` |
   | 插件 manifest | `pony-plugin-v1` | `docs/contracts/plugin.v1.schema.json` + 0067 模块 |
2. **conformance kit**：
   - 进程内：`cargo test -p pony-agent-core --test conformance`；
   - 外部（第三方 provider / 插件作者）：`cargo run -p pony-agent-core --bin conformance -- --kit <dir> --strict`，退出码语义固定（0 通过 / 2 用例失败 / 3 配置非法）。
   - 用例集 v1 固定 12 例（6 正 6 负），负例逐例独立断言，禁止整批笼统失败：越权能力、畸形 payload、超时、取消、apiVersion 不符、未知能力。
3. **扩展点归属表**：`docs/architecture/extension-points.md` —— "想加 X 该挂哪"，与 0064 三层模型一一对应。**归属表是生成物**（从 registry 的 source 枚举与 trait 清单生成），手写副本即为第二真值源，禁止。
4. **契约演进门禁**：任何契约变更登记 `breaking` / `additive` 并附兼容证据；序列化字段只增不改；错误码 append-only（未知走 `unknown:*`）；golden 一旦冻结，改它就是 breaking。
5. **复用既有资产**：`check-ts-contract.mjs` 与 `ts-contract.snapshot.json` 扩展而非新建；契约哈希纳入 `npm run verify`。

## 候选方案

- **只写文档不建可执行套件**：契约无法被第三方机械验证，等于没有契约。否决。
- **把 kit 做成仓库外脚本**：CI 无法机械执行门禁，回归靠自觉。否决。
- **归属表手写维护**：与 registry 枚举必然漂移（`builtin_tools` 的长度推断分支就是前车之鉴）。改为生成物。
- **一次性冻结全部五份契约**：T2/T3 的真实形状尚未落地时冻结会立刻返工。改为 T4 分两段：W2 出骨架（schema + 正例），W3 在 T2/T3 落地后冻结（golden + 12 例全绿）。

## 后果

- 可扩展性从"能加"变为"可承诺"：第三方按 schema 与 kit 自证兼容。
- 代价：每次契约变更需更新 golden 并跑 kit；归属表需要生成脚本（新增 `scripts/gen-extension-points.mjs` 或等价 Rust bin）。
- 与既有治理的关系：本 ADR 不改变 `npm run verify` 的语义，只在其内部增加契约哈希与 kit 两段。

