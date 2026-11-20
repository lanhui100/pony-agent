# Agent Note: 构建工程四面治理——编译慢 / CPU 占用 / 交叉编译 / 产物占盘（不换语言）

Status: implemented

2026-11-20。起因：用户问「能否用 Go 改造」，逐条复盘后结论是**不换语言**——四个痛点的根因都不在
Rust 语言本身，而在本仓的构建设施与门禁缺口。本记录落四条改动与其取舍依据。

## Problem

先量后改。当日实测（16 核 / 1.5T 家目录盘，多会话共用）：

| 现象 | 实测 | 根因 |
|---|---|---|
| 编译慢 | 单跑 release 3m07s–5m32s，超订时 90min 未完成（`.dev-team/rfc-2026-11-17-build-budget.md`） | 并发超订 + 无编译缓存 + 单 crate 213k 行 |
| CPU 占用高 | load 45.86 / 16 核（2026-11-17 事故） | lane 各自开满核；`.dev-team/build-budget.json` 的 `load_ceiling_factor` **只在配置里声明，脚本从未执行**；且 `.cargo/config.toml` 的 `jobs = 3` 与预算的 4 是两份真源 |
| 交叉编译麻烦 | `dev:tauri` / `clean:*` / `schemas:*` 全部写死 `powershell -File *.ps1`；本机无 pwsh | 脚本层平台绑定；「换平台跑一遍」只能手工拼命令 |
| 产物占盘 | 产物合计 147G（target 94G + target-test 36G + target-check 16G） | `target/debug/incremental` 46G + `target-test/debug/incremental` 13G + `target-check` 12G，**全部不在任何清理档位里**（light 只删槽位、deep 才删根 target） |

关键观察：**incremental 目录合计 71G 是最大单块浪费**，而既有 `clean-targets.sh` 的两档
（light 删槽位 / deep 删根 target）刚好都碰不到它：light 保留根 target，deep 会把 213k 行 crate
的依赖产物一起抹掉（代价 20min+ 冷编）。缺的是「只清中间态、保留 deps」这一档。

## Decision

1. **负载熔断落地（CPU）**：`scripts/cargo-lane.sh` 实现 `load_ceiling_factor`——1 分钟 load >
   系数×核数 时先等（默认 300s / 10s 轮询）。**超时后默认继续并大声告警**，`--strict` 硬失败（exit 3），
   `--no-wait` 立即止损，`--allow-busy` 显式跳过。理由：共享机器上的 load 常来自本仓之外的进程
   （实测同期有 `social_media_copilot` 的 `cargo test --workspace`），硬失败只会让门禁假红。
2. **并发值单源（CPU）**：`.cargo/config.toml` 的 `jobs` 对齐预算的 `max_jobs_per_lane`（本轮实测后由 4 调为 8，
   见「第二轮补记」），并新增 `cargo-lane.spec.sh` P6
   断言两处必须相等（漂移即红）。config 保留 jobs 只是给「绕过 lane 直接敲 cargo」兜底。
3. **npm 的 cargo 入口全部走 lane（CPU）**：`scripts/invoke-rust-target.mjs` 增加
   `buildLaneInvocation` / `shouldUseLane`，`cargo:check:shared` / `cargo:test:shared` / `check:fast`
   等命令从「直接 spawn cargo」改为「经 `bash scripts/cargo-lane.sh`」，堵住既有的加锁漏洞
   （`PONY_BUILD_LANE=0` 可临时关闭；win32 需显式 `=1`，不假设 bash 存在）。
