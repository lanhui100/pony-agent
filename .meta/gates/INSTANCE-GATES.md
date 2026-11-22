# 实例门登记（本仓自有；不属框架骨架文件）

> 为什么单独一个文件：`.meta/gates/README.md` 在 ponygo 的 `UPGRADE_UNIVERSE` 内（逐字节锁定），
> 往里追加实例门会让 `ponygo upgrade --check` 长期报漂移（实测 rc=1）。框架 README 只登记 A1–A9；
> 实例门与「承诺 → 命令」写在这里，既不制造骨架漂移，也不把门禁藏进无人看的地方。

| 门 | 脚本 | 负样本 spec | 接线 | 退出语义 |
|---|---|---|---|---|
| backlog 账本门 | `.meta/gates/backlog-gate.sh` | `.meta/gates/backlog-gate.spec.sh` | pre-commit（哨兵块之外）+ `ponygo-governance.yml` + `npm run verify`（`backlog:gate`） | 0 放行 / 非零拒绝；账本缺失 → 明确跳过；v2 过渡期只读兼容放行（NOTE）；布局不一致（有 log 缺 MODEL）→ 拒绝；无 `node` → 拒绝 |

## 承诺 → 非零退出命令

| 承诺 | 命令 | 命中判据 |
|---|---|---|
| 账本记录合法且跨归档/租约对账干净 | `bash .meta/gates/backlog-gate.sh` | `lint`（批次头/事件结构/时间单调/fold 不变量/状态字段合法性/租约-状态互斥/ID 唯一）+ `doctor`（ID 重复/孤儿租约/租约-状态互斥/归档清单）+ `verify-archive`（归档批次 sha256 封存） |
| 门自己会拒绝非法样本 | `bash .meta/gates/backlog-gate.spec.sh` | P1–P3 放行（v3 单批次 / 双批次并集 / v2 过渡）+ N1–N8（批次头缺失 / 未知事件 / v2 缺 fingerprint / 归档被改动 / 归档未入清单 / 锁存而记录非 in_progress / 有 log 缺 MODEL / 无 node）+ H（pre-commit 接线：门在场放行 / 门缺失 fail-closed） |

## 契约与真源

- 可读契约：`backlog/README.md`（布局/字段/状态机/租约/归档/命令）
- 判定真源：`scripts/backlog.mjs`（`validateRecord` / `lint` / `doctor`）——本目录脚本只编排，不复刻判据
- 诚实边界：spec 只证明「门会拒非法样本」；账本内容真实性靠 review。`doctor` 不解析 git 历史，
  无法识别「同时改记录与高水位」的伪装。
- 本地钩子前提：`git config core.hooksPath .githooks`（git 不克隆该配置；新 clone 跑
  `npm run hooks:install`）。未装配时本地门禁静默失效、CI 仍兜底——这是 git 的机制限制，不是门禁缺陷。
