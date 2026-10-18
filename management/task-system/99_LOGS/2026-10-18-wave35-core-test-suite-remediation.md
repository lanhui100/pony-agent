# 2026-10-18 wave-35 pony-agent-core 测试套件修复(11 失败 + 2 编译阻断 + 全量真死锁)

## 起始状态与范围

起点是用户侧的两个问题(桌面端上传 trace 报 `PonySentry is disabled`),收尾时发现 `pony-agent-core`
测试套件长期不可交付,遂进入修复波。三个独立问题:

1. **11 个恒定失败**(在干净基线 `6af176e` 上用 `git stash` 对照法复现,与本轮改动无因果)。
2. **2 个 `-D warnings` 编译阻断**(`cargo check --all-targets` 下未使用导入)。
3. **全量套件真死锁**:`cargo test -p pony-agent-core --lib` 20 分钟不能终止。

## 根因(均有 path:line 或命令证据)

| 组 | 根因 | 判定 |
|---|---|---|
| A(6 例) | wave-22(806803f)加入 `workspace_edit_file` 读前写硬不变量后,旧用例未传 `expectedHash` | 测试缺陷 |
| A(2 例) | `plan_control` 是 `ToolKind::Write`(`tools.rs:6274`),被 wave-30b 草稿态门禁 Deny ⇒ `start_execution` 不可达;而 `plan_state.rs:336` 记载它正是「unlock write operations」的机制 ⇒ **门禁锁死自己的解锁器** | 实现缺陷 |
| B(4 例) | Windows 专有语义在 Linux 执行:`..\` 字面量、盘符卷边界、大小写折叠、符号链接 canary 缺 `outside/f.txt` 夹具 | 测试缺陷 |
| C(挂起) | `runtime/tests.rs:5447` 的进程级 hop-limit override 在抢进程级 `MOCK_SERVER_GATE` **之前**设置 ⇒ 并发用例 turn 提前中止 ⇒ 请求数 < mock 队列 ⇒ `finish()`=`handle.join()` 永不返回 ⇒ 全局闸门永不释放 ⇒ 60 个 mock 用例全部阻塞(实测进程 0.5% CPU、15+ 线程 futex 等待) | 实现缺陷(测试基建) |
| C(flake) | 错误文本含带随机端口的 URL,`retry.rs` 用裸子串码表匹配 ⇒ 端口 `:42953` 把永久错误误判为可重试 | 实现缺陷(生产判据) |
| D/E | 2 个测试文件的未使用导入 | 机械 |

## 修复与验证

- 提交:`b7139b0`(A/B/C/D/E 组)、`93ed559`(retry 判据的 char-boundary 回归修复)、`07cbe89`(收口归档)。
- **全量**:`RUSTFLAGS="-D warnings" cargo test -p pony-agent-core --lib` → `1062 passed; 0 failed; 0 ignored`,~12–14s,
  Excel 0(修复前 `EXIT=124`/1200s 被杀、0 行摘要、16 条 `>60 seconds`)。Lead 5/5 连跑 + Test Agent 独立 5/5 复跑绿。
- **死锁定向**:`-- agent::runtime::tests::` 由「200s 被杀 + 4 FAILED + 15 条 >60s」→ `127 passed` 6.8s。
- 冻结契约回归:wave22 `1 passed`、wave30b `3 passed`、ponysentry 5 目标 38 passed 全绿。
- 用例数 1058 → 1062;`#[ignore]` 实测 0;无新增依赖。

## 本轮自身引入并被拦下的 P0(重要教训)

C3 首版 `strip_endpoint_fragments` 用 `str::rfind(...)+1` 的**字节**索引做切片边界 ⇒ URL 紧贴多字节字符
(`访问https://…`、`错误：https://…`、`🔒https://…`)时 panic `byte index N is not a char boundary`,
且入参是 **provider 重试循环里的上游可控文本** ⇒ 会打掉整次 turn。

- 由 **L3 视角 B(安全/协议)** 判 FAIL 发现;Lead 复核反例成立后打回;修法改为「只从 `://` 剥到 URL 终点」,同时消解剥离过宽。
- **首版自带回归测试捕获不到它**(输入在 URL 前总有 ASCII 分隔符)——由 Test Agent 用独立探针 crate 实测 4 例后补为硬要求。
- 修订后变异自证:退回旧实现 ⇒ 19 passed 变 11 passed/8 failed(可退回性排除)。

## 审查与门禁

- L3-A(并发/状态流转)PASS 0 findings;L3-B FAIL→修复→增量复审 PASS;L3-P(五问)PASS 0 findings
  (3 条 test-only 偏差登记于 `.dev-team/test-only-exemptions-wave35.json`)。
- Test Agent 反作弊门禁 17/17 类注入作弊全部捕获;其自身两类仪器缺陷(TSV 生成顺序致假阳性、探针丢落盘写入致假阴性)已自证、修复并把缺陷期日志隔离为 `INVALID_*`。
- 元复盘:`.dev-team/meta-retrospective-wave35.md`(`dev-meta-lint.py` PASS)。
- 决策记录:`docs/decisions/0038-wave35-suite-remediation.md`。

## 残留事项(已登记 backlog,待后续处理)

`PA-138` 草稿门禁 session 解析 fail-open(预存)｜`PA-139` 全量并行 flake ≈7.7%/次(本波解除遮蔽)｜
`PA-140` CI 平台覆盖缺口(ubuntu/macos 不跑 core 测试)｜`PA-141` Draft 自解锁的安全姿态裁决｜
`PA-142` `path_permission.rs:764` Linux 零断言假绿(预存)｜`PA-143` 码表子串匹配过宽｜
`PA-144` scheme-less `host:port` 仍污染分类｜`PA-145` `recovery.rs` 第二套平行判据。

## 续作锚点

- 全量绿基线:`07cbe89`(核心),用例数 1062。
- 重跑命令:`RUSTFLAGS="-D warnings" cargo test -p pony-agent-core --lib`(约 12–14s,新增 flake 观测请连跑多轮)。
- 门禁脚本:`tests/acceptance/remediation/wave35_gate.sh`(`--static-only` / `--snapshot` / `--full` / `--verify-log` / `--compare-logs`)。
- 注意:本仓库同期存在其它会话(TUI e2e 波)在飞改动,其 `git add -A` 会卷入无关文件;提交必须定向 `git add`。
