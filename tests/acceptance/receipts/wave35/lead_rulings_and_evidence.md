# wave35 验收裁决记录与物理证据(Test Agent 产出;写域 tests/acceptance/receipts/wave35/)

时间: 2026-10-18 11:10 (+08:00) / 基线 commit: f70b1f3 / 门禁: tests/acceptance/remediation/wave35_gate.sh

## (a) 全量跑协议 —— 不并行,收据由我产出
- Lead 的后台全量仍在飞行中(pid 2713611/2713620,`timeout 1200 cargo test --lib` → `full_lib_run.log`)。
  **本 Agent 未启动第二个全量**,避免 cargo target 锁串行化。
- Lead 那份 log 的即时完整性机械校验(只读,未占用 cargo):
  `bash tests/acceptance/remediation/wave35_gate.sh --verify-log tests/acceptance/receipts/wave35/full_lib_run.log`
  → Exit 1,`[LOG_NO_SUMMARY]`(无 'test result:' 终态汇总)+ `[LOG_HANG_MARKER]`(含 'has been running for over 60 seconds')
  ⇒ 该文件此刻为**未完成/挂起中**状态,不作为 G3/G4 收据;待其结束后复校。
- Barrier 后我方动作(已就绪,已用 stub cargo 干跑验证三种分支):
  1. `bash tests/acceptance/remediation/wave35_gate.sh --full`(独占 cargo 锁,G4 硬顶 900s)
  2. `bash tests/acceptance/remediation/wave35_gate.sh --compare-logs <lead_log> <my_log>`
- 干跑证据: `fullmode_dryrun_stubcargo.log`(绿→Exit 0 / 1 failed→Exit 1 五条违规 / 挂起→Exit 1 G4_HANG_TIMEOUT)。

## (b) path_permission 2 处 #[cfg(windows)] 拆分 → ACCEPT(带 WARN),本 Agent 独立核验结论
机械证据(`git diff f70b1f3 -- .../path_permission.rs`):
- 删除块为 `if cfg!(windows) { assert_eq!(zone, WorkspaceRoot) } else { assert_eq!(zone, AuthorizedExternal) }`;
  保留并升级为 `assert_eq!(zone, WorkspaceRoot)` + `#[cfg(windows)]` 门禁。
- 我方独立理由(与 Lead 结论一致,但判据不同):`cfg!(windows)` 是**编译期常量**,该 if/else 在任一平台上恒有且仅有一条分支为死代码,
  被删的那条断言在 Linux 上**不可达且曾是错误预期**(与本波 "Linux 恒失败" 现象同源);不是把有效断言降级。
- 补偿证据:文件级断言 50→55(不降反升);新增 2 个 POSIX 等价测试(含"同 root 内绝对路径放行"负向对照,排除"一律拒绝"式假绿);
  门禁按「门禁拆分 + 文件总量不降 + 新增测试断言数 ≥ 降幅」判合规并**保留 WARN 输出**(见 static_only_20261018T110941.log)。
- 最终确认权归 G3(该 POSIX 测试在真实全量跑中通过)。门禁此处为 WARN 而非静默通过,Lead 可随时改判。

## (c) tool_surface unused-import → 核验 PASS(非 allow 消警)
`git diff f70b1f3 -- crates/pony-agent-core/tests/tool_surface_wave20_contract.rs`:
- `-use pony_agent_core::agent::tools::{ builtin_turn_tool_contract_views, canonical_tool_name, ToolRegistrySnapshot, };`
  `+use pony_agent_core::agent::tools::{builtin_turn_tool_contract_views, canonical_tool_name};`
- 现文件 `grep '#\[allow'` = 0 处;`grep ToolRegistrySnapshot` = 0 处(符号彻底移除)。⇒ 正解,判 PASS。
- 已固化为硬门禁(该文件专属):新增 `#[allow(..)]` → FAIL `[TOOL_SURFACE_ALLOW_ATTR]`;
  无 use 行增删 → FAIL `[TOOL_SURFACE_IMPORT_UNTOUCHED]`。红队场景 C 验证可捕获:gate_selftest.log。

## (d) tui.rs / Cargo.lock 显式豁免(不判定、不普查、不触碰)
门禁输出原文:
- `INFO G2 显式豁免路径(Lead 裁决 d): crates/pony-agent-core/src/bin/tui.rs Cargo.lock`
- `INFO G2 已从越界判定中剔除: Cargo.lock,crates/pony-agent-core/src/bin/tui.rs`
- 依据:同批 untracked 产物 `.dev-team/l2at-done-tui-e2e.json`、`tests/acceptance/l2at-tui-e2e.sh` 指向 TUI e2e 会话;
  `Cargo.lock` 变更源自已提交的 release bump 6af176e(0.1.149→0.1.150)。本波 diff 范围仅 5 文件。
- 本 Agent 全程未写入这两个路径(写域自守见下)。

## 残余盲区声明(L3 语义审查覆盖,机器判据到不了)
1. 等值断言被替换为"更弱但仍成立"的断言(如 `assert_eq!(x, 3)` → `assert!(x >= 0)`):各宏计数保持不变,静态门禁无法识别。
2. 平台门禁补偿规则:门禁 + 文件总量不降 + 新增测试断言数达标即放行;理论上可被"加一条带真断言但语义空洞的新测试"利用。
3. 语义化断言洗白需人工/L3 读码,本门禁只保证**计数、种类、区域、结构**四个维度的守恒。

## 写域自守
本 Agent 仅写入:`tests/acceptance/remediation/`(wave35_gate.sh、wave35_gate_selftest.sh)、`tests/acceptance/receipts/wave35/`。
未触碰任何 `src/`、`tests/`(crate 内)代码,未触碰 tui.rs 与 Cargo.lock。红队 worktree 均建于 /tmp 并在退出时移除(`git worktree list` 无残留)。