4. **增量档清理 + 产物预算门（占盘）**：`clean-targets.sh` 新增 `--incremental`——只删各槽位
   `debug|release/incremental`，deps/build/release 全保留；`artifacts-status.sh` 新增 `--budget`，
   按 `.dev-team/artifacts-budget.json` 判单目录与 incremental 合计，超预算 exit 1 并给出**当下可执行**的
   处置命令（有增量中间态 → `--incremental`；没有 → 明说只能 `--deep` 冷编，不给无效建议），
   缺预算文件 fail-closed exit 3。`.ps1` / `.sh` 双实现同步，`artifacts-tools.spec.sh` 补 5 条负样本。
   `target` 预算暂定 64G 以覆盖「既有陈旧依赖产物」这段已知债（清完增量后 target 仍 49G，其中约 43G
   是不可增量式清理的陈旧 rlib；降到 ~30G 需一次计划内 `--deep`），预算文件 notes 写明下调条件。
5. **平台入口去 PowerShell 硬绑定（交叉编译）**：新增 `scripts/run-portable.mjs`（任务表按平台分派，
   未登记即失败，不静默回退）+ `start-tauri-dev.sh` / `normalize-generated-schemas.sh` 两个 POSIX 实现；
   `npm run dev:tauri / clean:tauri:* / clean:incremental / schemas:*` 一律改为
   `node scripts/run-portable.mjs <task>`。新增 `cross-target-status.sh` 把「哪一层能交叉」说清：
   核心二进制可交叉到 windows-gnu（需 mingw + target），Tauri 桌面端与 macOS 在 Linux 上不可行。
6. **加速设施可见化（编译慢）**：新增 `build-accel-check.sh` 体检 sccache / mold / lld / mingw / 已装
   target / 增量目录体积，给安装建议；lane 的 sccache 注入**默认 auto**（只在冷编/全树场景注入，见
   第二轮补记——sccache 与增量编译硬互斥，无条件注入会让日常构建当场 exit 101），mold 仅在
   `PONY_BUILD_FAST_LINK=1` 时注入（改 `RUSTFLAGS` 会让既有产物全量失效，不能默认开）。
   新增 `npm run check:fast`（`--lib` 单 crate）作为日常快回路，与 `cargo:check:shared`（`--all-targets`）分工。

## Alternatives considered

- **A. 整仓 Go 重写**（用户最初提问的方向）：否决。39 万行 Rust（含 15.7 万行测试）迁移属季度级工程；
  Tauri 桌面壳（updater / single-instance / vibrancy / WebView2）、ratatui-image 终端图形协议、
  clippy 棘轮与 `check-ts-contract` 契约全绑 Rust 生态，Go 侧无 1:1 等价物。**四个痛点都不在语言层**，
  换语言只会把可修复的构建设施问题换成不可逆的迁移成本。
- **B. `cargo clean` 全清 / 直接 `--deep`**：否决。会连带删掉 600+ 依赖的编译产物，冷编 20min+；
  而 71G 里真正可再生的只有增量中间态，代价与收益不成比例。
- **C. 全局关掉 `incremental`**：否决。日常小改的增量收益真实存在；把「清中间态」做成独立档位，
  由预算门在涨到阈值时提示，比一刀切更符合实际使用曲线。
- **D. 合并 target-check / target-test 到单一 target 目录**：暂缓。能省 16G 并让 check 复用 test 的
  元数据，但与「dev server 长驻持锁」冲突（测试会被一次 dev 重编阻塞），且要改 4 处契约
  （`SLOT_TARGET_DIRS` / ps1 / 清档表 / AGENTS.md）。本轮先靠 incremental 档拿到 68G，
  合并留待有真实墙钟收益数据时再评估。
- **E. load 超限一律硬失败**：否决（见 Decision 1）。`--strict` 保留给 CI/Lead 场景。
- **F. mold 默认启用**：否决。会改 `RUSTFLAGS`，使既有产物全部失效重编——今天的收益立刻变成明天的冷编。
- **G. 拆分 `pony-agent-core` 为多 crate**：延后。这是「编译慢」的结构性解，但属跨模块大改，
  与本轮设施治理正交；先让设施与门禁到位，再谈拆分。

## Receipts

- `bash scripts/cargo-lane.spec.sh` → 18/18 PASS（原 11 条 + 新增 P6 并发单源 / P7 熔断可见 /
  P8 allow-busy / P9 --no-wait exit 3 / P10 加速缺失不改命令形状）。
