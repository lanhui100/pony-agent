//! ADR 0099 **阶段 3-A** 验收：会话目录包（bundle）导出 / 导入。
//!
//! 真源仍是每会话一份 append-only 日志；本套用例证明它可被**打包成可传输单元**再完整
//! 还原（换机 / 分享 / 审计），且导入路径**不碰 SQLite 索引**。
//!
//! 八类用例（全部真断言，无「跑通即放行」）：
//!
//! 1. [`dir_bundle_round_trip_is_field_by_field_identical`] —— 目录包往返：导出 → **全新
//!    目录**导入 → `fold_session` 的 `SessionState` 与源端**逐字段一致**；
//! 2. [`tar_bundle_round_trip_is_field_by_field_identical`] —— 单文件包（手写最小 ustar，
//!    不压缩）往返同样逐字段一致，且真的是 ustar 形态；
//! 3. [`tampered_events_are_rejected_without_writing_anything`] —— 篡改 `events.jsonl`
//!    一个字节 ⇒ 校验失败（EXIT=2）、目标根零落盘；
//! 4. [`truncated_event_stream_is_rejected_with_a_clear_report`] —— 删掉末行 ⇒ 先被 sha256
//!    拦下；把哈希改对（模拟「篡改者同步改了 manifest」）后由 eventCount/seq 一致性拦下；
//! 5. [`unsupported_bundle_version_is_rejected`] —— `version: 99` ⇒ 拒绝并说明；
//! 6. [`duplicate_import_is_refused_then_force_backs_up_the_old_copy`] —— 重复导入默认
//!    EXIT=3；`--force` 后旧目录保留为 `.bak-*`（含 CLI 端到端退出码）；
//! 7. [`dry_run_writes_nothing`] —— 目标日志根目录文件清单 + mtime + 大小前后一致；
//! 8. [`import_writes_no_sqlite_index_until_rebuild_index`] —— 导入后 `sessions` 表无该行，
//!    `rebuild-index` 后出现且内容与日志 fold 一致。
//!
//! 隔离：所有库、日志与包都落在 `std::env::temp_dir()` 下的唯一目录，**绝不触碰**
//! `~/.local/share/PonyAgent/`；fixture `Drop` 清理。

use std::path::{Path, PathBuf};

use pony_agent_core::agent::session::{
    export_session, import_bundle, read_session_state, rebuild_index, session_log_root_for_db,
    BundleError, BundleFormat, ImportOptions, SeparateTraceTableMode, SessionBackend,
    SessionLogStore, SessionState, SessionStore, TurnTraceRecord,
};
use pony_agent_core::agent::sqlite_session::SqliteSessionBackend;

const SESSION_ID: &str = "pa_3a_bundle";

// ---------------------------------------------------------------------------
// 夹具
// ---------------------------------------------------------------------------

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let unique = format!(
            "pony-0099-3a-bundle-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock before unix epoch")
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&root).expect("create fixture root");
        Self { root }
    }

    /// 一个「应用数据目录」（`sessions.db` + `session-logs/` 同级）。
    fn app_dir(&self, label: &str) -> PathBuf {
        let dir = self.root.join(label);
        std::fs::create_dir_all(&dir).expect("create app dir");
        dir
    }

    fn db(&self, app_dir: &Path) -> PathBuf {
        app_dir.join("sessions.db")
    }

    fn log_root(&self, app_dir: &Path) -> PathBuf {
        session_log_root_for_db(&self.db(app_dir))
    }

    fn store(&self, app_dir: &Path) -> SessionStore {
        SessionStore::with_backend(Box::new(SqliteSessionBackend::new_with_trace_mode(
            self.db(app_dir),
            SeparateTraceTableMode::WriteSeparate,
        )))
    }

    fn folded(&self, app_dir: &Path, session_id: &str) -> SessionState {
        read_session_state(&SessionLogStore::new(self.log_root(app_dir)), session_id)
            .unwrap_or_else(|error| panic!("fold `{session_id}`: {error}"))
            .state
    }

    /// 目标日志根下该会话的落点目录（导入成功/失败都以它为准）。
    fn session_dir(&self, app_dir: &Path, session_id: &str) -> PathBuf {
        let cwd = current_cwd();
        SessionLogStore::new(self.log_root(app_dir))
            .session_dir(session_id, &cwd)
            .expect("session dir")
    }

    /// 日志根的文件清单 + mtime + 大小（`--dry-run` 零写入的物理凭证）。
    fn tree_fingerprint(&self, root: &Path) -> Vec<(String, u64, u128)> {
        let mut entries = Vec::new();
        if !root.exists() {
            return entries;
        }
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for item in std::fs::read_dir(&dir).expect("read dir").flatten() {
                let path = item.path();
                let meta = item.metadata().expect("metadata");
                if meta.is_dir() {
                    stack.push(path);
                    continue;
                }
                let relative = path
                    .strip_prefix(root)
                    .expect("strip root")
                    .to_string_lossy()
                    .replace('\\', "/");
                let mtime = meta
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|duration| duration.as_nanos())
                    .unwrap_or_default();
                entries.push((relative, meta.len(), mtime));
            }
        }
        entries.sort();
        entries
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn current_cwd() -> String {
    pony_agent_core::agent::session::current_log_cwd()
}

