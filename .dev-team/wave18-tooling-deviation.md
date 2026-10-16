# wave-18 工具降级记录（T1 / L2-P 协议特派员）

- 任务：`task-1`
- 目的：显式记录本波 skill 工具链缺失与等价替代，**禁止静默跳过**（"未运行"必须写明原因与替代物，且替代物不等价处必须声明）。

---

## 1. 缺失事实（物理收据）

| 工具 | 期望位置 | 实测 | 结论 |
|---|---|---|---|
| `dev-stub-lint.py` | skill 包 `dev-team` 目录 | `ls -la /home/dm/.agents/skills/dev-team/` → 仅 `SKILL.md`（51,351 B）；`ls -la /home/dm/.claude/skills/dev-team/` → 仅 `SKILL.md`（25,865 B）+ `SKILL.md.bak` | **未安装** |
| `dev-red-purity.py` | 同上 | 同上 | **未安装** |

命令与 Exit Code（E7）：

```
$ ls -la /home/dm/.agents/skills/dev-team/ /home/dm/.claude/skills/dev-team/
/home/dm/.agents/skills/dev-team/:
-rw-rw-r-- 1 dm dm 51351 Oct  8 21:14 SKILL.md
/home/dm/.claude/skills/dev-team/:
-rw-rw-r-- 1 dm dm 25865 Oct  6 22:11 SKILL.md
-rw-rw-r-- 1 dm dm 17761 Oct  6 22:11 SKILL.md.bak
EXIT=0
```

**旁注（避免与 wave-17 记录冲突）**：`/tmp/dev-team-sandbox/scripts/` 下存在同名**参考副本**（`dev-stub-lint.py`、`dev-red-purity.py`、`dev-nfr-lint.py`）。该目录是历史沙箱抽取物，**不是** skill 包内容、**不被** 本仓库任何门禁引用（`.dev-team/wave17-protocol-receipt.md:120` 已裁定其为"沙箱参考件"）。本波**不采用**其作为门禁执行体，理由见 §2/§3。

---

## 2. `dev-stub-lint.py` —— 本波不适用（N/A，非"通过"）

- 工具原理（读 `/tmp/dev-team-sandbox/scripts/dev-stub-lint.py:1-20`）：基于 Python `ast` 解析，遍历 `FunctionDef` / `AsyncFunctionDef`，判定"函数体 <= 2 条语句且只允许 `raise NotImplementedError`"。
- **输入面**：仅 Python `.py` 源文件。
- **本波改动面**：`crates/**/*.rs`、`src/**/*.ts`、`scripts/*.mjs`、`.dev-team/**/*.md|json` —— **零 `.py`**，且本波**不新增任何接口空桩**（task-1 为协议冻结 + 凭证清理，无新增 trait/函数签名）。
- **裁定**：`dev-stub-lint.py` 对本波**物理不可用且无语义对象**，记为 `N/A`（不适用），**不得**记为 PASS/通过，**不得**记为静默跳过。
- 等价物：无（本波无 stub 面可校验）。若 T2 在迁移测试时引入临时空桩，须单独重开该项并在其收据中声明。

---

## 3. `dev-red-purity.py` —— 由 Test Agent 产出 machine-readable JSON 分类报告替代（**非等价**）

- 工具原理（读 `/tmp/dev-team-sandbox/scripts/dev-red-purity.py:1-25`）：接受一条测试命令，**要求该命令必须失败**（`returncode != 0`），并在输出中匹配 `VALID_EXCEPTIONS = ("NotImplementedError", "TimeoutError", "ConcurrencyConflictError", "ValueError")` 之一，否则判 `RED PURITY ERROR`。
- **本波不适用之处**：本波红相位断言的是**行为契约**（默认零 egress / 裸 hex 命中），预期失败形态是 `assert` 断言失败与 `check-secrets` 门禁 Exit 1，**不**是上述 4 类商业异常类型；直接套用会把合法的红相位误判为"infrastructure error"。
- **替代物**：由 **Test Agent** 产出 machine-readable JSON 分类报告，落盘 `.dev-team/wave18-red-purity.json`（路径由 Test Agent 确认，Lead 可裁定改名）。
- **替代物最小 schema（冻结，Test Agent 必须满足）**：

```json
{
  "wave": "wave-18",
  "red_phase_command": "<完整可复现命令>",
  "exit_code": 1,
  "failure_class": "assertion_failure | gate_exit_1 | compile_error | infrastructure_error",
  "is_pure_red": true,
  "cases": [
    { "id": "C1-default-token-none", "expect": "fail", "observed": "fail", "failure_class": "assertion_failure", "evidence_line": "<文件:行 + 断言文本>" }
  ],
  "infrastructure_error_detected": false
}
```

- 硬约束：`failure_class` 必须逐条给出且**只能**取自枚举；`infrastructure_error_detected=true` 时 `is_pure_red` 必须为 `false`；禁止只给"测试失败"而不给分类（等价于零假测试）。
- **不等价声明**：`dev-red-purity.py` 的"异常类型白名单"是自动机器判定；本替代物是**人工/Agent 分类 + 结构化留痕**，其可信度依赖分类者诚实性。为部分补偿，要求每条 case 附 `evidence_line`（可被第三方复算的物理位置），并在 Test Agent 报告中给出原始 stdout 尾部。

---

## 4. 未被降级的工具（已实际运行，供对照）

| 工具 | 状态 | 收据 |
|---|---|---|
| `.dev-team/nfr-lint.py` | **项目自带，实际运行** | `python3 .dev-team/nfr-lint.py` EXIT=0；`--instance .dev-team/nfr-baseline-wave18.json` EXIT=0；`--self-test` EXIT=0（21 cases / 0 failures）；`--mutation-test` EXIT=0 |
| `scripts/check-secrets.mjs` | **项目自带，实际运行** | `runChecks({mode:"all"})` EXIT=0（补盲前 `violations=0`，证明失明；补盲仿真 `violations=4`，证明命中） |
| `node /tmp/w18-rule-probe.mjs {sha,spec,repo,full}` | **本次自建只读探针，实际运行** | 四模式 EXIT 全 0；未写入仓库 |

---

## 5. 汇总

| 工具 | 本波状态 | 处置 |
|---|---|---|
| `dev-stub-lint.py` | **未安装 → N/A（无适用对象）** | 显式登记，无等价物需求 |
| `dev-red-purity.py` | **未安装 → 替代** | Test Agent 产出 `.dev-team/wave18-red-purity.json`（§3 schema），**声明非等价** |
| `dev-nfr-lint.py` | 未安装 | 由项目自带 `.dev-team/nfr-lint.py` 等价覆盖（wave-17 已裁定，本波复验 EXIT=0） |
