# wave-62 交付说明：工具级取消（清偿 wave-60 §6-ter A9）

## 0. 一句话
停止不再只是"让循环在下一检查点退出"：turn 级取消令牌已下沉到工具执行层，`Run`（子进程）在取消后 **≤1s** 被真杀、返回 `aborted` + `tool_cancelled`，副作用不再发生；`composite` 子级取消不再毒化整轮。

## 1. 波次元数据
| 项 | 值 |
|---|---|
| 分级 | B（跨 dispatcher/tool_runtime/governed_executor/tools/runtime + 并发与进程生命周期；命中 blast 关键字 thread/spawn/lock/Command） |
| 隔离 | worktree `/home/dm/wt/pony-wave62`，分支 `wave-62/tool-cancel`（主工作区被并发 wave-61 占用，禁止在其上写） |
| 基线 | `60e7cb8`（含 wave-60 全部交付） |
| 协议冻结 | `4c0fdc4`（字段面）+ `1266e49`（补遗 v2：C5/桥接/融合/清理） |
| 红相锚定 | `d3defb0` → 复锚 `e4bd02e`（tag `wave-62-red`） |
| 契约 | `.dev-team/contracts/wave-62-tool-cancel.md` |

## 2. 判据 → 实现落点
| 判据 | 落点 |
|---|---|
| C1 每轮一个令牌，仅 accepted 才 cancel，未知 turn→None，重注册沿用实例 | `execution_control.rs:143/188-197/260` |
| C2 dispatcher 观测 turn 令牌；`None` 行为不变；子级 cancel 不毒化整轮（自有 OR turn） | `dispatcher.rs:828-836 / 845-851` |
| C3 `Run` 在取消后真杀子进程并返回 `aborted`/`tool_cancelled`；取消优先于 timeout | `tools.rs:2969-3110`（`CommandCancelWatcher`：10ms 分片、≤100ms 节拍、Drop join）+ 轮询 20ms |
| C4 不动 wave-60 冻结测试 | `git status` 无 `tests/stop_*_wave60.rs` 改动 |
| **C5（端到端，补遗 v2）** 生产默认执行器路径必须真杀 | `tool_runtime.rs:52-55`（`PrimitiveToolHandlerRequest.cancellation`，`#[serde(skip)]`）+ `governed_executor.rs:368-370` 透传 + `dispatcher.rs:440-452/1176-1180` 取消码保真 |

## 3. 机器收据（Lead 亲跑）
| 门禁 | 结果 |
|---|---|
| `--test wave62_tool_cancel` | Exit 0 · 7 passed |
| `--test wave62_tool_cancel_e2e` | Exit 0 · 2 passed（aborted + tool_cancelled + ≤1000ms + marker 未生成） |
| wave-60 三件套（stop_contract/stop_escalation/stop_interruptible_read） | Exit 0 · 7/9/3 passed |
| `cargo test --lib -j 1 -- --test-threads=2` | Exit 0 · 1210 passed（基线一致） |
| `cargo check --all-targets -j 1` | Exit 0 |

## 4. 审查与复核
| 通道 | 结论 |
|---|---|
| L3-A 并发/资源 | **FAIL** · 1 BLOCKING（生产桥接丢令牌，附可跑反例：cancel 后 1217ms 返回、marker 已生成）+ 3 ADVISORY |
| L3-B 契约/兼容/NFR | **FAIL** · 同根 BLOCKING（C3 判据在生产链路不成立）+ 2 ADVISORY |
| 修复后定向复核 | 见 `.dev-team/review-wave62-recheck.json` |

**两路独立复现同一 BLOCKING** 是本波最有价值的拦截：冻结的 C3 用例直连 `ToolRouter`，恰好绕过生产桥接，7/7 全绿却与真实链路不符。补遗 v2 因此新增 **C5 端到端判据**（生产执行器 + marker 副作用断言），并以红相复锚（`e4bd02e`）。

## 5. 已知局限（不阻断）
1. **Unix 非进程组杀**：沿用 `ProcessManager` 既有 `child.kill()` 语义，孙进程可能存活（timeout 分支同源，属上游既有局限）。
2. **令牌表不回收**：`ExecutionControlRegistry.tokens` 随 turn 增长（与既有 `turns` 表同生命周期），会话级有界。

## 6. 合并状态
本波在**隔离分支**交付（`wave-62/tool-cancel`），**尚未并入 main**：主工作区仍有并发 wave-61 未提交改动，`git merge` 会破坏其工作树。合并动作待 wave-61 收口后由 Lead 执行（`git merge --no-ff wave-62/tool-cancel`）。