fn canonical<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).expect("serialize")
}

fn contents(session: &SessionState) -> Vec<String> {
    session
        .history
        .iter()
        .map(|message| message.content.clone())
        .collect()
}

/// 造一个「有内容」的会话：2 轮 turn + 1 条 trace（history / nodes / cursor / branches /
/// traces 全都有东西，往返才有可证伪性）。
fn seed_session(fixture: &Fixture, app_dir: &Path) -> Vec<String> {
    let mut store = fixture.store(app_dir);
    for (user, assistant) in [("问一", "答一"), ("问二", "答二")] {
        store
            .append_turn_fallible(Some(SESSION_ID), user, assistant, None, Vec::new())
            .unwrap_or_else(|error| panic!("append_turn 必须成功：{error}"));
    }
    store.record_turn_trace(
        Some(SESSION_ID),
        TurnTraceRecord {
            turn_id: "turn-0".to_string(),
            title: "turn-0 标题".to_string(),
            phase: "completed".to_string(),
            provider_name: Some("mock".to_string()),
            total_tokens: Some(42),
            ..Default::default()
        },
    );
    let log = fixture.log_root(app_dir);
    let session_dir = fixture.session_dir(app_dir, SESSION_ID);
    assert!(
        session_dir.join("session.jsonl").is_file(),
        "前置：真源日志必须存在（{}）",
        log.display()
    );
    vec![
        "问一".to_string(),
        "答一".to_string(),
        "问二".to_string(),
        "答二".to_string(),
    ]
}

/// 逐字段比对：`SessionState` 整份 JSON + task 点名的字段各自再断言一遍。
fn assert_round_trip(source: &SessionState, imported: &SessionState, context: &str) {
    assert_eq!(
        canonical(imported),
        canonical(source),
        "{context}：导入后 fold 出的 SessionState 必须与源端**逐字段一致**"
    );
    assert_eq!(contents(imported), contents(source), "{context}：history 必须一致");
    assert_eq!(imported.title, source.title, "{context}：title 必须一致");
    assert_eq!(imported.turn_count, source.turn_count, "{context}：turn_count 必须一致");
    let source_nodes: Vec<&str> = source
        .history_nodes
        .iter()
        .map(|node| node.node_id.as_str())
        .collect();
    let imported_nodes: Vec<&str> = imported
        .history_nodes
        .iter()
        .map(|node| node.node_id.as_str())
        .collect();
    assert_eq!(imported_nodes, source_nodes, "{context}：history_nodes 必须一致");
    assert!(
        !source_nodes.is_empty(),
        "{context}：前置条件——源端必须真的有历史节点，否则「一致」没有意义"
    );
    assert_eq!(
        canonical(&imported.history_cursor),
        canonical(&source.history_cursor),
        "{context}：history_cursor 必须一致"
    );
    assert_eq!(
        canonical(&imported.history_branches),
        canonical(&source.history_branches),
        "{context}：history_branches 必须一致"
    );
    let source_turns: Vec<&str> = source
        .turn_trace_history
        .iter()
        .map(|trace| trace.turn_id.as_str())
        .collect();
    let imported_turns: Vec<&str> = imported
        .turn_trace_history
        .iter()
        .map(|trace| trace.turn_id.as_str())
        .collect();
    assert_eq!(
        imported_turns, source_turns,
        "{context}：turn_trace_history 必须一致"
    );
    assert!(!source_turns.is_empty(), "{context}：前置条件——源端必须有 trace");
}