- `bash scripts/artifacts-tools.spec.sh` → 13/13 PASS（新增 P1c 增量档语义 / P1d 空场景 /
  P1e 三条预算门）。
- `npx vitest run tests/invoke-rust-target-slots.spec.ts tests/run-portable-tasks.spec.ts` → 13/13 PASS。
- `bash scripts/clean-targets.sh --incremental`：删除 5 个增量目录，`df` 由 435G/32% 降到 **367G/27%，
  释放 68G，耗时 5.47s**；`target/release/pony` 与全部 deps 保留。清理后产物合计 **147G → 78G**，
  incremental 剩余 190M。
- `bash scripts/artifacts-status.sh --budget`：清理前 4 项超预算 exit 1（target 94G、target-check 16G、
  target-test 36G、incremental 71G）；清理后全部在预算内 exit 0。
- `npm run check:fast`（`-p pony-agent-core --lib`）：增量目录清空后 **40.42s exit 0**，同时验证
  `invoke-rust-target → cargo-lane → cargo` 的自动包装链路端到端可用。
- 前端全量单测：1579 例 1561 通过 / 12 跳过；6 个失败均为并发满载（load 26）下的 5s 超时假红，
  低负载复跑 8 个涉事文件 **145 例全绿**（含 `ponygo observe audit --level ob2` 由 exit 3 恢复 exit 0）。
- 门禁：`version:check` / `adr:check` / `backlog:gate` / `contract:check` 均 exit 0；
  `size:check` exit 1 为**既有**的 3 条超限（`config.rs` / `provider/tests.rs` / `runtime/tests.rs`，
  本次未触碰这些文件，基线未跟进），非本轮回归。
- `bash scripts/build-accel-check.sh`：如实暴露 sccache / mold / lld / mingw 全缺、已装 target 仅
  `x86_64-unknown-linux-gnu`（安装建议属人工动作，未在本轮执行；第二轮已装 sccache）。

## 第二轮补记（2026-11-20 晚，聚焦前两痛：编译慢 + CPU 占用）

第一轮把「设施的开关」装上了，但没做**标定**：`max_jobs_per_lane = 4` 与「sccache 自动注入」都是拍脑袋值。
本轮先量后改，两条都按实测修正，并补上可重跑的测量入口 `scripts/build-bench.sh`（`npm run bench:build`，
在既有槽位内做单变量对照、峰值内存按 rustc 子进程 RSS 采样、统一走 lane 不绕过并发预算）。

### 标定一：并发 4 → 8（CPU 预算与墙钟同时受益）

`cargo check --all-targets`（`pony-agent-core`：21.4 万行生产 + 15.7 万行测试，target-check 槽位，16 核 / 30G）：

| jobs | 冷编墙钟(s) | 热编墙钟(s) | 峰值 rustc RSS |
|---|---|---|---|
| 4 | 247.7 | 142.3 | 2.4G |
| 8 | 151.5 | 151.5 | 3.8G |
| 12 | 123.5 | 128.5 | 5.0G |

读法：**冷编（依赖需重编）对并发敏感，热编几乎不敏感**（±10% 落在噪声带）——热编被「单个 21.4 万行 crate
只能由一个 rustc 进程编」卡死，这恰恰是「拆分 crate」（Alternatives G）才是结构性解的反证。故取 **8**：
冷编 1.63×，热编不退化，峰值 3.8G 在 30G 机 + 他人会话共存下仍有余量；12 job 再快 19% 但峰值 5G，
保留为按需覆盖（`PONY_BUILD_JOBS=12`）。`max_jobs_per_lane` 与 `.cargo/config.toml` 的 `jobs` 同步改 8。

### 标定二：sccache 与增量编译**硬互斥**（推翻第一轮的「自动注入」）

