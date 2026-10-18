# wave35 最终验收报告(freeze-2 = 93ed559)

验收主体: Test Agent(独立于 Executor) · 门禁基线 f70b1f3 · 冻结 revision **93ed559**
结论: **ACCEPT —— `--full` Exit 0 / 0 违规 / 1062 passed / 0 failed**
(本文件取代 `final_acceptance_report.md`(freeze-1)与 `evidence_round3_prefreeze2.md` 中已作废的读数)

## 1. G3/G4 原始结果(我方收据为主体)
| 项 | 结果 |
|---|---|
| `RUSTFLAGS="-D warnings" cargo test -p pony-agent-core --lib` | `test result: ok. 1062 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 12.94s` |
| 退出码 / 墙钟 | **Exit 0** / **14s**(硬顶 900s;`has been running for over` 出现 0 次) |
| `--test tool_surface_wave20_contract` | `ok. 2 passed; 0 failed` / Exit 0 |
| `--lib -- retry` 子集 | `ok. 49 passed; 0 failed` / Exit 0 |
| 独立重复测量 | 额外 5 轮全量:**5/5 绿**,每轮 13–14s(`flake_rate_probe_20261018T123633.log`) |

## 2. `--compare-logs` 结论(逐行计数比对,剔除耗时抖动)
| 对照 | A(Lead) | B(我方) | 结论 |
|---|---|---|---|
| freeze-2 首次 | 1061 passed / **1 failed** | 1062 passed / 0 failed | **MISMATCH**(即第 6 节那例 flake 样本,非我方不一致) |
| freeze-1 | 1061 passed / 0 failed | 1062 passed / 0 failed | MISMATCH,差异仅 +1 用例(freeze-2 新增测试) |
**事实说明**:收据目录内**只有上述两份 Lead 全量 log**;Lead 所述"freeze-2 同 revision 5/5 全绿"未留下 log 文件,故无法做 green-vs-green 的同类比对。我方那 5 轮绿有逐轮 log(`flake_rate_runs_20261018T123633/run_*.log`)。

## 3. G1 反作弊静态门禁(9 文件,0 违规)
| 文件 | 断言总数 基线→当前 | 备注 |
|---|---|---|
| tools.rs | 431→431 | — |
| governed_executor.rs | 59→59 | plan_control 豁免(仅 1 个 canonical primitive) |
| path_permission.rs | 50→**55** | 2 处平台门禁拆分(2 条 WARN,Lead 已判 ACCEPT)+ 2 个 POSIX 等价测试(6 断言,含负向对照) |
| document_conversion.rs | 21→21 | — |
| runtime/tests.rs | 658→**660** | C1/C2;`MOCK_SERVER_FINISH_TIMEOUT=5000ms`(该常量**基线不存在**,非"10s→5s") |
| control_plane/mod.rs | 555→555 | C2 有界 join |
| retry.rs | 96→**110** | 生产区新增 63 行 / 测试函数 39→**42** / 覆盖判定 **YES**;Unicode 边界有覆盖 |
| tests/tool_surface_wave20_contract.rs | 16→16 | use 删1/加1;新增 `#[allow]`=0(删导入,非消警) |
| tests/wave31_lsp_client_test.rs | 8→8 | 衍生豁免 PASS:剔除 `use` 行后逐字节一致 |

## 4. G2 用例数与平台门禁
- `cargo --list` 实测 **1062**(下限常量 1062 = 基线 1058 + 本波新增 4)。
- `#[ignore]` 属性(整 `src/` 树,基线 0):**0**。
- `cfg(windows)` 62→64;scope 内新增 2 处;新增测试 6 个 ⇒ 计数守恒。静态 `#[test]` 1068→1074。
- 显式豁免(Lead 裁决 d):`bin/tui.rs`、`Cargo.lock`,不判定、不普查。
- `UNICODE_URL_BOUNDARY_UNCOVERED`:**freeze-2 下已自动消失**,转为 INFO「新增测试含多字节紧邻 URL 的输入(Unicode 边界有覆盖)」——作为新测试充分性旁证。

## 5. boundary probe 对齐 93ed559(物理敏感性)
- 对齐方式:收据内记录的 `retry.rs sha256 = f04ae4342cb0960069485c4810bbb6df11af7193a739bc2b17bb50a44a971b43`
  **等于** 93ed559 工作区该文件 sha256 ⇒ 实验对象与冻结内容同源(当时 HEAD 尚未提交,但文件内容一致)。