/// 复制一个目录树（构造「坏包」时用）。
fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("create copy dir");
    for entry in std::fs::read_dir(from).expect("read source") {
        let entry = entry.expect("dir entry");
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("copy file");
        }
    }
}

fn read_manifest(path: &Path) -> serde_json::Value {
    let raw = std::fs::read_to_string(path).expect("read manifest");
    serde_json::from_str(&raw).expect("parse manifest")
}

fn write_manifest(path: &Path, value: &serde_json::Value) {
    std::fs::write(path, serde_json::to_string_pretty(value).expect("serialize manifest"))
        .expect("write manifest");
}

// ---------------------------------------------------------------------------
// 1. 目录包往返
// ---------------------------------------------------------------------------

#[test]
fn dir_bundle_round_trip_is_field_by_field_identical() {
    let fixture = Fixture::new("dir");
    let source_app = fixture.app_dir("source");
    let expected = seed_session(&fixture, &source_app);
    let source_state = fixture.folded(&source_app, SESSION_ID);
    assert_eq!(contents(&source_state), expected);

    let bundle = fixture.root.join("bundle-dir");
    let report = export_session(
        &fixture.log_root(&source_app),
        SESSION_ID,
        &bundle,
        BundleFormat::Dir,
        "test-version",
        None,
    )
    .expect("export 目录包必须成功");
    assert_eq!(report.exit_code(), 0, "附件全部成功 ⇒ EXIT=0");
    assert!(bundle.join("manifest.json").is_file(), "包里必须有 manifest.json");
    assert!(bundle.join("events.jsonl").is_file(), "包里必须有 events.jsonl");
    // 事件条数必须与包内 `events.jsonl` 的真实行数一致（header 之外每行一条事件）。
    let event_lines = std::fs::read_to_string(bundle.join("events.jsonl"))
        .expect("read events.jsonl")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();
    assert_eq!(
        report.event_count,
        event_lines - 1,
        "报告里的事件条数必须等于包内事件行数（header 之外）"
    );
    assert_eq!(
        report.last_seq,
        Some(report.event_count as u64 - 1),
        "lastSeq 必须等于 eventCount-1（seq 从 0 起连续）"
    );
    let manifest = read_manifest(&bundle.join("manifest.json"));
    assert_eq!(manifest["format"], serde_json::json!("pony.session.bundle"));
    assert_eq!(manifest["version"], serde_json::json!(1));
    assert_eq!(manifest["sessionId"], serde_json::json!(SESSION_ID));
    assert_eq!(manifest["cwd"], serde_json::json!(current_cwd()));
    let files = manifest["files"].as_array().expect("files array");
    assert!(
        files.iter().any(|entry| entry["path"] == serde_json::json!("events.jsonl")),
        "files 必须登记 events.jsonl：{manifest}"
    );
    // manifest 自身不在 files 里（自引用哈希无解），但必须有 sha256 保护其它每个文件。
    for entry in files {
        let sha = entry["sha256"].as_str().expect("sha256");
        assert_eq!(sha.len(), 64, "sha256 必须是 64 位十六进制");
        assert!(entry["bytes"].as_u64().is_some());
    }

    // —— 全新目录导入 ——
    let target_app = fixture.app_dir("target");
    let import_report = import_bundle(
        &fixture.log_root(&target_app),
        &bundle,
        ImportOptions::apply(),
    )
    .expect("import 目录包必须成功");
    assert_eq!(import_report.exit_code(), 0);
    assert_eq!(import_report.event_count, report.event_count);
    assert_eq!(import_report.last_seq, report.last_seq);
    assert!(import_report.backup.is_none(), "首次导入不应产生备份");

    let imported_state = fixture.folded(&target_app, SESSION_ID);
    assert_round_trip(&source_state, &imported_state, "目录包");

    eprintln!(
        "[1] dir bundle：events={} lastSeq={:?} files={:?} → 往返 {} 条消息 / {} 节点 / {} trace 完全一致",
        report.event_count,
        report.last_seq,
        report.files,
        imported_state.history.len(),
        imported_state.history_nodes.len(),
        imported_state.turn_trace_history.len()
    );
}

