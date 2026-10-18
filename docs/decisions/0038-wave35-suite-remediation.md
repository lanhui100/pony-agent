# 0038 wave-35 测试套件修复：全量挂起真死锁、plan 草稿门禁自锁死与读前写契约漂移

Status: implemented

## 背景

`cargo test -p pony-agent-core --lib` 长期处于不可交付状态，共三类问题：

1. **11 个恒定失败**（在干净基线 `6af176e` 上用 stash 对照法复现，与其它改动无关）：
   `agent::tools::tests::edit_file_*`(4)、`agent::governed_executor::tests::*`(3)、
   `agent::path_permission::tests::*`(3)、`agent::document_conversion::tests::*`(1)。
2. **2 个 `-D warnings` 编译阻断**：`tests/tool_surface_wave20_contract.rs` 与
   `tests/wave31_lsp_client_test.rs` 的未使用导入（`cargo check --all-targets` 下失败）。
3. **全量套件真死锁**：`cargo test -p pony-agent-core --lib` 20 分钟不能终止
   （Lead 实测 `EXIT=124`、日志 0 行 `test result` 摘要、16 条 `has been running for over 60 seconds`）。
   进程 0.5% CPU、15+ 线程停在 `futex_wait_queue`、mock 线程停在 `accept()` ⇒ 非慢，是死锁。

## 诊断（每项均有 `path:line` 或命令证据）

| 组 | 现象 | 根因 | 判定 |
|---|---|---|---|
| A | 6 个用例返回 `fs_not_observed` | `workspace_edit_file` 在 commit 806803f（wave-22）加入「读前写」硬不变量后，这些旧用例未传 `expectedHash` | TEST-DEFECT |
| A | 2 个用例返回 `permission_denied/plan_draft_gate` | `plan_control` 是 `ToolKind::Write`（tools.rs:6274），被 wave-30b 草稿态门禁（governed_executor.rs:139-166）Deny ⇒ `merge`/`complete_step`/`replace`/`start_execution` 全不可达；而 `plan_state.rs:336` 明确记载 `start_execution` 是「unlock write operations」的机制 ⇒ **门禁锁死了自己的解锁器** | IMPL-DEFECT |
| B | 4 个平台相关用例红 | 用例写成 Windows 专有语义却在 Linux 执行：`..\` 字面量（document_conversion.rs:366）、盘符 `C:\`/`D:\` 卷边界与大小写折叠（path_permission.rs:750/707）、`real_symlink_escape_canary` 缺 `outside/f.txt` 夹具致 canonicalize ENOENT | TEST-DEFECT |
| C | 全量永久挂起 | `runtime/tests.rs:5447` 的 `ToolHopLimitOverrideGuard::set(1)` 写的是**进程级** OnceLock，却在 `MockHttpServer::start`（抢进程级 `MOCK_SERVER_GATE`）**之前**设置 ⇒ 并发 gated 用例读到 hop=1 → turn 提前中止 → 实际请求数 < mock 队列长度 → mock 线程永不等齐 → `finish()`=`handle.join()` 永不返回 → 全局 gate 永不释放 → 60 个 mock 用例全部阻塞 | IMPL-DEFECT（测试基建） |
| C | `running_graph_stream_*` 相位 flake（150 次 2 次，p≈2.8%） | SSE 错误文本含 `响应预览: {endpoint}`（含随机端口），`retry.rs:317-322` 用裸子串码表在整段 message 上匹配 ⇒ 端口形如 `:42953`/`:40891` 把永久错误误判为 `TransientRetryable` ⇒ 5 次重试耗尽 mock 队列 | IMPL-DEFECT（生产判据） |
| D/E | `-D warnings` 阻断 | 2 个测试文件的未使用导入 | TEST-DEFECT（机械） |

## 候选方案（Alternatives considered）

**plan 草稿门禁自锁死**
1. *维持现状*：否决——plan 永远无法离开 Draft，且该会话所有写入被永久 Deny。
2. *改 `tools.rs:6274` 的 `ToolKind`*：否决——`Write` 类别被沙箱门、预算、审计等多处消费，爆炸半径过大。
3. *在门禁判定中豁免 `TOOL_PLAN_CONTROL`（选定）*：改动 1 个判定表达式；已冻结的 wave-30b 契约测试用 dummy `"Write"`/`"Run"` 描述符，不受影响；`primitive_name` 的唯一生产来源是内置目录（tools.rs:517）+ 别名唯一性约束（tools.rs:571-587）⇒ 无别名/大小写绕过（L3-A A4 复核）。

**平台相关用例**
1. *删除或用例整体 `#[cfg(windows)]`*：否决——Linux 覆盖归零，且体积/大小写折叠与符号链接逃逸是安全敏感面。
2. *平台等价重写（选定）*：文档路径改用 `std::path::MAIN_SEPARATOR`；符号链接 canary 补夹具；两个 Windows 专有语义用例加 `#[cfg(windows)]` 并各补 1 个带**负向对照**的 POSIX 等价测试（跨根绝对路径必拒 + 同根必放行；`/WS` 不得折进 `/ws` + 反斜杠不得逃逸）。净用例数不降，Linux 断言数 50→55。

