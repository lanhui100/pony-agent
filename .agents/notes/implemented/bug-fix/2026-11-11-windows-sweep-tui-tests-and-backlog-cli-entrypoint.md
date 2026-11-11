# Agent Note: Windows 首跑扫尾——状态行轮播相位钉住、会话库沙箱旁路、Ctrl+Z 平台化断言、backlog CLI 入口判定

Status: implemented

## Problem

Windows 全量首跑把四类「只在 Windows 才显形」的缺陷暴露为红相（Linux 本地从未跑到）：

1. **`tui_dialog_wave39::a12` 墙钟随机红（全平台潜伏，本单最终未保留改动）**：ADR 0119 把底面行右槽改为「模型身份 ⇄ 权限模式」3s 时分轮播，但 `status_carousel_phase` 在 `status_carousel_started_at = None` 时用**墙钟**取相位；a12 未钉住相位起点，落在权限相位即红（Windows 实测 `实际 Rgb(162,155,146)（symbol="权"）`，而断言要求 GHOST）。按周期 6000ms、Model hold 3000ms 计算，Linux 上同样是约 50% 概率的 flake。**归属转移**：同波另一路（Windows 扫尾主线）已在同一文件引入 `pin_carousel_phase(state, model_phase)` 夹具（偏移量取自产品常量、并当场自证相位生效），并迁移了 a12/a4/权限相位三条断言；本单据此**撤回**自己那行 `status_carousel_started_at = Some(Instant::now())`，避免两处相位夹具并存（该文件写域归其所有）。
2. **`tui_persistence_truthfulness::persistence_evidence_reads_the_real_session_database` 沙箱在 Windows 失效且污染真实库**：用例用 `XDG_DATA_HOME` / `HOME` / `LOCALAPPDATA` 重定向数据目录；但 `dirs::data_local_dir()` 在 Windows 走 known-folder API（`SHGetKnownFolderPath`），**不读**任何环境变量 ⇒ 集成测试写进 `%LOCALAPPDATA%\PonyAgent\sessions.db`，沙箱目录里自然没有库文件，断言必红（Windows 实测：「前置条件：会话库应落在沙箱里…」）。
3. **`tui_signal_reentry::r6` 要求不存在的平台能力**：`tui::input::is_suspend_shortcut` 在非 unix 平台恒 `false`（Windows 无 SIGTSTP / `fg`，挂起即无法恢复），而用例写死「裸 Ctrl+Z 必须命中」。
4. **`scripts/backlog.mjs` 在 Windows 上作为 CLI 静默不执行（解阻项，非 B064 原清单）**：`isDirect` 用 `path.resolve(new URL(import.meta.url).pathname)` 与 `path.resolve(process.argv[1])` 比较；Windows 上 URL pathname 是 `/C:/repo/scripts/backlog.mjs`，经 `path.win32.resolve` 得 `\C:\repo\…`，与 argv[1] 的 `C:\repo\…` 永不相等 ⇒ `isDirect` 恒 false ⇒ `node scripts/backlog.mjs lint` 空 stdout、exit 0、账本不落盘。后果是 `tests/backlog-cli.spec.ts` 2 例恒红，连带 `v10-baseline-freeze` 的「实跑既有全量单测」meta 用例恒红，**Windows verify job 在「前端单元测试」步骤即失败，根本走不到 Rust 测试步骤**——这使 B064 的剩余清单在 Windows 上无法被观测。

5. **`resolve` 不推进高水位 ⇒ governance CI 恒红（平台无关，本波解阻时发现）**：`.next-id` 语义是「下一个候选编号」，`record` 会回写、`resolve` 不回写；一旦闭环掉「当前最高编号」的任务且未再开新任务，`.next-id` 就落后于「活跃∪归档」最大编号，`scripts/backlog.mjs lint` 判 `ID B0xx 越过高水位（疑似手工改号）`，`.github/workflows/ponygo-governance.yml` 的 lint 步骤随即失败（实测：main 上连续多次 governance 红，`B066` 越界）。

## Decision

