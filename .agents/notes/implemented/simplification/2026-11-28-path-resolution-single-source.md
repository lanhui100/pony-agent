# Agent Note: 路径类平行实现收敛为单源（sqlite 路径与 CLI/TUI 日志路径）

Status: implemented

## Problem

6 层 10-crate 拆分与前端拆解之后，两处"路径规则"各自留了两份逐字相同的实现，且都没有任何测试会因漂移而红：

1. **SQLite 会话库路径**：`crates/pony-protocol/src/sqlite_paths.rs` 与
   `crates/pony-storage/src/sqlite_session/sqlite_paths.rs` 是同一份逻辑的两个副本（目录优先级
   `PONY_AGENT_DATA_DIR` > 平台数据目录 > `$HOME` > cwd，末级 `PonyAgent/sessions.db`；测试隔离分支
   各写一份）。两者只差两行 `cfg` 属性，规则本体逐字相同。改一处而漏另一处时，`pony-storage`
   写库的目录与 `pony-observability`（spool / frontend diagnostics）读库的目录会分叉，表现为
   "数据明明写了却读不到"，而编译与测试都绿。
2. **CLI/TUI 运行日志路径**：`crates/pony-tui/src/tui.rs` 与 `crates/pony-cli/src/main.rs` 各有一个
   私有 `fn bin_log_file_path()`，函数体逐字相同（`dirs::state_dir()` > `$HOME/.local/state`，
   追加 `pony-agent/logs/pony-agent.log`）。两份同时演化时 `pony` 与 `ponycli` 会把结构化运行日志
   写进不同目录，取证面直接分裂。

## Decision

1. **SQLite 路径：规则单源 + 各 crate 只保留构建形态分支**。
   `pony_protocol::sqlite_paths` 新增无 `cfg` 的 `production_sqlite_path()`（唯一一份目录优先级实现），
   `default_sqlite_path()` 变为"测试隔离 / 生产"二选一的薄壳。
   `pony-storage` 的 `sqlite_paths.rs` 从 59 行副本降为薄包装：`default_sqlite_path()` 按本 crate
   的 `cfg(test)` 选分支、`data_dir_override` 直接 re-export。
   **为什么不能整段 re-export**：`cfg(test)` 只对正在编译的 crate 生效；`pony-storage` 跑单测时依赖
   `pony-protocol` 是按 `cfg(not(test))` 编的，直接 re-export 会让 storage 的单测落到真实用户库。
   隔离分支必须留在本 crate —— 这条理由写在薄包装的文件头注释里，防止后来者"优化"回去。
2. **日志路径：唯一实现下沉到 `pony_observability::otel::log::bin_log_file_path()`**（该模块本就持有
   512KiB 轮转、降级与脱敏），`tui.rs` 与 `cli/main.rs` 的私有副本删除，两处调用点改为直接调用单源。
   TUI 的 `ObservabilityBootstrap` 注释保留其**语义契约**（settings > env > 默认端点、上报默认关闭），
   但不再复述路径规则。

## Alternatives considered

- **方案 A（采纳）：规则单源 + 保留各 crate 的 `cfg` 薄壳**。删掉副本里的规则本体，只留"这个 crate 走哪一支"的决定，Deletion Test 通过：删掉薄壳会让 `cfg(test)` 语义扩散到调用方（storage 的单测要么落真实库、要么每处手动传路径）。
- **方案 B：整段 re-export `pony_protocol::sqlite_paths::default_sqlite_path`**。落选：`cfg(test)` 不跨 crate 传播，storage 单元测试会读写真实用户库 —— 这是既有 `PONY_AGENT_DATA_DIR` 旁路与 `unique_test_sqlite_path()` 存在的全部理由，退回去等于重开"测试污染生产库"的缺陷。
- **方案 C：把 `pony-storage` 的路径解析改成显式入参（每个调用点传 `PathBuf`）**。落选：`default_sqlite_path()` 有 6 个生产调用点（`SessionStore::new` / `open_lazy` / `startup_meta` / `goal` / tui `persistence_status`），改成显式入参是把"数据放哪"这条规则复制 6 份，正好与本次目标相反。
- **方案 D：把日志路径规则放进 `pony-protocol`**。落选：`pony-protocol` 是纯契约/类型层（见 `2026-11-28-protocol-decouple-and-bridge-inversion.md` 对它的净化方向），日志落盘是运行期 IO 关注点，归 `pony-observability` 更贴合分层；且该模块已持有日志 sink 的全部实现。

## Removed surface

- `crates/pony-storage/src/sqlite_session/sqlite_paths.rs`：删除 `data_dir_override()` 与
  `unique_test_sqlite_path()` 的本地实现（改为单源引用）。
- `crates/pony-tui/src/tui.rs`、`crates/pony-cli/src/main.rs`：删除私有 `fn bin_log_file_path()`。

检索收据（确认无外部消费者、只有这两处私有副本）：

```
$ rg -n 'fn bin_log_file_path' crates src-tauri
crates/pony-tui/src/tui.rs:14313
crates/pony-cli/src/main.rs:1215
$ rg -n 'sqlite_paths::(data_dir_override|default_sqlite_path|unique_test_sqlite_path)' crates src-tauri
crates/pony-observability/src/otel/spool.rs:173
crates/pony-observability/src/otel/spool.rs:175
crates/pony-observability/src/frontend_diagnostics.rs:514
crates/pony-tools/src/workspace_hotloader.rs:780
```

## Reintroduction conditions

只有在下列条件之一成立时才允许重新出现"第二份路径实现"：

1. 某个 crate 需要与 `PonyAgent/sessions.db` **不同**的目录规则（例如把 OTEL spool 挪到独立可配目录），
   且该差异必须是显式参数而非另一份默认值函数；此时新增的是**参数**，不是第二个真源。
2. `pony-protocol` 真的被剥离为纯契约层（见 protocol 解耦提案落地），路径解析无处可放时，
   它必须整体迁出到唯一的新家，并同步删除本记录指向的两个调用点 —— 不允许两份并存过渡。

判定方式：`rg -n 'join\("PonyAgent"\)' crates` 必须只命中 `pony-protocol/src/sqlite_paths.rs` 一处；
`rg -n 'pony-agent\.log' crates` 的路径拼接必须只命中 `pony-observability/src/otel/log.rs`。

## Consequences

- 改"数据放哪"只需动 `pony-protocol/src/sqlite_paths.rs`；改"日志放哪"只需动
  `pony-observability/src/otel/log.rs::bin_log_file_path`。
- 净删除约 75 行重复实现（`sqlite_paths.rs` 59→31 行、两处 `bin_log_file_path` 各 6 行），
  新增 57 行（含单源函数与解释 `cfg` 不跨 crate 的注释）。
- 该收敛的机械看守仍是 review 级（无门禁能判"两个函数是否语义重复"）；上面的 `rg` 判定式已写进
  本文，可作为评审时的一次性检查。
- 同批未收敛、已单独立项的部分：`DynamicProviderResolver` 在 TUI 与 CLI 的**语义**差异（TUI 注册表优先、
  CLI 硬编码 `ponyllm`）涉及 CLI 对外行为，按红线留待人工裁决，见 `[NEEDS_HUMAN]` 记录。