// ---------------------------------------------------------------------------
// 2. 单文件形态（手写最小 ustar）
// ---------------------------------------------------------------------------

#[test]
fn tar_bundle_round_trip_is_field_by_field_identical() {
    let fixture = Fixture::new("tar");
    let source_app = fixture.app_dir("source");
    seed_session(&fixture, &source_app);
    let source_state = fixture.folded(&source_app, SESSION_ID);

    let bundle = fixture.root.join("bundle.tar");
    let report = export_session(
        &fixture.log_root(&source_app),
        SESSION_ID,
        &bundle,
        BundleFormat::Tar,
        "test-version",
        None,
    )
    .expect("export 单文件包必须成功");
    assert_eq!(report.exit_code(), 0);
    assert_eq!(report.format_label, "tar");
    let bytes = std::fs::read(&bundle).expect("read tar");
    assert!(bytes.len() >= 1024 && bytes.len() % 512 == 0, "tar 必须是 512 对齐");
    assert_eq!(
        &bytes[257..262],
        b"ustar",
        "单文件形态必须是 ustar（不压缩，未接 zstd）"
    );
    // 收尾两个全零块。
    assert!(
        bytes[bytes.len() - 1024..].iter().all(|byte| *byte == 0),
        "tar 必须由两个全零块收尾"
    );
    assert!(!bundle.join("events.jsonl").exists(), "单文件形态不应残留目录");

    let target_app = fixture.app_dir("target");
    let import_report = import_bundle(
        &fixture.log_root(&target_app),
        &bundle,
        ImportOptions::apply(),
    )
    .expect("import 单文件包必须成功");
    assert_eq!(import_report.exit_code(), 0);
    let imported_state = fixture.folded(&target_app, SESSION_ID);
    assert_round_trip(&source_state, &imported_state, "单文件包");

    eprintln!(
        "[2] tar bundle：{}B / events={} → 往返逐字段一致（history={} nodes={} traces={}）",
        bytes.len(),
        report.event_count,
        imported_state.history.len(),
        imported_state.history_nodes.len(),
        imported_state.turn_trace_history.len()
    );
}

// ---------------------------------------------------------------------------
// 3. 坏包拒绝（篡改一个字节）
// ---------------------------------------------------------------------------

#[test]
fn tampered_events_are_rejected_without_writing_anything() {
    let fixture = Fixture::new("tamper");
    let source_app = fixture.app_dir("source");
    seed_session(&fixture, &source_app);
    let bundle = fixture.root.join("bundle-dir");
    export_session(
        &fixture.log_root(&source_app),
        SESSION_ID,
        &bundle,
        BundleFormat::Dir,
        "test-version",
        None,
    )
    .expect("export");

    // 篡改一个字节（把 events.jsonl 中间的一个 ASCII 字符换掉，长度不变）。
    let events_path = bundle.join("events.jsonl");
    let mut bytes = std::fs::read(&events_path).expect("read events");
    let index = bytes.iter().position(|byte| *byte == b'"').expect("find quote");
    bytes[index] = b'\''; // 让该行 JSON 结构变化，同时字节数不变
    std::fs::write(&events_path, &bytes).expect("write tampered events");

    let target_app = fixture.app_dir("target");
    let error = import_bundle(
        &fixture.log_root(&target_app),
        &bundle,
        ImportOptions::apply(),
    )
    .expect_err("篡改后的包必须被拒绝");
    assert_eq!(error.exit_code(), 2, "坏包必须 EXIT=2：{error}");
    let message = error.to_string();
    assert!(
        message.contains("events.jsonl"),
        "报错必须点名具体文件：{message}"
    );
    assert!(
        !fixture.session_dir(&target_app, SESSION_ID).exists(),
        "校验失败时目标会话目录必须**不存在**（零落盘）"
    );
    assert!(
        fixture.tree_fingerprint(&fixture.log_root(&target_app)).is_empty(),
        "目标日志根必须完全没被写入"
    );
    eprintln!("[3] 篡改一个字节 → EXIT=2「{}」，目标根零落盘", message);
}

