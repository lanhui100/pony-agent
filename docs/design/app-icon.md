# Pony Agent App Icon

## 目标

为 `Pony Agent` 提供一套统一的桌面应用图标，要求同时满足：

- 能延续当前产品的暖米色、深咖色、克制工程感
- 在小尺寸下依然可识别，不依赖复杂细节
- 同时传达 `pony` 与 `agent/runtime/workflow` 语义

## 图形思路

这一轮不再坚持单一草案，改为并行生成 3 个候选方向，统一约束如下：

- 浅底，不走厚重深色底
- 一眼看出是 `pony` 头部侧脸
- 不使用复杂负形、过重阴影和难辨认字母隐喻
- 仅保留少量暖金元素表达 `agent / runtime / trace`

这样做有三个好处：

- 第一眼能看出是马头，而不是抽象符号
- 在 `16px - 32px` 尺寸下仍然保留稳定轮廓
- 仍然保留当前产品偏克制、偏工程感的暖色体系

候选里只保留很轻的暖金轨迹或节点，表达：

- agent 的执行路径
- runtime 的状态流转
- 工作台中的 trace / tool / run loop 语义

## 配色

- 背景浅米：`#FBF8F1 -> #ECE4D6`
- 主形暖棕：`#4E3D30` 附近
- 强调暖金：`#C67938` 附近

## 文件

- 生成脚本：[scripts/generate_app_icons.py](C:/Users/HUAWEI/Documents/pony-agent/scripts/generate_app_icons.py)
- 当前默认导出目录：[src-tauri/icons](C:/Users/HUAWEI/Documents/pony-agent/src-tauri/icons)
- 候选预览目录：[docs/design/icon-candidates](C:/Users/HUAWEI/Documents/pony-agent/docs/design/icon-candidates)
- 应用内品牌组件：[src/components/PonyBrandIcon.vue](C:/Users/HUAWEI/Documents/pony-agent/src/components/PonyBrandIcon.vue)
