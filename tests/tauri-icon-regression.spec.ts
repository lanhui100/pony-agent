import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

const repoRoot = path.resolve(__dirname, "..");
const tauriConfigPath = path.join(repoRoot, "src-tauri", "tauri.conf.json");
const tauriLibPath = path.join(repoRoot, "src-tauri", "src", "lib.rs");
const tauriPlatformPath = path.join(repoRoot, "src-tauri", "src", "platform.rs");

describe("tauri icon regression", () => {
  it("keeps the required bundled icon assets configured", () => {
    const config = JSON.parse(fs.readFileSync(tauriConfigPath, "utf8")) as {
      bundle?: { icon?: string[] };
    };

    const iconList = config.bundle?.icon ?? [];

    expect(iconList).toEqual(
      expect.arrayContaining([
        "icons/taskbar-64.png",
        "icons/128x128.png",
        "icons/128x128@2x.png",
        "icons/256x256.png",
        "icons/512x512.png",
        "icons/icon.ico",
        "icons/icon.png",
        "icons/Square150x150Logo.png",
        "icons/StoreLogo.png"
      ])
    );
  });

  it("binds the default window icon to the main window at runtime", () => {
    const rustSource = fs.readFileSync(tauriLibPath, "utf8");
    const platformSource = fs.readFileSync(tauriPlatformPath, "utf8");

    // 窗口获取仍在 lib.rs 的 setup 中；图标绑定逻辑已外提到 platform.rs。
    expect(rustSource).toContain('app.get_webview_window("main")');
    expect(platformSource).toContain("app.default_window_icon().cloned()");
    expect(platformSource).toContain("if let Err(e) = window.set_icon(icon)");
    expect(platformSource).toContain("tauri::image::Image::from_bytes(ICON_PNG)");
  });
});
