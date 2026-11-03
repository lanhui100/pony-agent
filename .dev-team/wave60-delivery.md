# wave-60 交付说明：可中断停止（结构化 stop_reason + 立即应答 + 可中断读通道 + 宽限期收口 + 双击 Esc）

## 0. 一句话
停止不再依赖"等循环走到检查点"：点击即刻进 `stopping` 态（同一微任务）、后端同步回一条 `turn:stop_requested`、provider 停滞读按 250ms tick 观测取消、2s 宽限期后监督者强制收口并派发终态；桌面端与 TUI 共用同一套停止语义，且都支持双击 Esc。

## 1. 波次门禁与提交
| 项 | 值 |
|---|---|
| 分级 | B（跨 core/tauri/TUI/前端 4 面 + wire 字段追加 + 阻塞读通道；命中 blast 关键字 thread/lock/io） |
| 基线 commit | 7510042 |
| 协议冻结 | 6930ef1 `chore(protocol): freeze wave-60 stop contract & interface stubs` |
| 红相锚定 | f8769fe `test(wave-60): freeze red-phase acceptance tests`（tag `wave-60-red`） |
| 红相重锚 | c575243 `test(wave-60): migrate request_stop call shape to frozen 2-arg signature`（Lead 一票：纯形状迁移） |
| 契约 | `.dev-team/contracts/wave-60-interruptible-stop.md`（含补遗 v2 A1–A7 / v3 A8–A11） |
| NFR | `.dev-team/nfr-baseline-wave60.json` |
| 回归基线 | `.dev-team/regression-baseline-wave60.json` |

## 2. 交付内容（判据 → 实现落点）
| 判据 | 实现落点 |
|---|---|
| C1 应答语义/幂等/粘滞 reason | `crates/pony-agent-core/src/agent/execution_control.rs` |
| C2 立即应答事件（恰好一条） | `crates/pony-agent-core/src/agent/control_plane/mod.rs`（`stop_turn` + `register_turn_event_sink`）、`turn_flow.rs::emit_stream_stop_requested` |
| C3 终态携带 stopReason | `turn_flow.rs::emit_stream_cancelled` + `runtime/turn_control.rs` |
| C4 停滞读可中断（≤250ms tick，≤1s 退出） | `provider/responses_api.rs`（`ChunkPoll` 帧循环）、`provider/mod.rs` |
| C5 宽限期收口（幂等、锁外派发、线程不常驻） | `crates/pony-agent-core/src/agent/stop_escalation.rs` |
| C6 注册表按 turn_id + 顺序 request_stop→取消位→abort | `src-tauri/src/{lib.rs,tauri_adapter.rs,turn_task_registry.rs}` |
| C7 桌面端双态 + stopReason 文案 + 正文不被哨兵覆盖 | `src/stores/runtime.ts`、`src/lib/runtime/status.ts`、`src/types/runtime.ts`、`src/components/HomeWorkspace.vue`、`src/components/chat/WorkspaceComposer.vue` |
| C8 双击 Esc（桌面 TS + TUI Rust 同判据） | `src/lib/runtime/stop-sequence.ts`、`crates/pony-agent-core/src/agent/stop_sequence.rs` |
| C9 TUI 事件驱动终态（无本地伪造） | `crates/pony-agent-core/src/bin/tui.rs`（`apply_stop_requested/apply_stop_terminal` + sink 两分支） |

## 3. 机器收据（Lead 亲自复跑，非转述）
| 门禁 | 命令 | 结果 |
|---|---|---|
| G1 | `cargo test --manifest-path crates/pony-agent-core/Cargo.toml --test stop_contract_wave60 --test stop_escalation_wave60 --test stop_interruptible_read_wave60` | Exit 0 · 7+9+3 passed |
| G2 | `cargo test --manifest-path src-tauri/Cargo.toml --test stop_registry_wave60` | Exit 0 · 3 passed |
| G3 | `cargo check --manifest-path crates/pony-agent-core/Cargo.toml --all-targets` | Exit 0（1 条测试内 dead_code 警告） |
| G4 | `cargo test --manifest-path crates/pony-agent-core/Cargo.toml --lib` | Exit 0 · 1209 passed |
| G5 | `cargo test --manifest-path crates/pony-agent-core/Cargo.toml --test tui_batch2_regression --test tui_batch3_regression` | Exit 0 · 12+16 passed（n14/n15 未改） |
| G6 | `npx vitest run tests/stop-stop-turn.spec.ts tests/stop-sequence.spec.ts tests/runtime-store.spec.ts tests/HomeWorkspace.spec.ts tests/ask-store.spec.ts tests/acceptance/stage-1-cancel-sentinel.spec.ts` | Exit 0 · 6 files / 223 passed |
| G7 | `npx vue-tsc --noEmit` | Exit 0 |
| G8 | `npm run contract:check` | Exit 0（Rust 37 键 = TS 37 键） |
| G9 | `audit-diff.sh --base 7510042` · `check-declaration.sh .` | Exit 0 · 结论 PASS |

