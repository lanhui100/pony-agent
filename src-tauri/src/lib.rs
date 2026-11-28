pub use pony_agent_core::agent;
mod blocking_helper;
/// 宿主命令面：按业务域分文件，`lib.rs` 只做装配（B1795831358336 Phase 1）。
mod commands;
mod goal_events;
mod goal_resume;
mod platform;
mod tauri_adapter;
/// wave-60 C6（A1 裁决）：注册表需被 `src-tauri/tests/` 直接引用 ⇒ 改 `pub mod`。
pub mod turn_task_registry;

use crate::commands::diagnostics::StreamDebugMetricsState;
use crate::commands::settings::load_app_settings;
use crate::commands::{
    acp, ask, capability, diagnostics, goal, history, host, plan, session, settings, system, team,
    turn, workspace,
};
use crate::turn_task_registry::TurnTaskRegistry;
use agent::control_plane::HostControlPlane;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Manager;

/// 退出 flush 的幂等短路位（C5.3）：`ExitRequested` 与 `Exit` 共享，避免预算叠加。
static EXIT_FLUSH_DONE: AtomicBool = AtomicBool::new(false);

/// 退出路径的统一 OTel flush 出口（Wave-4 C5 / Wave-4.1 A5 / r2-dim1 attempt 2）。
///
/// 契约：
/// - 有界：单次 flush 预算直接复用 core 的 [`pony_agent_core::agent::otel::EXIT_FLUSH_BUDGET_MS`]
///   （= `EXPORT_TIMEOUT_MS` = 3000ms），**不再**在桌面端另立常量 —— 两份常量各写各的正是
///   r2-dim1 attempt 1 的缺陷来源：core 侧预算 1000ms < 导出层 3000ms 时，
///   「退出前 flush」在网络非瞬时下必然超时，等于没加；
///   配合 `otel::shutdown()`（内部等待档位 0ms，队列已在 flush 中排空）退出总预算 ≤3000ms，
///   满足 NFR `observability.exporter_flush_timeout_ms` 与 C5 验收判据#1；
/// - 幂等：`ExitRequested` 与 `Exit` 共享 [`EXIT_FLUSH_DONE`] 短路，重复事件不再叠加预算；
/// - 未装配（otel 装配失败/非桌面启动路径）时静默跳过；未完成只降级 warn JSON 日志；绝不 panic。
fn flush_otel_on_exit(stage: &str) {
    if pony_agent_core::agent::otel::config().is_none() {
        return;
    }
    if EXIT_FLUSH_DONE.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Err(message) = pony_agent_core::agent::otel::force_flush(
        pony_agent_core::agent::otel::EXIT_FLUSH_BUDGET_MS,
    ) {
        pony_agent_core::agent::otel::log::log_json(
            "warn",
            &format!("退出 flush（{stage}）未在预算内完成：{message}"),
            None,
        );
    }
}