- 实测：`CARGO_INCREMENTAL` 存在时 sccache 直接拒绝编译——`sccache: incremental compilation is prohibited`，
  cargo **exit 101**。而 dev/test profile 默认 `incremental = true`，所以第一轮写的「装了 sccache 就自动注入」
  会让**每一次日常构建当场红**（本轮一装 sccache 立即复现）。
- 改为 `PONY_BUILD_SCCACHE=auto`：只在「调用方已给 `CARGO_INCREMENTAL=0`」或「`--release`」的冷编/全树场景注入，
  且注入时**同时压 `CARGO_INCREMENTAL=0`**（互斥是前提，不是可选项）；`=1` 强制换缓存、`=0` 关闭。
- 依赖缓存收益（/tmp 探针，取本仓真实依赖子集 122 个 crate，`cargo check`，8 job）：
  冷编 **41.1s** → 空 target dir 全命中 **12.8s**（3.2×，114 次命中，缓存 41M）。适用 = 新 worktree 引导、`--deep` 清后冷编。
- sccache 以**绝对路径**注入：`~/.local/bin` 常不在 PATH 里（本机就是），裸名会静默退化成「没缓存」。

### 收据

- `bash scripts/cargo-lane.spec.sh` → **24/24 PASS**。P1/P3/P7/P10 的期望并发值改为从 `build-budget.json` 派生
  （不再写死数字，调值不再连带改用例）；新增 P11（绝对路径 wrapper + 强制关增量）、P12（auto 不劫持日常增量构建）、
  P13（release 自动启用）。
- `python3 ~/.agents/skills/dev-team/scripts/dev-build-budget-lint.py .` → PASS（每 lane ≤8 job）。
- 复跑标定：`bash scripts/build-bench.sh --jobs 4,8,12 --all-targets`（冷编口径先清增量或换空槽位）。
- 仍待人工动作：mold / lld / mingw 缺（`bash scripts/build-accel-check.sh` 如实列出，需 apt + sudo，本轮未装）。

### Alternatives considered（第二轮追加）

- **A′. 保持「装了 sccache 就自动注入」**：否决——实测与 dev/test 的 `incremental = true` 硬冲突（exit 101），
  等于把「提速设施」做成「日常构建全红」。
- **B′. 默认 12 job 吃满 2.0× 冷编收益**：否决为默认值。峰值 5G 且同期常有本仓之外的工程在同一台机上跑，
  叠加后正是 2026-11-17 事故的形状；8 已拿到 1.63×，剩下 19% 交给 `PONY_BUILD_JOBS` 按需换。
- **C′. 关掉 incremental 换取缓存命中**：否决为默认。日常小改的增量收益是真实的（本次实测 8.14s/次），
  缓存只在「换 worktree / 深清」时才有价值，因此做成按场景 opt-in 而不是全局取舍。

## 第三轮补记（2026-11-20 夜，外部对抗审核后的修复）

用 ACP 委派外部 agent（Antigravity）对前两轮改动做对抗式审核，结论「修后合并」，提 4 条 P0 / 5 条 P1 / 2 条 P2。
逐条复核后**采纳 10 条**、**记录不采纳 3 条**（含 1 条误报与 2 条会破坏既有契约）。

### 已修

