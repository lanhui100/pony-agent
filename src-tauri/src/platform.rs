//! Platform/window-specific code extracted from `lib.rs`: compile-time embedded
//! icons, the Win32 taskbar icon FFI, window vibrancy/acrylic/blur effects, and
//! cross-platform URL opening.  Kept separate from the Tauri command wrappers so
//! the command layer stays a pure routing layer.

use tauri::Manager;

/// Icons embedded at compile time so they don't depend on runtime path resolution.
/// `icon.png` is a 512×512 RGBA PNG; `icon.ico` contains multiplatform frames (16–256).
static ICON_PNG: &[u8] = include_bytes!("../icons/icon.png");
#[cfg(target_os = "windows")]
static ICON_ICO: &[u8] = include_bytes!("../icons/icon.ico");

/// Setup system tray icon and menu for Pony Agent.
/// Provides "打开主窗口" (Open window) and "退出" (Quit) options.
/// Left clicking or double clicking the tray icon will also restore/focus the main window.
pub fn setup_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let open_item = MenuItem::with_id(app, "open", "打开 Pony Agent", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open_item, &quit_item])?;

    let icon = match tauri::image::Image::from_bytes(ICON_PNG) {
        Ok(img) => img,
        Err(_) => app
            .default_window_icon()
            .cloned()
            .ok_or("no default window icon available")?,
    };

    let _tray = TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .tooltip("Pony Agent")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            }
            "quit" => {
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            }
        })
        .build(app)?;

    Ok(())
}

/// Load the compile-time embedded icon.png into the main window, and on Windows
/// also explicitly set ICON_BIG for the taskbar (Tauri's set_icon() only sends
/// ICON_SMALL).
pub fn apply_icons(app: &tauri::App) {
    if let Some(window) = app.get_webview_window("main") {
        // === Load icon from compile-time embedded ICON_PNG ===
        // (avoids BaseDirectory::Resource resolving to target/debug/ in dev mode)
        match tauri::image::Image::from_bytes(ICON_PNG) {
            Ok(icon) => {
                eprintln!(
                    "[icon-debug] Decoded embedded icon.png: {}x{}",
                    icon.width(),
                    icon.height()
                );
                if let Err(e) = window.set_icon(icon) {
                    eprintln!("[icon-debug] set_icon err: {e}");
                    // fallback
                    if let Some(fb) = app.default_window_icon().cloned() {
                        let _ = window.set_icon(fb);
                    }
                }
            }
            Err(e) => {
                eprintln!("[icon-debug] from_bytes err: {e}");
                if let Some(fb) = app.default_window_icon().cloned() {
                    eprintln!(
                        "[icon-debug] fallback default {}x{}",
                        fb.width(),
                        fb.height()
                    );
                    let _ = window.set_icon(fb);
                }
            }
        }

        // Windows: also explicitly set ICON_BIG for the taskbar,
        // since Tauri's set_icon() only sends ICON_SMALL.
        #[cfg(target_os = "windows")]
        set_taskbar_icon_win32(app.handle());
    }
}

/// Apply the window material effect: acrylic/blur on Windows, vibrancy on macOS.
/// No-op on other platforms.
pub fn apply_window_style(window: &tauri::WebviewWindow) {
    #[cfg(target_os = "windows")]
    {
        use window_vibrancy::{apply_acrylic, apply_blur};

        if apply_acrylic(window, Some((20, 24, 34, 135))).is_err() {
            let _ = apply_blur(window, Some((20, 24, 34, 120)));
        }
    }

    #[cfg(target_os = "macos")]
    {
        use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial};
        let _ = apply_vibrancy(window, NSVisualEffectMaterial::HudWindow, None, None);
    }

    // Mark the parameter used on platforms where no effect is applied.
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let _ = window;
}

/// Hosts permitted for external browser opening (exact match, lowercase).
/// A new call site MUST extend this list in sync.
const ALLOWED_OPEN_HOSTS: [&str; 4] = [
    "github.com",
    "api.github.com",
    "exa.ai",
    "s3.local.ponyjob.top",
];

