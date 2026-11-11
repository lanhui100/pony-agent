# Agent Note: Windows 全量集成测试常开 `--no-fail-fast`（首错即停的代价与替代）

Status: implemented

> **归档冻结说明**：本文档为原历史决策 `docs/archive/decisions/0121-windows-ci-no-fail-fast.md` 之归档快照，仅供溯源。

## 背景

- 集成测试主体只在 `windows-latest` 的 verify job 执行（`ubuntu`/`macOS` 的 core-platform-check 此前只跑 `cargo test --lib`），故「产品已改、用例未跟」与平台缺口只能在该 job 暴露。本 ADR 同时登记配套补丁：PTY 族 4 例在 Windows 上被平台 gating（缺口 B067）后，`core-platform-check` 的 ubuntu 分支新增一个 Linux-only 集成执行点（只跑 `tui_startup_input_guard` + `tui_overlay_cancel_acceptance`），避免「gating = 覆盖消失」；`--lib` 之外的集成面在 Linux 上仍只覆盖这一子集，其余集成面继续由 windows job 承担。
- 该 job 的 core 测试步骤原为 `npm run cargo:test:shared -- --package=pony-agent-core`。`cargo test` 默认**首个失败用例即中止**（fail-fast），于是同一 runner 上后续的测试二进制根本不会执行。
- 实测代价：Windows 全量首跑以「十连红」形式逐层剥出缺口——B050（会话日志强制锁）→ B051（8.3 短路径）→ B052/B053（读写锁与分隔符）→ B054（折叠前放锁）→ B056（过期断言）→ B057（OTLP 假 collector 不回 200）→ B066（hook 夹具走 PowerShell）等。每轮 CI ≈ 40 分钟只暴露**一个**二进制的问题。
- 收敛手段：临时验证分支把该步骤改为 `--no-fail-fast` 后，**一次运行**即拿到 11 个二进制 / ~23 例的完整清单（run 42988189901），据此按族批修。

## 决策

1. verify job 的 core 测试步骤**常开** `--no-fail-fast`：
   `npm run cargo:test:shared -- --package=pony-agent-core --no-fail-fast`。
2. 该标志只改变「失败后是否继续」，**不改变退出码语义**：任一用例失败，步骤仍以非零退出，门禁强度不降。
3. 代价显式接受：红的时候日志更长、失败面更宽；换来的是「一次运行拿到完整缺口清单」，替代「每轮只剥一层」。
4. **配套：PTY 族 gating 必须有 Linux 执行点**。PTY 驱动依赖 Python `pty/termios`（Unix-only），
   Windows 侧改 `#[cfg_attr(windows, ignore)]`（缺口登记 B067）后，若 Linux 不执行就等于覆盖归零；
   故 ubuntu 分支新增只跑这两个二进制的步骤（见背景）。该补丁只覆盖 PTY 子集，不改变「其余集成面
   仍只在 windows job 执行」的事实。
5. 传参链已核：`package.json` → `scripts/invoke-rust-target.ps1`（`ValueFromRemainingArguments` 追加到 cargo test，并以 `exit $LASTEXITCODE` 传递退出码）。

## 候选方案

- **保持 fail-fast（不改）**：CI 反馈更短，但每次只暴露首个失败二进制；实测需 ~10 轮 CI 才能清完一批平台缺口，总墙钟与 token 成本更高。否决。
- **分片跑测试二进制（矩阵 job）**：反馈最快，但需要维护二进制清单并新增 job/矩阵，成本与漂移风险高于收益。否决（留作将来若单 job 超时的预案）。
- **`--no-fail-fast` + 失败时自动聚合摘要**：更易读，但要改 npm 脚本与日志解析，本批不做；先落最小改动，摘要需求出现再加。推迟。
- **只在验证分支开**：会重新引入「主 CI 看不见全貌」的问题，缺口仍靠多轮剥出。否决。

## 后果

- 失败时日志与红面变宽：需要在读日志时按 `test result: FAILED` 与二进制名定位；`Running` 行与失败段交错属正常现象（归档清单时按二进制聚合）。
- 与 `docs/observability.yaml` 无冲突：不改对外接口、网络 IO、后台任务、LLM 调用或 UI，属结构性 N-A；不改 CI 门禁判据（`.meta/gates/` 未动）。
- 后续：若某次全量运行时长逼近 job 上限，优先拆分片矩阵（本 ADR 的推迟项），而不是回退 fail-fast。

