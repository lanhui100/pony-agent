# wave35 最终验收报告(Test Agent 独立产出)

冻结 revision: **b7139b0** · 门禁基线: **f70b1f3** · 执行时间: 2026-10-18 12:07–12:09 (+08:00)
结论: **ACCEPT —— `--full` Exit 0,0 违规**

## 1. G3/G4 原始结果(我方收据为主体)
| 项 | 结果 |
|---|---|
| `RUSTFLAGS="-D warnings" cargo test -p pony-agent-core --lib` | `test result: ok. 1061 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 12.39s` |
| 退出码 / 墙钟耗时 | **Exit 0** / **13s**(硬顶 900s,G4 通过) |
| 挂起标记 `has been running for over` | **0 次** |
| `--test tool_surface_wave20_contract` | `ok. 2 passed; 0 failed` / Exit 0(含 `-D warnings` 编译) |
| `--lib -- retry`(分类子集不得放宽) | `ok. 48 passed; 0 failed`,Exit 0 |
| `--list` 用例数 | **1061**(下限常量 1061 = 历史基线 1058 + 本波新增 3) |

原始 stdout 落盘: `receipts/wave35/lib_full_20261018T120756.log`

## 2. `--compare-logs` 交叉比对结论
- Lead 收据 `full_lib_run.log` 与我的 `lib_full_20261018T120756.log`:两份 `test result` 计数**完全一致**
  (`ok. 1061 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out`)→ `RESULT: PASS — 0 违规(mode=comparelogs)`。
- Lead 那份的完整性复核(非空 / 含终态汇总 / 无挂起标记 / 无编译 error)同样 **PASS**。

## 3. G1 反作弊静态门禁(9 文件,0 违规)
| 文件 | 断言总数 基线→当前 | 备注 |
|---|---|---|
| tools.rs | 431→431 | — |
| governed_executor.rs | 59→59 | — |
| path_permission.rs | 50→**55** | 2 处 `#[cfg(windows)]` 拆分(2 条 WARN,Lead 已裁决 ACCEPT)+ 新增 2 个 POSIX 等价测试(6 条断言,含负向对照) |
| document_conversion.rs | 21→21 | 仅 `unwrap()`→`expect()` 等强度提升 |
| runtime/tests.rs | 658→**660** | C1/C2:新增测试 2 条断言;C2 有界 join 常量 `MOCK_SERVER_FINISH_TIMEOUT=5000ms` |
| control_plane/mod.rs | 555→555 | C2 同款有界 join,无断言变化 |
| retry.rs | 96→**102** | **生产判据变更**:生产区新增 55 行;测试函数 39→**41**;覆盖判定 **YES**;2 个新测试均含"判据未被削弱"负向对照(HTTP 502/503/504/408/429 仍须命中;`connection refused`/`dns failure` 仍须命中) |
| tests/tool_surface_wave20_contract.rs | 16→16 | `use` 删除 1/新增 1;**新增 `#[allow]` = 0**(删除 `ToolRegistrySnapshot` 导入,非 allow 消警) |
| tests/wave31_lsp_client_test.rs | 8→8 | 衍生豁免核验 **PASS**:剔除全部 `use` 行后其余内容与基线**逐字节一致**;断言/区域逐项完全相等 |

全局硬门禁:新增 `#[ignore]` = 0;新增 `#[cfg(not(test))]` / `#![cfg(..)]` / 否定式 `cfg` = 0;恒真断言 = 0;
`let _ =` 与 `unwrap_or*` 在测试区内净增 = 0;零断言新测试 = 0;断言助手洗白 = 0;测试抽空 = 0。

## 4. G2 用例数与平台门禁守恒
- 静态 `#[test]` 函数数 1068→**1073**(基线-当前均按 commit 现算,非缓存快照)。
- `cargo --list` 实测 **1061** ≥ 下限 1061。
- `#[ignore]` 属性(整 `src/` 树,基线 0):**0**。
- `#[cfg(windows)]` 62→64;scope 内新增 2 处,新增测试 5 个 ⇒ 计数守恒(2 门禁 ≤ 5 新增)。
- 显式豁免(Lead 裁决 d,不判定/不普查):`crates/pony-agent-core/src/bin/tui.rs`、`Cargo.lock`。

