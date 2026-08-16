# PA-088 Spec: 会话存储新写路径（原子写 + refs + 轻量投影）

> 修订版 v7（2026-08-15）：采纳 consultant v6 终审——新增 **P0-MIX 分派矩阵**（全局 trace mode ×
> session authority × mutation 类型的运行时分派），补 P0-1 三态 API / P0-2 JSON 表示 / P0-5 canonical
> identity（RFC 8785）/ P0-6 authority 默认值。混合模式测试为放行门禁。

## 1. 背景与目标

**背景**：`sessions.db` 最大会话 43.77MB（historyNodes O(n²) 冗余）。加载会话时前端 JSON.parse 卡死。止血已完成。

**目标（本卡：新写路径）**：
1. 启用 WriteSeparate + 原子写入（新会话 trace 落表，blob 不膨胀）
2. HistoryNode 持久化剥离 trace（内存保留快照），轻量引用
3. 轻量节点投影（全路径）
4. **分派矩阵**：存量（LegacyBlob/DualWrite）与新会话（Authoritative）在同一 backend 安全共存

**非目标**：存量迁移 → PA-090；多版本/拆表/写队列/分页 → PA-089。

## 2. 关键决策

### 2.1 latest-wins（产品已确认）
### 2.2 剥离只在序列化层 + materialize 仅 load_store
### 2.3 refs 数据模型：`Option<Vec<TurnTraceRef>>`
- `None` = legacy/缺失（可内嵌兜底）；`Some([])` = authoritative 空（禁兜底）
- `TurnTraceRef = { turn_id, updated_at_ms, version? }`

### 2.4 原子 API 三态（P0-1 闭合）
```rust
enum AtomicPersistOutcome { Succeeded, Unsupported }  // Err 走 Result
fn persist_session_atomic(&self, p: AtomicSessionPersist) -> Result<AtomicPersistOutcome, String>
```
- `Ok(Succeeded)`：本次保存完成，调用方**不得**再走旧路径
- `Ok(Unsupported)`：仅限 `LegacyBlob`/`DualWrite` 会话（且全局模式 WriteSeparate）允许走**一次**兼容路径；**Authoritative 会话收到 Unsupported 视为错误（fail closed）**
- `Err(e)`：真实失败——回滚 + 传播 + 标记 `dirtySessions[session_id]`（内存集合，下次保存重试）；**禁止**任何 replace/upsert/save_store 补偿写
- 失败传播：`save_session_to_backend` / `persist_session_and_trace_change` 改为返回 `Result`（或等价），不再 void 吞错

### 2.5 轻量投影（P0-3）
所有 SessionSnapshot 构造路径节点清空 trace；选中节点 trace 只放顶层；`serde_json::to_vec(SessionRuntimeView)` < 5MB 硬断言。

### 2.6 authority 跨端协议字段（P0-6 闭合）
- `SessionSnapshot.turnTraceAuthority: "host" | "cache" | "absent"`——**始终序列化**（不用 skip_serializing_if）；旧客户端缺失 → 默认 `"absent"`
- 前端 `mergeRuntimeViews`：host authoritative + `[]` → **覆盖缓存**；host 非权威（cache/absent）→ 缓存 fallback；host 权威 + 非空 → host 优先

## 3. 技术方案

### 3.0 分派矩阵（P0-MIX，核心）

**写入口分派**（每个写入口同时判断全局模式 + session 状态）：

| 全局模式 | Session 状态 | 写路径 | 说明 |
|---|---|---|---|
| WriteSeparate | 新 session（无旧数据） | **原子包（直接 authoritative）** | 新会话无历史负担，直接表权威 |
| WriteSeparate | LegacyBlob（存量） | 首次写：单事务**晋升 DualWrite**（blob+table 双写）；不自动 authoritative | 保持可读写；等 PA-090 迁移 |
| WriteSeparate | DualWrite（存量） | blob + table 原子双写（**保持现状**） | 不因全局模式静默改单写 |
| WriteSeparate | TraceTableAuthoritative | **原子包**；blob 只存 refs；save_store 跳过 table 写 | 本卡新会话路径 |
| Off / DualWrite（全局） | 任意 | 旧路径（save_store / upsert_session） | 兼容窗口 |
| 回滚到 Off | Authoritative | **先 materialize 回填 blob → 清 authoritative → 再切 flag**（阻塞式） | 阻止裸切 |

