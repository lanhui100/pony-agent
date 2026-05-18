# PA-004 定义 provider 与 tool 抽象

## 状态

- Status: `Ready`
- Priority: `P1`
- Owner: `Codex`

## 目标

在 Pony Agent 中建立统一的模型与工具抽象，避免后续实现强耦合。

## 输出

- `Provider` trait
- `ToolRouter` trait
- `ToolCall` / `ToolResult`

## 验收标准

- Rust runtime 不直接依赖某个具体模型厂商
- 工具执行结果是结构化的

## 当前进展

- 已有最小 `ProviderManager`、provider preset、protocol 区分与真实请求路径。
- `run_turn()` 已不再只依赖纯 mock，而是可以命中真实 provider 并在失败时回退。
- `start_turn_stream()` 已经把 provider 调用进一步推到了统一事件流里。
- 但这些能力还主要集中在具体实现里，尚未收敛为稳定的 `Provider` trait / `ToolRouter` trait / `SessionStore` 边界。

## 下一步动作

在现有运行中的最小实现上做抽象收敛：

- 从 `provider.rs` 中提炼统一 provider 接口
- 明确 `request / response / fallback / source` 这些运行时字段的边界
- 为后续工具执行链预留结构化 `ToolCall` / `ToolResult`
- 把“核心 runtime”与“Tauri/HTTP 等接入层”分开思考，避免 provider 抽象继续长在 UI 交付层里

## 当前卡点

- 代码事实已经先走到了“可用”，抽象层设计反而落后于实现
- 需要避免在仍快速演进的阶段过早冻结 trait 设计
- 需要兼顾两条未来路径：一条是 Claude 式更完整 query loop，一条是 Hermes 式多接入层桥接

## 断点续跑提示

开始前先复查：

- `docs/architecture/runtime.md`
- `docs/tauri-rust-refactor.md`
