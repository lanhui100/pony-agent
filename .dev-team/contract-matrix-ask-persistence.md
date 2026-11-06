# Ask（Interaction 控制请求）持久化与生命周期契约矩阵

> 波次：`ask-persistence`｜冻结人：Lead（task-1）｜状态：**frozen**
> 真值优先级：本文 > 代码注释。任何条目变更必须由 Lead 原地改写并登记文末流水，禁止另行追加章节。

## 0. 背景与裁决

现状（基线 `4e4fedc` + 本波之前的一次修复）：`PendingControlRequest` 只存在于
`GovernedDispatcher.pending_requests`（内存 `Mutex<BTreeMap>`，`dispatcher.rs:492`），进程退出即蒸发；
`expires_at_ms = now + DispatchBudgetConfig::control_request_expiry_ms`（默认 60_000，`budget.rs:85`）
是唯一时效来源；`can_consume` 用 `now_ms <= expires_at_ms` 判定可消费。Ask 与 Approval 共用该常量。

用户裁决（2026-11-06）：**未决 Ask 是会话状态，不是计时资源**。只要它是会话里最后一个未完成交互，
重启/重连/换端后都应重建面板等待回答。墙钟 TTL 退出产品语义。

## 1. Non-Goals（本波不做）

- 不做多端实时同步/推送（仍靠轮询）；不做跨用户共享作答。
- 不重写 Approval（`ApprovalResumeUnavailable` 语义保持不变）；Approval 的 TTL 行为另案。
- 不引入新外部服务/新依赖；不做 ask 富文本、附件、多选扩展。
- 不改 `builtin:ask_user` 的模型可见 schema（`question/options/description`）。

## 2. 数据模型（P1，Protocol Agent 唯一落盘）

`crates/pony-agent-core/src/agent/ask_persistence.rs`

```rust
pub struct PersistedAsk {
    pub request_id: String,        // control-<12 位序号>
    pub version: u64,              // CAS 版本，初始 1，每次迁移 +1
    pub nonce: String,             // 防重放；必须随请求落盘（跨进程 CAS 的前提）
    pub session_id: String,
    pub run_id: Option<String>,
    pub turn_id: String,
    pub call_id: String,
    pub kind: PendingControlRequestKind, // 复用 tool_runtime 既有枚举（Interaction | Approval），不另造 AskKind
    pub prompt: String,
    pub options: Vec<String>,      // 生产 schema 为 string 数组
    pub state: AskLifecycleState,
    pub created_at_ms: u64,
    pub answered_at_ms: Option<u64>,
}
```

- API 形态：以 `pub trait AskPersistenceStore: Send + Sync` 暴露（`load_pending` / `upsert` /
  `transition` / `supersede_pending_on_user_message` / `gc_orphans`），实现由会话存储侧提供；
  空桩已冻结于 `ask_persistence.rs`。`Send + Sync` supertrait 使 `Box<dyn AskPersistenceStore>`
  可注入 Tauri State / 跨线程共享，方法契约不变。
- **构造入口（测试与宿主共用，禁止在测试里自造实现）**：
  `open_ask_store(db_path: &Path)` / `open_in_memory_ask_store()` → `Box<dyn AskPersistenceStore>`；
  空桩阶段函数体为 `unimplemented!()`（语言原生红相原语）。
- 序列化：serde `rename_all = "camelCase"`；枚举 `rename_all = "snake_case"`。
- 存储：落 `sessions.db`（与 `sessions`/`normalized_*` 同库），表 `pending_asks`，
  主键 `(session_id, request_id)`；索引 `(session_id, state)`。**必须提供向前迁移**（旧库可直接打开，
  缺失表按需创建），迁移脚本沿用 `scripts/migrate-sessions-schema.mjs` 的既有模式。
- **禁止**依赖 `expires_at_ms` 判活；该字段在持久化记录中**不出现**（GC 保留期如需，用 `created_at_ms` + 归档事件推导）。