- 读数:AS-IS(修法)**19 passed / exit 0**;MUTATED(机械退回 `rfind(...)+(index+1)` 字节切片)**11 passed / 8 failed / exit 101**。
- SENSITIVE 8 条 = `auto_4..auto_8`(**取自 freeze-2 自身回归测试的字面量**:`访问https://…`、`错误：https://…`、`🔒https://…`)+ 我的 3 条 unicode 探针;
  panic 原文含 `byte index 4 … inside '问'`、`byte index 7 … inside '：'`、`byte index 1 … inside '🔒'`。
  ⇒ **可退回性已被排除**:freeze-2 的回归测试确实捕获该 P0。
- **仪器缺陷披露(我方)**:探针 v1 的变异步骤丢了一行落盘写入却仍打印 `MUTATION_APPLIED`(仅凭替换计数),导致跑的是未变异代码并得出**假结论 `INSENSITIVE`**;
  已修为「替换 → 落盘 → 重读 sha256 比对 + `index + 1` 存在性校验」三重校验,校验不过即 Exit 3 且不产出结论。
  4 份缺陷期日志已改名 `INVALID_retry_boundary_probe_*.log` 并加无效头,**与有效收据明确区分、不参与任何结论**。

## 6. 残留 flake 独立判定(Lead 指定,不修)
失败用例 `agent::control_plane::tests::lifecycle_boundary_checkpoint_does_not_override_default_submission_plan`;
形态:首个请求 `attempt=1/5` → `connect refused`(1ms, endpoint `127.0.0.1:42967`)→ `class=NonRetryable` → `Abort` → 无 checkpoint → 测试 `.expect(...)` panic(mod.rs:5577)。

**(ii) 是否本波引入 —— 根因预存,本波"解除遮蔽"(机械证明)**
`flake_masking_probe_20261018T123335.log`(逐字提取码表与剥离函数,对日志原文实测):
`PRE_FIX raw_hits_rate_limit=true`(原文含 `42967`,`429` 命中 RATE_LIMIT 码表)vs `POST_FIX stripped_hits_rate_limit=false`(剥离后无 `429`)。
⇒ freeze-1 之前该 1ms connect-refused 被判**可重试** ⇒ 退避重试给了 mock 绑端口/accept 的时间 ⇒ 竞态被掩盖;
freeze-2 剥离 URL 后判**不可重试** ⇒ 立即 Abort ⇒ 同一竞态暴露为硬失败。**竞态本身预存,C3 修复只是让它可见。**

**(i) 是否与新「5s 有界 join / 全局 mock 闸门」语义相关 —— 判定:不相关(有证据的否定)**
- 失败发生在**首个请求**(1ms connect-refused);有界 join 作用于**用例收尾**等待 mock 线程退出,相位不符;
- 全量 log 内 `mock server thread still running` / join 超时诊断出现 **0 次**;若曾发生 join 超时,panic 会落在**那个**用例身上;
- 端口是每用例 `127.0.0.1:0` 独立分配,不存在跨用例端口复用;
- 已核验 provider 解析路径**无**进程级选择缓存(`StaticResolver` 每用例新建 base_url)⇒ 排除"陈旧 endpoint";
- 路径中唯一进程级耦合是 `runtime_helper.rs` 的 `OnceLock<Runtime>` + `rt.enter()`(**预存**,非本波):共享 tokio runtime 在 1062 用例并行下饱和,是**待验证的候选通道**;
  但 `connect refused` 属 OS 层无可监听者,与"runtime 饱和导致超时"的形态不同 ⇒ **确切机制未定位,需加诊断取证**(建议:mock 在返回前自证可连接 / 打印 bind 端口与就绪标志)。

**(iii) 概率估计与建议处置**
- 样本池:Lead freeze-2 全量 **1/7 失败**;我方 freeze-2 全量 **0/6**(1 次验收 + 5 轮复测)⇒ 合并 **1/13 ≈ 7.7%** 每次全量;Wilson 95% 上界 ≈ 26%。单独我方 5 轮 0/5 上界 43%(样本小,不足以判"已消失")。
- 建议(不实施):①优先**确定性**修复 —— mock 服务端 `bind` 成功后经 channel/AtomicBool 向客户端发布"就绪",客户端在有界条件等待中取用(**禁固定 sleep**),从根上消除启动竞态;
  ②若本波必须收口,按仓库 Flaky 治理条款允许**最多 2 次受控重试**并输出警告日志记录,同时把该用例登记为已知 flake;
  ③**禁止**通过重新放宽 classify(把 URL/端口数字重新纳入判据)、或提高 `MOCK_SERVER_FINISH_TIMEOUT` 来"修"——前者回退 C3 安全修复,后者与 NFR 基准冲突。

