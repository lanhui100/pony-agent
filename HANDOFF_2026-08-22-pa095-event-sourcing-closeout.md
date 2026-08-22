# HANDOFF 2026-08-22 — PA-095 事件溯源收口实施交接

> 供新会话续接。当前任务：**PA-095 事件溯源收口（ADR 0008 承诺缺口关闭，P0）**。
> 任务卡：`management/task-system/03_TASKS/PA-095-event-sourcing-closeout.md`
> OpenSpec change：`openspec/changes/event-sourcing-closeout/`（tasks.md 已勾选进度）

## 一、背景纠偏（新会话必读）

用户最初认为进度在 "PA-080 实施阶段"，实际状态是：
- PA-079 / PA-080 **均已 Done 收口**（2026-08-13，已归档）
- PA-081（侧边栏树，P1）Ready 未开工
- 工作树的未提交改动属于 **PA-095**（2026-08-21 立卡），本轮全部工作围绕它展开

## 二、本轮完成的缺口实施（7 项中已完成 5 项主体）

### ✅ #1 同步 run_turn 接入事件流（P0）
- turn:started/completed 全路径发射 + 7 处失败路径 `emit_sync_turn_failed`
- `sync_turn_event_id` 一次性计算贯穿全路径（nanos+进程内原子计数器）
- 终态信封取自发射本身：`emit_event` 返回 `TurnEventEnvelope`，TurnResult/trace 信封与事件流同源序列号
- 端到端测试 `graph_sync_run_turn_persists_event_stream_end_to_end`

### ✅ #5 event_schema_version 落地（P1）
- store_metadata seed/校验/回填四分支；缺失 key 视为 v1 回填
- `load_turn_events_checked` fail loud + SessionStore `event_stream_degraded` 标记 + `is_event_stream_degraded` 查询入口
- `IGNORABLE_EVENT_TYPES` 空常量（读取路径消费：清单内跳过、清单外 fail loud）
- 测试 `event_schema_version_contract_four_branches`

### ✅ #4 StepStart/StepEnd + chunk step（P1）
- `TurnStreamEvent.step`（serde default wire 兼容）+ `emit_stream_event_with_step`
- followup 循环 hop 索引贯穿 delta 闭包；`build_step_start_event` / `build_step_end_events`（与 usage 相邻成对）
- **顺带修复生产数据丢失 bug**：followup delta 闭包 session_id 原传 None → followup chunk 全部丢失

### ✅ #2 append_turn 投影化（P0，阶段 B 登记）
- 事件源物化：从事件表读取最近 turn 的 UserMessage/AssistantMessage 文本 + reasoning
- 取数时序：persist 闭包与 flush requester 均 **commit 先于 clear**
- 增量持久化阶段 A：`2×AppendMessage + UpdateSessionMeta + 单会话行落库`，**save_store 整包写 = 0**（CountingBackend 结构性断言）
- characterization 快照防护 + 防架空探针（caller≠事件文本）
- **登记待办**：阶段 B 会话行级按 facet 增量、wal 尺寸基线采样

### 🔶 #3 对拍测试 + 豁免清单（主体完成，⚠️ 有一个场景修复未验证）
- 豁免清单常量化：`projection.rs::parity` 模块（EXEMPTIONS 9 项附理由 + AGREED_* 字段集 + EXEMPT_TIMELINE_KINDS + trace_field/timeline_entry_field 取值器）
- 生产形态 harness：control plane + SQLite 表读事件，重试吸收并行通道抢占
- **6 个对拍场景**：无工具 sync / 单工具流式 / failed hook-fail / 多 hop 流式 / **cancelled（修复未验证，见下）** / multi-turn
- 反向探针 4 个落地（clock/title 装饰/checkpoint_persist/failed 末 hop/streaming chunk 文本）
- **对拍驱动收敛修复 4 项**：投影 phase 设置、ProviderUsage provider 取名称、call_tool label/state 同构、存储侧 timeline 补 return_result 条目

## 三、⚠️ 中断时的进行中状态（最优先！）

**最后一个编辑已应用但未验证**：`crates/pony-agent-core/src/agent/execution_control.rs` 的
`register_turn` 改为**重注册时保留已设置的 stop 请求**（`carried_stop_requested_at_ms`），
并给 `ExecutionControlRegistry` 加了 `#[derive(Clone)]`（state 改 Arc 包裹）。

原因：cancelled 对拍场景需要"预注册 turn → request_stop → 再经 control_plane.start_turn_stream
（其内部 register_turn 无条件覆盖 checkpoint，丢失 stop）"。该修复让停止请求跨注册存活。

**下一步动作**：
```bash
cargo test -p pony-agent-core --lib parity_
```
预期 6 个 parity 测试全绿。若 cancelled 场景仍失败：
- 检查 `refresh_execution_checkpoint_projection` 是否基于 `stop_requested_at_ms` 正确推导 status
- 对照既有测试 `start_turn_stream_can_emit_cancelled_when_stop_requested_before_plan`（runtime/mod.rs ~10455）

