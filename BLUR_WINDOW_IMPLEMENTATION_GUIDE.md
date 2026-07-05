# 毛玻璃窗口实现指南

> 基于 **Blur Win** 项目（Tauri 2 + Vue 3 + Vite 8.1 + Tailwind CSS 4）的架构方案。
> 将实现拆解为三层：**原生窗口层 → CSS 玻璃壳层 → UI 内容层**，以及窗口缩放动画优化。

---

## 目录

1. [架构总览](#1-架构总览)
2. [原生窗口层：Tauri 配置与 Rust 后端](#2-原生窗口层tauri-配置与-rust-后端)
3. [CSS 玻璃壳层：`.glass-window`](#3-css-玻璃壳层glass-window)
4. [UI 内容层与布局](#4-ui-内容层与布局)
5. [Glass Intensity 组件](#5-glass-intensity-组件)
6. [窗口缩放动画优化](#6-窗口缩放动画优化)
7. [快速接入清单](#7-快速接入清单)

---

## 1. 架构总览

```
┌──────────────────────────────────────────────────┐
│                  桌面壁纸/背景                      │
├──────────────────────────────────────────────────┤
│  ┌────────────────────────────────────────────┐  │
│  │  ① 原生窗口层 (Tauri)                       │  │
│  │  ┌──────────────────────────────────────┐  │  │
│  │  │  ② CSS 玻璃壳层 (.glass-window)       │  │  │
│  │  │  ┌────────────────────────────────┐  │  │  │
│  │  │  │  ③ UI 内容层                     │  │  │  │
│  │  │  │  · 标题栏 (可拖动区域)             │  │  │  │
│  │  │  │  · 内容区 (Hero + 控制面板)       │  │  │  │
│  │  │  │  · 光晕背景装饰 (Glow)            │  │  │  │
│  │  │  └────────────────────────────────┘  │  │  │
│  │  └──────────────────────────────────────┘  │  │
│  └────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────┘
```

每一层的职责：

| 层级 | 技术载体 | 核心作用 |
|------|----------|----------|
| **原生窗口层** | Tauri `tauri.conf.json` + Rust `window-vibrancy` | 无边框透明窗口 + 操作系统级 Acrylic/Blur 背板 |
| **CSS 玻璃壳层** | 全局 CSS 类 `.glass-window` | 渐变基底 + 噪点叠加 + `backdrop-filter: blur()` + 内阴影 |
| **UI 内容层** | Vue 组件 + Tailwind + Motion for Vue | 可拖动标题栏、Hero、控制面板、按钮、光晕 |

---

## 2. 原生窗口层：Tauri 配置与 Rust 后端

### 2.1 `tauri.conf.json` —— 窗口配置要点

```jsonc
{
  "app": {
    "windows": [
      {
        "title": "Blur Win",
        "label": "main",
        "width": 920,
        "height": 620,
        "minWidth": 720,         // 进入 compact 模式前的最小尺寸
        "minHeight": 500,
        "center": true,
        "decorations": false,    // 必须！移除原生窗口边框/标题栏
        "transparent": true,     // 必须！让窗口背景透明，露出桌面
        "shadow": true,          // 保留原生窗口阴影
        "resizable": true        // 允许窗口缩放
      }
    ]
  }
}
```

**关键约束：**
- `decorations: false` + `transparent: true` 是毛玻璃效果的前提
- `shadow: true` 确保无边框窗口仍有操作系统级阴影

### 2.2 Rust `lib.rs` —— 应用 Acrylic/Blur 背板

```rust
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let window = app
                .get_webview_window("main")
                .expect("main window should exist");

            #[cfg(target_os = "windows")]
            {
                use window_vibrancy::{apply_acrylic, apply_blur};

                // 优先尝试 Acrylic（Win 10/11），失败则回退到 Blur
                if apply_acrylic(&window, Some((15, 20, 32, 145))).is_err() {
                    let _ = apply_blur(&window, Some((15, 20, 32, 125)));
                }
            }

            #[cfg(target_os = "macos")]
            {
                use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial};
                let _ = apply_vibrancy(&window, NSVisualEffectMaterial::HudWindow, None, None);
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

**要点说明：**
- 使用 `window-vibrancy = "0.6.0"` crate（Cargo.toml 中添加该依赖）
- `apply_acrylic(窗口, Some((R, G, B, A)))` —— 传入 RGBA 颜色作为底色，窗口区域会呈现 Windows Acrylic 效果，让桌面背景透过
- 若 Acrylic 不可用（如某些 Windows 版本），回退到 `apply_blur`（简单高斯模糊）
- macOS 直接使用 `apply_vibrancy` 和 `NSVisualEffectMaterial::HudWindow`

---

## 3. CSS 玻璃壳层：`.glass-window`

这是**核心视觉层**，通过一层 CSS 类实现玻璃质感。定义在 `src/assets/index.css` 中。

### 3.1 完整代码

```css
.glass-window {
  isolation: isolate;

  /* ① 多层渐变基底：白色透层 + 三色径向光晕 */
  background:
    linear-gradient(
      135deg,
      rgba(255, 255, 255, 0.20),
      rgba(255, 255, 255, 0.06) 42%,
      rgba(255, 255, 255, 0.11)
    ),
    radial-gradient(circle at 18% 15%, rgba(55, 225, 214, 0.45), transparent 34%),
    radial-gradient(circle at 88% 22%, rgba(255, 163, 71, 0.28), transparent 28%),
    radial-gradient(circle at 55% 92%, rgba(119, 91, 255, 0.28), transparent 36%);

  /* ② 边框 + 多层内阴影 */
  border: 1px solid rgba(255, 255, 255, 0.2);
  box-shadow:
    inset 0 1px 0 rgba(255, 255, 255, 0.24),
    inset 0 0 0 1px rgba(255, 255, 255, 0.07),
    inset 0 -24px 80px rgba(0, 0, 0, 0.18);

  /* ③ 背景模糊（磨砂玻璃核心） */
  backdrop-filter: blur(30px) saturate(1.6);
}

/* ④ ::before 半透明叠加层——控制玻璃不透明度 */
.glass-window::before {
  position: absolute;
  inset: 0;
  z-index: 0;
  pointer-events: none;
  content: "";
  background: rgba(255, 255, 255, 0.16);
  opacity: var(--glass-opacity, 0.74);
}
```

### 3.2 各属性作用解读

| 代码段 | 作用 | 效果 |
|--------|------|------|
| `linear-gradient(...)` | 顶部到底部的白色透层 | 让玻璃有深浅变化，避免均匀的平板感 |
| `radial-gradient(circle at ...)` 三个 | 青、橙、紫三色径向渐变 | 模拟环境光折射，丰富玻璃色彩层次 |
| `border: 1px solid rgba(255,255,255,0.2)` | 半透明白色描边 | 玻璃边缘高光 |
| `box-shadow: inset 0 1px 0 rgba(255,255,255,0.24)` | 顶部内发光 | 模拟光源从上方照入 |
| `box-shadow: inset 0 0 0 1px rgba(255,255,255,0.07)` | 双层内边框 | 增加玻璃厚度感 |
| `box-shadow: inset 0 -24px 80px rgba(0,0,0,0.18)` | 底部大范围暗角 | 模拟玻璃底部的环境阴影 |
| `backdrop-filter: blur(30px) saturate(1.6)` | 背景模糊 + 饱和度增强 | 核心磨砂玻璃效果 |
| `::before` 叠加层 | 半透明白色遮罩 | 通过 `--glass-opacity` CSS 变量控制玻璃"浓度" |
| `isolation: isolate` | 创建新的层叠上下文 | 防止 `::before` 影响子元素层级 |

### 3.3 `--glass-opacity` 变量

由 Vue 组件通过 `shellStyle` 动态设置：

```typescript
const shellStyle = computed(() => ({
  "--glass-opacity": Math.max(0.42, 56 / 100),  // 固定 0.56
  "--compact-progress": compactProgress.value,
}))
```

模板中绑到 `.glass-window` 元素：

```html
<motion.div :style="shellStyle" class="glass-window ...">
```

值越大 · 白色叠加越强 → 玻璃越不透、越"实"；值越小 → 玻璃越透、桌面背景越明显。

---

## 4. UI 内容层与布局

### 4.1 完整布局结构

```html
<main class="min-h-screen bg-transparent text-foreground">
  <motion.div class="glass-window ...relative...">
    <!-- 光晕装饰 (Glow) -->
    <div class="...bg-cyan-300/35 blur-3xl" />
    <div class="...bg-amber-300/25 blur-3xl" />

    <!-- 标题栏 (可拖动) -->
    <header class="relative z-10 flex h-14 ..." @mousedown="startDragging">
      <!-- 左侧：Logo + 标题 -->
      <!-- 右侧：窗口控制按钮 (紧凑/最大化/关闭) -->
    </header>

    <!-- 内容区 -->
    <section class="relative z-10 grid ...">
      <!-- 左栏：Hero (Badge + 标题 + 描述) -->
      <!-- 右栏：控制面板 (Glass Intensity 卡片) -->
    </section>
  </motion.div>
</main>
```

### 4.2 标题栏拖动

```html
<header @mousedown="startDragging">
```

```typescript
async function startDragging() {
  try {
    await appWindow.startDragging()
  } catch (error) {
    console.error("Failed to start window drag", error)
  }
}
```

- 标题栏的 `@mousedown` 调用 Tauri 的 `startDragging()` 实现原生窗口拖动
- 按钮区需加 `@mousedown.stop` 防止拖动事件冒泡

### 4.3 窗口控制按钮

| 按钮 | 功能 | 调用方法 |
|------|------|----------|
| 紧凑按钮 | 在 "Compact" 和 "Restore" 间切换 | `toggleCompactWindow()` |
| 最大化 | 窗口最大化/还原 | `appWindow.toggleMaximize()` |
| 关闭 | 关闭窗口 | `appWindow.close()` |

### 4.4 进入/离开动画（Motion for Vue）

使用 `motion-v` 库做入场动画：

```html
<motion-div
  :initial="{ opacity: 0, scale: 0.96, y: 18 }"
  :animate="{ opacity: 1, scale: 1, y: 0 }"
  :transition="{ duration: 0.55, ease: [0.22, 1, 0.36, 1] }"
>
```

缓动曲线 `[0.22, 1, 0.36, 1]` 是自定义的 ease-out 效果（类似 Material Design 的 deceleration curve）。

各自区域有独立的延迟和弹簧动画（`type: 'spring'`），让内容按序渐入。

---

## 5. Glass Intensity 组件

### 5.1 组件结构

Glass Intensity 不是独立组件文件，而是内嵌在 `App.vue` 的控制面板卡片中：

```html
<motion.div class="no-drag ... rounded-2xl border border-white/16
                    bg-slate-950/35 p-5 shadow-2xl shadow-black/20
                    backdrop-blur-2xl will-change-transform"
            :style="controlPanelStyle"
            :initial="{ opacity: 0, x: 400 }"
            :animate="{ opacity: 1, x: 0 }"
            :transition="{ type: 'spring', damping: 24, stiffness: 155, delay: 0.2 }">
  <!-- 标题行: "Glass intensity" + 百分比 + Glow 开关 -->
  <div class="mb-5 flex items-start justify-between gap-4">
    <div>
      <p class="text-xs uppercase text-foreground/48">Glass intensity</p>
      <p class="mt-1 text-3xl font-semibold tabular-nums">56%</p>
    </div>
    <Button variant="glass" @click="glow = !glow">
      {{ glow ? "Glow on" : "Glow off" }}
    </Button>
  </div>

  <!-- 三个技术指标卡片 -->
  <div class="mt-6 grid grid-cols-3 gap-2">
    <FrameCard label="Frame" value="Borderless" />
    <FrameCard label="Backend" value="Tauri 2" />
    <FrameCard label="Vite" value="8.1" />
  </div>
</motion.div>
```

### 5.2 层级关系

Glass Intensity 控制面板位于 `.glass-window` **内部**，自身也叠加玻璃效果：

- `backdrop-blur-2xl` —— 面板自身也有模糊
- `border border-white/16` —— 半透明描边
- `bg-slate-950/35` —— 深色半透明底
- `shadow-2xl shadow-black/20` —— 投影

它相当于"玻璃上的玻璃卡片"，形成视觉层次感。

### 5.3 按钮 Glass variant

```typescript
props.variant === "glass" &&
  "border border-white/16 bg-white/10 text-foreground shadow-[inset_0_1px_0_rgba(255,255,255,0.2)] hover:bg-white/16"
```

与主壳层保持一致的半透明描边 + 内发光风格。

### 5.4 Glow 光晕装饰

两个 `blur-3xl` 的大色块放在 `.glass-window` 内部相对定位：

```html
<div v-if="glow" class="pointer-events-none absolute left-8 top-9
            h-36 w-36 rounded-full bg-cyan-300/35 blur-3xl" />
<div v-if="glow" class="pointer-events-none absolute bottom-8 right-10
            h-40 w-40 rounded-full bg-amber-300/25 blur-3xl" />
```

它们之所以能产生"光晕"效果，是因为：
1. 用 `blur-3xl`（Tailwind 最大模糊）扩散颜色
2. 颜色本身就是半透明的 `rgba`（如 `bg-cyan-300/35`）
3. `.glass-window` 的 `backdrop-filter` 会进一步与背景融合

---

## 6. 窗口缩放动画优化

这是项目中**最精细的部分**，解决窗口大小变化时视觉掉帧和一次性跳变的问题。

### 6.1 问题背景

Tauri 的 `setPosition()` + `setSize()` 是异步的原生调用。如果连续快速调用，每次都等待前一次完成再发起下一次，会产生卡顿。如果不控制频率（每帧都发），会导致 IPC 过载和 UI 线程阻塞。

### 6.2 核心优化策略

```
requestAnimationFrame 驱动动画时间轴
         │
         ▼
  计算当前帧的目标位置/尺寸
         │
         ▼
  节流控制: 两次原生调用至少间隔 ≈22ms (1000/45 ≈ 45fps)
         │
         ▼
  不等待上一次完成 → 在飞任务时不发起新调用
         │
         ▼
  动画循环持续 → 最终收敛到目标值 → 强制做最后一次同步
```

### 6.3 实现代码拆解

```typescript
const nativeFrameInterval = 1000 / 45  // ≈22.2ms，约 45fps 的节流
```

#### 变量定义

```typescript
let lastNativeFrame = 0           // 上一次原生调用的时间戳
let nativeUpdateInFlight = false  // 是否有在飞的调用
let nativeUpdatePromise = Promise.resolve()  // 用于最后 await
```

#### 发送原生帧（带在飞保护）

```typescript
const applyNativeFrame = (frame) => {
  nativeUpdateInFlight = true

  nativeUpdatePromise = Promise.all([
    appWindow.setPosition(new PhysicalPosition(frame.x, frame.y)),
    appWindow.setSize(new PhysicalSize(frame.width, frame.height)),
  ]).then(() => undefined)
   .finally(() => {
     nativeUpdateInFlight = false  // 完成后释放锁
   })

  return nativeUpdatePromise
}
```

#### 动画循环

```typescript
const tick = async (now) => {
  const elapsed = now - startedAt
  const progress = Math.min(1, elapsed / duration)   // 0→1
  const eased = easeOutCubic(progress)                // 缓动后

  // 原生窗口有可选的延迟
  const nativeProgress = Math.min(1, Math.max(0, (elapsed - nativeDelayMs) / duration))
  const nativeEased = easeOutCubic(nativeProgress)

  // 插值当前帧位置
  const frame = {
    x: lerp(from.x, to.x, nativeEased),
    y: lerp(from.y, to.y, nativeEased),
    width: lerp(from.width, to.width, nativeEased),
    height: lerp(from.height, to.height, nativeEased),
  }

  // 同步更新 CSS 层的 compact-progress
  compactProgress.value = progressFrom + (progressTo - progressFrom) * eased

  // ★ 节流控制：至少间隔 nativeFrameInterval 才发一次
  if (!nativeUpdateInFlight && now - lastNativeFrame >= nativeFrameInterval) {
    lastNativeFrame = now
    applyNativeFrame(frame).catch(reject)
  }

  if (elapsed < duration + nativeDelayMs) {
    requestAnimationFrame(tick)     // 继续下一帧
  } else {
    // ★ 结束收敛：确保最终位置精确
    await nativeUpdatePromise
    await applyNativeFrame(to)      // 最后一次同步到终点
    compactProgress.value = progressTo
    resolve()
  }
}

requestAnimationFrame(tick)
```

### 6.4 缓动函数

```typescript
function easeOutCubic(t: number) {
  return 1 - Math.pow(1 - t, 3)
}
```

easeOutCubic 让动画先快后慢，视觉上更自然。

### 6.5 Compact 模式切换流程

```
紧凑化流程:
  保存当前窗口位置/尺寸 → 设置 minSize(360×230)
  → animateWindowFrame(当前尺寸→360×230, compactProgress=0→1)
  → 同步 CSS 层所有内容的缩放/位移/隐藏

恢复流程:
  设置 minSize(720×500) → 先跳转到 restore 尺寸
  → animateCompactProgress(1→0, 300ms) → CSS 内容动画展开
```

### 6.6 CSS 层同步动画（compactProgress）

```typescript
const heroStyle = computed(() => ({
  opacity: 1 - compactProgress.value * 0.08,
  transform: `translate3d(0, ${compactProgress.value * -8}px, 0)
              scale(${1 - compactProgress.value * 0.08})`,
}))

const titleStyle = computed(() => ({
  transform: `scale(${1 - compactProgress.value * 0.22})`,
  transformOrigin: "left top",
}))

const copyStyle = computed(() => ({
  opacity: 1 - compactProgress.value * 0.22,
  transform: `translate3d(0, ${compactProgress.value * -6}px, 0)`,
}))

const controlPanelStyle = computed(() => ({
  opacity: Math.max(0, 1 - compactProgress.value * 1.35),
  pointerEvents: compactProgress.value > 0.5 ? "none" : "auto",
  transform: `translate3d(${compactProgress.value * 28}px, 0, 0)
              scale(${1 - compactProgress.value * 0.08})`,
  visibility: compactProgress.value > 0.96 ? "hidden" : "visible",
}))
```

每个区域对 compactProgress 的敏感度不同：

| 元素 | 变化 | 系数 | 效果 |
|------|------|------|------|
| Hero 区域 | 缩小 + 上移 + 淡出 | 0.08 | 轻微回缩 |
| 标题字号 | 缩放 | 0.22 | 明显缩小 |
| 描述文字 | 上移 + 淡出 | 0.22 / 6px | 较快消失 |
| 控制面板 | 右移 + 缩小 + 快速淡出 | 1.35 / 28px | 先消失，避免拥挤 |

### 6.7 `windowShellClass` 动态调整

```typescript
const windowShellClass = computed(() =>
  compact.value
    ? "glass-window relative -left-[2px] -top-[2px] grid h-[calc(100vh+2px)] w-[calc(100vw+4px)] min-h-0 overflow-hidden rounded-none"
    : "glass-window relative -left-[2px] -top-[2px] grid h-[calc(100vh+2px)] w-[calc(100vw+4px)] min-h-[500px] overflow-hidden rounded-none",
)
```

- Compact 模式下 `min-h-0`：允许窗口缩小到 230px 高度而不被 CSS `min-height` 阻止
- 正常模式下 `min-h-[500px]`：保持内容不挤压
- `-left-[2px] -top-[2px]` 配合 `h/[w]-calc(...+2/4px)` 消除 border 导致的 1px 间隙

---

## 7. 快速接入清单

### Step 1: 项目初始化

```bash
npm create tauri-app@latest -- --template vue-ts
cd your-project
npm install tauri-plugin-opener window-vibrancy @tauri-apps/api
npm install motion-v lucide-vue-next
```

### Step 2: 配置原生窗口

- 设置 `tauri.conf.json` 中 `decorations: false` + `transparent: true` + `shadow: true`
- 在 `src-tauri/src/lib.rs` 添加 `window-vibrancy` 的 Acrylic/Blur 调用

### Step 3: 添加 CSS 玻璃壳层

在 `src/assets/index.css` 中定义 `.glass-window` 类（渐变、边框、阴影、`backdrop-filter`、`::before` 叠加）。

### Step 4: 实现布局

- `<main>` 容器：`bg-transparent`
- `<header>` 标题栏：可拖动区域 + 窗口控制按钮
- `<section>` 内容区：Hero 标题 + 控制面板卡片
- 光晕装饰：`blur-3xl` 半透明圆形色块

### Step 5: 实现缩放动画

- 参考 `animateWindowFrame()` 的 rAF + 节流控制模式
- 定义 `compactProgress` 驱动 CSS 层内容同步动画
- 使用 `easeOutCubic` 缓动函数

### Step 6: 可选——入场动画

使用 `motion-v` 的 `:initial` / `:animate` / `:transition` 给各区域添加渐入效果。

---

> 本指南对应项目代码：[Blur Win](https://github.com/your-org/blur-win)（Tauri 2 + Vue 3 + Vite 8.1 + Tailwind CSS 4）