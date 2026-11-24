/// 将模块内测试函数名解析为当前集成测试二进制在 `cargo test -- --exact` 下的完整过滤名。
pub fn exact_filter(module_path: &str, fn_name: &str) -> String {
    match module_path.split_once("::") {
        Some((_, rel)) => format!("{rel}::{fn_name}"),
        None => fn_name.to_string(),
    }
}