**读路径**：按 session 状态分派——Authoritative 表读（按 refs）；DualWrite/LegacyBlob dual-read（merge_trace_history 已有逻辑，494-496）。

**禁止项**：
- 全局 WriteSeparate 下，LegacyBlob 会话不得只写 blob 后残留旧 table rows（dual-read 时旧 table 覆盖新 blob）→ 首次写必须事务晋升 DualWrite
- DualWrite 会话不得因全局模式静默改只写 table
- save_store 不得覆盖 Authoritative 会话的 table（跳过）

### 3.1 原子写入包
```rust
struct AtomicSessionPersist {
    session_id: String,
    stripped_blob: SessionState,       // trace 已剥离 + refs 写入
    trace_union: Vec<TurnTraceRecord>, // 当前状态全量
    top_level_refs: Option<Vec<TurnTraceRef>>,
    node_refs: HashMap<String, Option<Vec<TurnTraceRef>>>,
}
```
SqliteSessionBackend（WriteSeparate + Authoritative）实现：单事务 merge_latest 写表 → 写 blob + refs → 置 authoritative；失败全回滚。调用链：save_to_backend / persist_session_and_trace_change / save_session_to_backend 的 Authoritative 分支走原子包；其余走分派矩阵。

### 3.2 refs 维护 + JSON 表示（P0-2 闭合）
- JSON 表示契约：
  | Rust | JSON | 语义 |
  |---|---|---|
  | `None` | 字段缺失 / `null` | legacy / 缺失（内嵌兜底） |
  | `Some([])` | `[]` | authoritative 空（禁兜底） |
  | `Some(v)` | 数组 | 有序引用（materialize 按此序） |
- 维护点全量：record_history_node / commit_history_node_from_live_state / sync_latest_history_node / ensure_history_graph / hydrate_session_from_node / checkout / fork
- materialize（load_store）：按顶层/节点 refs 查表（批量防 N+1）；`None`+LegacyBlob 内嵌兜底；`Some([])` 空；缺失 ref 空+告警+degraded 计数

### 3.3 轻量投影（全路径）
snapshot_from_state（选中/普通）、context.rs、runtime/mod.rs：节点清空 trace、保留 turnId/refs/摘要；选中节点 trace 只放顶层；authority 字段始终输出。

### 3.4 merge 与幂等
- `trace_version` 列幂等 ALTER TABLE
- `merge_latest_traces_tx`：条件 upsert（excluded.updated_at_ms > current 覆盖；相等保留表）
- **terminal 16 格矩阵**（mutation 携带 phase；**所有写入口**——merge/upsert/replace/terminal update 统一过矩阵）：
  | 当前\新 | running | done | error | cancelled |
  |---|---|---|---|---|
  | running | 更新 | 更新 | 更新 | 更新 |
  | done | 拒绝 | ts 更新覆盖 | 拒绝 | 拒绝 |
  | error | 拒绝 | 拒绝 | ts 更新覆盖 | **拒绝**（平级） |
  | cancelled | 拒绝 | 拒绝 | 拒绝 | ts 更新覆盖 |
- **hook canonical identity（P0-5 闭合）**：`SHA-256(turn_id ‖ hook_point ‖ hook_order ‖ hook_name ‖ JCS(structured_result))`——**JCS = RFC 8785 规范 JSON 序列化**（保持跨重试稳定）；identity hash 持久化到 trace 扩展字段；同 identity 事务内去重 no-op；历史无 hash hook 视为不同（不误去重）
- `replace_exact_traces_tx` 保留（DualWrite/Off 兼容路径用）；prune：软上限 128 + refs 保护（扫描顶层+全部节点 refs）+ fail closed + refs 写入后执行