/// Pure allowlist check for URLs opened in the system browser.
///
/// Only `https` URLs whose lowercased host exactly matches
/// `{github.com, api.github.com, exa.ai}` pass; everything else fails closed.
/// Path/query/fragment are unrestricted.
pub fn is_allowed_open_url(url: &str) -> bool {
    // Pre-check: raw control characters reject, even as a prefix/suffix.
    if url
        .bytes()
        .any(|b| b == b'\r' || b == b'\n' || b == b'\t')
    {
        return false;
    }
    let candidate = url.trim();
    if candidate.is_empty() {
        return false;
    }
    // String-level trailing-dot probe BEFORE crate parsing: the crate may
    // normalize a trailing dot away, so the raw authority is rejected first.
    {
        let raw_hp = authority_hostport(candidate);
        let raw_host = match raw_hp.rfind(':') {
            Some(i) => &raw_hp[..i],
            None => raw_hp,
        };
        if raw_host.ends_with('.') {
            return false;
        }
    }
    // Mandatory `url` crate parsing — hand-rolled parsing is forbidden.
    let parsed = match url::Url::parse(candidate) {
        Ok(u) => u,
        Err(_) => return false,
    };
    // Scheme: only `https` (`http` is permanently rejected; the crate
    // lowercases the scheme, so mixed-case `HTTPS` normalizes here).
    if parsed.scheme() != "https" {
        return false;
    }
    let host = match parsed.host_str() {
        Some(h) if !h.is_empty() => h,
        _ => return false,
    };
    // Non-ASCII host (IDN lookalike) rejects outright: the `url` crate
    // converts Unicode hosts to punycode, so the raw authority is checked
    // for ASCII as well as the parsed host.
    if !authority_hostport(candidate).is_ascii() {
        return false;
    }
    if !host.is_ascii() {
        return false;
    }
    // Lowercase normalization (paranoia alongside the crate's own), then a
    // second trailing-dot probe on the normalized host.
    let host = host.to_ascii_lowercase();
    if host.ends_with('.') {
        return false;
    }
    // Exact host match — subdomain lookalikes (`github.com.evil.test`,
    // `evil-github.com`) never match exactly.
    if !ALLOWED_OPEN_HOSTS.contains(&host.as_str()) {
        return false;
    }
    // Userinfo / credentials reject even when the host matches.
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return false;
    }
    // Explicit port rejects even when the host matches. The `url` crate drops
    // default ports per WHATWG (so `:443` reads as `port() == None`), hence
    // the additional string-level explicit-port probe.
    if parsed.port().is_some() || has_explicit_port(candidate) {
        return false;
    }
    true
}

/// Raw hostport probe on the URL authority (pre-crate).
///
/// Returns the text after `://` up to the first `/?#`, stripped of userinfo
/// (only the text after the last `@` is the real hostport).
fn authority_hostport(candidate: &str) -> &str {
    let after_scheme = match candidate.find("://") {
        Some(i) => &candidate[i + 3..],
        None => return "",
    };
    let auth_end = after_scheme
        .find(|c| c == '/' || c == '?' || c == '#')
        .unwrap_or(after_scheme.len());
    let authority = &after_scheme[..auth_end];
    match authority.rfind('@') {
        Some(i) => &authority[i + 1..],
        None => authority,
    }
}

/// String-level explicit-port probe on the URL authority.
///
/// Allowlisted hosts are plain ASCII domains, so any `:` in the hostport is
/// an explicit port — including `:443`, which `Url::port()` cannot see (the
/// `url` crate drops default ports per WHATWG, reading `:443` as
/// `port() == None`).
fn has_explicit_port(candidate: &str) -> bool {
    authority_hostport(candidate).contains(':')
}

// Windows-only: `ShellExecuteW` binding (bare FFI, no new crates).
#[cfg(target_os = "windows")]
#[link(name = "shell32")]
unsafe extern "system" {
    fn ShellExecuteW(
        hwnd: isize,
        lpoperation: *const u16,
        lpfile: *const u16,
        lpparameters: *const u16,
        lpdirectory: *const u16,
        nshowcmd: i32,
    ) -> isize;
}