// ---------------------------------------------------------------------------
// 4. 事件流被截断
// ---------------------------------------------------------------------------

#[test]
fn truncated_event_stream_is_rejected_with_a_clear_report() {
    let fixture = Fixture::new("truncate");
    let source_app = fixture.app_dir("source");
    seed_session(&fixture, &source_app);
    let good_bundle = fixture.root.join("bundle-good");
    export_session(
        &fixture.log_root(&source_app),
        SESSION_ID,
        &good_bundle,
        BundleFormat::Dir,
        "test-version",
        None,
    )
    .expect("export");

    // (a) 直接删掉末行：先被 sha256 拦下。
    let plain = fixture.root.join("bundle-truncated");
    copy_tree(&good_bundle, &plain);
    let events_path = plain.join("events.jsonl");
    let raw = std::fs::read_to_string(&events_path).expect("read events");
    let mut lines: Vec<&str> = raw.lines().collect();
    assert!(lines.len() >= 3, "前置：事件流至少 3 行（header + ≥2 事件）");
    lines.pop();
    std::fs::write(&events_path, format!("{}\n", lines.join("\n"))).expect("write truncated");
    let target_a = fixture.app_dir("target-a");
    let error = import_bundle(&fixture.log_root(&target_a), &plain, ImportOptions::apply())
        .expect_err("截断包必须被拒绝");
    assert_eq!(error.exit_code(), 2);
    assert!(
        error.to_string().contains("events.jsonl"),
        "报错必须点名 events.jsonl：{error}"
    );
    assert!(!fixture.session_dir(&target_a, SESSION_ID).exists(), "零落盘");

    // (b) 截断 + 同步改对 sha256/bytes（模拟「篡改者也改了清单」）：必须由
    //     eventCount/seq 一致性拦下（manifest 不参与自校验，所以要结构性兜底）。
    let forged = fixture.root.join("bundle-truncated-forged");
    copy_tree(&good_bundle, &forged);
    let events_path = forged.join("events.jsonl");
    let raw = std::fs::read_to_string(&events_path).expect("read events");
    let mut lines: Vec<&str> = raw.lines().collect();
    lines.pop();
    let truncated = format!("{}\n", lines.join("\n"));
    std::fs::write(&events_path, &truncated).expect("write forged truncated");
    let manifest_path = forged.join("manifest.json");
    let mut manifest = read_manifest(&manifest_path);
    let digest = pony_agent_core::agent::session::bundle::sha256_hex(truncated.as_bytes());
    for entry in manifest["files"]
        .as_array_mut()
        .expect("files array")
        .iter_mut()
    {
        if entry["path"] == serde_json::json!("events.jsonl") {
            entry["sha256"] = serde_json::json!(digest);
            entry["bytes"] = serde_json::json!(truncated.len() as u64);
        }
    }
    write_manifest(&manifest_path, &manifest);
    let target_b = fixture.app_dir("target-b");
    let error = import_bundle(&fixture.log_root(&target_b), &forged, ImportOptions::apply())
        .expect_err("哈希被改对的截断包仍必须被拒绝");
    assert_eq!(error.exit_code(), 2);
    let message = error.to_string();
    assert!(
        message.contains("eventCount") || message.contains("seq"),
        "报错必须指明事件数与 seq 不一致：{message}"
    );
    assert!(!fixture.session_dir(&target_b, SESSION_ID).exists(), "零落盘");
    eprintln!("[4] 截断：sha256 拦截 +（改对哈希后）结构性校验拦截「{message}」");
}

// ---------------------------------------------------------------------------
// 5. 版本不支持
// ---------------------------------------------------------------------------