### 3.5 前端
- types/runtime.ts：HistoryNode 加 `turnId`/`turnTraceRefs`；SessionState 顶层 refs + authority 字段
- history.ts：historyNodeStableTurnId 优先 turnId；cloneHistoryNodes 复制 refs
- sessions.ts mergeRuntimeViews：host 权威优先（含空数组覆盖缓存）
- checkpoint/回滚：依赖修复后 turnId；trace 面板读顶层

## 4. 影响面与依赖
后端：session.rs / sqlite_session.rs / graph_projection.rs / history_commands.rs / context.rs / runtime/mod.rs。前端：types/runtime.ts / history.ts / sessions.ts / runtime.ts。依赖：止血完成；PA-090 依赖本卡；PA-089 依赖本卡 + PA-090。

## 5. 任务拆解
| # | 子任务 | 负责 | 依赖 |
|---|---|---|---|
| 1 | 分派矩阵（全局×session×mutation 分派 + 晋升规则） | backend-dev | 无 |
| 2 | 原子写入包（三态 API + 事务 + 失败传播 + dirty） | backend-dev | 1 |
| 3 | refs 数据模型（Option<Vec> + JSON 契约）+ 全构造点 + materialize | backend-dev | 2 |
| 4 | merge/terminal 矩阵/hook identity/prune | backend-dev | 2 |
| 5 | 轻量投影 + authority 字段 + payload 实测 | backend-dev | 3 |
| 6 | 前端兼容（Option refs/turnId/host 优先） | frontend-dev | 3 |
| 7 | 测试补充（含混合模式矩阵测试） | tester | 全部 |

## 6. 风险、回滚与迁移
- P0：分派矩阵错误（混合模式测试门禁）；原子失败补偿写（三态 + 故障注入）
- P1：latest-wins（已确认）；hook 幂等（JCS identity）
- P2：旧数据兼容窗口（LegacyBlob/DualWrite 保持可读写）
- 回滚：切回 Off 前 Authoritative 会话 materialize 回填（阻塞式）；备份 sessions.db + WAL + SHM

## 7. 测试计划
1. **分派矩阵全组合**：WriteSeparate × {新/Legacy/DualWrite/Authoritative} × {save_store/upsert/mutation/terminal/hook} 的行为断言（混合模式测试）
2. **晋升规则**：LegacyBlob 首次写 → 单事务晋升 DualWrite；DualWrite 不静默改单写；Authoritative 收到 Unsupported 报错
3. **原子三态**：Succeeded 不触发旧路径；Err 无补偿写 + dirty 标记 + 重试（故障注入 + 操作计数）
4. **save_store 不覆盖**：Authoritative 全量保存后 table 完整
5. **Option<Vec> JSON 契约**：缺失/null/[]/非空四态序列化 + materialize 行为
6. **分支污染**：fork 后重启主分支顶层不含 fork trace
7. **terminal 矩阵**：全 16 格 + 写入口全覆盖（merge/upsert/replace 一致）
8. **hook 幂等**：同 JCS identity 重复 no-op；不同共存；JCS 跨重试稳定
9. **payload 实测**：serde_json::to_vec(SessionRuntimeView) < 5MB 硬断言
10. **host 优先**：authoritative + [] 覆盖缓存；absent 默认值
11. **File/Memory backend 编译测试**
12. core lib 全量 + 前端 vitest 全量

## 8. 验收标准
1. 新会话：trace 落表、blob 仅 refs、无丢失；存量会话（Legacy/DualWrite）可读写
2. 分派矩阵正确（混合模式测试全过）
3. 原子三态正确 + 失败无补偿 + dirty 重试
4. 运行中 checkout/fork 语义不变；重启 materialize 正确
5. payload < 5MB
6. 前端 checkpoint/回滚正常；host 优先含空数组覆盖
7. core lib + 前端 vitest 全绿

## 9. 审核记录
v1-v5：迭代收敛（WriteSeparate 缺陷 → 方案 Y → 原子写 → 接口契约 → 拆迁移）。
v6（终审不通过）：新增 P0-MIX 分派矩阵要求 + P0-1/2/4/5/6 细节。
v7（2026-08-15）：补 3.0 分派矩阵、2.4 三态 API、2.6 authority 默认值、3.4 JCS identity。待最终复审。