## 3. 状态机（P2，唯一合法迁移）

```
Pending ──answer──▶ Answered
   │                  ▲
   ├──cancel/ignore──▶ Cancelled
   ├──new user msg───▶ Superseded
   └──session gone───▶ Orphaned
```

| 迁移 | 触发者 | 前置 | 副作用 |
| :-- | :-- | :-- | :-- |
| `Pending → Answered` | 宿主（answer） | state==Pending 且 version/nonce 匹配 | version+1，写 `answered_at_ms`，触发回答落点（§5） |
| `Pending → Cancelled` | 用户显式「忽略此提问」/ 取消 | 同上 | version+1 |
| `Pending → Superseded` | **用户新消息进入会话** | state==Pending | version+1；不再出现在待决列表 |
| `Pending → Orphaned` | 会话归档/删除 | state==Pending | 由 GC 批量处理 |
| 终态 → 任意 | — | — | **拒绝**（返回稳定错误码，不得 panic） |

- 终态（Answered/Cancelled/Superseded/Orphaned）不可再迁移；重复提交必须被 CAS 拒绝且**不得**产生用户可见英文内部串。
- `Superseded` / `Orphaned` 不再参与 `load_pending`；`Answered` 保留供审计（可按保留期 GC）。

## 4. CAS 语义（P3，跨进程仍成立）

- **`transition_scoped_with_answer`（L3-final 追加，冻结签名）**：`fn transition_scoped_with_answer(&self, session_id: &str, request_id: &str, expected_version: u64, expected_nonce: &str, from: AskLifecycleState, to: AskLifecycleState, answer: &serde_json::Value) -> AskPersistenceResult<PersistedAsk>`；状态迁移与 `answer_json` 必须同一事务提交，默认实现 fail-closed。测试替身（如 `FailMarkLandedStore`）必须转发该新方法，不得返回伪成功。
- **`transition` 签名必须携带 `(request_id, session_id, expected_version, expected_nonce)`**，并在同一事务内与存储中的 `version`/`nonce`/`state` 逐项比对；nonce 不符 ⇒ `ask_nonce_mismatch`（该错误码必须被真实引用，禁止只定义不使用）。L3-A/L3-B 双审均以可跑反例判定「nonce 无持久化真值」为 BLOCKING，本行即修复判据。
- `answer/cancel` 必须校验 `(request_id, version, session_id)` 三元组 **+ 服务端自持 nonce**：
  - 客户端**不传** nonce（nonce 只存在于存储，服务端从记录派生，客户端无法伪造）；
  - 宿主调用必须带 `session_id` 作用域；记录的 `session_id` 与调用作用域不一致 ⇒ 拒绝（跨会话作答非法）；
  - 任一项不符 ⇒ 拒绝，错误码沿用 §4 稳定码；缺 `session_id` 作用域 ⇒ `ask_scope_required`。
- 校验读取来源是**持久化记录**，不是进程内存；因此在 A 进程创建、B 进程（重启后）作答也必须成立。
- 拒绝错误码稳定：`ask_state_conflict` / `ask_version_mismatch` / `ask_nonce_mismatch` / `ask_not_found`；
  客户端只允许展示 §6 的中文文案，原始串只进运行时日志。
- **不再存在** `cannot be consumed: … expiry …` 路径：TTL 不再是拒绝理由。

## 5. 控制面 API（P4）

保持既有 Tauri 命令名（`ask_list_pending` / `ask_answer` / `ask_cancel` / `ask_expire`），语义调整：

| 命令 | 新语义 |
| :-- | :-- |
| `ask_list_pending(session_id)` | 从持久化 `load_pending(session_id)` 读；**投影不带 nonce**；`session_id` 缺省/空 ⇒ 返回空数组（fail-closed，禁止跨会话泄露 requestId+version，错误码 `ask_scope_required`） |
| `ask_answer(request_id, version, answer)` | CAS + 写终态 + 回答落点 |
| `ask_cancel(request_id, version)` | CAS + `Cancelled`（= 显式忽略） |
| `ask_expire(now_ms?)` | **退化为 GC**：`Orphaned` 清理与保留期回收，不得把 Pending 变 Expired |