#[test]
fn unsupported_bundle_version_is_rejected() {
    let fixture = Fixture::new("version");
    let source_app = fixture.app_dir("source");
    seed_session(&fixture, &source_app);
    let bundle = fixture.root.join("bundle-dir");
    export_session(
        &fixture.log_root(&source_app),
        SESSION_ID,
        &bundle,
        BundleFormat::Dir,
        "test-version",
        None,
    )
    .expect("export");

    let manifest_path = bundle.join("manifest.json");
    let mut manifest = read_manifest(&manifest_path);
    manifest["version"] = serde_json::json!(99);
    write_manifest(&manifest_path, &manifest);

    let target_app = fixture.app_dir("target");
    let error = import_bundle(
        &fixture.log_root(&target_app),
        &bundle,
        ImportOptions::apply(),
    )
    .expect_err("version=99 必须被拒绝");
    assert_eq!(error.exit_code(), 2);
    assert!(
        matches!(&error, BundleError::Validation { step: "version", .. }),
        "必须是 version 步的校验失败：{error:?}"
    );
    assert!(
        error.to_string().contains("99"),
        "报错必须点名不支持的版本号：{error}"
    );
    assert!(!fixture.session_dir(&target_app, SESSION_ID).exists(), "零落盘");
    eprintln!("[5] version=99 → EXIT=2「{error}」");
}

// ---------------------------------------------------------------------------
// 6. 重复导入：默认拒绝，--force 备份旧目录（含 CLI 端到端退出码）
// ---------------------------------------------------------------------------

#[test]
fn duplicate_import_is_refused_then_force_backs_up_the_old_copy() {
    let fixture = Fixture::new("duplicate");
    let source_app = fixture.app_dir("source");
    seed_session(&fixture, &source_app);
    let bundle = fixture.root.join("bundle-dir");
    export_session(
        &fixture.log_root(&source_app),
        SESSION_ID,
        &bundle,
        BundleFormat::Dir,
        "test-version",
        None,
    )
    .expect("export");

    let target_app = fixture.app_dir("target");
    import_bundle(&fixture.log_root(&target_app), &bundle, ImportOptions::apply())
        .expect("首次导入必须成功");
    let first_log = fixture
        .session_dir(&target_app, SESSION_ID)
        .join("session.jsonl");
    let first_bytes = std::fs::read(&first_log).expect("read imported log");

    // 默认拒绝（库层 API）。
    let error = import_bundle(&fixture.log_root(&target_app), &bundle, ImportOptions::apply())
        .expect_err("目标已存在 ⇒ 默认拒绝");
    assert_eq!(error.exit_code(), 3, "目标已存在必须 EXIT=3：{error}");
    assert!(matches!(error, BundleError::TargetExists { .. }));

    // CLI 端到端：同一条路径同样给 EXIT=3（证明 CLI 接线与退出码契约一致）。
    let cli = std::process::Command::new(env!("CARGO_BIN_EXE_ponycli"))
        .arg("sessions")
        .arg("import")
        .arg(&bundle)
        .arg("--db")
        .arg(fixture.db(&target_app))
        .output()
        .expect("run ponycli sessions import");
    assert_eq!(
        cli.status.code(),
        Some(3),
        "CLI 重复导入必须 EXIT=3\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&cli.stdout),
        String::from_utf8_lossy(&cli.stderr)
    );

    // --force：旧目录改名 `.bak-*`（不删），新内容覆盖。
    let forced = import_bundle(
        &fixture.log_root(&target_app),
        &bundle,
        ImportOptions {
            dry_run: false,
            force: true,
        },
    )
    .expect("--force 必须成功");
    let backup = forced.backup.as_ref().expect("--force 必须给出备份路径");
    assert!(backup.is_dir(), "旧目录必须被改名保留：{}", backup.display());
    let backup_name = backup
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    assert!(
        backup_name.contains(".bak-"),
        "备份目录名必须含 `.bak-`：{backup_name}"
    );
    assert!(
        backup.join("session.jsonl").is_file(),
        "备份里必须保留旧日志"
    );
    assert_eq!(
        std::fs::read(&first_log).expect("read log after force"),
        first_bytes,
        "覆盖后日志内容必须与首次导入逐字节相同（同一份包）"
    );

    eprintln!(
        "[6] 重复导入：API/CLI 均 EXIT=3；--force 后旧目录={}",
        backup.display()
    );
}

// ---------------------------------------------------------------------------
// 7. dry-run 零写入
// ---------------------------------------------------------------------------

