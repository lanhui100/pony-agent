# Agent Note: trace 审核 harness 三处自证缺陷修复（metrics 通道分裂 / 串臂 / 空冻结集）

Status: implemented

## Context

r2-dim1 attempt 1 的 verify → verdict 期间，harness 自身产出三类**假结论**，且都不是实现缺陷
而是**取证/判定层缺陷**（会把「不达标」判成达标，或把「达标」判成不达标）：

1. **metrics 通道 schema 分裂**：`collect_dim1.py`（维度 1 采集器）只输出维度 1 自有字段
   （`calls` / `tools` / 时长分解），而 `fetch.sh`（维度 6 采集器）只输出 trace 级字段
   （`trace_release` / `root_attributes`）。`verdict.sh` / `ratchet.sh` 只接受**单个** `--metrics`：
   - 用 dim1 metrics 跑棘轮 → dim6 的 A2 恒判 `trace_release=null`（实测 18/18 假红）；
   - 用 dim6 metrics 跑本维度 → L3/L4 恒缺证据（`arm=candidate sessions=0`）。
   两条路都红，但原因完全不同，人会误读成「两个维度都回归」。

2. **串臂**：臂目录带 attempt 后缀（`base` → `base-a2`，`candidate` → `candidate-a2`），
   而 `--metrics` 由调用方给路径。臂重跑换名后沿用旧 metrics，断言就会拿 A 臂的会话去判 B 臂的
   span。实测：新臂 `candidate-a2` 被旧 metrics 判成 L3 不可归因 4 条、L4 超阈 1 条，
   换成新 metrics 后 L3 实际为 0。

3. **空断言冻结集静默放行**：`state.py check-freeze` 只遍历「已冻结的哈希」，
   `assertion_hashes={}` 时循环零次 → 打印「冻结一致」并 exit 0。而 `assertion_hashes` 为空
   意味着**防篡改根本没装上**（阈值脚本可被随意改写而检查恒绿）。r2-dim1 批准 → implement
   迁移时漏跑 `freeze`，实际就是空集，而 verdict 的 step 0 判了 PASS。

## Decision

1. `collect_dim1.py` 输出 **trace 级超集字段**（`trace_release` / `trace_version` /
   `trace_total_cost` / `service_name` / `client_service_name` / `deployment_environment` /
   `root_attributes`），使**一份 metrics 同时满足维度 1 与维度 6**。只补证据字段，
   **不动任何阈值口径**。
2. `lib.sh` 新增 `metrics_arm_guard`：断言脚本开头校验 metrics 的 `session_id` 集合**覆盖**
   本臂 `sessions.tsv`；不覆盖即 exit 2（证据缺失，UNKNOWN ≠ PASS），不折算成断言失败。
   `assertions/dim1.sh` 接入；`assertions/dim6.sh` **暂不接入**——它的哈希已冻结在
   r1-dim6 快照里，改动会使历史轮复跑变成假漂移（4）；待 dim6 开新轮时一并接入。
3. `state.py check-freeze`：`corpus_hash_frozen=true` 且 `assertion_hashes` 为空 → exit 2。
   语义从「没有漂移」纠正为「防篡改未装上」，由 `verdict.sh` 统一判 UNKNOWN。
4. `selftest.sh` 补两条红相（串臂 metrics → 2；空冻结集 → 2），夹具臂补 `sessions.tsv`
   与 metrics `session_id`。自测 26 → **31 条，全绿**。

## Alternatives considered

- **让 `verdict.sh` 接受多个 `--metrics`（按维度分文件）**：能解 schema 分裂，但要在
  verdict/ratchet/断言三层各加一次「按维度挑 metrics」的分派逻辑，且棘轮集里每新增维度都要
  再改一次分派。超集字段是单点改动（采集器一处），判定层零改动。否决。
- **给 metrics 加 schema 版本号并在断言里校验**：只能报「不兼容」，不能自动满足两维度；
  仍需调用方跑两次采集。否决。
- **串臂靠「臂目录名 + metrics 文件名约定」约束**：约定不可机械检查，且实测已失效一次
  （旧 metrics 与旧臂同名）。session_id 覆盖校验是机器可查的判据。否决。
- **串臂直接判 fail（1）**：串臂属「证据不可用」而非「断言不达标」，判 1 会污染「本维度确实
  不达标」的信号，也会让棘轮把「证据缺失」记成回归。判 2 与既有 UNKNOWN 词表一致。否决。
- **`assertion_hashes` 为空时直接判冻结漂移（4）**：4 的语义是「事后篡改」，而空集的成因是
  「批准时漏跑 freeze」，两者处置不同（前者须新提案重审，后者补跑 freeze 即可）。判 2 更准确。
  否决。

## 机械校验

```bash
bash scripts/audit/selftest.sh          # 31/31，exit=0
bash scripts/audit/freeze-check.sh --round r1-dim6   # 历史轮仍可复跑，exit=0
```

## 遗留

- `assertions/dim6.sh` 的同源守卫待其开新轮时接入（当前被 r1 冻结哈希锁住）。
- 基线臂 `base-a2` 有 3 会话 `/api/public/observations` 返回空（trace 存在、观测缺失），
  成因未定位（Langfuse 侧摄取不完整 vs 客户端未导出）；已按 `missing=true` 如实计入，
  不填 0、不折算 pass。
