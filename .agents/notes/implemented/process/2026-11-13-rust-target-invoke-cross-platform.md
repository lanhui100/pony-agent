# Agent Note: Rust target 槽位调用器跨平台化（B140）

Status: implemented

## Problem

`npm run verify` 的 Rust 编译检查步 `cargo:check:shared` 硬编码 Windows PowerShell：

```
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/invoke-rust-target.ps1 check check
```

本机（dev 服务器，Linux，无 `pwsh`/`powershell`）执行到该步必然 `sh: powershell: not found`
（exit 127），导致 `verify` 整链**结构上不可能变绿**。代价不是多一条红，而是：

1. 本机无法得到一次完整的 `verify` 退出码，验收只能退化为「逐项手跑 + 口头声明」，
   而常载命约第 2 条要求机械可查的承诺必须有一条非零退出的命令。
2. 该缺口已被下游绕过：`scripts/audit/build-install.sh` 用「探到无 pwsh 就自动降级为等价序列」
   兜底，任何在本机跑的审核回执都记录 `verify_mode=linux-fallback`。降级分支一旦成为常态，
   它与真 `verify` 的差异就没人再核对。
3. 同一族脚本（`cargo:test:shared`、`cargo:test:exact[:b|:c]`）同样被锁死，Linux 侧无法用
   仓库约定的 `target-test*` 槽位跑测试。

## Decision

新增 `scripts/invoke-rust-target.mjs`（node，零依赖）作为槽位调用器的跨平台实现，语义逐项对齐
PowerShell 版：`node scripts/invoke-rust-target.mjs <slot> <cargo 子命令> [cargo 参数...]` 等价于
`cargo <子命令> --manifest-path <repo>/src-tauri/Cargo.toml --target-dir <repo>/<槽位目录> [参数...]`，
退出码透传 cargo（cargo 不在 PATH = 127，用法错误 = 1）。

- `package.json` 的 5 个 `cargo:*:shared` 脚本改为调用 mjs；脚本名与参数形状不变，
  对外契约零变化。
- `scripts/invoke-rust-target.ps1` **原样保留**，继续服务既有的直接调用方（CI
  `core-platform-check` 的 `shell: pwsh` 步骤、文档里的直连用法）。不改成「ps1 转调 mjs」的 shim，
  理由是那段 PowerShell 在本机无法执行验证，改动等于把未经实测的代码推给 Windows CI。
- 两份槽位表的一致性由 `tests/invoke-rust-target-slots.spec.ts` 机械守住：解析 ps1 的
  `$targetMap` 与 mjs 的 `SLOT_TARGET_DIRS` 逐槽比对，并断言 5 个 npm 脚本指向 mjs 且不含
  `powershell|pwsh`。`npm run test:unit` 在 `verify` 链内，故漂移即红。
- `scripts/audit/build-install.sh` 不再按平台自动降级：默认走全量 `npm run verify`；
  等价序列保留为显式逃生口 `--verify-mode=fallback`（回执字段 `verify_mode` 取值从
  `npm-verify|linux-fallback` 收敛为 `verify|fallback`）。

## Alternatives considered

- **改 ps1 为 shim 转调 mjs（单一实现）**：语义上最干净，但本机没有 `pwsh`，无法对 shim 做一次
  真执行；把未实测的 PowerShell 推给 Windows CI 验证，等于用 CI 当本地编译器。选「双实现 + parity 用例」
  换取 Windows 侧路径零风险。
- **在 `verify` 里加 `pwsh` 存在性探测 + 条件跳过**：跳过即「门禁在 Linux 上不存在」，
  与「不因换跑法判 pass」直接冲突，且跳过不会留下任何红信号。
- **装 `pwsh` 到本机（apt/pip 包）**：把仓库可移植性缺陷转成环境前置条件，新机器/CI 复现一次
  就重演；且与「机械可查的承诺」目标反向。
- **只修 `cargo:check:shared`，其余 4 个测试槽位继续走 PowerShell**：同源缺陷留半条，
  下一个人踩 `cargo:test:shared` 时还要再做一次判断。

## Consequences

- 本机 `npm run verify` 整链可绿（`cargo:check:shared` exit 0，`Finished dev profile in 1.63s`），
  「verify 绿」重新成为可引用的验收证据。
- 代价是两份槽位实现并存。漂移由 `tests/invoke-rust-target-slots.spec.ts` 收口；
  新增槽位必须同时改两处，否则 `test:unit` 红——这是本次决策唯一需要长期维护的成本。
- Windows 侧行为不变：`cargo:*:shared` 从「PowerShell 起进程」变成「node 起进程」，
  cargo 的 argv 与 `--target-dir` 槽位一致；CI `verify` job 已用 `setup-node`，
  `core-platform-check` 的 pwsh 直连步骤未改动。
- 复活条件：若某天 `pwsh` 成为本机常规存在且 ps1 成为唯一实现，可回收 mjs 与 parity 用例；
  回收前必须先让「ps1 转调 mjs」在本机真跑过一次。