#[test]
fn dry_run_writes_nothing() {
    let fixture = Fixture::new("dry_run");
    let source_app = fixture.app_dir("source");
    seed_session(&fixture, &source_app);
    let bundle = fixture.root.join("bundle-dir");
    export_session(
        &fixture.log_root(&source_app),
        SESSION_ID,
        &bundle,
        BundleFormat::Dir,
        "test-version",
        None,
    )
    .expect("export");

    let target_app = fixture.app_dir("target");
    let log_root = fixture.log_root(&target_app);
    let before = fixture.tree_fingerprint(&log_root);
    let report = import_bundle(&log_root, &bundle, ImportOptions::dry_run())
        .expect("dry-run 校验必须通过");
    let after = fixture.tree_fingerprint(&log_root);
    assert!(report.dry_run, "报告必须标注 dry-run");
    assert_eq!(report.event_count > 0, true, "预演也要报告事件条数");
    assert_eq!(
        before, after,
        "dry-run 必须零写入：日志根的文件清单 / 大小 / mtime 前后必须完全一致"
    );
    assert!(
        !log_root.exists() || !fixture.session_dir(&target_app, SESSION_ID).exists(),
        "dry-run 不得创建目标会话目录"
    );
    assert!(
        !fixture.db(&target_app).exists(),
        "dry-run 不得创建/修改会话库文件"
    );
    eprintln!("[7] dry-run：日志根 {} 个文件前后指纹一致，零写入", after.len());
}

// ---------------------------------------------------------------------------
// 8. 不写索引：导入后 rebuild-index 才出现
// ---------------------------------------------------------------------------

#[test]
fn import_writes_no_sqlite_index_until_rebuild_index() {
    let fixture = Fixture::new("no_index");
    let source_app = fixture.app_dir("source");
    seed_session(&fixture, &source_app);
    let source_state = fixture.folded(&source_app, SESSION_ID);
    let bundle = fixture.root.join("bundle-dir");
    export_session(
        &fixture.log_root(&source_app),
        SESSION_ID,
        &bundle,
        BundleFormat::Dir,
        "test-version",
        None,
    )
    .expect("export");

    let target_app = fixture.app_dir("target");
    // 先建库（建表由首次打开完成），再导入 ⇒ 断言导入本身没有写索引行。
    {
        let backend = SqliteSessionBackend::new_with_trace_mode(
            fixture.db(&target_app),
            SeparateTraceTableMode::WriteSeparate,
        );
        let _ = backend.load_store();
    }
    import_bundle(&fixture.log_root(&target_app), &bundle, ImportOptions::apply())
        .expect("import");

    let conn = rusqlite::Connection::open(fixture.db(&target_app)).expect("open db");
    let rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE conversation_id = ?1",
            rusqlite::params![SESSION_ID],
            |row| row.get(0),
        )
        .unwrap_or(0);
    assert_eq!(
        rows, 0,
        "导入**只落日志与附件**：SQLite `sessions` 表不得因导入出现该行（索引由 rebuild-index 补）"
    );
    drop(conn);

    // rebuild-index 后索引补齐，且内容与日志 fold 一致。
    let report = rebuild_index(
        &SqliteSessionBackend::new_with_trace_mode(
            fixture.db(&target_app),
            SeparateTraceTableMode::WriteSeparate,
        ),
        pony_agent_core::agent::session::RebuildOptions::apply(),
    )
    .expect("rebuild-index");
    assert!(
        report.rebuilt.contains(&SESSION_ID.to_string()),
        "重建必须覆盖导入的会话：{:?}",
        report.rebuilt
    );
    let backend = SqliteSessionBackend::new_with_trace_mode(
        fixture.db(&target_app),
        SeparateTraceTableMode::WriteSeparate,
    );
    let loaded = backend.load_store().expect("load_store");
    let indexed = loaded
        .sessions_view()
        .get(SESSION_ID)
        .unwrap_or_else(|| panic!("重建后索引里必须有该会话"));
    assert_eq!(contents(indexed), contents(&source_state), "索引内容必须与源端一致");
    assert_eq!(indexed.turn_count, source_state.turn_count);
    eprintln!(
        "[8] 导入后 sessions 行=0；rebuild-index 后 rebuilt={:?} 且内容一致（{} 条消息）",
        report.rebuilt,
        indexed.history.len()
    );
}

