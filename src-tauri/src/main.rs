#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // 确保主线程处于活动的 Tokio 1.x Reactor 运行时上下文中，
    // 避免 WebView2 处理网络请求/IPC 时发生 "there is no reactor running" panic。
    let _rt = tokio::runtime::Runtime::new().expect("failed to start tokio runtime");
    let _guard = _rt.enter();
    pony_agent_lib::run();
}