| # | 位置 | 问题 | 修法 |
|---|---|---|---|
| P0-1 | `scripts/start-tauri-dev.sh` | `cargo-lane.sh run` 让长驻桌面进程一直持全局 flock ⇒ 其他会话的 check/test 无限期等锁 | 改 `build`（编译完即释放锁）后直接运行 `target/debug/pony-agent` |
| P0-2 | `scripts/build-bench.sh` | 每轮 `mktemp` 的峰值文件只回收最后一轮，前几轮泄漏在 /tmp | 轮内 `rm -f`，EXIT trap 仅兜底 |
| P0-3 | `scripts/build-bench.sh` | `ps -eo rss,comm` 抓全机 rustc ⇒ 采样被他人会话污染，标定失真 | 只累加以本任务为根的进程子树 |
| P0-4 | `scripts/build-bench.sh` | 某轮失败被后续轮成功掩盖，且失败轮以「超快墙钟」进表 | 任一失败即置标志、表内标 `FAILED`、最终非零退出 |
| P1-1 | `scripts/normalize-generated-schemas.sh` | 无变更也 `mv` ⇒ 刷新 mtime，`--watch` 与 Vite/Tauri 监视器无限重载 | `cmp -s` 相同则只删临时文件 |
| P1-2 | `scripts/artifacts-status.sh` | incremental 合计只统计 `debug/`，与 `--incremental` 档的 debug+release 口径不一致 ⇒ release 侧膨胀漏报 | 同时遍历 `debug` 与 `release` |
| P1-3 | `scripts/cargo-lane.sh` | `grep '--release'` 子串匹配会误触发 sccache（并强改 `CARGO_INCREMENTAL=0`） | 遍历参数列表精确判等 |
| P1-4 | `scripts/clean-targets.ps1` | `$fixedTargets` 漏 `target-perf`（与 `.sh` 漂移，Windows 侧清不掉） | 补 `target-perf` |
| P2-1 | `scripts/artifacts-status.sh` | `node_modules` 计入「产物合计」⇒ 数字虚高（它是前端依赖，不归清理档管） | 从 `DIRS` 剔除 |
| P2-2 | `scripts/build-bench.sh` | `--jobs ""` 会让 `for x in ${EMPTY}` 退化成语法错误 | `parse_list` 校验非空/非负，并区分 jobs≥1、sccache≥0 |

### 记录不采纳

- **锁路径加 UID**（建议 `/tmp/pony-agent-build-${UID}.lock`）：本仓是单用户 + 多 worktree 场景，加 UID 反而让同一用户在提权/非提权会话间绕开同一把锁；跨用户共享机已有 `PONY_BUILD_LOCK` 覆盖通道。改路径还要连带改 `build-budget.json` 真源与 spec 断言，收益不抵。
- **`--with-frontend` 改成只清前端、不连带清槽位**：`--with-frontend` 在 AGENTS.md 与 `clean-targets.sh` 里的语义是「**额外**」，`artifacts-tools.spec.sh` P3 也按该语义断言；改成互斥属于契约变更，不在本轮范围。
- **`clean-targets.sh` 进程替换内 `exit 2` 不终止父进程**：该 `exit` 只用于早退分支，后续 realpath 安全闸会拦住空候选；不为此重构结构。

### 收据（第三轮）

- `bash scripts/cargo-lane.spec.sh` → 24/24 PASS。
- `bash scripts/artifacts-tools.spec.sh` → **14/14 PASS**（新增 P1f：`target-test/release/incremental` 超预算必须 exit 1，堵住「只测 debug」的假绿）。
- `bash scripts/build-bench.sh --jobs 8 --sccache 0` → 实跑 33.59s / 峰值 1432MB（lib 口径，进程树采样），表格与退出码正常。
- 参数校验：`--jobs 0` → exit 2；`--target-dir target-xxx` → exit 2。
- `bash scripts/normalize-generated-schemas.sh` 连跑两次：第二次 mtime 不变（幂等成立）；首轮补了 4 个 schema 的结尾换行（`git diff` 仅 `\ No newline at end of file`）。
- `bash -n` 覆盖全部改动脚本，全部通过。

## 第四轮补记（2026-11-20 深夜，第二次外部对抗审核 + 增量编译标定）

再委派 Antigravity 对「第一轮设施 + 新增 `build-bench.sh`」做第二轮审核，报 1 条 P0 / 4 条 P1 / 3 条 P2。
复核后**采纳 3 条**（其余 2 条 P1 是「已修但未同步到另一文件」的同源漂移，按第三条一起处理），P2 记入待办。

### 已修

