# Agent Note: src-tauri 宿主命令面按业务域拆分（Phase 1）

Status: implemented

## Problem

`src-tauri/src/lib.rs` 单文件承载 97 个 `#[tauri::command]`（68KB），同时混装着退出 flush 装配、
托盘初始化、日志路径解析、OTel 装配与整个命令实现面。`backlog:B1795831358336` 的审计证据显示
该文件是**近 1 月排名第 1 的合并冲突热点（97 次修改）**：任何一条命令的小改都要动同一个文件，
多 Agent/多分支并行时必然在 `use` 块与函数体之间反复冲突；同时命令实现与装配逻辑无边界，
`run()` 的改动风险与被 `tauri_command_surface.rs` 冻结的 97 个命令名耦合在同一份 diff 里。

审计的另一条事实是 `docs/architecture/structural-debt-proposals.md` 记录的「92 个命令中仅 6 个 async」，
热读命令（`list_sessions` 等 SQLite 读）曾被内联在 WebView IPC 线程上。本 Phase 不改变这一点
（异步化属行为面改动），只解决**文件级耦合**。

## Decision

**1) 新建 `src-tauri/src/commands/`，按业务域一文件**

14 个子模块：`acp` `ask` `capability` `diagnostics` `goal` `history` `host` `plan` `session`
`settings` `system` `team` `turn` `workspace`。每个子模块自包含（`use`、域内视图结构体、域内
helper 与命令实现在同一文件），因此「改某个域」只需打开一个文件——这就是 Locality of Change。
`commands/mod.rs` 只声明子模块，承担「命令面有哪些域」这一唯一事实。

**2) `lib.rs` 只保留装配**

余下内容：`mod` 声明、`EXIT_FLUSH_DONE` + `flush_otel_on_exit` + `handle_run_event`、
`run()`（插件注册、`manage` 状态、`setup`、`generate_handler![…]` 注册表、窗口事件、build/run）。
`lib.rs` 从 1775 行降到 256 行。

**3) 命令面契约不变（红线）**

97 个命令名、`camelCase` 参数签名、成功返回形状逐字未动（逐条脚本比对 97 条签名，9 处差异全为
rustfmt 尾逗号/空格，语义零差异）；`generate_handler![…]` 的注册**集合**与 origin/main 完全一致
（97 = 97，仅加模块前缀）。注册**顺序**改为按域聚集（原顺序是历史上随实现追加的混合序），
这是无契约影响的变化——Tauri 的 invoke 按名字路由，不依赖注册顺序；但为免误读，本 ADR 明确记录
这一项并非逐字保留。迁移是**纯搬家**：脚本核对拆分前后 `src/` 树内 `fn` 名字集合完全一致
（103 个顶层函数无遗漏、无新增），`static`/`struct`/`const`/`enum` 亦无丢失。

**4) 契约测试随之扩面并加严**

`src-tauri/tests/tauri_command_surface.rs` 原实现 `include_str!("../src/lib.rs")` 只扫单文件，
拆分后声明不再在 `lib.rs`，会退化成「声明集合恒空」的假绿。改为**运行时遍历 `src/` 全树**
收集 `#[tauri::command]` 声明（新增命令域文件无需改测试，也不需要第二份手维护的文件清单），
并新增反向断言 `every_declared_command_is_registered`：原测试只判「注册 ⊆ 声明」，
漏注册（声明了却没进 `generate_handler!`）仍会漏网；现在双向都判。

**5) clippy 棘轮基线的路径重映射**

`scripts/clippy-baseline-src-tauri.json` 按 `(lint, path)` 记账，纯搬家会让 133 条 `lib.rs` 键
在账本上「消失」而 17 条 `commands/*` 键「新增」。处理口径：Linux 分册用
`rust:lint:tauri:update` 实测重生成；`win32`/`darwin` 分册按同一映射重写。允许跨平台套用本次
映射的**唯一依据**是实测事实：`src-tauri/src/commands/**` 零 `#[cfg(...)]`
（`grep -rn "cfg(" src-tauri/src/commands/` 无命中），且拆分前三平台的 `lib.rs` 条目集合与计数
逐字相同；平台差异只存在于 `src-tauri/src/platform.rs`（win32 独有 9 项），该文件本轮未动。
总量 3323 → 3301（无上涨）。

其中唯一计数变化是 `clippy::needless_pass_by_value` 130 → 108（Δ−22）：这不是放宽，而是**收紧**——
该 lint 对「可被外部调用的 API」默认不告警，命令函数从 `fn`（模块私有）改为 `pub(crate)` 后
clippy 的 exported-api 判定发生变化，22 条不再告警。基线随实测下调，意味着这 22 条若重新出现
会立即判红。

## Alternatives considered

- **方案 A：按域拆 `commands/*` + `lib.rs` 只装配（采纳）**。Deletion Test：删掉这层拆分，
  「改一条命令要不要碰装配文件」这个判断会重新扩散到每个调用方（所有并行分支），且
  `tauri_command_surface.rs` 的冻结清单与命令实现被迫同居一份 diff——复杂度是扩散而非消失，
  故该层保留。
- **方案 B：同时拆 `pony-tui/tui.rs`（14.7k 行）**。落选：任务要求「严格按
  Phase 1(src-tauri) → Phase 2(tui) 分批」，一次动两个巨石无法在一次 review 内验证等价性。
  Phase 2 仍欠账，见 Consequences。
- **方案 C：拆文件同时把 86 个同步命令异步化（`BlockingHelper`）**。落选：异步化会改 IPC 调用
  语义（错误时机、取消语义、前端 loading 状态假设），属于行为面变更，与「搬家」混在同一提交
  会让 review 无法区分「搬错了」与「异步化引入的回归」。本 Phase 只搬家，异步化待独立任务。
- **方案 D：拆文件 + 在 `lib.rs` 保留 `pub use commands::*` 让路径透明**。落选：这正是
  「新旧并存」的兼容层——`generate_handler!` 用哪个路径都能编译，反而放过了「命令忘了注册」；
  显式 `acp::acp_list_agents` 前缀让注册表同时成为所属域的说明书。

## Consequences

- 新增命令的落点从「`lib.rs` 里找位置」变成「`commands/<域>.rs` 里追加 + `mod.rs` 声明」；
  新增业务域需同时改 `mod.rs` 与 `lib.rs` 的 `use` —— 这是有意的两处显式登记，代价换来域边界可见。
- `tauri_command_surface.rs` 现在读文件系统（`CARGO_MANIFEST_DIR`），不再是纯 `include_str!`；
  代价是测试依赖工作树结构，收益是新增域文件零维护。若将来 `src/` 出现非命令的
  `#[tauri::command]`（如 `bin/` 下的探针），会被计入声明集合而要求注册——届时按需收窄扫描面。
- 已知欠账（Phase 2，未开工）：`crates/pony-tui/src/tui.rs` 仍为 14.7k 行、仍在
  `scripts/file-size-baseline.json` 的豁免名单里；`src-tauri` 侧 86 个同步命令仍未异步化。
  二者都需独立任务与独立 review。
- `win32`/`darwin` 分册的映射是推导值而非实测值：CI 在 Windows/macOS 上跑一次
  `rust:lint:tauri` 即可证伪；若实测有差，按 `lint-baseline-bootstrap` 工作流在对应平台重生成后覆盖。