## 5. 红队自证(门禁自身有牙)
`gate_selftest.log`:**17/17 项通过** —— 场景 A 12 类注入全捕获、B 平台门禁失衡、C `#[allow]` 消警、
D1 仅删导入无假阳性(Exit 0)、D2 叠加非导入改动被捕获(Exit 1)。

## 6. 需 Lead 知晓的事实(不含判据变更)
1. **pre-freeze 运行**:11:38 那次 `--full` 已按指令 kill,收据首行标记
   `# PRE-FREEZE,非最终交付收据`,且**未产出任何 cargo 收据**(全程停在锁预检)。
2. **并发更正**:11:38–11:47 门禁逐 30s 记录到本仓库内 1–10 个并发 cargo 任务,
   与"无并发写者"的假设不符 ⇒ 我已把预检修为「仅统计真实 cargo 二进制(`/proc/<pid>/exe` 以 `/cargo` 结尾)
   且 cwd 在本仓库内」,并把 900s 计时后移到**锁获取之后**。
3. **僵尸管线假占锁**:pid 2611973(`bash | grep | head`,无 cargo 子进程)曾被旧判据当成锁持有者,
   导致一次 600s 空等;新判据已排除该类,重新发起后 13s 内跑完。
4. **`MOCK_SERVER_FINISH_TIMEOUT` 前提更正**:该常量在基线 f70b1f3 中**不存在**(基线无等价物),
   故"由 10s 下调为 5000ms"指的是本波自身中间态;G1 基线普查始终从 `f70b1f3` 现算,**无需重取快照**。
5. 预存缺陷(非本波引入,未修,见 `evidence_round2.md`):`path_permission.rs:764` 的
   `if cfg!(windows){…}` 无 else ⇒ Linux 上零断言仍报 ok;`ci.yml` 唯一跑 core 测试的 job 是 windows-latest。
6. 残余盲区(交 L3 语义审查):弱断言替换、平台门禁补偿规则;`provider/mod.rs:5991` 的
   「更长数字串内子串命中」既有契约(仅登记事实)。

## 7. 收据与 sha256(便于文件被并发会话移动/提交后对齐)
| 文件 | sha256 |
|---|---|
| `remediation/wave35_gate.sh` | 16aba8b28cdcec64445c01907bfb4f6a8275691306ec4a5586221394cf694653 |
| `remediation/wave35_gate_selftest.sh` | e66df265a38386fda7498a390eed7ff33fa52b962bf86198f67040ddcc53de41 |
| `receipts/wave35/full_gate_FROZEN_b7139b0_20261018T120751.log` | af1458202e471ca6bdac2706bff406ed6958361131f229232418dd976116e54a |
| `receipts/wave35/lib_full_20261018T120756.log`(G3 原始 stdout) | cd5c8d9fe345c7c90576904fd0391463e69ef3ead8d378844029f4980e2e33cc |
| `receipts/wave35/lib_list_20261018T120755.txt`(G2 --list) | ea7a217089247931ea6da41b72e83ec4763f71b9f2bbfc0149e9eaf7ffce4ee3 |
| `receipts/wave35/tool_surface_wave20_20261018T120756.log` | f1541346d9627dec846138466a222d5c503047e3e6643f457ac77d0d3ec74da8 |
| `receipts/wave35/retry_subset_20261018T120756.log` | 3ca718f512c393936b5eadabe98a9eeaaccdffb9489d5ecf833390b26927132d |
| `receipts/wave35/gate_selftest.log` | f7b78ffdebeb57e9849f2e084d4af7d7c0fb4a74bf2915f9e7dd4d2d0ae2704b |
| `receipts/wave35/static_only_20261018T115217.log` | 7b6969cba4ef00c183003521da57ed880c3ad41a9e95cdd4888846028cbae9fd |

复跑命令:
```
bash tests/acceptance/remediation/wave35_gate.sh --static-only
bash tests/acceptance/remediation/wave35_gate.sh --full
bash tests/acceptance/remediation/wave35_gate.sh --verify-log <log>
bash tests/acceptance/remediation/wave35_gate.sh --compare-logs <a> <b>
bash tests/acceptance/remediation/wave35_gate_selftest.sh
```
写域自守:仅 `tests/acceptance/remediation/` 与 `tests/acceptance/receipts/wave35/`;未触碰任何 `src/`、crate `tests/` 文件、tui.rs、Cargo.lock。
