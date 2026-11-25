//! B163：Tauri 命令面清单锁定（ACP P2 可选面的三条命令）。
//!
//! 为什么用源码清单测试：`invoke_handler` 是宏，注册与否**不影响编译**——漏注册的命令在
//! 运行时才以 `command not found` 暴露，而前端 `invoke` 的失败通常被降级成一条 toast。
//! 这里做交叉核对：`#[tauri::command]` 声明的命令集合 ⊇ `generate_handler![…]` 注册集合，
//! 且三条 ACP 命令两侧都在册。
//!
//! 断言纪律：只钉"名字在册"这一条可机械判定的事实；命令行为由核心 crate 的契约测试
//! （`acp::host` 单元测试 / `acp_client_contract.rs`）负责。

const LIB_RS: &str = include_str!("../src/lib.rs");

/// `generate_handler![ … ]` 里注册的命令名（按出现顺序）。
fn registered_commands() -> Vec<String> {
    let marker = "tauri::generate_handler![";
    let start = LIB_RS
        .find(marker)
        .unwrap_or_else(|| panic!("src/lib.rs 必须包含 `{marker}`"))
        + marker.len();
    let rest = &LIB_RS[start..];
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

/// `#[tauri::command]` 标注声明的函数名。
fn declared_commands() -> Vec<String> {
    let mut declared = Vec::new();
    let lines: Vec<&str> = LIB_RS.lines().collect();
    for (index, line) in lines.iter().enumerate() {
        if line.trim() != "#[tauri::command]" {
            continue;
        }
        // 命令签名可能跨行（多参数），只取第一个含 `fn ` 的行。
        for candidate in lines.iter().skip(index + 1).take(8) {
            let trimmed = candidate.trim();
            let without_async = trimmed.strip_prefix("async ").unwrap_or(trimmed);
            if let Some(rest) = without_async.strip_prefix("fn ") {
                let name: String = rest
                    .chars()
                    .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
                    .collect();
                if !name.is_empty() {
                    declared.push(name);
                }
                break;
            }
        }
    }
    declared
}

#[test]
fn acp_commands_are_declared_and_registered() {
    let declared = declared_commands();
    let registered = registered_commands();
    for command in ["acp_list_agents", "acp_delegate_start", "acp_delegate_cancel"] {
        assert!(
            declared.iter().any(|name| name == command),
            "B163 违约：`{command}` 必须带 `#[tauri::command]` 声明，实际声明集合 {declared:?}"
        );
        assert!(
            registered.iter().any(|name| name == command),
            "B163 违约：`{command}` 必须在 `generate_handler![…]` 注册（漏注册只会在运行时\
             以 command not found 暴露），实际注册集合 {registered:?}"
        );
    }
}

#[test]
fn every_registered_command_has_a_declaration() {
    let declared = declared_commands();
    let registered = registered_commands();
    let dangling: Vec<&String> = registered
        .iter()
        .filter(|name| !declared.iter().any(|declared_name| declared_name == *name))
        .collect();
    assert!(
        dangling.is_empty(),
        "注册集合里出现未在源码中声明的命令 {dangling:?}（清单解析或代码结构已漂移）"
    );
}