回答落点（`Pending → Answered` 之后）：存在 graph ask wait 绑定 ⇒ resume；否则把答案作为 tool result
注入该会话下一轮（复用 `take_pending_ask_injection`），**禁止**静默丢弃。

**注入队列与 ack（L3-P 双审裁决，2026-11-06）**：会话/run 维度的 ask 注入是
`BTreeMap<key, VecDeque<Injection>>`（旧单对象持久化兼容读）；消费流程为
`peek → provider 请求使用 → 成功终态（completed/suspended）才 ack 移除`，
failed/cancelled 必须保留供下次续轮；ack 只弹队首且校验 `request_id`。
`Answered` 记录需落 `landed_at_ms`（landing 账本），镜像/注入落地失败时由 sweep 从 durable 重建，
使 `ask_mirror_sync_failed` 可自愈；disk 是唯一真值，镜像失败不得回滚 disk。
无作用域 `transition` 入口在生产 API **fail-closed**（`ask_scope_required`），测试改用 `transition_scoped`。
**答案载荷与 landed 真值（L3-final 裁决，2026-11-06）**：`pending_asks` 必须持久化
`answer_json`（真实用户答案，旧表 ALTER 前向迁移）；sweep 重建注入只能用该真实载荷，
禁止任何占位/伪载荷。`landed_at_ms` **只在**「graph resume 成功」或「已入队会话注入」后写入；
resume 失败必须保持 unlanded 并由 sweep 重试（bound run 先试 resume、失败降级入队）。
GC 报错时**不得**推进镜像；保留期 GC 不得删除 `Answered` 且 `landed_at_ms IS NULL` 的记录。


宿主轮次入口（L3 复核后追认）：`HostControlPlane::start_turn_stream` 委派
`start_ask_aware_turn_stream`（TUI/CLI/harness 共用），使每轮都在真实 graph run 内执行；
`start_turn_stream_legacy` 保留为回落路径。该委派改变 CLI/harness 的 run 身份行为，
属**已追认的契约修订**（原 C1「逐字不变」作废），后续以 `ask_resume_e2e` 作为回归锚点。

## 6. 客户端行为（P5）

TUI（`crates/pony-agent-core/src/bin/tui.rs`）与桌面（`src/stores/ask.ts` + **`src/components/chat/WorkspaceComposer.vue`**，
后者是真实挂载点；`src/components/AskPanel.vue` 经 grep 确认为**零挂载死代码**，本波只做「去 TTL 后可编译」的最小适配、不删除）**同版**：

1. **无倒计时、无"已过期"态**；不得以本地时钟预判可否提交。
2. **恢复即重建**：启动 / `resume` / 打开会话时 list 一次未决 Ask 并建面板；同一 `request_id` 只允许一个面板。
3. **显式忽略**：面板提供「忽略此提问」⇒ `ask_cancel` ⇒ 面板关闭且不再出现。
4. 用户发出新消息后，旧面板必须消失（宿主置 `Superseded`；客户端刷新时同步移除）。
5. 冲突/失效提示统一中文（"该提问已失效，已同步最新待处理请求"），**禁止**上屏宿主英文内部串。
6. 轮询可以自停，但**打开会话时必须重新拉取**（不得依赖"空列表 ⇒ 永久停轮询"）。

## 7. 迁移与兼容（P6）