1. **a12 相位钉住：让位给同波主线**（本单不落改动）。同一文件同波已由主线提交 `pin_carousel_phase(state, model_phase)` 夹具并迁移 a12/a4/权限相位断言；两份夹具并存会造成相位口径分裂，故本单撤回自己的写法，`tui_dialog_wave39.rs` 写域整体归主线，本单不再触碰。
2. **新增数据目录旁路 `PONY_AGENT_DATA_DIR`（测试注入专用）**：`default_sqlite_path()`（`agent/sqlite_session.rs`）与 `default_storage_path()`（`agent/session/store.rs`）在 `cfg(not(test))` 下**最优先**读它，未设置时默认路径逐字不变；`cfg(test)` 分支仍走 `unique_test_*` 临时隔离。集成测试同时把 `XDG_DATA_HOME`/`HOME`/`LOCALAPPDATA` 指向 `decoy` 目录，并**新增反例断言**「decoy 下不得出现 sessions.db」——在 Linux 与 Windows 上都证明旁路真的生效，而不是碰巧绿。
3. **r6 按平台断言**：`cfg!(unix)` → 命中为真；非 unix → 命中为假，理由写进断言消息。反向对照（多余修饰键 / `'Z'` / `Ctrl+C`）两平台共用保留。
4. **`backlog.mjs` 入口判定改为 URL 比较**：两侧先 `fs.realpathSync` 再 `pathToFileURL`，win32 下做小写归一（大小写不敏感文件系统），POSIX 保持逐字比较；解析失败保守返回 `false`。
5. **`resolve()` 闭环时一并 `writeHighWater(paths, id)`**：与 `record()` 的分配口径对齐（单调不减、删除/归档不复用编号）；`lint` 的比较方向不动（`n >= nextId` 才是对的，放宽成 `n > nextId` 会让 `allocateId` 发出的编号与既有编号撞车）。

## Alternatives considered

- **给三个用例加 `#[cfg_attr(windows, ignore)]`（沿用 B067 PTY 缺口模式）**：落选——PTY 是平台**能力**缺失（Python `pty`/`termios` 不存在），而这三个用例在 Windows 上可执行、且对应真实用户可见契约（状态行配色、退出提示不得撒谎、快捷键语义）。用 ignore 会把「可修的断言缺陷」伪装成「平台不可能」，等于去覆盖换绿。
- **在 a12 里同时断言两个相位**：落选——相位断言已由 `tui_permission_mode_acceptance` 的双相位用例承担；在这里重复会制造两处需要同步维护的相位口径，正是 ADR「一个事实一个家」要避免的。
- **把 `PONY_AGENT_DATA_DIR` 做成用户可见配置项（写进 AppSettings / 文档）**：落选——本次只需要测试注入缝，扩大成用户契约要额外的配置校验、可观测性与文档面，超出最小闭包。
- **改 `isDirect` 为 `process.argv[1]?.endsWith('backlog.mjs')` 之类的宽松匹配**：落选——宽松匹配会让「同名脚本」误判为直接执行（例如被 `tests/` 内同名 harness import 时），URL 逐字比较才是原来想要的语义。
- **把 `default_storage_path()` 一并改成读 XDG（跨平台统一 `dirs` 行为）**：落选——那是改变产品默认存储位置的不可逆面（老数据不迁移就读不到），远大于本工单范围；旁路方案对未设置 env 的用户零行为变化。
- **Windows 上跳过持久化用例**（`cfg!(unix)` 包裹）：落选——「退出提示不得撒谎」是 P0 事故验收（ADR 0106/0107 语境），Windows 用户同样在这条契约下；放弃覆盖等于承认缺口而不修。

## Consequences

- 行为变化（用户可见）：无（未设置 `PONY_AGENT_DATA_DIR` 时路径解析逐字不变）。
- governance 红收口口径：代码侧修复后，`.next-id` 的历史滞后值需由下一次 `record`/`resolve` 自然推进（当前工作区已为 68），故 main 上 governance 变绿依赖账本数据同步，而非再次改代码。
- 新增面：`PONY_AGENT_DATA_DIR` 环境变量，语义「数据目录根（其下仍拼 `PonyAgent/`）」，定位为**测试注入缝**，与 backlog CLI 的 `PONY_BACKLOG_ROOT` 同构；未写入用户文档。
- 测试强度：持久化用例新增一条反例断言（覆盖增强，非放宽）；r6 由「单平台硬编码」变为「双平台各自有断言」；a12 的相位确定性由同波主线夹具承担。
- 机器收据：`cargo test --target-dir target-test -p pony-agent-core --test {tui_signal_reentry,tui_persistence_truthfulness}` 均 exit 0（`tui_dialog_wave39` 138 passed 亦实测 exit 0，但该文件改动归主线）；`npx vitest run tests/backlog-cli.spec.ts` exit 0；`node scripts/backlog.mjs lint|doctor` exit 0。
- Windows 真机收据（`[verify-only]` 分支 `verify/b064b-1794391831`，run 43001473440）：`tui_signal_reentry` **exit 0 / 111 passed**、`tui_persistence_truthfulness` **exit 0 / 112 passed**、日志打印 `ALL TARGETED TEST BINARIES PASSED ON WINDOWS`；`node scripts/backlog.mjs lint` 在 Windows 上**已能执行并输出真实判定**（`[backlog-lint] ID B066 越过高水位…FAIL`，exit 1）——这正是入口判定修复前「空 stdout + exit 0」症状的消失证据，暴露出的红是本单第 5 条高水位缺陷（已同单修复）。
- 残留（如实登记）：Windows PTY 相关覆盖缺口仍归 **B067**；`tui_render_layout` 两条 ADR 0078/0119 契约冲突断言由同波另一路（状态行口径 + ADR supersede 注记）处理，本单未触碰其写域。