## 7. 残余风险与待裁决
1. L3 语义审查仍覆盖:弱断言替换、平台门禁补偿规则(计数类判据到不了的语义弱化)。
2. 预存缺陷(未修,已登记):`path_permission.rs:764` 无 else ⇒ Linux 零断言报 ok;`ci.yml` 唯一跑 core 测试的 job 是 windows-latest;`governed_executor.rs:150-159` 非 default 会话 Draft 写保护 fail-open(含 `env_session` 常量匹配缺陷)。
3. 6 节 flake 的处置权归 Lead。
4. `provider/mod.rs:5991` 的「更长数字串内子串命中」既有契约(仅登记事实)。
5. 我方两处仪器缺陷均已修复并留痕(门禁 `added_in_tests.tsv` 构建顺序、探针落盘校验)—— 参见 §5 与 §4 的 WARN 由假阳性转 INFO。

## 8. 收据、哈希与复跑
| 文件 | sha256 |
|---|---|
| `remediation/wave35_gate.sh` | a445ac2e551196540eb7860c4293f508fae5ac11120b7a148e9a0b5ff25c0db8 |
| `remediation/wave35_gate_selftest.sh` | e66df265a38386fda7498a390eed7ff33fa52b962bf86198f67040ddcc53de41 |
| `remediation/wave35_retry_boundary_probe.sh` | 5bef18f84ec29b3e139b4c7cf8f95c1bbaa6c7f57ee8a717a3d30c4a7d378d6b |
| `remediation/wave35_flake_masking_probe.sh` | 38e92667ecd8f89f64c62f1b7250baa4c77d26818c190647659d4102a922d33b |
| `remediation/wave35_flake_rate_probe.sh` | b252fd56d072a875407c703bfd58a05af9d886a08949243d23623b808713cfe0 |
| `receipts/wave35/FINAL_full_freeze2_93ed559_20261018T122949.log` | 93261c7c70301455e9e8e9f860d59c336df1e946ab764078e90745ba00fabda8 |
| `receipts/wave35/lib_full_20261018T122953.log`(G3 原始 stdout) | 761a0a689e3a751adda42478f226c3e37c76e0d18ddd21227035bb627e2c87a3 |
| `receipts/wave35/lib_list_20261018T122953.txt`(G2 --list) | 86368a12a7513d0e2d38229c27e28d7dc45d839bf9a9617584709f02abb43bae |
| `receipts/wave35/retry_boundary_probe_20261018T122332.log` | 2bf0a867996e1f3a9f8567dd8726018b27ea9e043232f404d4368ee9e458ca57 |
| `receipts/wave35/flake_masking_probe_20261018T123335.log` | 6765e34a9ec02fc8ffe87a49913d6992448fb3ce5bd7f278824c01ffaaddcf5a |
| `receipts/wave35/flake_rate_probe_20261018T123633.log` | fef5e33569abc03722e4374e545c9533ff98b2c591301b3961a04e42104ff259 |
| `receipts/wave35/gate_selftest.log`(红队 17/17) | f7b78ffdebeb57e9849f2e084d4af7d7c0fb4a74bf2915f9e7dd4d2d0ae2704b |

复跑:
```
bash tests/acceptance/remediation/wave35_gate.sh --full
bash tests/acceptance/remediation/wave35_gate.sh --compare-logs <a> <b>
bash tests/acceptance/remediation/wave35_gate_selftest.sh
bash tests/acceptance/remediation/wave35_retry_boundary_probe.sh
bash tests/acceptance/remediation/wave35_flake_masking_probe.sh
bash tests/acceptance/remediation/wave35_flake_rate_probe.sh 5
```
写域自守:仅 `tests/acceptance/remediation/` 与 `tests/acceptance/receipts/wave35/`;未触碰任何 `src/`、crate `tests/`、`tui.rs`、`Cargo.lock`。
