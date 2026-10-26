# scripts/observability —— 仓库内可观测性变更门禁

## 这是什么

`docs/observability.yaml::enforcement_points` 声明了各横切能力的**唯一合法收口点**与禁止模式。
本目录的 `gate.mjs` 是这条契约在 **CI 上可执行**的引擎。

## 为什么不是直接用 audit-diff.sh

权威审计器 `audit-diff.sh` 属于本机 skill
（`~/.agents/skills/observability-onboarding/scripts/`），它 source 同目录的 `lib.sh`（约 800 行，
含集群探针/契约读取），**GitHub Actions runner 上没有这份 skill**。

若 CI 直接调用它，只会因文件缺失变成空转——门禁「装了但什么都没拦」，是最糟的一种假绿。
因此 CI 走仓库内引擎；两个引擎的判定口径由
`tests/acceptance/wave47/gate-conformance.spec.ts` 用同一批夹具交叉校验。

## 规则来源

**规则不在代码里**。`gate.mjs` 只做三件事：

- **D1**：声明文件被改动时，HEAD 提交信息必须含 `[decl-change-approved]`；
- **A（越界实现）**：变更文件命中某收口点的 `forbidden_in_other_files[].pattern` 即违规
  （豁免 `tests/`、`docs/`、`fixtures/`、`*.md`，与权威器同口径）；
- **B（收口点失效）**：声明路径不存在，或本次变更后行数跌破基线 50%。

新增/修改规则请改 `docs/observability.yaml`，不要改本脚本——脚本一旦内置语言关键字，
规则所有权就从项目侧漂走了。

## 本地使用

```bash
node scripts/observability/gate.mjs                     # 默认比对 HEAD~1 + 工作区
GATE_BASE=HEAD node scripts/observability/gate.mjs      # 只看工作区改动
GATE_BASE=origin/main node scripts/observability/gate.mjs
```

退出码：`0` 通过｜`2` 违规｜`4` 声明或工具错误（fail-closed）。

## 已知约束

YAML 解析只覆盖本项目声明实际使用的子集（2 空格缩进、`key: value`、`- item`）。
**遇到不认识的结构直接 fail-closed 退出 4**，不猜——猜错会让门禁静默放行。
若确需扩展声明写法，请同步扩展 `parseEnforcementPoints` 并补 conformance 用例。
