# Wave 4.1 测试矩阵与红相锚定计划

## 1. 任务范围
- B077 (size:check): 修复构建门禁红灯
- B082 (check-secrets): 解决历史验证收据哈希冲突
- B080 (环境密封): 修复 delta_notice 逻辑，杜绝宿主配置泄露

## 2. 测试策略 (Test Strategy)

| 任务 | 等级 | 测试方式 | 验收标准 |
| :--- | :--- | :--- | :--- |
| B077 | L2 | 构建流水线仿真 | Exit 0，红灯变绿 |
| B082 | L2 | 哈希冲突边缘情况单测 | 哈希计算唯一，冲突率 0% |
| B080 | L3 | 对抗测试 (L2-AT) | 无法读取宿主 settings.json，delta_notice 仅播报过滤后的有效技能 |

## 3. 红相锚定计划 (Red Phase Anchoring)
我们将编写契约测试，并将其冻结在 `tests/acceptance/wave4_1_test.py` (L2-T) 和 `tests/acceptance/adversarial/b080_test.py` (L2-AT)。

完成后执行：`git add tests/ && git commit -m "test(wave-4.1): freeze functional & adversarial red-phase acceptance tests"`
