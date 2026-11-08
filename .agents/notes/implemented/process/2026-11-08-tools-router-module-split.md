# Agent Note: `agent/tools.rs` 模块化拆分（P0-P3，单文件 14841 → 13 个 ≤1200 行模块）

Status: implemented

## Context

`crates/pony-agent-core/src/agent/tools.rs` 拆分前 14841 行、68 个顶层 `pub` 项，同时承担六类
彼此无关的职责：工具契约元数据（`ToolDescriptor*` / `ToolPermission*` / 注册表快照）、名字映射与
权限判定、内置工具 JSON schema 装配（`builtin_tools` 单函数 1070 行）、workspace 文件与路径面、
`workspace_run_command`（631 行 / 嵌套 10 层）、web 抓取与 pin、以及 team/goal/todo/job 等编排桥接。
测试代码 4705 行（32%）内联在同一文件。

机器证据（非主观判断）：`npm run size:check`（ADR 0090 基线棘轮）退出码 1——文件 14841 行 >
基线 14816 行；基线中 9 条 tools 侧条目覆盖全部超限项（`builtin_tools` 1070 行、`run_command`
631 行、`gather_context` 479 行、`write_file` 206 行，`edit_file`/`run_command` 嵌套 9-10 层）。

硬约束：147 处外部引用 `crate::agent::tools::*`；`governed_executor.rs` 把每个 primitive 桥接回
该模块，因此它不是可删的死代码，拆分必须保持路径不破。

## Decision

1. **原地模块化为 `src/agent/tools/`**：`tools.rs` → `tools/mod.rs`；`mod.rs` 保留 `ToolRouter`
   结构体、构造器、`execute_internal` 分发表与 `TOOL_*` 常量，其余按关注点下沉：

   | 模块 | 行数 | 职责 |
   |---|---|---|
   | `mod.rs` | 610 | `ToolRouter` + primitive 分发表 + `TOOL_*` 常量 + `pub use` 再导出面 |
   | `contract.rs` | 729 | `ToolKind`/`ToolExposure`/权限声明/描述符/注册表快照/`ToolSurface` + 自校验 |
   | `exec.rs` | 374 | `ToolCall`/`ToolResult`/`ToolOutcome`/`ToolExecutionContext` + `error_result` 系列 |
   | `schema.rs` | 1164 | `builtin_tools()` schema 装配、`with_description`、contract view |
   | `names.rs` | 559 | canonical/model-visible/product 名称、别名、执行策略、默认权限事实 |
   | `workspace.rs` | 1176 | 读/写/编辑/列目录/path_info/glob/搜索 + 分段读取与 start_line 三态 |
   | `paths.rs` | 711 | workspace 相对路径、root 解析、路径分类、受控 tmp、路径修复 |
   | `run.rs` | 1165 | `run_command`：进程生命周期、超时提升、危险命令拦截、Windows 批处理 |
   | `web.rs` | 931 | web 抓取/搜索、pin 到已验证地址、正文解码、超时重试 |
   | `composite.rs` | 1038 | `workspace_batch` / `workspace_gather_context` 与嵌套计划/摘要 |
   | `instructions.rs` | 357 | 附加指令注入（touched 记忆 + 预算裁剪） |
   | `orchestration.rs` | 886 | subagent/team/goal/todo/present/job/skill 桥接处理器 |
   | `contract_view_tests.rs` | 589 | 契约投影测试（原内联模块，独立成文件） |

   测试再拆为 `tools/tests/`（`mod.rs` 夹具 + `composite`/`web`/`workspace`/`run`/`orchestration`
   五个用例文件，132 个测试，最大 1202 行）。

2. **可见性只做最小放大**：跨模块复用的私有辅助与 `impl ToolRouter` 方法升级为 `pub(super)`
   （= `tools` 及其后代可见），`pub(crate)` 项保持原级别；`mod.rs` 用显式
   `pub use self::<mod>::{...}` 维持 `crate::agent::tools::<item>` 名称空间，因此 147 处外部引用零改动。
   不把任何内部助手升为 `pub`。

3. **显式依赖替代 glob 链**：子模块不写 `use super::other::*`，缺失符号由编译器逐条报出后补
   `use super::<mod>::<item>;`（一次性机械收敛，最终 `cargo check` 0 warning 0 error）。