/// 进程级 run-event 处理器（Wave-4 C5 退出拦截）。
///
/// 抽成具名函数而非内联闭包：`run()` 已在代码规模棘轮上（`scripts/file-size-baseline.json`
/// 允许 ≤200 行），而这段退出语义自带 4 行注释——内联会把主装配函数挤出预算。
///
/// 语义（不得改动）：`ExitRequested` 与 `Exit` 两个分支都必须触发**有界** force_flush
/// （硬上限 3000ms，对齐 NFR `observability.exporter_flush_timeout_ms`），保证缓冲区中
/// 尚未发出的 span 在进程退出前完成投递；两条路径幂等，重复触发不得 panic / 死锁。
fn handle_run_event(_app_handle: &tauri::AppHandle, event: tauri::RunEvent) {
    match event {
        tauri::RunEvent::Exit => {
            flush_otel_on_exit("exit");
            // NFR resource_release：flush 后释放 OTel provider 及其导出 runtime（幂等）。
            pony_agent_core::agent::otel::shutdown();
        }
        // 退出请求（进程可能继续运行，如托盘常驻）同样有界 flush，但不释放 provider。
        tauri::RunEvent::ExitRequested { .. } => flush_otel_on_exit("exit_requested"),
        _ => {}
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        // PA-103: updater/process plugins are registered so the capability permission IDs
        // (updater:allow-check, updater:allow-download-and-install, process:allow-restart)
        // resolve in this build. The tauri.conf.json updater block stays absent until the
        // release-owner gate provisions a real endpoint and public key; the frontend adapter
        // is disabled (SIGNED_UPDATER_ENABLED=false), so the plugin commands are never invoked.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(HostControlPlane::new())
        .manage(StreamDebugMetricsState::new())
        .manage(TurnTaskRegistry::with_max_concurrent(3))
        // O1-B：桌面端续轮驱动者继承「该会话最近一次提交的授权模式」（缺省 read_only）。
        .manage(goal_resume::SessionAuthorizationModes::default())
        .setup(|app| {
            // B130：goal 领域事件 → `goal://*` 宿主事件的唯一装配点。必须在任何 goal
            // 变更发生前完成安装（否则首批 create/update 无事件；UI 只能等 turn 结束再拉）。
            goal_events::install(app.handle().clone());
            if let Some(window) = app.get_webview_window("main") {
                platform::apply_icons(app);
                platform::apply_window_style(&window);
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
                // 如需开发时打开 DevTools 调试面板，可解除下行注释或按 F12
                // #[cfg(debug_assertions)]
                // window.open_devtools();
            }
            if let Err(e) = platform::setup_tray(app) {
                eprintln!("[tray-warning] failed to setup tray icon: {e}");
            }

            // Wave43 D2/D3：provider 进程启动时装配一次（开关只翻转 flag，不重建 provider）。
            // 装配失败必须降级为日志：trace 不可用绝不阻塞主业务启动。
            //
            // Wave-4 C4.2：桌面端结构化日志落盘（追加 + 512KiB 轮转），路径由 app_data_dir 解析；
            // 解析/创建失败只影响落盘，不影响 stdout 与主链路。
            if let Ok(app_data_dir) = app.path().app_data_dir() {
                pony_agent_core::agent::otel::log::set_log_file_path(
                    app_data_dir.join("logs").join("pony-agent.log"),
                );
            }

            // Wave-4 C1：配置来源优先级 settings > env > 内置默认（公网端点 https://otel.ponyjob.top）。
            let app_settings = load_app_settings(app.handle().clone());
            let otel_settings = app_settings.otel_settings_snapshot();
            // wave-51 C1（审计 §4 P0-1）：resource `service.version` 取安装包版本（单一来源：
            // 宿主注入 > env `PONY_OTEL_SERVICE_VERSION` > core crate 版本）。
            let app_version = app.package_info().version.to_string();
            if let Err(err) = pony_agent_core::agent::otel::init_with_service_version(
                pony_agent_core::agent::otel::OtelConfig::from_settings(Some(&otel_settings)),
                Some(app_version.as_str()),
            ) {
                // F4：走统一 JSON 日志面（stdout 单行 JSON），不得新增明文 eprintln 边界。
                pony_agent_core::agent::otel::log::log_json(
                    "warn",
                    &format!("otel 装配失败，trace 上报已禁用：{err}"),
                    None,
                );
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            host::health_check,
            host::inspect_host,
            host::load_execution_checkpoint,
            host::load_graph_run_checkpoint,
            turn::run_turn,
            turn::start_graph_run,
            turn::start_graph_run_stream,
            turn::continue_graph_run,
            turn::continue_graph_run_stream,
            turn::resume_graph_run,
            turn::resume_graph_run_stream,
            turn::stop_turn,
            turn::stop_graph_run,
            turn::fail_turn_for_watchdog,
            turn::graph_list_ask_waits,
            turn::graph_bind_ask_wait,
            turn::graph_resume_ask,
            ask::ask_list_pending,
            ask::ask_lifecycle_state,
            ask::ask_answer,
            ask::control_approve_request,
            ask::ask_cancel,
            ask::ask_expire,
            plan::plan_mode_get,
            plan::plan_mode_set,
            plan::plan_mode_commit,
            plan::plan_mode_review,
            goal::todo_list,
            goal::goal_get,
            goal::goal_budget_lines,
            goal::goal_create,
            goal::goal_update,
            goal::goal_command,
            team::team_get_roster,
            team::team_get_tasks,
            team::team_claim_task,
            team::team_complete_task,
            history::load_build_context_observation,
            history::load_session_runtime_view,
            history::resolve_graph_run_submission_plan,
            history::load_retrieved_context,
            history::load_history_graph,
            history::load_model_monitor_summary,
            history::load_model_monitor_session_drilldown,
            history::load_history_cursor,
            history::checkout_history_node,
            history::restore_branch_head,
            history::fork_from_history_node,
            history::switch_history_branch,
            history::delete_session,
            history::compact_session,
            session::list_sessions,
            session::upload_session_trace,
            session::session_rename,
            session::session_archive,
            session::session_unarchive,
            workspace::get_workspace_root,
            workspace::import_attachment,
            workspace::workspace_list,
            workspace::workspace_create,
            workspace::workspace_rename,
            workspace::workspace_delete,
            workspace::authorize_path,
            workspace::revoke_authorization,
            workspace::list_authorizations,
            settings::load_app_settings,
            settings::save_app_settings,
            settings::load_provider_registry,
            settings::save_provider_registry,
            settings::save_provider_registry_without_env_sync,
            settings::fetch_provider_models,
            settings::get_service_api_key,
            settings::set_service_api_key,
            capability::list_available_tools,
            capability::list_capability_sources,
            capability::apply_mcp_source_snapshot,
            capability::mcp_health,
            capability::load_mcp_sources,
            capability::list_capabilities,
            capability::inspect_capability,
            capability::inspect_capability_source,
            capability::list_skills,
            capability::inspect_skill,
            diagnostics::record_stream_debug_metrics,
            diagnostics::load_stream_debug_metrics,
            diagnostics::append_frontend_trace_events,
            diagnostics::query_frontend_trace_window,
            diagnostics::query_frontend_stall_snapshots,
            diagnostics::clear_frontend_trace_before,
            diagnostics::export_frontend_trace_json,
            diagnostics::export_frontend_trace_chrome_trace,
            system::open_url,
            system::ponysentry_capture,
            system::usage_governance_summary,
            acp::acp_list_agents,
            acp::acp_delegate_start,
            acp::acp_delegate_cancel
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // 当用户点击关闭按钮时，拦截默认的关闭销毁行为，隐藏窗口并保持后台/托盘常驻
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("failed to build Pony Agent")
        .run(handle_run_event);
}
