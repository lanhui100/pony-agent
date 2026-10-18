# wave35 验收证据(第二轮:Barrier 暂停 + 9 文件扩展)

时间: 2026-10-18 11:52 (+08:00) / 基线 commit: f70b1f3 / 门禁: tests/acceptance/remediation/wave35_gate.sh

## 1. `--full` 暂停与 pre-freeze 记录(Lead 11:47 叫停)
- 我方 `--full` 于 11:38:39 启动,11:47:44 按 Lead 指令 kill。
- 收据 `full_gate_20261018T113839.log` 首两行已就地标记:
  `# PRE-FREEZE,非最终交付收据:本次 --full 被 Lead 叫停 …禁止用于验收裁决。`
- **该次运行未产出任何 cargo 收据**(无 `lib_list_*.txt`):全程停留在 `preflight_wait_lock`,
  即 target 锁被他人占用时它没有去争锁。可作为「门禁不会在争用下产生污染性收据」的正面证据。

## 2. 事实更正:邀请我跑全量的时刻并非「无并发写者」
11:38–11:47 门禁预检逐 30s 记录到**本仓库内** 1–10 个并发 cargo 任务(pid 2611973、3038957、3047199、3067793 …),
另有 `/home/dm/ponyllm` 仓库的 cargo 任务同机运行。据此我做两处修正:
- 预检只统计 **cwd 在本仓库内**的 cargo 进程(`/proc/<pid>/cwd`),不再被其它仓库的重编译拖住(此前会空等)。
- 900s 计时从**锁获取之后**才开始,避免把锁等待误判成 G4_HANG_TIMEOUT(预检上限 `WAVE35_LOCK_WAIT`,默认 1500s)。

## 3. G1 范围扩到 9 文件
新增: `runtime/tests.rs`、`control_plane/mod.rs`(原本已在 `runtime/`、`control_plane/` 目录口径内)、
`retry.rs`、`tests/wave31_lsp_client_test.rs`。`--static-only` Exit 0。

- **retry.rs(C3 生产判据变更)** 机械实测:生产区新增行 **55**;测试函数 39→**41**(+2);
  判定 = 生产变更已由新增测试覆盖 **YES**。两个新测试均带**负向对照**:
  `stage2_classify_ignores_endpoint_port_digits` 除断言端口不得触发可重试外,另断言
  `HTTP 502/503/504/408/429` 原文仍必须命中;`stage2_classify_ignores_endpoint_url_wording` 另断言
  `connection refused` / `dns failure` 仍必须命中。⇒ 不是"一律不重试"式假绿。
- **wave31_lsp_client_test.rs(编译兼容衍生豁免)** 专属判据:剔除全部 `use` 语句后,
  文件其余内容必须与基线**逐字节一致**;断言/区域普查必须**逐项完全相等**(不允许增也不允许减);
  新增 `#[allow(..)]` → FAIL。实测 `衍生豁免核验 PASS`;diff 恰为 `-LspRange` 与 `-use serde_json::json;`。
- 红队新增场景 D 双向验证:仅删导入 → Exit 0(无误报);叠加非导入改动 → Exit 1 `DERIVED_NON_IMPORT_CHANGE`。

## 4. 红队自证总览(gate_selftest.log)
17/17 项全通过:A 场景 12 类注入 + B 平台门禁失衡 + C `#[allow]` 消警 + D1 无假阳性 + D2 非导入改动被捕获。

## 5. 预存缺陷(非本波引入,**不修**,仅登记)
1. `path_permission.rs:764`(fn `win_prefix_normalization_strips_verbatim`,行 757 起)
   `if cfg!(windows) { assert_eq!(..); assert_eq!(..); }` **无 else**。
   已核验基线同一形态(`git show f70b1f3:…` 第 8-10 行为同样结构)⇒ 预存。
   Linux 上该用例执行 **0 条断言却上报 ok**,违反仓库「严禁无断言测试」停止线。
   注:门禁的 `ZERO_ASSERT_TEST` 只判**新增**测试,故不会误伤这条预存用例(已确认 static-only 0 违规)。
2. `.github/workflows/ci.yml`:唯一执行 `cargo test -p pony-agent-core` 的 job 是 `verify`(`runs-on: windows-latest`,行 13-14 → 行 55-57 步骤);
   `core-platform-check`(matrix: ubuntu-latest/macos-latest)只跑 `check`(`Core 非 Windows cfg 编译检查`),
   **从不在 Linux/macOS 上执行 core 测试**。⇒ 平台相关用例在 POSIX CI 从未被执行,本波 Linux-only 失败天然不可被 CI 捕获。

## 6. 事实登记(不裁决):`provider/mod.rs:5991` 的数字子串契约
`agent/provider/mod.rs` 中 `is_rate_limit_error_matches_provider_wording`(行 5990)断言
`is_rate_limit_error("{\"error\":{\"message\":\"inference tpm exhausted\",\"code\":\"429001\"}}")` 为 **true**。
事实:
- 该输入不含 `://`,C3 的 `strip_endpoint_fragments` 对其为空操作 ⇒ **仍绿**;executor 的"全绿"陈述成立。
- 该断言依赖 `RATE_LIMIT_LONG_BACKOFF_HINTS` 含裸词 `"429"`,命中的是**更长数字串 `429001` 内的子串**,
  即"数字子串命中"在**非 URL 语境**下仍是既有契约(本波未改动该用例)。
- 与 retry.rs 新用例形成的口径差异:同一机制(`contains` 数字)在 URL 语境内须被忽略、在非 URL 语境内须命中,
  分界是**位置**(是否位于 `scheme://` 片段)而非语义。若将来 provider 把 `code` 放进 endpoint/URL 片段,
  同一 payload 的分类会翻转。
- 处置:仅登记事实,不作判据变更、不改判(按 Lead 指示)。