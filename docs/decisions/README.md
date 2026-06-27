# Pony Agent 决策记录

本目录用于记录 Pony Agent 的重要技术决策。

采用 ADR 风格：

- 决策背景
- 候选方案
- 权衡
- 结论
- 影响

## 索引

- [0001 双轨重构：保留 Hermes 作为参考实现](0001-dual-track-rebuild-with-hermes-reference.md)
- [0002 优先使用 Tauri UI 作为运行时测试界面](0002-tauri-ui-over-tui.md)
- [0003 前端使用 Vue 3 + Pinia，暂不上 vue-router](0003-frontend-stack-vue-pinia-no-router.md)
- [0004 前端工作台先采用原生 TypeScript + Vite 壳层（已被 0003 覆盖）](0004-frontend-shell-workbench-direction.md)
- [0005 第一阶段视觉方向采用暖色极简风](0005-visual-direction-warm-minimalism.md)
- [0006 API Key 存储从 env-first 演进到统一 SecretStore](0006-api-key-evolution-env-first.md)
- [0007 将缓存命中提升为一等产品指标，按阶段落地](0007-cache-hit-as-first-class-product-metric.md)

## 维护规则

- 重大架构选择必须新增 ADR
- ADR 一旦落地，尽量不改编号
- 如果决策被推翻，应新增后续 ADR，而不是直接抹去历史
