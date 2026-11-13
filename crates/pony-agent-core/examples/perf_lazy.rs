//! B148 阶段 3/4 的「Ready 口径」性能探针（诊断资产，不参与生产构建）。
//!
//! 与 `perf_probe` 的分工：本探针**只测懒装载链路**，不跑全量装载，因此没有
//! 「上一相位刚写了 660 MB 造成 WAL 恢复」这类串扰——它回答的是「用户按回车到
//! 控制面可用」的真实成本分布。
//!
//! 用法（**必须指向库副本**）：
//!   PONY_AGENT_DATA_DIR=/tmp/b148-fresh \
//!   cargo run --release -p pony-agent-core --example perf_lazy -- \
//!     /tmp/b148-fresh/PonyAgent/sessions.db <session_id> 3
//!
//! 相位：
//! - `A_meta_steps`：新后端连接 + `load_store_metadata` / `load_session_meta_index` /
//!   `validate_event_schema` 三段的独立耗时；
//! - `B_open_lazy`：生产入口 `SessionStore::open_lazy()`（Ready 的真实成本）；
//! - `C_hydrate`：单会话定向 hydrate + 只读快照（resume 首屏代理）；
//! - `D_list_parity`：稀疏列表 id 序列 vs 全量装载（一次性，最贵）。

use std::path::PathBuf;
use std::time::Instant;

use pony_agent_core::agent::session::{SeparateTraceTableMode, SessionBackend, SessionStore};
use pony_agent_core::agent::sqlite_session::SqliteSessionBackend;

fn vm_status() -> String {
    let s = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let mut out = Vec::new();
    for line in s.lines() {
        if line.starts_with("VmHWM:") || line.starts_with("VmRSS:") {
            out.push(line.trim().to_string());
        }
    }
    out.join(" | ")
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let db_path = PathBuf::from(
        args.get(1)
            .cloned()
            .unwrap_or_else(|| "/tmp/b148-fresh/PonyAgent/sessions.db".into()),
    );
    let session_id = args
        .get(2)
        .cloned()
        .unwrap_or_else(|| "pa_01a1b478-e830-76b2-99b9-17a3df8b11fd".into());
    let rounds: usize = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(3);

    println!("== perf_lazy ==");
    println!("db_path={}", db_path.display());
    println!("session_id={session_id}");
    println!("rounds={rounds}");
    println!(
        "PONY_AGENT_DATA_DIR={:?}",
        std::env::var("PONY_AGENT_DATA_DIR").ok()
    );
    if let Ok(meta) = std::fs::metadata(&db_path) {
        println!("db_bytes={}", meta.len());
    }
    println!("baseline {}", vm_status());

    // 相位 A：元数据子步骤（每次都用**新后端**，等价于每次冷启动的连接成本）。
    for r in 0..rounds {
        let t = Instant::now();
        let be = SqliteSessionBackend::new_with_trace_mode(
            db_path.clone(),
            SeparateTraceTableMode::WriteSeparate,
        );
        let construct_ms = t.elapsed().as_secs_f64() * 1000.0;
        let t = Instant::now();
        let metadata = be.load_store_metadata();
        let metadata_ms = t.elapsed().as_secs_f64() * 1000.0;
        let t = Instant::now();
        let index = be.load_session_meta_index();
        let index_ms = t.elapsed().as_secs_f64() * 1000.0;
        let t = Instant::now();
        let schema = be.validate_event_schema();
        let schema_ms = t.elapsed().as_secs_f64() * 1000.0;
        println!(
            "phase=A_meta_steps round={} construct_ms={:.3} metadata_ms={:.3} has_metadata={} index_ms={:.3} index_sessions={} schema_ms={:.3} schema_ok={}",
            r + 1,
            construct_ms,
            metadata_ms,
            metadata.is_some(),
            index_ms,
            index.as_ref().map(|entries| entries.len()).unwrap_or(0),
            schema_ms,
            schema.is_ok()
        );
    }

    // 相位 B：生产入口（Ready 成本）。
    for r in 0..rounds {
        let t = Instant::now();
        let store = SessionStore::open_lazy();
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        let list = store.list_sessions();
        println!(
            "phase=B_open_lazy round={} ms={:.3} sparse={} meta_sessions={} list_len={}",
            r + 1,
            ms,
            store.is_sparse(),
            store.meta_entries().len(),
            list.len()
        );
        println!("phase=B_after_ready {}", vm_status());
    }

    // 相位 C：resume 首屏代理。
    for r in 0..rounds {
        let mut store = SessionStore::open_lazy();
        let t = Instant::now();
        let loaded = store.hydrate_session(&session_id);
        let snapshot = store.snapshot_at_readonly(Some(&session_id), None, &[]);
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        println!(
            "phase=C_hydrate round={} ms={:.3} loaded={loaded} history={}",
            r + 1,
            ms,
            snapshot.history.len()
        );
    }

    report_parity();

    println!("final {}", vm_status());
    println!("== done ==");
}