然后跑全量确认 ExecutionControlRegistry 的 Clone/Arc 改动无回归（dispatcher 等有使用）。

## 四、实现期发现并修复的关键缺陷（避免重复踩坑）

1. **turn_id 碰撞**：动态 elapsed().as_nanos() 在 Windows 计时器精度（~100ns）下同会话连续两轮产生相同 id → trace 互相覆盖 + 事件 flush 失败。这是 monitor-summary 测试 flake 的根因。
2. **信封二次分配**：legacy `build_terminal_turn_event_envelope` 在事件发射间分配序号导致 TurnResult sequence 与事件流错位——改为发射返回信封直接复用；三处既有测试断言更新 seq=1→2。
3. **followup chunk 数据丢失**：delta 闭包 session_id=None → 空 session 缓冲永不 flush。
4. **append_turn 死锁**：物化 flush 需 sessions 写锁，而 persist_turn_outcome 持锁调用 append_turn → 自锁。修复：写锁前预提交 + requester try_write 化（锁忙转 Err 不挂死）。
5. **并行测试互相清注册表**：多播 sink 改守卫式（Drop 按 Arc::ptr_eq 移除自身）；materialize/e2e 测试用重试循环而非 clear。

## 五、已知模式与陷阱（新会话写测试必读）

- **全局 EVENT_PERSIST_REGISTRY 单槽**：任何 HostControlPlaneBuilder::build() 都会覆盖生产通道。依赖它的测试必须用 `register_event_persist_test_sink`（守卫式多播，不受抢占）或重试循环。
- **负载敏感既有 flake**：`pa093_fold_10k_events_stays_under_budget`（<2000ms 断言）等在并行高负载下偶发超时，与本次改动无关。
- **文件行尾 CRLF**：perl 单行替换注意 `\r\n`；大段插入用 Write 临时文件 + `cat >>` 或 Edit 工具。
- **孤儿进程**：多次超时后台 cargo test 会残留 rustc.exe/cargo.exe（持 target 锁）。清理：`taskkill //F //IM rustc.exe; taskkill //F //IM cargo.exe`（本轮已清过一次）。

## 六、验证基线（改动前状态）

- core lib **860** tests 全绿（4 连稳定，parity 6 项中有 3 项通过 + cancelled 修复待验证 + multi_hop 待首跑）
- tool_router_regression 13 / session_regression 5 / provider_registry_regression 8 / src-tauri lib 6 / vitest 399+10skip / cargo:check:shared 全绿
- ⚠️ 注意：execution_control.rs 与 parity 新增 3 场景的改动在上述数字之后，需重新验证

## 七、剩余工作（按优先级）

1. **立即**：验证 cancelled 修复（第三节），跑全量 ×4 确认稳定
2. **#3 收尾**：squash/fork-checkout/legacy-mixed 场景、build_context provider 元数据探针
3. **#6 cursor_version 退役**（P2）：值来源切 event_watermark；checkout/fork/squash 发射 history-control 事件保证水位单调（消除 ABA）；四类 command 冲突检测改水位比较
4. **#7 trace cache ref-only**（P2）：live 写入剥离 observation payload；flush 同事务回填 ref
5. **#8 归档事务**：归档 PA-091~093 changes + 本 change；canonical spec 同步
6. **并行待办**（登记在卡）：#2 阶段 B 行级 facet 增量、wal 基线采样
7. PA-081（侧边栏树，P1）在 PA-095 收口后启动

## 八、主要改动文件清单（均未提交）

| 文件 | 改动概要 |
|---|---|
| `agent/runtime/mod.rs` | #1 事件化 + #4 step + 信封捕获 + parity 测试模块 + 等价性/端到端测试 |
| `agent/turn_flow.rs` | emit_event 返回信封 + with_step 变体 + step builders + dispatch 多播 + flush 注册表 + IGNORABLE 常量消费 |
| `agent/session.rs` | append_turn 物化 + 增量持久化 + degraded 标记 + supports_turn_events + characterization/materialize 测试 |
| `agent/sqlite_session.rs` | schema 版本四分支 + checked load ignorable 分流 |
| `agent/projection.rs` | parity 常量/取值器 + phase 结算 + call_tool 同构 + settle 兜底收窄 |
| `agent/control_plane/mod.rs` | commit-before-clear + flush requester(try_write) + CountingBackend + 物化/e2e 测试 + load_turn_events_checked 读面 |
| `agent/execution_control.rs` | Clone + register_turn 保留 stop（**未验证**） |
| `agent/sse_adapter.rs` | TurnStreamEvent.step 字段补位 |
| `openspec/changes/event-sourcing-closeout/tasks.md` | 进度勾选 |
| `management/task-system/03_TASKS/PA-095-*.md` | 任务卡进展 |

## 九、验证命令速查

```bash
cargo test -p pony-agent-core --lib                 # core 全量（~860）
npm run cargo:test:regression                       # 回归三件套 13/5/8
npm run cargo:test:lib                              # src-tauri lib 6
npm run test:unit                                   # vitest 399+10skip
npm run cargo:check:shared                          # 共享检查
```
