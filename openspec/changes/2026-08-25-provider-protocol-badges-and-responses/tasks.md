# 任务拆解（含审核修订后的依赖序）

## 后端——阶段一：类型地基（B1/B2 串行先行，其余后端任务依赖它）✅

- [x] B1 ProviderProtocol 三值化 + serde rename/alias + 家族枚举与 `is_anthropic()/is_openai_family()` 助手；编译器盲区 grep 门禁清单逐项勾销
- [x] B2 行为分叉位点签名枚举化 + native transcript 形态回归断言

## 后端——阶段二：新模块（B3/B5 可并行）+ config 域 ✅

- [x] B6 config.rs：模型 base_url 字段 + resolve_selection 解析序 + endpoints 三值构造 + load_storage 失败备份（含同前缀去重）+ 别名四位置往返测试
- [x] B3 responses_api.rs：公共帧阅读器（read_sse_data_lines 通用核心 + 直测）→ 请求构建、转换器（外来形态守卫含 thinking）、SSE 累积器、sync/stream 解析（call_id 统一、失败终态 Err、usage cache_hit_source 断言）+ 单测
- [x] B5 model_catalog.rs + fetch_provider_models 命令注册 + 解析单测

## 后端——阶段三：接线 ✅

- [x] B4 ProviderManager 四路分发接线 + 流式门控/回退语义对齐 + thinking pattern 家族解析

## 前端 ✅

- [x] F1 types/store：三值协议 + baseUrl 字段 + 规范化单点 + 家族能力目录 + fetchModelCatalog/addModelsFromCatalog + runtime 兜底值
- [x] F2 提供商编辑行（名称|徽章×3|API Key 同级）+ 高级设置 + 关徽警示 + authType 保留 + create→edit 防孤儿
- [x] F3 模型表单 ID 目录多选/手动 + 高级设置（协议限已启用集合）
- [x] F4 输入面背景统一
- [x] F5 收起自动取消编辑态（跨区连带收起先取消、create 收起复位、锁移除）
- [x] F7 指针与行尾动作悬停显隐（两级行内 trigger cursor-pointer；group-hover/focus-within；区头容器 group）
- [x] F6 测试更新与新增（ProviderConfigPage 重写+新增用例、providers.store 新增 tri-value 块、runtime-store fixture 迁规范名）

## 门禁与收口

- [x] G1 npm run test:unit 全绿（32 文件 / 530 passed）
- [x] G2 vue-tsc --noEmit 通过
- [x] G3 cargo check:shared 通过
- [x] G4 cargo:test:shared 全 crate 测试通过
- [x] G5 双 reviewer 代码审核意见采纳/回改（见 design.md 审核记录·代码阶段）
- [ ] G6 ADR 0014（已落盘 docs/decisions/0014）+ 两份 spec delta 合入 openspec/specs + proposal 验收核对
