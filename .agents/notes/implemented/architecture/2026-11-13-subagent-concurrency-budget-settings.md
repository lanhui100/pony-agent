# Agent Note: subagent 并发上限改为可配置（默认 4、硬顶 8、设置页可调）

Status: implemented

## Problem

事故实测（2026-11-12 dev 主机）：4 个 TUI 会话各挂多个 subagent，单进程 RSS 7.7GB、
VmHWM 8.5GB，而主会话 transcript 仅 27MB —— 多个 subagent 的完整上下文与工具输出在同一
进程内并行驻留，内存峰值被并发数线性放大。

代码侧当时已有全局并发门（`SUBAGENT_MAX_CONCURRENT = 2`，`subagent_busy` fail-closed），
但存在两个结构性问题：

1. **上限是编译期常量**：用户无法按机器内存调整。3 已不够用（主会话 + 2 队友即占满），
   而 8GB 机器与 32GB 机器被迫用同一个数字。
2. **口径分散**：上限只在 `subagent_runtime` 内部出现，设置层没有对应字段，落盘/读回/
   运行时注入三处都无法表达「用户配置的并发预算」。

## Decision

1. **默认值 4**（`DEFAULT_SUBAGENT_MAX_CONCURRENT`）。评估口径：单 subagent turn 峰值
   ≈{完整上下文 + 工具输出 + 图片解码}，4 路并发在 16GB 机器上约 6–8GB，仍留主会话与
   系统余量；3 在「主会话 + 2 队友」场景无并行余地（用户明确反馈「3 太少」）。
2. **硬顶 8**（`SUBAGENT_MAX_CONCURRENT_CEILING`）。设置值再大也 clamp 到 8，防止误填把
   进程推向 OOM；32GB 级机器可用满 8。
3. **单一真源**：`normalize_subagent_max_concurrent(Option<usize>) -> Option<usize>` 同时被
   设置层（`AppSettingsStore::save_view`，落盘前规范化并回显生效值）与运行时门
   （`InflightGuard::acquire`，每次 acquire 重读）使用；`set_subagent_max_concurrent` 是
   唯一写入口。
4. **设置面**：`AppSettings.subagent_max_concurrent`（落盘键 `subagentMaxConcurrent`，
   `None` = 未配置）；Tauri `load_app_settings` / `save_app_settings` 保存即注入（无需重启）；
   设置页「通用」新增数字输入（1–8，默认 4，失焦/回车保存，越界以后端回显为真值）。
5. **不改超限语义**：仍返回 `subagent_busy`（既有 wave37 t3-J 契约）。`drive_teammate_turn_in`
   的失败补偿会把消息原序回滚入队，因此队友路径天然具备「下次重试」的排队语义；本次不引入
   全局等待队列，避免改变已冻结的错误契约。

## Alternatives considered

- **A. 只把常量从 2 改成 4**：落选。仍无法按机器内存调整，8GB 机器会被迫接受 4 路峰值。
- **B. 超限排队等名额**（账本原建议之一）：落选。同步 `subagent` 工具调用会从「立即失败」
  变成无界等待，改变 wave37/wave61 已冻结的 busy 契约；队友路径已有 requeue 重试语义，
  全局队列收益不抵契约变更成本。若未来需要，应作为独立决策并同步迁移 busy 断言。
- **C. 并发上限放进 env**：落选。桌面用户不可发现、不可热更，且与设置面来源优先级
  （settings > env > 默认）并行维护两套入口。
- **D. 每个 subagent 独立 token 预算**（账本原建议之二）：落选（本波范围外）。token 预算
  需要请求面预估与压缩联动（B116 的下一跳预估），与并发上限解耦验证更安全，另行立项。
- **E. 默认 4 + 硬顶 8 + 设置页可调（采纳）**：一个数字有界、可解释、可回滚，越界不拒绝
  整包保存（clamp 后回显），兼顾默认安全与高配机器。

## Consequences

- 默认并发从 2 提升到 4：单进程峰值内存上限相应上升，8GB 机器用户应下调到 1–2；
  设置页有范围提示，但**不做机器内存自适应**（留给后续按可用内存预估的独立决策）。
- 上调上限对在途 turn 无影响，只影响后续 acquire；下调同理（不会取消已在跑的 turn，
  引擎无回合内 stop API）。
- 单测引入进程级静态上限的串行守卫（`LIMIT_TEST_GUARD`）并在 Drop 恢复默认值；跨测试二进制
  各自独立进程，无共享污染。
- 旧常量 `SUBAGENT_MAX_CONCURRENT` 已删除（消费者仅内部与 wave37，同批迁移），
  wave37 t3-J 改为按生效上限占满许可。