| # | 位置 | 问题 | 修法 |
|---|---|---|---|
| P0 | `scripts/build-bench.sh` | 每轮用 `awk 'END{print $0" "p}'` 原地重写结果文件，只保留末行 ⇒ `--jobs 4,8,12` 的对比表永远只剩一行 | 每轮写独占 `row_out`，主 shell 把「本轮行 + 本轮峰值」合成一条**追加**，历史行一字不动 |
| P1 | `scripts/clean-targets.sh` | `--incremental` 分支重置候选列表，吞掉 `--with-frontend` 已收集的前端目录，而日志仍打印「+ frontend」 | 重置后重新追加前端候选；`artifacts-tools.spec.sh` 新增 **P1g** 锁死该组合 |
| P1 | `scripts/build-accel-check.sh` | 增量体积只数 `debug/incremental`，与 `--incremental`/`--budget` 的 debug+release 口径漂移 ⇒ release 侧膨胀漏报 | 与另两处对齐，同时遍历 `debug` 与 `release` |

### 增量编译标定（回答「incremental 到底值不值」）

`bash scripts/build-bench.sh --jobs 8 --sccache 0 --incremental {1,0} --touch crates/pony-agent-core/src/agent/retry.rs`
（target-check 槽位，lib 口径，进程树采样峰值 RSS）：

| incremental | 墙钟 | 峰值 RSS | 说明 |
|---|---|---|---|
| 1（暖，连跑两次） | **8.09s / 8.41s** | 1.04G / 1.02G | 改一个叶子模块的日常回路 |
| 0 | **31.28s** | 1.44G | 同一次改动退化为整 crate 重查 |

结论：增量编译在本仓给 **3.9×**（8s vs 31s），这就是 `PONY_BUILD_SCCACHE` 默认 `auto`（日常不注入、只在冷编/`--release` 换缓存）的量化依据；
「关增量换 sccache」在本机是净亏，只有「换 worktree / `--deep` 清后冷编」才划算。

### 并发值复标（jobs=8 的依据补全）

`bash scripts/build-bench.sh --jobs {4,8,12} --sccache 0 --all-targets`（同一冷编起点，增量关）：

| jobs | 墙钟 | 峰值 RSS |
|---|---|---|
| 4 | 135.11s | 2.5G |
| 8 | **125.27s** | 3.8G |
| 12 | 126.27s | 4.9G |

12 job 相对 8 已经**没有墙钟收益**（±1%），却多 1.1G 峰值 —— 维持 `max_jobs_per_lane = 8`，需要抢时间时用 `PONY_BUILD_JOBS=12` 临时覆盖。

### 收据（第四轮）

- `bash scripts/cargo-lane.spec.sh` → 24/24 PASS；`bash scripts/artifacts-tools.spec.sh` → **15/15 PASS**（新增 P1g）。
- `bash scripts/build-bench.sh --jobs 4,8 --sccache 0`（lib）→ 两行齐出（32.15s / 34.78s），确认 P0 修复后多轮对比不再丢行。

### 未修（登记下一轮，属「平台入口」面）

- **`scripts/invoke-rust-target.ps1` 绕过 lane**：`.mjs` 已被本轮的 `buildLaneInvocation` 包成 lane 调用（`grep -c cargo-lane` = 3），
  `.ps1` 仍是裸 `cargo`（= 0），于是 **Windows / 主 CI（`runs-on: windows-latest`，`shell: pwsh`）这条路依旧不吃并发预算与全局 flock**。
  这是「一份逻辑两份实现必然漂移」的实证，也说明 `.ps1` 的收敛方向应是「单 Node 实现 + CI 改 node 入口」，而不是再写第三份 `.sh`。

## Alternatives considered（第四轮追加）

- **A″. 给 `build-bench.sh` 加 `--repeat N` 取中位数**：暂缓。当前 ±10% 噪声来自共享机 load，重复轮次会成倍放大标定成本（每轮 = 一次真实全树 check）；先用「同起点 + 冷/热分开报」控噪，等真要卡 5% 级差异再加。
- **B″. 默认 `jobs=12`**：否决，见上表——12 已无墙钟收益，只加内存峰值。
- **C″. 把 `--incremental` 与 `--with-frontend` 改成互斥报错**：否决。`--with-frontend` 在 AGENTS.md 里是「**额外**」语义，改成互斥是契约变更；补候选 + 加 P1g 用例成本更低且不改变既有契约。