## 4. 全量前端套件的环境限制（必须是可复核陈述，不是借口）
本仓 `tests/acceptance/pds-streaming/*` 的 V10 元测试会**子进程递归跑全量 vitest**；该套件在本波验收窗口内单次全量跑出现 >20min 未收敛，故 Lead 中止并以**定向套件**（G6，覆盖本波全部前端触点 + 既有取消哨兵迁移用例）作为前端门禁收据。
另：**本工作区存在并发 wave-61 会话**（09:24 起写入 `src/components/HomeStatusPanel.vue`、`tests/acceptance/wave61/`、`tests/e2e/wave61-*` 等，并已产生提交 069587a / 1a16a51）。wave-61 红相测试在我方重复跑全量时必然红（23 条，逐条属状态面板圆环迁移与 v10 连带），与本波零交集；本波提交严格按路径定向暂存，未纳入任何 wave-61 文件。

## 5. 已裁决偏离（契约 §6-ter）
| # | 偏离 | 处置 |
|---|---|---|
| A8 | `register_turn_event_sink` 用`Arc<S: TurnEventSink+Send+Sync>` 而非 `Arc<dyn>` | 接受（`TuiTurnSink` 持 mpsc Sender，非 Sync，dyn 不可满足） |
| A9 | 未引入第二套取消 token；取消源复用 ExecutionControlRegistry | **SCOPE-DEFERRED**：长工具执行期外部进程不被立刻杀掉，循环在下一检查点退出，UI 由宽限期(2s)先解锁；已登记 backlog |
| A10 | emitter 由 core 绑定到同一 sink | 接受 |
| A11 | 测试文件 1 条 dead_code 警告 | 接受 |
| A12 | `turn:cancelled` 不再写 `payload.text`（哨兵不覆盖正文） | 接受（§0 非目标 2：落库/历史语义不变） |

## 5-bis. 对抗审查与复核（三道外部审查 + 一次定向复核）
| 通道 | 结果 | 处置 |
|---|---|---|
| L3-A 并发/调度/状态流转 | FAIL · 1 BLOCKING（emitter 表无配对回收）+ 4 ADVISORY | BLOCKING 已修（`StopEscalation::unregister_emitter` + `unregister_turn_event_sink` 调用 + 单测）；ADVISORY 中"sink 注册时序竞态"升级为必修并已修（`lib.rs:102/131/160` prepare→register_turn_sink→spawn） |
| L3-B 契约/协议/安全 | PASS · 8 ADVISORY（含 TS `kind` 缺 `stop_requested`） | 该条已修（`src/types/runtime.ts:1108-1112`）；其余接受（TUI `let _ = tx.send` 沿用既有模式等） |
| L3-P 生产六问 | FAIL · 2 BLOCKING（新业务错误行裸 eprintln；spawn 失败无同步收口）+ 4 ADVISORY | 已修：改 `log_json_with_fields`；spawn 失败走同一 `escalate_now` 真收口 |
| 定向复核（recheck） | **PASS** · 4/4 FIXED · lint Exit 0 | `.dev-team/review-wave60-recheck.json` |

三份审查产物 lint 均 Exit 0（`dev-review-lint.py` + 显式 audited_scopes）。审查覆盖并集 = 本波全部 19 个变更文件（`runtime/tests.rs` 为纯派生物，由 Lead 复核）。

## 6. 残余风险与后续
1. **A9 缺口**：`bash`/MCP 等长工具执行期间无强杀；建议下一波把 per-turn token 注入 dispatcher/工具执行器（已登记）。
2. **并发会话写冲突**：`src/components/chat/WorkspaceComposer.vue` 同时被 wave-60 与 wave-61 声明写域；本波改动仅 8 行（stopping 禁用态读 store），收口前需确认 wave-61 合入后该行为仍在。
3. **注册超限残留**（recheck ADVISORY）：`lib.rs:102-103` 先注册 sink 后 spawn，若任务注册超限被 abort，`TaskCleanupGuard` 未创建 ⇒ 该轮 sink/emitter 残留（有界于失败次数）。下一波可把 sink 注册移入 spawn 成功回调。
4. **重复终态**：监督者与循环各可能发一条 `turn:cancelled`；前端按 `activeTurnId` 门禁丢弃，TUI 同法；core 不保证全局单发（契约 C5 已注明）。