**全量挂起**
1. *引入 `serial_test` + `#[serial]`*：否决——需新增依赖（Cargo.lock 无），且不解决队列失配与 gate 永不释放。
2. *调大各种超时*：否决——掩盖缺陷，挂起依旧。
3. *override 移入 gate 持有区间 + mock `finish()` 改有界 join（选定）*：同时消除「全局 override 泄漏给并发用例」与「队列失配时永挂」；有界 join 超时即带 `expected/actual` 诊断 panic，把「整库挂死」降级为「单用例秒级失败」。
4. *彻底去全局化：per-test 隔离替代进程级 mock 闸门*：**延后**（爆炸半径大、易引入新的 flakiness），登记为下一波议题。

**SSE 预览端口数字误分类**
1. *测试里挑一个不含 429/408/504 的端口*：否决——只治标，真实生产误判仍在（上游 URL/正文同样可能携带这些数字）。
2. *判据前剥离 `scheme://…`（选定）*：分类/限流不再看 URL、主机与端口。

## 决策

- `governed_executor.rs`：`is_write_or_exec` 追加 `&& primitive_name != TOOL_PLAN_CONTROL`。
- `path_permission.rs` / `document_conversion.rs` / `tools.rs` / `governed_executor.rs`（测试部分）：按上表最小修改，**断言口径零放宽**。
- `runtime/tests.rs`：全局 hop-limit override 移入 gate 持有区间并在 `finish()` 前显式释放；mock `finish()` 改有界 join，预算 **5000ms**（对齐 `.dev-team/nfr-baseline.json` 的 `deadlock_watchdog_timeout_ms=5000`；Lead 将初始 10s 下调为基准值）。
- `control_plane/mod.rs`：同款有界 join。
- `retry.rs`：`strip_endpoint_fragments` **只从 `://` 剥到 URL 终点**，其前文本整体保留；切片边界只来自 `str::find`。
- `tests/tool_surface_wave20_contract.rs`、`tests/wave31_lsp_client_test.rs`：删除未使用导入（**不用** `#[allow]` 消警）。

## 本轮自身引入并被对抗审查捕获的回归（留档）

C3 首版实现用 `str::rfind(...).map(|index| index + 1)` 的**字节**索引做切片边界 ⇒ URL 紧贴多字节字符
（`访问https://…`、`错误：https://…`、`🔒https://…`）时 panic `byte index N is not a char boundary`。
可达路径是 provider 重试循环的**上游可控文本** ⇒ 会打掉整次 turn。

- 由 **L3-B 视角审查**（判 FAIL）发现，Lead 亲自复核反例成立后打回。
- 关键教训：首版**自带的回归测试不足以捕获它**（其输入在 URL 前总有 ASCII 分隔符）——Test Agent 用独立探针 crate
  实测 P1（ASCII 分隔）通过、P2/P3/P4（CJK/全角冒号/emoji 紧贴）panic，据此把「多字节紧邻 `://`」列为必修输入。
- 修订后由 Test Agent 做变异敏感性验证：退回旧实现 ⇒ 19 passed 变 11 passed/8 failed ⇒ 回归测试敏感，
  且门禁新增 `UNICODE_URL_BOUNDARY_UNCOVERED` 机械判据（生产区出现 `://`/`rfind`/字节切片而新增测试无多字节紧邻输入时报警）。

## 已知残留与预存缺陷（本轮不修，登记待办）

