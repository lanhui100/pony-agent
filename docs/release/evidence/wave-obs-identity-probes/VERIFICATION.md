# wave-obs-identity-probes 交付验证收据（Lead 实测）

- 基线：`4ed85bdd`
- 环境：Linux x86_64 / bash 5.x / python3 / PyYAML 可用
- 关联 ADR：无（本波是门禁判定口径修复，不改变运行时行为；运行时侧的 ADR 是 `0086`）

## 1. 问题：T4 假红，且「加声明」本身无效

`check-declaration.sh` 的 T4（tracing 动态证据）长期 FAIL，原因是**检索键被改写**而非「没数据」：

| 检索键 | 24h | 7d | 说明 |
| --- | --- | --- | --- |
| `service.name=pony-agent`（内置键） | 0 | 0 | 公网 OTLP edge 把 `service.name` 统一改写为 `desktop-client` |
| `service.name=desktop-client` | 117 | ≥200 | 全平台共用名，不能证明是本项目 |
| `resource.client.service.name="pony-agent"`（内置键） | 0 | 20 | 原名只落在 `client.service.name`，且 dev 机自报带 `-dev` 后缀 |
| `resource.client.service.name=~"pony-agent.*"`（新探针） | 117 | ≥200 | 本项目命名空间内收敛，非 `.*` 全通配 |
| `resource.pony.app.name="pony-agent"`（新探针） | 117 | ≥200 | `pony.` 前缀不参与 edge 改写，跨路径唯一一致键 |

> 数字为 2026-11-07 本轮实测（`exact` 游标，`probes_ok=8 / probes_failed=0`）。7d 值受 200 条查询上限约束，只作量级证据。

**第二层原因（更隐蔽）**：`audit-project.sh` 里 `identity_probes` 的解析与汇总代码都在，但**执行循环被整段注释掉**（原 413–427 行）。因此只往 `docs/observability.yaml` 加 `identity_probes` 声明不会有任何效果，T4 仍判内置键口径 → 仍假红。

## 2. 两处修复（拆两个交付面）

| 面 | 载体 | 内容 |
| --- | --- | --- |
| 项目声明 | `docs/observability.yaml` | `dimensions.tracing.query.identity_probes` 增 2 条 traceql 探针；`reviewed_at` → `2026-11-07` |
| 门禁能力 | skill 脚本（无 git，`~/.agents/skills/observability-onboarding/`） | 恢复执行循环，加 `if (( ${#IDENTITY_PROBES[@]} ))` 守护，避免 `set -u` 下空数组报错 |

skill 脚本不可版本化，故将修改以 patch 形式归档于本目录：`audit-project-identity-probes.patch`（`-p1`，目标 `scripts/audit-project.sh`）。

**应用方式**（skill 被覆盖/重装后）：

```bash
cd ~/.agents/skills/observability-onboarding && patch -p1 --dry-run -i <repo>/docs/release/evidence/wave-obs-identity-probes/audit-project-identity-probes.patch
```

## 3. 机器门禁（全部真实 exit）

| 门禁 | 命令 | 结果 |
| --- | --- | --- |
| patch dry-run | `patch -p1 --dry-run` 于备份副本 | exit 0，`APPLIED-EQUALS-SKILL`（应用结果与现网字节一致）、`bash -n` 通过 |
| skill 自测 | `tests/run_auditproject_tests.sh` | 用例 125 / 通过 125 / 失败 0，`结论：PASS`（含 C11a–C11q 探针专项；修复前为 113 过 / 12 红） |
| 声明一致性 | `check-declaration.sh /home/dm/pony-agent` | exit 0；metrics / tracing / logs / issues 四维全 PASS，`T4=PASS` |
| 观测增量 | `audit-diff.sh --decl-approved` | exit 0；`无越界实现、收口点完整`（13 文件 / 766 行新增） |

**关于 `audit-project.sh --json` 的 exit=2**：这是巡检**未叠声明**时的原始四维判定（metrics 维在 desktop 形态下天然无可 pull 的 `/metrics`，其 N-A 理由由声明层承载）。一致性判定的权威入口是 `check-declaration.sh`（exit 0），两者不是同一口径，不得互相替代。

## 4. 已知残留

- skill 脚本无版本管理，patch 是本仓唯一可追溯副本；skill 若被重装且未重放 patch，T4 会静默退回假红（现象：detail 出现内置两键 0 条命中）。诊断入口就是本文件 §1。
- `identity_probes` 当前只支持 `traceql` / `tags` 两种 `query_type`；探针失败会如实计入 `probes_failed` 而不会被吞成 0（由 C11m–C11o 锁定）。
- 本波未处理 metrics 维的 N-A 机器判据（仍依赖声明层人工裁决），属既有设计。
