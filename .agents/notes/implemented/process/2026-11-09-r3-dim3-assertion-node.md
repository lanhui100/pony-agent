# r3-dim3 断言节点与新冻结集成员

Status: implemented

- 日期：2026-11-09
- 关联：ADR 0101（trace 审核 loop）/ PA-169（确定性节点）
- 轮次：`r3-dim3`（维度 3 失败语义）

## Decision

1. 新增判定节点 `scripts/audit/assertions/dim3.sh`：D1 降级保真 / D2 超时终态三侧可判定 /
   D3 三侧结局无矛盾 / D4 控制组不误报；四者阈值均为「违规会话数 = 0」，证据缺失单列 exit=2。
2. 把 `assertions/dim3.sh` 与 `collect_dim3.py` 加入 `state.py` 的 `JUDGING_FILES` 冻结集。
3. `selftest.sh` 增加 dim3 的绿相 / 三条红相 / 两条参数校验夹具（pass 42 → 49）。

## 为什么

- 维度 3 的判定需要**跨三侧**（trace 根属性、headless 封套、JSONL 终态）对同一会话求交，
  无法复用 dim1（时延）或 dim6（上报自检）的断言；
- `collect_dim3.py` 已经把三侧事实归一进 metrics，故断言可在**离线夹具**上全量自测，
  不需要把网络不确定性带进门禁；
- 冻结集必须包含本轮实际裁决用的脚本，否则「阈值批准即冻结」名不副实
  （r2-dim1 实测过 `assertion_hashes` 为空仍被 check-freeze 判 PASS 的缺陷）。

## Alternatives considered

- **只写 assertions、不加入 JUDGING_FILES**：实现最省事，但 dim3.sh 可在批准后被随意改写而
  `check-freeze` 恒绿 —— 等于本轮没有防篡改，否决。
- **把 dim3 判定塞进 dim1.sh**：两者证据面不同（dim1 逐臂过滤 + 需要臂内 jsonl 的 usage 行；
  dim3 跨臂全量 + 只吃 metrics），合并会让 dim1 被迫读三臂 metrics，破坏「逐臂过滤命中 0 行 = UNKNOWN」
  的既有语义，否决。
- **让 dim3.sh 自己读臂目录比对封套/事件**：与 `collect_dim3.py` 重复解析同一批文件，且违反
  「一处口径一个家」；采集器已是唯一归一化点，断言只吃 metrics，否决。
- **把 `state.py` / `selftest.sh` 也纳入冻结集**：自指缺陷 —— 检查器修 bug 会立刻变成冻结漂移
  假红（r1-dim6 已实测一次），维持「检查器自身不冻结」的既有边界。

## 后果

- dim3.sh 一旦批准即冻结：任何判据修正都需补遗提案 + 重冻结（与 r2-dim1 F8 处理一致）；
- `collect_dim3.py` 的字段增删同样受冻结约束，故「metrics 需要新字段」必须在批准前提出。