## 第五轮补记（2026-11-20，`.ps1` 通用适配 + sccache 落地 + 外部评审修复）

承接第四轮「`invoke-rust-target.ps1` 仍是裸 cargo，Windows/主 CI 绕开并发预算」这条未修项，
本轮按「通用适配」而不是「删 PS1 / 再写一份 .sh」处理。

### 决策

1. **单一转派真源**：新增 `scripts/lib/posix-dispatch.ps1`。`*.ps1` 保持 Windows 入口身份，
   非 Windows 主机把执行交给同名 `.sh` / `.mjs` 落点；参数由**调用点显式翻译**
   （`-Deep` → `--deep` 这类映射写死在调用点，不自动猜——命名参数无法从 `$args` 还原，
   自动翻译会在参数改名时静默错配）。
2. **逻辑重复的脚本退化成薄壳**：`invoke-rust-target.ps1`、`check-version-sync.ps1` 用
   `-AlwaysDelegate`（**所有平台**含 Windows 都转派单一 Node 真源），把「一份逻辑两份实现必然漂移」
   这一类缺陷从根上消除——第四轮实测的「`.ps1` 裸 cargo 绕过 lane」正是这一类。
3. **无落点的脚本 fail-closed**：`bump-version.ps1`（376 行 bump 逻辑，含 AutoDetect/Stage/DryRun，
   与 `release.mjs` 语义不同，没有可安全转派的等价物）以 `-WindowsOnly` 显式退出 4 并指路
   `npm run release`；缺口登记在白名单里，不伪装成支持。
4. **门禁机械化**：新增 `scripts/check-ps1-portability.sh`——每个 `scripts/*.ps1` 必须落进
   「有 POSIX 落点 / Windows-only 白名单 / 自身已适配」三类之一；匹配**只认非注释行**
   （否则把转派语句注释掉仍会绿）。`check-ps1-portability.spec.sh` 提供 P1–P7 七条正负样本。
5. **sccache 落地**：`sccache 0.19.0`（musl 预编译）校验 sha256 后装入 `~/.local/bin`；
   `cargo-lane.sh` 的 `resolve_tool` 改为「PATH 之外再搜 `~/.local/bin` / `/usr/local/bin`」
   并注入**绝对路径** `RUSTC_WRAPPER`（裸名在子进程 PATH 缺失时静默失效）。
   策略维持「增量优先」：auto 不劫持日常构建（`RUSTC_WRAPPER` 进指纹会让增量失效），
   release 自动启用，冷编/跨 worktree 显式 `PONY_BUILD_SCCACHE=1`。

### Alternatives considered（第五轮追加）

- **J. 直接删掉 `.ps1`，只留 POSIX 脚本**：否决。Windows 侧仍有不可替代的平台逻辑
  （Job Object / WebView2 / 符号链接与注册表策略），删掉等于用「Linux 能跑」换「Windows 不能跑」。
- **K. 在 dispatcher 里自动把命名参数翻译成长选项**：否决。参数名不是稳定契约，
  自动翻译在改名时静默错配；调用点手写 3 行翻译后，错就是静态门红。
- **L. 让 `bump-version.ps1` 转派 `release.mjs`**：否决。`release.mjs` 会 `git commit` + `git tag`，
  与 bump 语义不等价，转派等于把「只改版本号」变成「顺手发版」。

### Receipts（第五轮）

- `bash scripts/check-ps1-portability.sh` → **PASS**（7 个 `.ps1` 全部登记：5 个有 POSIX 落点，
  1 个 Windows-only，1 个自身已适配）。
