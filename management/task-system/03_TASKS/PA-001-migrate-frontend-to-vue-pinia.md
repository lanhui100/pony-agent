# PA-001 迁移前端到 Vue + Pinia

## 状态

- Status: `Done`
- Priority: `P0`
- Owner: `Codex`

## 目标

将当前根目录前端从原生 TS + DOM 骨架升级为：

- Vue 3
- TypeScript
- Pinia

并保持 Tauri 前端入口兼容。

## 输出

- 更新前端依赖
- 建立 Vue 应用入口
- 建立基础 store 结构
- 保留单页工作台形态

## 验收标准

- `npm install` 成功
- `npm run build` 成功
- 前端可以通过 Vue 渲染工作台骨架
- 暂不引入 `vue-router`

## 当前进展

已完成：

- 安装 `vue`、`pinia`、`@vitejs/plugin-vue`、`vue-tsc`
- 将前端入口切换为 Vue 应用
- 建立 Pinia runtime store
- 保持单页工作台结构

## 下一步动作

转入 `PA-005`，开始把当前工作台接入真实 turn 执行链路。

## 当前卡点

- 无

## 断点续跑提示

如果下次继续，先看：

- `docs/guides/frontend.md`
- `docs/decisions/0003-frontend-stack-vue-pinia-no-router.md`
