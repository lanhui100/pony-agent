# 元框架复盘（Wave-60：可中断停止）

> 对象：wave-60 —— 停止按钮延迟的根因修复（结构化 stop_reason / 立即应答 / 可中断读通道 / 宽限期收口 / 双击 Esc）。
> 交付：`.dev-team/wave60-delivery.md`；契约：`.dev-team/contracts/wave-60-interruptible-stop.md`（含补遗 v2/v3）。
> 团队：Lead + protocol-agent + test-agent + executor-core + executor-surface（常驻 5）+ 5 次 Subagent（2 Explorer、L3-A/B/L3-P、recheck）。
> 交付区间：`7510042` → `680499c`（tag `wave-60`；红相 tag `wave-60-red` = `c575243`）。
> 审查收据：`.dev-team/review-wave60-a.json`、`review-wave60-b.json`、`review-l3p-wave60.json`、`review-wave60-recheck.json`（均 lint Exit 0）。

## 1. 通信拓扑与信噪比（Topology & Noise）

- **拓扑正确**：Protocol/Test/Executor×2 是有状态连续体（桩→红→绿→复审），落常驻 Teammate（峰值 5 ≤ 8）；
  两路 L3 + L3-P + recheck 是无状态投影，落 fresh Subagent。
- **写域正交做到位**：core-rust 与 surface 并行，`crates/.../src/agent/**` ‖ `src/**`+`tui.rs` 零交叉；
  唯一共享文件 `WorkspaceComposer.vue` 最终被**排除出提交**（见 §3）。
- **Token 浪费点**：Lead 在收敛阶段用 3 次 `sleep` 轮询审查产物（wait_agent 对 Subagent 不生效，报 no-active-peer），
  属可避免空转；改进：把"等 Subagent"换成"用 job_output/文件 mtime 事件驱动 + 期间做真活"。
- **信噪比好**：全部回执为「结论 + file:line + Exit Code」，无套话；executor-core 主动上报 3 处偏离并请求裁决，
  属正面行为（而非先斩后奏）。
- **Explorer 价值实证**：两路 E1 交付把 `phaseLabel 零消费者`、`stopRequested 死变量`、TUI sink `_ => {}` 丢弃
  `turn:cancelled`、`resolveAgentRuntimeStatus` 在 `!isSubmitting` 恒 null 四条"反直觉事实"提前钉死，
  直接改写了 C7/A2 的验收口径 —— 若靠 Lead 边读边推，这四条很可能被漏到绿相才发现。

## 2. 门禁穿透与误杀率（Gate Penetration / False Negatives & Positives）

- **真实拦截（L3 价值）**：3 路审查共 4 条 BLOCKING，全部是机器门禁（34 条验收用例全绿）无法发现的：
  ① emitter 表无配对回收（`register_emitter`/`unregister_turn_event_sink` 不成对，按 turn 无界增长，附 `strong_count` 反例）；
  ② 新增业务错误行裸 `eprintln!`（违反 NFR json+trace_id）；
  ③ spawn 失败**注释声称**同步收口实际没有 ⇒ C5 终态丢失（注释与实现背离，机器门禁不比对注释）；
  ④ sink 注册晚于 `register_turn` 的时序竞态（窗口内 stop 被接受 ⇒ 零 `turn:stop_requested`、监督者无 emitter）。
  四条均修复并经 recheck PASS（4/4 FIXED，lint Exit 0）。
- **门禁穿透（流程，Lead 自身）**：① 首次给审查者的 `--scopes "a,b,c"` 逗号串被脚本当作单一 scope ⇒
  三份产物 lint 一度全部不闭合，是 Lead 的调用错误而非产物缺陷，已改显式多参重跑；
  ② 红相首版含 1 参/2 参 `request_stop` 形参自相矛盾（两份测试互相矛盾），由 executor 阻塞上报才暴露 ——
  根因是 Lead 在冻结契约时未把"签名唯一性"写成可机械核对的条目。
- **误杀**：0 条 BLOCKING 误判；L3-A 提的"stopping 期间 Ctrl+C/Esc 被吞"与 L3-B 的"TUI `let _ = tx.send`"
  经裁决为 ADVISORY 接受（宽限期 2s 内由监督者解锁；后者沿用文件既有模式）。
- **门禁自身假绿（新发现）**：本波全量 `npx vitest run` 因 `tests/acceptance/pds-streaming` 的 V10 元测试
  **子进程递归跑全量**而在本工作区 >20min 不收敛，只能降级为定向套件（223 passed）+ 明确声明。
  该元测试在并发波次同存的工作区里不具备可重复性，建议登记为专项治理项。

## 3. 分工契约与隔离有效性（Contract Isolation）

- **测试分权守住**：Executor 在红相阶段两次请求"改测试形状"，均被拒绝；改由 Test Agent 执行**纯形状迁移**
  并由 Lead 一票重锚（`c575243`），判据零改动 —— 这是本波最重要的流程胜利，否则"测试跟着实现走"的口子一旦开了就再也关不上。
- **写域冲突真实发生**：本工作区存在**并发 wave-61 会话**（09:24 起同 branch/main、同工作树，提交 069587a/1a16a51/b1d62f3/52a7a21）。
  冲突点三处：`provider/mod.rs` 测试模块、`control_plane/mod.rs` 测试断言、`WorkspaceComposer.vue` 圆环移除。
  处置：① 提交前按 **hunk 过滤**（剔除含 wave-61 语义的 hunk，`git apply --cached`）只提交本波内容；
  ② `WorkspaceComposer.vue` 整体**排除**（改为接受"stopping 期间主按钮不禁用"，后端幂等兜底）；
  ③ 全程未使用 `git add .` / `git reset --hard` / `git clean`。
- **共享工作区的代价**：`tests/**` 被并发波次红相污染后，本波无法用"全量绿"作为门禁，只能用定向套件 + 归因说明；
  这是环境事实，不是验收降级 —— 已在交付 §4 写入可复核陈述。
- **滚动契约账本**：契约文件补遗 v2（A1–A7）/v3（A8–A11）共 11 条裁决全部原地登记，无追加式膨胀。

## 4. 元协议迭代建议（Self-Evolving Protocol）

1. **红相冻结前增加"签名唯一性"机械检查**：本波因两份测试对同一方法形参自相矛盾而浪费一次往返。
   建议在 `dev-review-lint` 或独立小脚本中对 `tests/**` 提取被测符号调用形状，若同符号出现不同 arity ⇒ FAIL。
2. **`--scopes` 调用形态写进 skill 正文**：逗号串会被 `nargs="*"` 当作单一 scope，导致"必然不闭合"。
   建议 skill 示例固定为「空格分隔多参」，并在 lint 内对含逗号的 scope 直接报错提示（而非静默不闭合）。
3. **并发会话写冲突协议**：建议 dev-team 在开波前强制探测"同工作区其他波次的活跃写域"
   （如扫描近 60 分钟内 `.dev-team/contracts/wave-*.md` 的 mtime + `git status` 归属），命中即要求 lead 采用
   worktree 隔离或进入 hunk 隔离模式（本波是事后才发现的）。
4. **Subagent 等待原语**：`wait_agent` 对 Subagent 返回 `no-active-peer`，导致 Lead 只能轮询。
   建议 skill 明确"等 Subagent 用 job/文件事件 + 期间做真活"，并禁止 `sleep` 轮询。
5. **验收口径的"通道选择"要前置**：本波 A2 的教训——"终态文案落到哪个通道"必须在契约阶段用 Explorer 事实钉死
   （`phaseLabel` 零消费者这类事实），否则测试会写出"永远不可能成立"的断言。