/// 只打印值的规模（避免把整段历史刷屏）：数组给长度、字符串给长度与前 40 字符。
fn summarize_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Array(items) => format!("array(len={})", items.len()),
        serde_json::Value::String(text) => {
            let preview = text.chars().take(40).collect::<String>();
            format!("str(len={}, {:?})", text.chars().count(), preview)
        }
        other => {
            let raw = other.to_string();
            if raw.chars().count() > 60 {
                format!("{}…", raw.chars().take(60).collect::<String>())
            } else {
                raw
            }
        }
    }
}

/// 相位 D/E：列表 id 一致性 + 逐 id 快照等价（需要一次全量装载做对照）。
fn report_parity() {
    // 相位 D：列表 id 一致（一次性全量装载，最贵，放最后）。
    let sparse = SessionStore::open_lazy();
    let sparse_ids = sparse
        .list_sessions()
        .iter()
        .map(|item| item.conversation_id.clone())
        .collect::<Vec<_>>();
    let t = Instant::now();
    let full = SessionStore::new();
    let full_ms = t.elapsed().as_secs_f64() * 1000.0;
    let full_ids = full
        .list_sessions()
        .iter()
        .map(|item| item.conversation_id.clone())
        .collect::<Vec<_>>();
    println!(
        "phase=D_list_parity identical={} sparse_len={} full_len={} full_load_ms={:.1}",
        sparse_ids == full_ids,
        sparse_ids.len(),
        full_ids.len(),
        full_ms
    );
    if sparse_ids != full_ids {
        let mut reported = 0usize;
        for (index, (sparse_id, full_id)) in sparse_ids.iter().zip(full_ids.iter()).enumerate() {
            if sparse_id != full_id {
                println!(
                    "phase=D_list_parity_diff index={index} sparse={sparse_id} full={full_id} \
                     sparse_prev={:?} full_prev={:?}",
                    sparse_ids.get(index.saturating_sub(1)),
                    full_ids.get(index.saturating_sub(1))
                );
                reported += 1;
                if reported >= 3 {
                    break;
                }
            }
        }
    }

    // 相位 E：真实库上「每 id 可 resume 且首屏历史一致」——抽样比对定向 hydrate 快照
    // 与全量装载快照的 JSON 等价性（同一进程、同一库、同一环境 ⇒ 应当逐字段相同）。
    let step = (full_ids.len() / 8).max(1);
    let sample = full_ids
        .iter()
        .step_by(step)
        .take(8)
        .cloned()
        .collect::<Vec<_>>();
    let mut lazy_store = SessionStore::open_lazy();
    let mut equal = 0usize;
    for session_id in &sample {
        let hydrated = lazy_store.hydrate_session(session_id);
        let lazy_snapshot = lazy_store.snapshot_at_readonly(Some(session_id), None, &[]);
        let full_snapshot = full.snapshot_at_readonly(Some(session_id), None, &[]);
        // 等价判据：**用户可见内容**逐字段一致（历史 / trace / 元数据统计）。
        // 刻意排除 `historyNodes`：无持久化图时节点由 `ensure_history_graph` 在内存合成，
        // 节点 id 内嵌「合成时刻」时间戳（`…-node-<ms>-<ordinal>`），两侧必然不同；
        // 这里只要求节点**数量**一致（结构等价）。
        let lazy_json = serde_json::to_value(&lazy_snapshot).unwrap_or(serde_json::Value::Null);
        let full_json = serde_json::to_value(&full_snapshot).unwrap_or(serde_json::Value::Null);
        let same = lazy_json.get("history") == full_json.get("history")
            && lazy_json.get("turnTraceHistory") == full_json.get("turnTraceHistory")
            && lazy_json.get("turnCount") == full_json.get("turnCount")
            && lazy_json.get("title") == full_json.get("title")
            && lazy_json.get("summary") == full_json.get("summary")
            && lazy_json.get("workspaceId") == full_json.get("workspaceId")
            && lazy_snapshot.history_nodes.len() == full_snapshot.history_nodes.len();
        if same {
            equal += 1;
        } else {
            let mut diffs = Vec::new();
            for key in [
                "history",
                "turnTraceHistory",
                "turnCount",
                "title",
                "summary",
                "workspaceId",
            ] {
                let lazy_value = lazy_json.get(key).cloned().unwrap_or(serde_json::Value::Null);
                let full_value = full_json.get(key).cloned().unwrap_or(serde_json::Value::Null);
                if lazy_value != full_value {
                    diffs.push(format!(
                        "{key}: lazy={} full={}",
                        summarize_value(&lazy_value),
                        summarize_value(&full_value)
                    ));
                }
            }
            if lazy_snapshot.history_nodes.len() != full_snapshot.history_nodes.len() {
                diffs.push(format!(
                    "historyNodes.len: lazy={} full={}",
                    lazy_snapshot.history_nodes.len(),
                    full_snapshot.history_nodes.len()
                ));
            }
            println!(
                "phase=E_snapshot_parity_diff session={} diffs=[{}]",
                session_id,
                diffs.join("; ")
            );
        }
        println!(
            "phase=E_snapshot_parity session={} hydrated={} identical={}",
            session_id, hydrated, same
        );
    }
    println!(
        "phase=E_snapshot_parity_summary sampled={} identical={}",
        sample.len(),
        equal
    );
}
