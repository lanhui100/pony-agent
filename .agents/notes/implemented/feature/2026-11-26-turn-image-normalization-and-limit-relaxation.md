# Agent Note: 取消单轮 3 张图片硬限制并引入请求前图片归一化

Status: implemented

## Problem

1. `crates/pony-agent-core/src/agent/runtime/trace_timeline.rs`（以及前端 `src/lib/runtime/file-attachments.ts`）将单轮图片上限硬编码为 `MAX_TURN_IMAGES = 3`，无法满足多图对比、连续截图排查等场景；同时历史图片召回函数 `recalled_image_limit` 复用同一常量，存在「放开单轮上限就把历史召回一并放成无限」的耦合风险。
2. 客户端在把 `TurnInputImage` 送往模型前不做像素与体积归一化：高分辨率截屏、带 EXIF/ICC/XMP 元数据的照片、16-bit 图像或动画 GIF 会直接以原始 base64 形态进入请求体，既放大 payload 体积（容易推高延迟或触发上游 413），也把多帧 GIF 等上游不兼容形态直接透传出去。
3. 本条与总纲 `.agents/notes/implemented/bug-fix/2026-11-24-native-compaction-split-and-overflow-rescue.md` 及请求体字节闸门 `.agents/notes/implemented/bug-fix/2026-11-26-request-body-gate-align-gateway-limit.md` 共同构成「请求载荷在装配侧合规收敛、传输层 128 MiB 只作畸形兜底」的完整链路。

## Decision

1. **拆分并放开单轮图片上限，独立锁定召回预算**：
   - `crates/pony-agent-core/src/agent/runtime/trace_timeline.rs` 将 `MAX_TURN_IMAGES` 调整为 `600`（对齐请求级张数上限），单轮内联图片负载上限 `MAX_TURN_IMAGE_BYTES` 调整为 `20 MiB`；
   - 新增独立常量 `MAX_RECALLED_TURN_IMAGES = 3` 供 `recalled_image_limit` 使用，解除「历史召回预算」与「单轮附件上限」的耦合；
   - 前端 `src/lib/runtime/file-attachments.ts` 同步将 `MAX_TURN_IMAGES` 对齐为 `600`（配套更新 `src/stores/runtime/__tests__/equivalence-wave2a.spec.ts`）。
2. **在 `resolve_turn_images` 后、`validate_turn_images` 前接入请求前归一化**（`crates/pony-agent-core/src/agent/runtime/image_normalization.rs`、`turn_prep.rs`）：
   - 复用工作区既有 `image 0.25.10` 依赖（在 `crates/pony-agent-core/Cargo.toml` 开启 `png`、`jpeg`、`gif`、`webp` 解码支持）；
   - **零成本放行判据**（`can_pass_through_normalization`）：仅当非 `image/gif`、单帧（`!animated`）、无元数据（`!carries_metadata`）、8-bit sRGB、体积 `<= 4 MiB`、总像素 `<= 2048 * 2048` 且长边 `<= 8192` 时原样字节放行，不做无谓重编码；
   - **确定性重编码流水线**：其余图片解码首帧并应用 EXIF 方向、剥离全部元数据、等比缩放到像素与长边预算内；无 alpha（及不透明 GIF）走 JPEG `[85, 75, 60]` 质量阶梯（首个达标即停，全不达标取最小产物而不报错），有 alpha 保留透明走 PNG 重编码；输出必须通过二次探测校验（单帧 8-bit sRGB/sRGBA、无元数据、alpha 兼容）；
   - **并发与异步隔离**：单图编解码通过 `tokio::task::spawn_blocking` + 2 路并发闸（`tokio::sync::Semaphore`）执行，避免多图阻塞 runtime 工作线程；对非法/占位数据走 fail-soft 透传并记录结构化 warn，避免归一化层越权拒绝。

## Alternatives considered

- **照抄 deepseek-harness 的 sha256 磁盘缓存 + transform version（`request-image.ts`）**：拒绝。本轮先落地无状态归一化以控制持久化复杂度与目录清理面；仅当后续实测多图重复轮次 CPU 延迟成为瓶颈时，再单独开卡引入内容寻址缓存。
- **使用 `image` 0.25 内置 WebP 编码器承载透明图片阶梯**：拒绝。`image` 0.25 默认 WebP 编码实现能力与压缩比有限且会额外引入编码路径复杂度；透明图片改走 PNG 编码保真、不透明图片走 JPEG `[85, 75, 60]` 阶梯已满足当前需求。
- **直接删除 `MAX_TURN_IMAGES` 而不拆分 `MAX_RECALLED_TURN_IMAGES`**：拒绝。会导致「这几张/那几张」历史召回随之放宽到 600 张，把旧轮附件全部拉进当前上下文窗口。
- **在归一化阶段发现阶梯耗尽即报错**：拒绝。拒绝权属于传输层与 `validate_turn_images` 字节闸门，归一化只负责尽力收敛到最小可用产物。

## Consequences

- 涉及源码：`crates/pony-agent-core/Cargo.toml`、`crates/pony-agent-core/src/agent/runtime/mod.rs`、`trace_timeline.rs`、`turn_prep.rs`、`image_normalization.rs`、`image_normalization_tests.rs`，以及前端 `src/lib/runtime/file-attachments.ts`、`src/stores/runtime/messages.ts`、`src/stores/runtime/__tests__/equivalence-wave2a.spec.ts`。
- 单轮可附加多于 3 张图片（经 5 张图单测与归一化 7 项矩阵单测锁定），且超大像素/超长边/带 EXIF/动画 GIF/带透明通道输入均可在入模前收敛。
- 残余边界：若多轮对话反复引用同一批大图且无缓存，每次都会触发归一化 CPU 运算（已受 2 路并发闸与 `spawn_blocking` 约束，若实测延迟超标再单开磁盘缓存卡）。
