# Agent Note: 会话 id 生成规范统一为 `pa_<uuidv7>`

Status: implemented

> **归档冻结说明**：本文档为原历史决策 `docs/archive/decisions/0072-session-id-unification.md` 之归档快照，仅供溯源。

## 背景

三个入口各自生成主会话 id，格式互不相同且无共同真值源：

- 桌面端：`src/lib/runtime/sessions.ts` 的 `createNextSessionId` 生成 `pa_<uuidv7>`，并做冲突去重；
- TUI：`crates/pony-agent-core/src/bin/tui.rs` 三处（`TuiState::default`、`clear_messages`、`/clear`）生成 `session-<uuidv4 前 8 位>`，**不去重**；
- CLI：`crates/pony-agent-core/src/bin/cli.rs` 三处（新建、`--continue` 兜底、`/clear`）生成 `cli-session-<uuidv4>`。

`pony_agent_core` 里没有统一的会话 id 生成器或格式校验，三个入口各自 `Uuid::new_v4`。由此产生两个实际风险：

1. TUI 的 id 只有 32 bit 随机段。TUI 与桌面端共用同一份会话存储（`data_local_dir/PonyAgent/sessions.json`，见 `agent/session/store.rs` 的 `storage_path`），存储按 session id 为键合并；一旦碰撞，两个会话互相覆盖数据。
2. session id 是可观测性的一等关联键（Langfuse `session.id`、turn trace、诊断报告），格式混杂会让排障时需要人工比对前缀。

同时，主/辅助会话的判定（`/resume` 默认列表排除 `subagent-` / `teammate-`）也只存在于 TUI 本地实现里。

## 候选方案

- **A（采纳）**：在 core 新增 `agent::session::id` 作为唯一真值源，提供 `new_session_id()`（`pa_<uuidv7>`）、`new_session_id_unless(exists)`（有界重试、耗尽返回 `None` 而不 panic）、`is_canonical_session_id()`、`classify_session_id()` 与 `is_main_session_id()`；TUI/CLI 全部改为调用它，桌面端继续在渲染进程生成同格式 id（新建会话要立即落本地状态，绕后端往返无收益），两端以同一形状契约的测试互相锁定。存量 `session-*` / `cli-session-*` 保持可读可 resume，只是不再生成。
- **B（落选）**：session id 改由 Rust 后端统一签发，前端新建会话先请求后端拿 id。收益仅是「单一实现」，代价是新建会话多一次 IPC 往返，且离线/后端未就绪时侧边栏无法先行落状态；共享的格式契约已足以消除不一致，无需为纯度牺牲交互路径。
- **C（落选）**：继续用 uuidv4，仅给三个入口补冲突去重。v4 无时间有序性，侧边栏「最新在前」需要额外比较 `updated_at_ms`；TUI 的 32 bit 短 id 不去重就仍是覆盖风险，去重后也仍与桌面端格式不同。
- **D（落选）**：收紧 `is_main_session_id` 为「必须是 `pa_` 前缀」的白名单。会让存量用户会话从 `/resume` 默认列表里凭空消失，属用户可见回归。

## 决策

会话 id 的格式、生成与分类全部收敛到 `crates/pony-agent-core/src/agent/session/id.rs`，规范形态为 `pa_` + RFC 9562 UUIDv7（小写连字符形式）。TUI 三处与 CLI 三处生成点改为调用 `new_session_id()`；CLI 因使用独立存储 `~/.pony-agent/cli-state/cli-sessions.json`，创建前经 `new_session_id_unless` 与已有会话键去重。`is_main_session_id` 语义与改造前 TUI 实现逐字节等价（只排除 `subagent-` / `teammate-`），TUI 的本地副本删除、改为引用 core。`uuid` crate 启用 `v7` feature。

格式契约的形状（Rust 侧 `crates/pony-agent-core/tests/session_id_contract.rs`、前端侧 `tests/sidebar-groups.spec.ts` 同形）：

```text
^pa_[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$
```

## 影响

- 新建会话（含 TUI/CLI 的 `/clear`）产出的 id 与桌面端同形，可观测性与排障不再需要按入口区分前缀。
- 存量 `session-<8hex>` / `cli-session-<uuidv4>` 会话仍出现在 `/resume` 默认列表并可被 `pony resume <id>` 打开；显式 id 一律接受的行为不变。
- 存储 schema、网络端点、环境变量均未新增或变更；`docs/observability.yaml` 四维归属不变。
- TUI 与桌面端共用 `sessions.json`，id 键碰撞概率由 32 bit 随机段降为 UUIDv7 的 74 bit/毫秒随机位。