- 旧库（无 `pending_asks` 表）打开即建表；旧内存态无数据可迁，不做回填。
- 兼容期：`list_pending_asks` 若持久化层不可用（表损坏/写失败）必须 fail-closed 并给出结构化错误，**不得**静默回退内存态。
- **测试替身例外**：允许在测试文件内实现 `AskPersistenceStore` 装饰器（如 `FailOnWriteStore`）以注入写失败；仅限测试、必须注释声明、不得进入生产路径。
- 版本四处同步（`package.json` / `Cargo.toml` / `tauri.conf.json` / `.version.json`）由发布流程负责，本波不 bump。

## 8. 验收映射（P7，Test Agent 冻结）

| 契约条 | 测试（`crates/pony-agent-core/tests/ask_persistence_acceptance.rs`） |
| :-- | :-- |
| P1 | `persisted_ask_roundtrip_survives_process_restart` |
| P2 | `state_machine_rejects_terminal_transitions` / `explicit_ignore_transitions_to_cancelled` |
| P3 | `cas_rejects_replay_after_restart` / `cas_rejects_version_mismatch_after_restart` |
| P4/P5 | `new_user_message_supersedes_pending_ask` |
| P6 | `no_wall_clock_expiry_path`（过期时间戳不再影响作答） |

L2-AT（`tests/acceptance/adversarial/ask_persistence/`）：并发双提交同一 request、重放旧 version、
非法状态越迁。全部确定性，禁止 sleep+概率。

## 9. C2-* 重冻清单（桌面契约）

本波**重冻面为 4 个既有冻结 spec**（由 Test Agent 迁移并重新红相锚定；Executor 只读）：

| 文件 | 迁移内容 |
| :-- | :-- |
| `tests/ask-pending-expiry.spec.ts` | 直接 import `isAskExpired` ⇒ 删除该导入与 TTL 断言，改「打开会话重建 / 忽略→Cancelled」 |
| `tests/ask-panel-expired.spec.ts` | C2-4「禁用 + 已过期」断言 ⇒ 改「无过期态 + 显式忽略」 |
| `tests/acceptance/wave17/tabular-nums-contract.spec.ts` | 倒计时 span 数量断言 ⇒ 移除倒计时后改为无倒计时断言 |
| `tests/ask-store.spec.ts` | `ask_expire` reconcile 段 ⇒ 改为 GC/忽略语义 |
| `crates/pony-agent-core/tests/ask_pending_contract.rs` | C1-4「过期后不再列出」⇒ 改为「新用户消息 Superseded 后不再列出」 |
| `crates/pony-agent-core/tests/ask_pending_adversarial.rs` | C4-3「时钟超期 fail-closed」⇒ 改为「重放/版本不符 fail-closed；时钟不参与判定」 |
| `tests/AskPlanPanel.spec.ts` | L103/L164 `ask-cancel-submit` 选择器 ⇒ `ask-ignore-submit`（随 AskPanel.vue 最小适配） |
| `tools/tui-audit/checks.py` | L1622 `ask.panel_visible` / L1683 `askloop.panel_visible` 的「esc 取消」正向断言 ⇒ 「esc 忽略此提问」（负向断言 L1653/1660/1724/1731 不动） |

新增断言（桌面）：打开会话重建面板、`Superseded` 后面板消失、忽略后不再出现。

## 10. 变更流水

- 2026-11-06 初版冻结（Lead，task-1）。
- 2026-11-06 §2 原地改写：`kind` 复用 `PendingControlRequestKind`；store 以 trait 形式冻结（与空桩一致）。
- 2026-11-06 §6/§9 原地改写：桌面真实挂载点为 `WorkspaceComposer.vue`（`AskPanel.vue` 零挂载）；重冻面扩为 6 个 spec。
- 2026-11-06 §2 原地改写：补构造入口 `open_ask_store` / `open_in_memory_ask_store`（测试与宿主唯一构造面）。
- 2026-11-06 §2/§6/§9 原地改写：trait 带 `Send + Sync` supertrait；TUI hint 改为「esc 忽略此提问」；重冻面扩为 8 个（含 tools/tui-audit/checks.py 与 tests/AskPlanPanel.spec.ts）。