// ---------------------------------------------------------------------------
// 9. CLI 端到端 + 附件降级（EXIT=4，不得静默成 0）
// ---------------------------------------------------------------------------

#[test]
fn cli_round_trip_and_attachment_degradation_is_exit_code_4() {
    let fixture = Fixture::new("cli_degraded");
    let source_app = fixture.app_dir("source");
    seed_session(&fixture, &source_app);
    let source_state = fixture.folded(&source_app, SESSION_ID);
    let bundle = fixture.root.join("bundle-dir");

    // CLI export（目录形态）。
    let export = std::process::Command::new(env!("CARGO_BIN_EXE_ponycli"))
        .arg("sessions")
        .arg("export")
        .arg(SESSION_ID)
        .arg("--out")
        .arg(&bundle)
        .arg("--db")
        .arg(fixture.db(&source_app))
        .arg("--format")
        .arg("dir")
        .output()
        .expect("run ponycli sessions export");
    let stdout = String::from_utf8_lossy(&export.stdout);
    assert_eq!(
        export.status.code(),
        Some(0),
        "CLI 导出必须 EXIT=0\nstdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&export.stderr)
    );
    assert!(stdout.contains("format: dir"), "必须报告形态：\n{stdout}");
    assert!(stdout.contains(&format!("session: {SESSION_ID}")), "必须报告会话：\n{stdout}");

    // 人为制造附件降级：manifest 登记两张附件——一张内容被改（sha 不符），一张实体缺失。
    let attachment_dir = bundle.join("attachments");
    std::fs::create_dir_all(&attachment_dir).expect("create attachments dir");
    let tampered = b"tampered-attachment-bytes".to_vec();
    std::fs::write(attachment_dir.join("broken.png"), &tampered).expect("write attachment");
    let manifest_path = bundle.join("manifest.json");
    let mut manifest = read_manifest(&manifest_path);
    let files = manifest["files"].as_array_mut().expect("files array");
    files.push(serde_json::json!({
        "path": "attachments/broken.png",
        "sha256": pony_agent_core::agent::session::bundle::sha256_hex(b"something-else"),
        "bytes": tampered.len() as u64,
    }));
    files.push(serde_json::json!({
        "path": "attachments/missing.png",
        "sha256": pony_agent_core::agent::session::bundle::sha256_hex(b"missing"),
        "bytes": 7,
    }));
    write_manifest(&manifest_path, &manifest);

    // CLI import：事件成功 + 附件降级 ⇒ EXIT=4（不是 0，也不是 2），且逐条报告。
    let target_app = fixture.app_dir("target");
    let import = std::process::Command::new(env!("CARGO_BIN_EXE_ponycli"))
        .arg("sessions")
        .arg("import")
        .arg(&bundle)
        .arg("--db")
        .arg(fixture.db(&target_app))
        .output()
        .expect("run ponycli sessions import");
    let stdout = String::from_utf8_lossy(&import.stdout);
    assert_eq!(
        import.status.code(),
        Some(4),
        "附件降级必须 EXIT=4\nstdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&import.stderr)
    );
    assert!(
        stdout.contains("attachments/broken.png") && stdout.contains("attachments/missing.png"),
        "降级必须逐条列出文件与原因（不得静默）：\n{stdout}"
    );
    assert!(
        stdout.contains("不写 SQLite 索引") || stdout.contains("rebuild-index"),
        "必须提示索引由 rebuild-index 补：\n{stdout}"
    );

    // 事件侧仍然完整导入（降级不影响真源）。
    let imported_state = fixture.folded(&target_app, SESSION_ID);
    assert_round_trip(&source_state, &imported_state, "CLI 往返 + 附件降级");
    // 坏附件不得落盘（降级 = 不入库，不是写坏文件）。
    let target_attachments = fixture
        .session_dir(&target_app, SESSION_ID)
        .join("attachments");
    assert!(
        !target_attachments.join("broken.png").exists(),
        "哈希不符的附件不得落盘"
    );

    eprintln!(
        "[9] CLI：export EXIT=0 / import EXIT=4（附件降级逐条报告），事件 {} 条仍完整往返",
        imported_state.history.len()
    );
}
