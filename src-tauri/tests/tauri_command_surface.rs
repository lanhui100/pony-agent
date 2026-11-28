//! B163：Tauri 命令面清单锁定（ACP P2 可选面的三条命令）。
//!
//! 为什么用源码清单测试：`invoke_handler` 是宏，注册与否**不影响编译**——漏注册的命令在
//! 运行时才以 `command not found` 暴露，而前端 `invoke` 的失败通常被降级成一条 toast。
//! 这里做交叉核对：`#[tauri::command]` 声明的命令集合 == `generate_handler![…]` 注册集合，
//! 且三条 ACP 命令两侧都在册。
//!
//! B1795831358336 Phase 1 后的扫描面：命令声明已按业务域迁到 `src/commands/*.rs`，
//! `src/lib.rs` 只保留 `generate_handler![…]` 注册表。故本测试改为**运行时遍历 `src/` 全树**
//! 收集声明（新增命令域文件无需改本测试，也无需维护第二份文件清单），注册表仍从 `lib.rs` 解析。
//!
//! 断言纪律：只钉"名字在册"这一条可机械判定的事实；命令行为由核心 crate 的契约测试
//! （`acp::host` 单元测试 / `acp_client_contract.rs`）负责。

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// 命令面声明的源码根：`src-tauri/src`。
fn src_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn collect_rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("无法读取目录 {}：{error}", dir.display()));
    for entry in entries {
        let path = entry.expect("读取目录项失败").path();
        if path.is_dir() {
            collect_rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

fn read_source(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| panic!("无法读取 {}：{error}", path.display()))
}

/// `generate_handler![ … ]` 里注册的命令名（按出现顺序，形如 `host::health_check`）。
fn registered_commands() -> Vec<String> {
    let source = read_source(&src_root().join("lib.rs"));
    let marker = "tauri::generate_handler![";
    let start = source
        .find(marker)
        .unwrap_or_else(|| panic!("src/lib.rs 必须包含 `{marker}`"))
        + marker.len();
    let rest = &source[start..];
    let end = rest
        .find(']')
        .unwrap_or_else(|| panic!("generate_handler![ 缺少闭合 `]`"));
    rest[..end]
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect()
}

/// 跳过 `#[tauri::command]` 与函数签名之间可能出现的其它属性（如 `#[expect(...)]`）与文档注释，
/// 返回签名行里的函数名。
fn command_name_after(lines: &[&str], marker_index: usize) -> Option<String> {
    // 属性可以跨行（`#[expect(\n  clippy::...,\n  reason = "..."\n)]`），故用括号配平判断属性是否结束。
    let mut open_brackets = 0i32;
    let mut inside_attribute = false;
    for candidate in lines.iter().skip(marker_index + 1).take(24) {
        let trimmed = candidate.trim();
        if trimmed.is_empty() {
            continue;
        }
        if inside_attribute || trimmed.starts_with("#[") {
            inside_attribute = true;
            open_brackets += trimmed.matches('[').count() as i32;
            open_brackets -= trimmed.matches(']').count() as i32;
            if open_brackets <= 0 {
                inside_attribute = false;
                open_brackets = 0;
            }
            continue;
        }
        if trimmed.starts_with("//") {
            continue;
        }
        let without_visibility = match trimmed.strip_prefix("pub(") {
            Some(rest) => rest.split_once(')').map_or(trimmed, |(_, tail)| tail.trim_start()),
            None => trimmed.strip_prefix("pub ").map_or(trimmed, str::trim_start),
        };
        let without_async = without_visibility
            .strip_prefix("async ")
            .map_or(without_visibility, str::trim_start);
        let rest = without_async.strip_prefix("fn ")?;
        let name: String = rest
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
            .collect();
        return (!name.is_empty()).then_some(name);
    }
    None
}

/// 全树收集的 `#[tauri::command]` 声明，键形如 `host::health_check`（模块名 = 文件 stem；
/// 直接写在 `lib.rs`/`main.rs` 里的命令用裸名）。
fn declared_commands() -> BTreeSet<String> {
    let root = src_root();
    let mut files = Vec::new();
    collect_rust_files(&root, &mut files);
    let mut declared = BTreeSet::new();
    for path in files {
        let module = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default()
            .to_string();
        let source = read_source(&path);
        let lines: Vec<&str> = source.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            if line.trim() != "#[tauri::command]" {
                continue;
            }
            let Some(name) = command_name_after(&lines, index) else {
                continue;
            };
            if module == "lib" || module == "main" {
                declared.insert(name);
            } else {
                declared.insert(format!("{module}::{name}"));
            }
        }
    }
    declared
}

fn normalized_registered(registered: &[String]) -> BTreeSet<String> {
    registered.iter().cloned().collect()
}

#[test]
fn acp_commands_are_declared_and_registered() {
    let declared = declared_commands();
    let registered = registered_commands();
    for command in ["acp_list_agents", "acp_delegate_start", "acp_delegate_cancel"] {
        assert!(
            declared.iter().any(|name| name.ends_with(command)),
            "B163 违约：`{command}` 必须带 `#[tauri::command]` 声明，实际声明集合 {declared:?}"
        );
        assert!(
            registered.iter().any(|name| name.ends_with(command)),
            "B163 违约：`{command}` 必须在 `generate_handler![…]` 注册（漏注册只会在运行时\
             以 command not found 暴露），实际注册集合 {registered:?}"
        );
    }
}

#[test]
fn every_registered_command_has_a_declaration() {
    let declared = declared_commands();
    let registered = normalized_registered(&registered_commands());
    let dangling: Vec<&String> = registered.difference(&declared).collect();
    assert!(
        dangling.is_empty(),
        "注册集合里出现未在源码中声明的命令 {dangling:?}（清单解析或代码结构已漂移）"
    );
}

/// B1795831358336 红线：拆分只搬家、不增删命令 —— 反向也要成立，
/// 否则「声明了但忘记注册」同样只会在运行时以 command not found 暴露。
#[test]
fn every_declared_command_is_registered() {
    let declared = declared_commands();
    let registered = normalized_registered(&registered_commands());
    let unregistered: Vec<&String> = declared.difference(&registered).collect();
    assert!(
        unregistered.is_empty(),
        "源码声明了 `#[tauri::command]` 但未在 `generate_handler![…]` 注册：{unregistered:?}"
    );
}
