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
- `ToolRouter / ToolCall / ToolResult` 最小闭环已经成立，当前已有 `time.now`、`echo.input`、`workspace.list_files`、`workspace.read_file`、`workspace.read_file_segment` 五个本地工具。
- OpenAI 已走 `tool_calls -> tool`，Anthropic 已走 `tool_use -> tool_result`，live provider 主路径不再依赖文本约定。
- `turn:tool` 与 `trace` 已开始承接工具错误态，工具失败不再只用文字弱提示。
- 对高确定性的工作区请求，runtime 已新增本地 planner 前置判定，能在进入远端 decision 之前直接命中 `workspace.*` 工具，减少无意义的 provider 超时。
- `ToolRouter` 所在的最小工具层已经开始承接多轮语境：当前可以结合最近用户消息回溯上一个文件，并把“第 N 行”映射到 `workspace.read_file_segment`。
- 但这些能力还主要集中在具体实现里，尚未收敛为稳定的 `Provider` trait / `ToolRouter` trait / `SessionStore` 边界。

## 下一步动作

在现有运行中的最小实现上做抽象收敛：

- 从 `provider.rs` 中提炼统一 provider 接口
- 明确 `request / response / fallback / source` 这些运行时字段的边界
- 把当前单工具闭环继续补强到更稳定的工具层，包括更细的错误恢复、更多工作区工具和更清晰的 tool event 语义
- 把当前“history 推断上一个文件”的临时规则，逐步收敛成更稳定的会话级文件上下文状态
- 把“核心 runtime”与“Tauri/HTTP 等接入层”分开思考，避免 provider 抽象继续长在 UI 交付层里

## 当前卡点

- 代码事实已经先走到了“可用”，抽象层设计反而落后于实现
- 需要避免在仍快速演进的阶段过早冻结 trait 设计
- 需要兼顾两条未来路径：一条是 Claude 式更完整 query loop，一条是 Hermes 式多接入层桥接
- 当前还没有进入多工具、并发工具、工具错误恢复和工具权限约束阶段

## 断点续跑提示

开始前先复查：

- `docs/architecture/runtime.md`
- `docs/tauri-rust-refactor.md`