- `bash scripts/check-ps1-portability.spec.sh` → **P1–P7 全 PASS**，含
  P7「把转派语句注释掉必须红」（守住注释假绿）、P5「新增未登记 ps1 必须红」。
- `bash scripts/cargo-lane.spec.sh` → **PASS**（含 P12 auto 不劫持增量构建 / P13 release 自动启用 sccache）。
- `bash scripts/artifacts-tools.spec.sh` → **PASS**。
- `npx vitest run tests/invoke-rust-target-slots.spec.ts tests/run-portable-tasks.spec.ts` → **14/14 PASS**。
  其中槽位 parity 用例由「两份槽位表逐槽比对」升级为「ps1 薄壳断言」：`invoke-rust-target.ps1`
  薄壳化后不再含 `$targetMap`，旧用例读空表必然红——属**旧契约残留**而非新回归，本轮同一提交内升级。
- 外部评审（Antigravity ACP）：首轮 `session/new` 30s 超时（`acp_timeout`），重试后完成，给出
  5 条意见（2 中 / 3 低），本轮**全部落盘修复**：
  ① 门禁 grep 命中注释造成假绿 → `strip_comments`；
  ② bench 采样 0.5s 过粗且退出竞态 → 0.1s + 未采到标 `n/a`（不再输出误导性的 0MB）；
  ③ 映射表解析为空静默跳过 → 判空即 `fail`；
  ④ bench 表格未标注热缓存偏置 → 表尾显式声明可比范围；
  ⑤ dispatcher 在 Windows 下仍打印「非 Windows 主机」→ 按环境区分文案。
  评审确认**不成立**的风险：`param()` 位置、`$args` splatting、`$PSScriptRoot` 双候选探测、
  dot-source 内 `exit` 语义、BOM 保留。
- **仍未做（显式留账）**：`build-bench.sh` 的 jobs 矩阵尚未在低负载独占窗口跑全量；
  `max_jobs_per_lane = 8` 的取值来自第四轮既有标定，本轮未重新标定。跑法已固化：
  `npm run bench:build -- --jobs 4,8,12`。

### CI 暴露的缺陷（同日修复，PR #34 首轮）

`.ps1` 薄壳化让 CI（`shell: pwsh`）**第一次真正进入 lane**，随即把一条既有缺陷打出来：

- 现象：`core-platform-check(macos-latest)` 与 `(ubuntu-latest)` 双双 fail（55s / 1m6s）。
- 日志序列：`[invoke-rust-target] 非 Windows 主机 → 转派 node scripts/invoke-rust-target.mjs …`
  （dispatcher 按预期生效）→ `[cargo-lane] fail-closed：缺少 flock（跨输出目录串行化不可用，拒绝无界并发）` → exit 1。
- 根因：`cargo-lane.sh` 把 `flock(1)` 当硬依赖。macOS 自带环境没有该命令，而「缺 flock 就拒绝」
  等价于「本仓在 macOS 上不可构建」。此前 `.ps1` 裸调 cargo（不吃 lane）把这条缺陷遮住了——
  正是本 PR 要消除的那类漂移。

**修复**：flock 存在时行为完全不变；缺失时降级为 **mkdir 原子锁**（POSIX 通用）——
锁目录存 `pid`，持有者已消亡则回收陈旧锁，等待 300s 超时 fail-closed(exit 3)；
`PONY_BUILD_LOCK_IMPL=auto|flock|mkdir` 可显式覆盖，显式要求 flock 但本机缺失时仍 fail-closed(exit 2)。
dry-run 在 mkdir 模式下如实打印 `mkdir-lock <path>.d`，不假装有 flock。

**收据**：`cargo-lane.spec.sh` 新增 P14（mkdir 锁可见且不假装 flock）/P15（显式 flock 回归保护）/
P16（非法值 fail-closed）全 PASS；`PONY_BUILD_LOCK_IMPL=mkdir bash scripts/cargo-lane.sh --version`
→ rc=0 且 `/tmp/pony-agent-build.lock.d` 无残留。
