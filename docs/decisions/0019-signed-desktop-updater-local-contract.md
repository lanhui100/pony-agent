# 0019 本地契约优先的签名桌面更新

Status: implemented

## 背景

Pony Agent 需要为 Tauri 桌面端预留签名更新路径，但当前工作区没有可验证的 Pony Agent 生产 endpoint、公钥、签名产物或 CI 发版证据。直接启用安装会把未验证的外部输入带入更新信任边界。

## 候选方案

1. **固定 Tauri updater 插件契约，并在 endpoint/公钥缺失时禁用**：保留签名校验、下载、安装和重启的类型边界；运行时不接受环境变量覆盖；当前 UI 只显示检查/安全禁用状态。选择此方案，因为它能先落地本地可测契约且不会伪造信任锚点。
2. **继续使用 GitHub Releases 元数据和任意资产链接安装**：改动较小，但无法证明签名、目标、版本或 artifact 完整性，且会混淆 check-only 数据与可安装候选，否决。
3. **提交占位或借用的公钥并启用安装**：看似完成配置，但会制造虚假安全证据并可能接受错误发行者，否决。

## 决策

桌面 host 在构建期 `signed-updater` feature 开启且存在真实 endpoint/公钥配置时才注册 Tauri updater/process 插件；本地契约默认不开启该 feature，前端只通过 typed opaque updater handle 调用受信任的 Rust plugin commands。GitHub 检查路径继续只负责发布元数据与发布页 CTA。

真实 endpoint、公钥、CI secret、签名产物及 Windows smoke 属于独立 release-owner gate，完成前不得开启 feature、不得填充配置、不得翻转启用常量。

## 影响

更新状态增加独立 signed candidate 与下载/安装/待重启/重启失败状态；权限只授予 updater check/download-and-install 与 process restart。后续启用前必须补充真实信任锚点、版本/目标矩阵验证、崩溃恢复和签名安装证据。
