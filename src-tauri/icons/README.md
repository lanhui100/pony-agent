# Pony Agent Icon 精修工作区

当前目录已经确认一版正式方向：基于 `logo_3.png` 的暖白圆角背景 App icon，并已可导出正式 Tauri 图标。

## 当前原则

1. 只基于 `logo_3.png`
不使用 `icon-source.png` 作为设计母体。

2. 精修，不重做
保留原图的双三角结构、负空间关系、横向连接带、圆角端点和橙红到紫色的渐变动势。

3. 先验收母版，再生成规格
正式导出基于审稿通过的 `review/bg-candidates/d-warm-clean-home-tone-v2.png`。

4. 不做粗糙透明抠图
`logo_3.png` 是白底位图源，直接抠透明会产生白边、脏边和噪点。正式透明版应在母版确认后再单独处理，或通过矢量重描解决。

## 当前审稿产物

审稿文件在 `review/` 目录：

- `logo_3_refined_master_1024.png`
- `logo_3_refined_512.png`
- `logo_3_refined_128.png`
- `logo_3_refined_32.png`
- `logo_3_compare.png`

背景候选在 `review/bg-candidates/`：

- `a-clean-mint.png`
- `b-soft-3d.png`
- `c-dark-tech.png`
- `d-warm-clean-home-tone.png`
- `d-warm-clean-home-tone-v2.png`

## 已隔离内容

上一轮不合格的生成图标已移动到 `_rejected/`，不应作为正式项目图标使用。

## 正式导出

正式导出脚本：

- `scripts/export_final_icons_from_candidate.py`

正式导出产物：

- `icon-master-approved.png`
- `32x32.png`
- `128x128.png`
- `128x128@2x.png`
- `256x256.png`
- `512x512.png`
- `icon.png`
- `icon.ico`
- `Square30x30Logo.png`
- `Square44x44Logo.png`
- `Square71x71Logo.png`
- `Square89x89Logo.png`
- `Square107x107Logo.png`
- `Square142x142Logo.png`
- `Square150x150Logo.png`
- `StoreLogo.png`

## 后续确认路径

1. 当前已确认暖白圆角背景版，可直接用于 Tauri 打包。
2. 如果后续需要透明底版本，应单独处理并检查深浅背景边缘质量。
3. 如果后续需要更强品牌系统，可再补 favicon / 托盘单色版 / 启动画面版。