4. **基线记账改为路径迁移、数值不变**：`scripts/file-size-baseline.json` 删除 `tools.rs` 的
   file-lines 条目（新文件均 ≤3000），8 条函数级条目改挂新家（`builtin_tools`→`schema.rs`、
   `gather_context`→`composite.rs`、`run_command`→`run.rs`、`write_file`/`edit_file`→`workspace.rs`），
   `value` 一律不动——棘轮只许降，不许借搬家洗白函数级存量债。

5. **扩展点生成器事实源同步**：`scripts/gen-extension-points.mjs` 读 `ToolDescriptorSource` 的
   路径改为 `agent/tools/contract.rs`，`docs/architecture/extension-points.md` 同步重生成。

## Alternatives considered

- **只搬测试（原 P0 作为终局）**：落选。文件仍 1 万行、函数粒度债务不变，只是把门禁刷绿。
  它作为执行顺序的第一步保留，但必须接 P1-P3。
- **拆独立 crate `pony-tools`**：落选（本轮）。收益是编译单元隔离；代价是 147 处引用、`governed_executor`
  桥接与 `src-tauri` 间接依赖全链改动，属"拆分 + 换架构"双重变更，无法逐步验收。
- **`pub use self::x::*` 全量 glob 再导出 + 子模块 `use super::*`**：落选。能一步编译通过，但依赖
  隐式名字链，后续任何同名符号会静默解析到别处；显式 `use` 让依赖可被 `cargo check` 校验。
- **保留单文件 + `// ponylint:file-size-exempt` 豁免**：落选。豁免围栏是给无法拆的生成物/单点，
  可拆的 1.4 万行文件用豁免等于取消 ADR 0090 的立意。
- **直接删除基线里 8 条 tools 条目（宣称清零）**：落选。`builtin_tools` 1070 行、`run_command`
  631 行等函数级超限仍在，删条目就是放开门禁；本轮不改这些函数体（纯搬移）。
- **放宽 `pub(crate)` → `pub` 以减少可见性改造**：落选。会把内部助手误升为公开契约面，扩大后续兼容负担。

## Verification

- `cargo check -p pony-agent-core --lib` → 0 error / 0 warning。
- `cargo check -p pony-agent-core --all-targets` → 0 error（含集成测试目标）。
- `cargo test -p pony-agent-core --lib` → **1388 passed / 0 failed**（其中 `agent::tools` 144 项）。
- 纯搬移审计（脚本比对 HEAD 原文）：360 个函数名、248 个顶层项全部落地，行内容 multiset 差异仅剩
  team-link D2 修复（`execute_drain_inbox` 增加调用者作用域守卫，本体由 `0c2ff30e` 提交），无代码丢失。
- `node scripts/check-file-size.mjs --baseline scripts/file-size-baseline.json` → tools 侧 0 条新增违规；
  仓库仅剩 1 条非本轮变更（`runtime/turn_stream.rs`，并发进行中的改动）。
- `cargo test -p pony-agent-core --test tool_surface_wave20_contract --test tool_timeout_policy_test --test wave42_skill_body --test conformance` → 28 + 2 + 15 + 7 全绿（工具面可见性/超时契约/技能正文/一致性）。
- `npm run ext:check` / `npm run version:check` / `npm run contract:check` → PASS。
- `tool_router_regression` 位于 `src-tauri/tests/`（`pony-agent` 包），本轮未运行；`-p pony-agent-core --all-targets` 已覆盖 core 侧全部目标。

## Consequences / 边界

- 函数级存量债（`builtin_tools` 1070 行、`run_command` 631 行、`gather_context` 479 行、嵌套 9-10 层）
  未被本轮消除，仍由基线 grandfathered 记账；要清零需再拆函数体，属独立变更。
- 本轮不改任何语句、不动行为；搬迁期间工作区里的 team-link D1/D2/D3 修复（`0c2ff30e`）随文件原样保留。
- 编译依赖：`tools/orchestration.rs` 引用 `crate::agent::orchestration::{normalize_target, teammate_target_for_session}`，
  故模块化必须落在那次修复之后，二者无法拆成独立提交。
- `crates/pony-agent-core/tests/*.rs` 里若干文档注释仍以 `tools.rs:<行号>` 指路（行号早已漂移），
  属历史叙述，未在本轮改写。