/// Encode `s` as NUL-terminated UTF-16 for Win32 APIs.
///
/// Windows uses `OsStrExt::encode_wide` (per spec); other platforms use
/// `str::encode_utf16` so the construction logic stays unit-testable without
/// spawning anything (pure constructor, no `Command`, no FFI call).
#[cfg(target_os = "windows")]
fn to_wide_null(s: &str) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    std::ffi::OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(all(not(target_os = "windows"), test))]
fn to_wide_null(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Open a URL in the system default browser. Fail-closed: URLs outside the
/// allowlist return `Err("url_allowlist_rejected:<host>")` (host only, never
/// path/query) and spawn nothing.
pub fn open_url_in_browser(url: &str) -> Result<(), String> {
    let candidate = url.trim();
    if !is_allowed_open_url(candidate) {
        // Rejection log carries scheme+host+reason only — never path/query/
        // fragment or the full URL.
        let (scheme, host) = url::Url::parse(candidate)
            .map(|u| {
                let h = u
                    .host_str()
                    .filter(|h| !h.is_empty())
                    .unwrap_or("unknown")
                    .to_owned();
                (u.scheme().to_owned(), h)
            })
            .unwrap_or_else(|_| ("unknown".to_owned(), "unknown".to_owned()));
        eprintln!("[open_url] rejected scheme={scheme} host={host} reason=allowlist");
        return Err(format!("url_allowlist_rejected:{host}"));
    }
    #[cfg(target_os = "windows")]
    {
        // Locked parameter shape: verb=`open`, file=URL UTF-16, params/dir=NULL,
        // SW_SHOWNORMAL=1; the URL never passes through a shell.
        let verb = to_wide_null("open");
        let file = to_wide_null(candidate);
        let ret = unsafe {
            ShellExecuteW(
                0,
                verb.as_ptr(),
                file.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1,
            )
        };
        return if ret > 32 {
            Ok(())
        } else {
            Err("open_failed".to_string())
        };
    }
    #[cfg(target_os = "macos")]
    {
        // `--` ends option parsing so a URL can never be taken as a flag.
        return std::process::Command::new("open")
            .args(["--", candidate])
            .spawn()
            .map(|_| ())
            .map_err(|_| "open_failed".to_string());
    }
    #[cfg(target_os = "linux")]
    {
        // `--` ends option parsing so a URL can never be taken as a flag.
        return std::process::Command::new("xdg-open")
            .args(["--", candidate])
            .spawn()
            .map(|_| ())
            .map_err(|_| "open_failed".to_string());
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        let _ = candidate;
        return Err("open_failed".to_string());
    }
}

/// Windows-only: set BOTH ICON_SMALL (title bar) and ICON_BIG (taskbar + Alt+Tab)
/// from the embedded icon.ico.  Tauri's set_icon() only sends ICON_SMALL.
#[cfg(target_os = "windows")]
fn set_taskbar_icon_win32(app: &tauri::AppHandle) {
    use raw_window_handle::HasWindowHandle;

    extern "system" {
        fn LookupIconIdFromDirectoryEx(
            presbits: *const u8,
            ficon: i32,
            cxdesired: i32,
            cydesired: i32,
            flags: u32,
        ) -> i32;
        fn CreateIconFromResourceEx(
            presbits: *const u8,
            dwresSize: u32,
            ficon: i32,
            dwver: u32,
            cxdesired: i32,
            cydesired: i32,
            flags: u32,
        ) -> isize;
        fn SendMessageW(hwnd: isize, msg: u32, wparam: isize, lparam: isize) -> isize;
    }

    const WM_SETICON: u32 = 0x0080;
    const ICON_BIG: isize = 1;
    const ICON_SMALL: isize = 0;
    const LR_DEFAULTSIZE: u32 = 0x0040;

    let window = match app.get_webview_window("main") {
        Some(w) => w,
        None => {
            eprintln!("[icon-debug] main window not found");
            return;
        }
    };
    let hwnd = match window.window_handle() {
        Ok(h) => match h.as_raw() {
            raw_window_handle::RawWindowHandle::Win32(wh) => wh.hwnd.get() as isize,
            _ => {
                eprintln!("[icon-debug] unexpected window handle type");
                return;
            }
        },
        Err(e) => {
            eprintln!("[icon-debug] window_handle err: {e}");
            return;
        }
    };
    eprintln!("[icon-debug] HWND = 0x{hwnd:x}");

    unsafe {
        // LookupIconIdFromDirectoryEx finds the best-matching icon resource
        // entry for the default icon size (SM_CXICON × SM_CYICON).
        let id = LookupIconIdFromDirectoryEx(
            ICON_ICO.as_ptr(),
            1, // fIcon = TRUE (icon, not cursor)
            0, // cx=0 → use system metric
            0, // cy=0 → use system metric
            LR_DEFAULTSIZE,
        );
        eprintln!("[icon-debug] LookupIconIdFromDirectoryEx -> ID offset {id}");

        if id > 0 {
            let data = &ICON_ICO[id as usize..];
            let hicon = CreateIconFromResourceEx(
                data.as_ptr(),
                data.len() as u32,
                1,          // fIcon
                0x00030000, // dwVer (Windows 3.0 format)
                0,
                0, // desired size = default
                LR_DEFAULTSIZE,
            );
            eprintln!(
                "[icon-debug] HICON = {:p}",
                hicon as *const std::ffi::c_void
            );

            if hicon != 0 {
                SendMessageW(hwnd, WM_SETICON, ICON_BIG, hicon);
                eprintln!("[icon-debug] WM_SETICON(ICON_BIG) OK (taskbar)");
                SendMessageW(hwnd, WM_SETICON, ICON_SMALL, hicon);
                eprintln!("[icon-debug] WM_SETICON(ICON_SMALL) OK (title bar)");
                // HICON now owned by the window
            } else {
                eprintln!("[icon-debug] CreateIconFromResourceEx returned NULL");
            }
        } else {
            eprintln!("[icon-debug] LookupIconIdFromDirectoryEx returned {id} (no match)");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── ICON_PNG: compiled-time embedded 512×512 icon.png ──

    #[test]
    fn embedded_icon_png_decodes_to_512x512() {
        let icon = tauri::image::Image::from_bytes(ICON_PNG)
            .expect("ICON_PNG must be a valid PNG image decodable by Image::from_bytes");
        assert_eq!(
            icon.width(),
            512,
            "embedded ICON_PNG must be 512px wide, got {}",
            icon.width()
        );
        assert_eq!(
            icon.height(),
            512,
            "embedded ICON_PNG must be 512px tall, got {}",
            icon.height()
        );
    }

    #[test]
    fn embedded_icon_png_is_not_empty() {
        assert!(
            !ICON_PNG.is_empty(),
            "ICON_PNG must not be empty (include_bytes! should embed a real file)"
        );
        assert!(
            ICON_PNG.len() > 1000,
            "ICON_PNG size ({}) suspiciously small for a 512×512 PNG",
            ICON_PNG.len()
        );
    }

    // ── ICON_ICO: compiled-time embedded icon.ico (Windows-only) ──

    #[cfg(target_os = "windows")]
    #[test]
    fn embedded_ico_has_valid_header() {
        // ICO header: 2B reserved(0) + 2B type(1) + 2B count
        assert!(ICON_ICO.len() >= 6, "ICO file too small for header");
        assert_eq!(ICON_ICO[0], 0, "ICO reserved byte 0 must be 0");
        assert_eq!(ICON_ICO[1], 0, "ICO reserved byte 1 must be 0");
        assert_eq!(
            ICON_ICO[2], 1,
            "ICO type must be 1 (icon), got {}",
            ICON_ICO[2]
        );
        assert_eq!(ICON_ICO[3], 0, "ICO type high byte must be 0");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn embedded_ico_contains_256x256_frame() {
        let count = u16::from_le_bytes([ICON_ICO[4], ICON_ICO[5]]);
        assert!(count >= 1, "ICO must have at least 1 frame, has {count}");

        let mut found_256 = false;
        for i in 0..count {
            let entry_off = 6 + (i as usize) * 16;
            if entry_off + 16 > ICON_ICO.len() {
                break;
            }
            let w = ICON_ICO[entry_off] as u32;
            let h = ICON_ICO[entry_off + 1] as u32;
            // In ICO, width/height of 0 means 256
            let w = if w == 0 { 256 } else { w };
            let h = if h == 0 { 256 } else { h };
            if w == 256 && h == 256 {
                found_256 = true;
                // Validate the frame data offset + size
                let data_size =
                    u32::from_le_bytes(ICON_ICO[entry_off + 8..entry_off + 12].try_into().unwrap());
                let data_off = u32::from_le_bytes(
                    ICON_ICO[entry_off + 12..entry_off + 16].try_into().unwrap(),
                );
                assert!(
                    data_size > 1000,
                    "256×256 frame data size ({data_size}) too small"
                );
                assert!(
                    (data_off as usize) + (data_size as usize) <= ICON_ICO.len(),
                    "256×256 frame data extends beyond file"
                );
            }
        }
        assert!(
            found_256,
            "ICO must contain a 256×256 frame (width=0, height=0 in ICO entry)"
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn embedded_ico_has_reasonable_frame_count() {
        let count = u16::from_le_bytes([ICON_ICO[4], ICON_ICO[5]]);
        assert!(
            count >= 3,
            "ICO should have at least 3 frames for multi-res support, has {count}"
        );
        assert!(
            count <= 20,
            "ICO has {count} frames — unusually high, may be accidental"
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn embedded_ico_all_entries_reference_valid_data() {
        let count = u16::from_le_bytes([ICON_ICO[4], ICON_ICO[5]]);
        for i in 0..count {
            let entry_off = 6 + (i as usize) * 16;
            if entry_off + 16 > ICON_ICO.len() {
                panic!("Frame {i} entry truncated");
            }
            let data_size =
                u32::from_le_bytes(ICON_ICO[entry_off + 8..entry_off + 12].try_into().unwrap());
            let data_off =
                u32::from_le_bytes(ICON_ICO[entry_off + 12..entry_off + 16].try_into().unwrap());
            assert!(data_size > 0, "Frame {i} has zero data size");
            assert!(
                (data_off as usize) + (data_size as usize) <= ICON_ICO.len(),
                "Frame {i} data [offset={data_off}, size={data_size}] exceeds file length {}",
                ICON_ICO.len()
            );
        }
    }

    // ── is_allowed_open_url: allow matrix ──

    #[test]
    fn allow_github_release_page() {
        assert!(is_allowed_open_url(
            "https://github.com/pony-agent/releases/tag/v0.1.91"
        ));
    }

    #[test]
    fn allow_api_github_and_exa() {
        assert!(is_allowed_open_url(
            "https://api.github.com/repos/owner/repo/releases/latest"
        ));
        assert!(is_allowed_open_url("https://exa.ai/search?q=pony"));
        assert!(is_allowed_open_url("https://s3.local.ponyjob.top/releases/latest.json"));
    }

    #[test]
    fn allow_uppercase_scheme_and_host_normalize() {
        assert!(is_allowed_open_url("HTTPS://GITHUB.COM/owner/repo"));
        assert!(is_allowed_open_url("https://GitHub.COM/owner/repo"));
        assert!(is_allowed_open_url("https://API.GITHUB.COM/x"));
    }

    #[test]
    fn allow_path_query_fragment_unrestricted() {
        assert!(is_allowed_open_url(
            "https://github.com/o/r/releases/tag/v1?a=1&b=2#notes"
        ));
        assert!(is_allowed_open_url("https://exa.ai/?q=a%20b#frag"));
    }

    // ── is_allowed_open_url: deny matrix ──

    #[test]
    fn deny_non_https_schemes_and_case_confusion() {
        for url in [
            "file:///etc/passwd",
            "FILE:///etc/passwd",
            "data:text/html,<h1>x</h1>",
            "DATA:text/html,hi",
            "javascript:alert(1)",
            "JavaScript:alert(1)",
            "JaVaScRiPt:alert(1)",
            "vbscript:msgbox(1)",
            "VBScript:msgbox(1)",
            "blob:https://github.com/abc",
            "BLOB:https://github.com/abc",
            "http://github.com/owner/repo",
            "HTTP://github.com/owner/repo",
        ] {
            assert!(!is_allowed_open_url(url), "must reject {url}");
        }
    }

    #[test]
    fn deny_lookalike_hosts() {
        for url in [
            "https://github.com.evil.test/",
            "https://evil-github.com/",
            "https://evilgithub.com/",
            "https://notexa.ai/",
            "https://exa.ai.evil.test/",
            "https://github.com./",
            "https://github.com./owner",
        ] {
            assert!(!is_allowed_open_url(url), "must reject {url}");
        }
    }

    #[test]
    fn deny_userinfo_and_explicit_ports() {
        for url in [
            "https://user@github.com/",
            "https://user:pass@github.com/",
            "https://github.com@evil.com/",
            "https://github.com:443/",
            "https://github.com:443/owner",
            "https://github.com:8443/",
            "https://api.github.com:443/x",
        ] {
            assert!(!is_allowed_open_url(url), "must reject {url}");
        }
    }

    #[test]
    fn deny_idn_non_ascii_and_punycode() {
        assert!(!is_allowed_open_url("https://githüb.com/owner"));
        assert!(!is_allowed_open_url("https://github。com/owner"));
        assert!(!is_allowed_open_url("https://xn--githb-vua.com/owner"));
    }

    #[test]
    fn deny_control_chars_empty_garbage_no_host() {
        assert!(!is_allowed_open_url(""));
        assert!(!is_allowed_open_url("   "));
        assert!(!is_allowed_open_url("\r\nhttps://github.com/owner"));
        assert!(!is_allowed_open_url("https://github.com/owner\n"));
        assert!(!is_allowed_open_url("https://github.com/\tevil"));
        assert!(!is_allowed_open_url(
            "nota url at all !@#$%^&*()_+-=[]{}|;':,./<>?"
        ));
        assert!(!is_allowed_open_url("https:///no-host-here"));
        assert!(!is_allowed_open_url("https://"));
        assert!(!is_allowed_open_url(&"A".repeat(8192)));
        assert!(!is_allowed_open_url("https://exa.ai:9999/"));
    }

    #[test]
    fn deny_error_carries_host_only() {
        // Fail-closed: host only in the error, never path/query.
        // All inputs here are rejected, so nothing is spawned.
        let err = open_url_in_browser("https://evil.test/secret?tok=abc#x")
            .expect_err("must reject");
        assert_eq!(err, "url_allowlist_rejected:evil.test");
        let err2 = open_url_in_browser("file:///etc/passwd").expect_err("must reject");
        assert!(
            !err2.contains("passwd"),
            "error must not leak path: {err2}"
        );
        let err3 = open_url_in_browser("nota url").expect_err("must reject");
        assert_eq!(err3, "url_allowlist_rejected:unknown");
    }

    // ── ShellExecuteW argument construction (no spawn) ──

    #[test]
    fn wide_null_encoding_is_nul_terminated_utf16() {
        // Pure constructor check — asserts the NUL-terminated UTF-16 shape
        // handed to ShellExecuteW without spawning anything.
        let url = "https://github.com/o/r?a=1&b=2";
        let verb = to_wide_null("open");
        let file = to_wide_null(url);
        assert_eq!(verb.last(), Some(&0), "verb must be NUL-terminated");
        assert_eq!(file.last(), Some(&0), "url must be NUL-terminated");
        assert_eq!(
            verb.len(),
            "open".encode_utf16().count() + 1,
            "verb length must be units + NUL"
        );
        assert_eq!(
            file.len(),
            url.encode_utf16().count() + 1,
            "url length must be units + NUL"
        );
        assert_eq!(
            String::from_utf16(&file[..file.len() - 1]).expect("valid UTF-16"),
            url,
            "round-trip must preserve the URL (incl. &)"
        );
        // `&` must pass through raw — no shell escaping exists on this path.
        let amp: Vec<u16> = "&".encode_utf16().collect();
        assert!(
            file.windows(1).any(|w| w == amp.as_slice()),
            "& must be preserved verbatim (no ^& mangling)"
        );
    }
}
