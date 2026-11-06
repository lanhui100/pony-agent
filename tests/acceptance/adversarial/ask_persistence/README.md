# 对抗矩阵：Ask 持久化 + 生命周期收口（T3 / L2-AT）

归属：Test Agent（task-3 / 波次 T3）｜契约源（只读）：`.dev-team/contract-matrix-ask-persistence.md`
（§2 数据模型、§3 状态机、§4 CAS 语义、§8 验收映射）｜ADR：
`docs/decisions/proposed/0074-ask-persistence-lifecycle.md`

本目录登记「未决 Ask 落盘 + 生命周期收口」这条链路的**确定性破坏注入面**：并发双提交同一
`request_id`、重放旧 `version`、非法状态越迁，以及归档（`Orphaned`）后的复活尝试。

## 0. 放置约定

Rust 侧破坏注入用例与功能验收用例**同文件**（本波 Test Agent 写域只有
`crates/pony-agent-core/tests/ask_persistence_acceptance.rs` 一个 Rust 测试文件，见 task-3
write scope）：

- 功能契约（P1–P6，契约 §8 映射）：`crates/pony-agent-core/tests/ask_persistence_acceptance.rs`
- 对抗破坏（A1–A4，本目录矩阵）：同文件 `adversarial_*` 用例
- 本目录只承载**对抗矩阵登记与复现口径**（与 `tests/acceptance/adversarial/metrics/` 同款分层）

所有用例统一经契约冻结的**唯一构造面**取 store：
`ask_persistence::open_ask_store(&Path) -> AskPersistenceResult<Box<dyn AskPersistenceStore>>`
（进程内场景另有 `open_in_memory_ask_store()`）；测试**不自行 impl trait**、不假设内部实现
（表名/列名/序列化布局一律不作为断言对象，只断言契约规定的可观察性质）。

## 1. 对抗矩阵（确定性；无 sleep、无随机、无概率）

| # | 破坏类别 | 注入构造 | 必须成立的不变量 |
| :-- | :-- | :-- | :-- |
| A1 | **并发双提交同一 request_id** | 两条独立实例（各自 store 句柄 / 各自 SQLite 连接）在 `std::sync::Barrier::new(2)` 处**固定会合**后，同时提交 `Pending → Answered(version=1)` | 恰好 **1 个**成功且版本前移到 2；恰好 **1 个**被 CAS 拒绝；终局记录离开待决列表；之后任何实例再提交仍被拒绝（不产生双写、不产生半迁移） |
| A2 | **重放旧 version** | 磁盘记录版本前移至 `3`（模拟「别的实例已推进」的陈旧视图），按固定顺序用 `version=1,2` 重放，再用 `3` 合法提交 | 陈旧版本一律拒绝且状态不变；当前版本必须成功（因果对照，证明拒绝理由是版本而非「一律拒绝」）；重启后重放已消费版本 `3` 仍被拒绝 |
| A3 | **非法状态越迁 `Answered → Pending`** | 已 `Answered`（version=2）的记录，遍历全部目标状态发起越迁，并额外谎称「当前仍是 Pending」 | 全部越迁被拒绝；记录不得「复活」进待决列表；同会话新 Ask 不受终态污染 |
| A4 | **`Orphaned` 复活** | 会话归档 → `gc_orphans` 转 `Orphaned` → 重启后按原版本作答 | 作答被拒绝；`Orphaned` 记录不出现在待决列表（终态不可被重启/重建流程复活） |

补充约束：

- **确定性**：不出现 `sleep`、不出现随机种子、不出现概率重试；并发用例的唯一同步原语是
  `Barrier`（固定会合点），断言与线程调度顺序无关（「恰好一个赢家」对两种交错都成立）。
- **无 panic 逃逸**：断言一律为 `assert!`（红相失败点落在测试文件内的断言行）；
  协议空桩期工厂的 `unimplemented!()` panic 由 `catch_stub_panic` 收敛为 `Err`，
  再由断言呈现为**断言失败**，不会被误判为 panic 逃逸。
- **无环境依赖**：数据库与目录一律落在 `std::env::temp_dir()` 的独立临时目录，
  用例结束即 `Drop` 清理（谁创建谁清理），不污染开发者真实会话库。

## 2. 如何运行

```bash
# 功能契约 + 对抗破坏（红相期必须按断言失败；空桩未实现前全部红）
cargo test -p pony-agent-core --test ask_persistence_acceptance

# 单条对抗注入（示例）
cargo test -p pony-agent-core --test ask_persistence_acceptance adversarial_concurrent_double_answer
```

## 3. 边界说明（本轮不覆盖）

- **真实多进程**：A1 用「同库两个独立连接 + Barrier」模拟双进程并发；真正的独立 OS 进程编排
  （`Command::spawn` 两个 host）成本高且与宿主装配耦合，留待 T5 客户端 PTY 复测覆盖重启重建。
- **控制面/Tauri 命令面**（`ask_list_pending` / `ask_answer` / `ask_cancel` / `ask_expire` 的
  落点注入）属 T4/T5 实现面；本波验收锚定契约冻结的 store 语义（§8 映射），命令面语义由客户端
  波次（`tests/acceptance/adversarial/ask_persistence/` 之外的 vitest 冻结面）覆盖。
- **回答落点**（graph ask wait resume / `take_pending_ask_injection`）不在本波 Test Agent 冻结范围。
