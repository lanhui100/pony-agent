# Session 65 - PA-020 Capability Monitor Panel

## 本轮目标

- 把 capability read-plane 从“只有 store/命令”推进到“前端已有最小只读消费”，但不引入新导航或新管理台

## 已完成

- 扩展 `src/components/ModelMonitorPage.vue`
  - 新增 capability sources 列表
  - 新增 source 下 capabilities 列表
  - 新增 capability inspect 详情区块
  - 保持只读，不引入 enable/disable、reconnect、执行等控制动作
- 沿用现有 Tauri read-plane 命令
  - `list_capability_sources`
  - `list_capabilities`
  - `inspect_capability`
- 扩展前端测试
  - `tests/ModelMonitorPage.spec.ts` 新增 capability 面板渲染验证
  - `tests/runtime-store.spec.ts` 新增 capability read-plane fallback 验证

## 验证

```powershell
cmd /c npm run test:unit -- ModelMonitorPage.spec.ts
cmd /c npm run test:unit -- runtime-store.spec.ts
cargo check --manifest-path src-tauri/Cargo.toml --lib
```

结果：

- `ModelMonitorPage.spec.ts` 6 项通过
- `runtime-store.spec.ts` 39 项通过
- Rust `cargo check` 通过

## 当前结论

- `PA-020` 前端现状已从“有 store、无消费”升级为“有最小只读调试消费”
- 当前 UI 落点放在 `ModelMonitorPage`，与会话侧栏、历史侧栏解耦，符合 capability 作为全局宿主读面的定位
- 下一步仍应回到 core 主线：真实 MCP source 注册、三类 capability 映射、permission/failure 完整合同