| 编号 | 内容 | 证据 | 性质 |
|---|---|---|---|
| F-1 | Plan 草稿门禁的 session 解析 fail-open：只查 args/env/`"default"`/`"test-session-draft-gate"`，而真实会话由 `runtime/mod.rs:298` 注入且 `write_file` schema 无 `session_id`；env 回退 `.map(\|_\| "env_session")` 忽略变量取值 | governed_executor.rs:150-159 | 预存（ff76305 引入）；本波豁免未削弱原本生效的保护 |
| F-2 | `win_prefix_normalization_strips_verbatim` 的 `if cfg!(windows) { assert… }` 无 `else` ⇒ Linux 上**零断言执行仍报 ok**（违反仓库「严禁无断言测试」停止线） | path_permission.rs:764 | 预存 |
| F-3 | CI 只在 `windows-latest` 跑 core 测试；ubuntu/macos 仅 `cargo check` ⇒ 平台相关用例在 Linux CI 从未执行 | ci.yml:14/61 与 :63-81 | 预存；建议补 Linux `--lib` 测试 job |
| F-4 | 全量并行下残留 flake：`lifecycle_boundary_checkpoint_…` 失败约 1/13 次全量跑（合并 Lead 与 Test Agent 样本；Wilson 上界 ≈26%）。形态为**首个请求 1ms connect refused**。单跑 20/20 绿、模块批 10/10 绿 | `receipts/wave35/full_lib_run_freeze2.log`、`FINAL_acceptance_report_freeze2.md` | 预存，**被本波解除遮蔽**（见下） |
| F-5 | 码表仍是子串匹配：`elapsed=40832ms` 可命中 `"408"`；`provider/mod.rs:5991` 把它当契约 | provider/mod.rs:5991 | 残留，须单独立项 |
| F-6 | `recovery.rs:160-167` 另有一套裸 `contains` 判据，无生产调用者（潜在漂移） | recovery.rs:160-167 | 潜在 |
| F-7 | **安全姿态变化**：豁免 `plan_control` 后，模型可在 Draft 态自行 `start_execution` 解锁写（`plan_state.rs:337` 无宿主审批） | governed_executor.rs:147 | 本轮决策，L3 要求留档 |
| F-8 | 同族残留（P3）：**无 `://` 的 scheme-less** `endpoint=127.0.0.1:42953/v1` 仍命中限流/服务端码表 → 仍可被误判为可重试；可达路径为「响应正文: {上游体}」（provider/mod.rs:1802，config 的 `base_url` 不校验 scheme）。C3 只剥离 `scheme://…` | L3-B 增量复审收据 | 残留，建议下一波按「按结构字段分类而非整段子串」立项 |

### F-4 的机制说明（重要：不是本轮引入，而是本轮**解除遮蔽**）

实践判据（Test Agent 从失败日志原文提取）：
- 修复前 `raw_hits_rate_limit=true` —— 失败请求的错误文本里端口 `42967` 的 `429` 命中码表 ⇒ 被误判为 `TransientRetryable`
  ⇒ 运行时**退避重试**，重试恰好成功 ⇒ 回合完成 ⇒ 断言通过 ⇒ **测试的失败被错误分类意外掩盖**。
- 修复后 `stripped=false` —— 同一 connect 失败被正确判为永久错误 ⇒ **立即 Abort** ⇒ 掩膜消失 ⇒ 测试暴露真实缺陷。

即：真正的问题是**测试基建在首个请求时连接被拒**（mock 监听就绪/端点归属存在竞态），此前长期被「端口数字导致的伪重试」掩盖。
已核验与本波 5s 有界 join **无关**（失败发生在首个请求 1ms，join 在收尾；日志中 join 超时诊断 0 次；端口每用例独立；
provider 解析无进程级缓存 ⇒ 已排除陈旧 endpoint 缓存）。唯一已定位的进程级耦合候选是预存的
`runtime_helper.rs` 的 `OnceLock<Runtime> + enter()`（共享运行时的 endpoint 归属），**确切机制未定位**，
故本轮**不做投机性改造**，处置建议（由 Test Agent 给出、Lead 采纳为下一波首选）：
1. mock 就绪**确定性握手**（禁固定 sleep）；2. 否则 ≤2 次受控重试 + 告警登记；
3. **禁止**用「放宽 classify」或「调大 `MOCK_SERVER_FINISH_TIMEOUT`」掩盖。

## 验证（物理收据）

- 冻结 revision：`b7139b0`（A/B/C/D/E 组）、`93ed559`（F 组 retry 修复）。
- 全量：`RUSTFLAGS="-D warnings" cargo test -p pony-agent-core --lib` → `1062 passed; 0 failed; 0 ignored`，~13s，
  Lead 5/5 连跑全绿 + Test Agent 独立复跑一致（`--compare-logs` 计数逐行相同）；修复前为 `EXIT=124`/1200s 被杀。
- 死锁定向：`-- agent::runtime::tests::` 由「200s 被杀 + 4 FAILED + 15 条 >60s」变为 `127 passed` 6.8s。
- 回归冻结契约：wave22 `1 passed`、wave30b `3 passed`、ponysentry 5 个目标 38 passed 全绿。
- 编译：`RUSTFLAGS="-D warnings" cargo check -p pony-agent-core --test wave31_lsp_client_test` 与 tool_surface 均 Exit 0。
- 用例数：1058 → 1062；`#[ignore]` 始终 0；无新增依赖。
- 审查：L3-A（并发/状态流转）PASS 0 findings；L3-B（安全/协议）FAIL→修订→增量复审；L3-P（五问）PASS 0 findings
  （3 条 test-only 偏差登记于 `.dev-team/test-only-exemptions-wave35.json`）。
- Test Agent 反作弊门禁：17/17 类注入作弊全部捕获（含 `#[allow]` 消警、弱断言、补偿规则）；
  其自身探针 v1 的假阴性（丢落盘写入却打印 `MUTATION_APPLIED`）已修正为「替换→落盘→读盘校验」三重自证。