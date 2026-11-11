# 波次 4 验收测试矩阵 (B077, B082, B080)

| 任务 | 级别 | 契约验证 (L2-T) | 对抗破坏注入 (L2-AT) |
| :--- | :--- | :--- | :--- |
| **B077 (Size Gate)** | L2 | 验证 `check-file-size.mjs` 基线同步机制，确保符合 size 策略。 | 注入畸形大小配置文件，验证 size gate 在非法配置下 Fail-Closed。 |
| **B082 (Secrets Gate)** | L2 | 验证 `check-secrets.mjs` 对合规哈希的白名单排除机制。 | 注入伪造的 64-hex 高熵字符串，确保 gate 能准确识别非白名单凭据。 |
| **B080 (Delta Notice)** | L3 | 验证 delta_notice 的 model_invocable 过滤器一致性及宿主环境隔离。 | 注入非法配置的宿主环境模拟，断言测试进程在不受宿主干扰下成功完成隔离测试。 |
