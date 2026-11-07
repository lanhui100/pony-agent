//! 原 `tui.rs` 内联测试模块 `mod cli_self_update_tests`（Step1 机械外移，断言零改动）。

    use super::*;
    use serde_json::json;
    use sha2::{Digest, Sha256};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, MutexGuard};
    use std::time::{Duration, Instant};

    // ── 临时目录（谁创建谁清理：Drop 时整目录删除） ────────────────────────
    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(label: &str) -> Self {
            static SEQ: AtomicUsize = AtomicUsize::new(0);
            let seq = SEQ.fetch_add(1, Ordering::SeqCst);
            let path = std::env::temp_dir().join(format!(
                "pony-update-test-{}-{seq}-{label}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).expect("必须能创建测试临时目录");
            Self { path }
        }

        fn join(&self, name: &str) -> PathBuf {
            self.path.join(name)
        }

        /// 残留断言：成功/失败路径都不得留下 `.new*` / `.part` / `.update.lock`。
        fn leftover_artifacts(&self) -> Vec<String> {
            std::fs::read_dir(&self.path)
                .expect("必须能列临时目录")
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.file_name().to_string_lossy().to_string())
                .filter(|name| {
                    name.contains(".new") || name.contains(".part") || name.ends_with(".update.lock")
                })
                .collect()
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        format!("{:x}", hasher.finalize())
    }

    /// 本机在发布侧平台标识白名单里的取值（仅用于构造清单夹具，不依赖 `current_platform()`）。
    fn test_platform() -> &'static str {
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("linux", "x86_64") => "linux-x86_64",
            ("linux", "aarch64") => "linux-aarch64",
            ("macos", "x86_64") => "macos-x86_64",
            ("macos", "aarch64") => "macos-aarch64",
            other => panic!("本用例组只覆盖 unix 平台，实际={other:?}"),
        }
    }

    fn manifest_json(
        version: &str,
        platform: &str,
        asset: &str,
        sha256: &str,
        size: u64,
    ) -> String {
        json!({
            "schema": "pony-cli-manifest",
            "version": version,
            "generated_at": "2026-11-07T00:00:00Z",
            "platforms": { platform: { "asset": asset, "sha256": sha256, "size": size } }
        })
        .to_string()
    }

    /// 伪造「新二进制」：`--version` 逐字打印与 `cli_version_string()` 同形的输出
    /// （`pony <ver> · <os>-<arch>` + `binary: <path>`），使 C6-5 的安装后回验可通过。
    fn fake_binary_script(version: &str, install_path: &Path) -> Vec<u8> {
        format!(
            "#!/bin/sh\nprintf 'pony {version} · {}-{}\\nbinary: {}\\n'\n",
            std::env::consts::OS,
            std::env::consts::ARCH,
            install_path.display()
        )
        .into_bytes()
    }

    // ── env 注入守卫：进程级环境变量必须串行化，用例结束逐项还原 ─────────────
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvGuard {
        _lock: MutexGuard<'static, ()>,
        saved: Vec<(&'static str, Option<String>)>,
    }

    impl EnvGuard {
        fn acquire(vars: &[(&'static str, String)]) -> Self {
            let lock = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
            let mut saved = Vec::new();
            for (key, value) in vars {
                saved.push((*key, std::env::var(key).ok()));
                std::env::set_var(key, value);
            }
            Self {
                _lock: lock,
                saved,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, value) in &self.saved {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    // ── 极简 HTTP mock（单线程 + 固定路由；不复用 runtime 的私有 MockHttpServer） ──
    enum Route {
        Json(String),
        Bytes(Vec<u8>),
        Redirect(String),
        NotFound,
        /// 声明 `Content-Length` 大于实发字节数（截断注入）。
        Truncated { declared: usize, actual: Vec<u8> },
        /// 先发出响应头，随后停滞 `stall_ms`（超时注入；停滞发生在被测算语里，不是测试盲等）。
        Stall { head: Vec<u8>, stall_ms: u64 },
    }

    struct MockServer {
        base: String,
        requests: Arc<Mutex<Vec<String>>>,
        shutdown: Arc<AtomicBool>,
        handle: Option<std::thread::JoinHandle<()>>,
    }

    impl MockServer {
        /// 先占端口再建路由：需要把 mock 自身 base URL 写进 `Location`（重定向用例）时必须两段式。
        fn bind() -> (TcpListener, String) {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
            let address = listener.local_addr().expect("mock server addr");
            (listener, format!("http://{address}"))
        }

        fn start(routes: Vec<(String, Route)>) -> Self {
            let (listener, base) = Self::bind();
            Self::serve(listener, base, routes)
        }

        fn serve(listener: TcpListener, base: String, routes: Vec<(String, Route)>) -> Self {
            listener.set_nonblocking(true).expect("nonblocking mock");
            let requests = Arc::new(Mutex::new(Vec::new()));
            let requests_for_thread = Arc::clone(&requests);
            let shutdown = Arc::new(AtomicBool::new(false));
            let shutdown_for_thread = Arc::clone(&shutdown);
            let handle = std::thread::spawn(move || {
                while !shutdown_for_thread.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => handle_mock_connection(stream, &routes, &requests_for_thread),
                        Err(ref error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            // 非阻塞 accept 轮询（2ms 粒度）以支持确定性 shutdown；不是测试盲等。
                            std::thread::sleep(Duration::from_millis(2));
                        }
                        Err(_) => break,
                    }
                }
            });
            Self {
                base,
                requests,
                shutdown,
                handle: Some(handle),
            }
        }

        fn path_of(&self, suffix: &str) -> String {
            format!("{}{}", self.base, suffix)
        }

        fn requests_snapshot(&self) -> Vec<String> {
            self.requests
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone()
        }
    }

    impl Drop for MockServer {
        fn drop(&mut self) {
            self.shutdown.store(true, Ordering::SeqCst);
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
    }

    fn handle_mock_connection(
        mut stream: TcpStream,
        routes: &[(String, Route)],
        requests: &Arc<Mutex<Vec<String>>>,
    ) {
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut buffer = [0u8; 4096];
        let Ok(read) = stream.read(&mut buffer) else {
            return;
        };
        if read == 0 {
            return;
        }
        let request = String::from_utf8_lossy(&buffer[..read]).to_string();
        let path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap_or("/")
            .to_string();
        requests
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(path.clone());
        // 最长后缀匹配：资产 URL 的路径段由实现决定，按资产名后缀命中，避免把 URL 构造方式写死进测试。
        let route = routes
            .iter()
            .filter(|(suffix, _)| path.ends_with(suffix.as_str()))
            .max_by_key(|(suffix, _)| suffix.len())
            .map(|(_, route)| route);
        match route {
            Some(Route::Json(body)) => write_mock_response(
                &mut stream,
                200,
                "application/json",
                body.as_bytes(),
            ),
            Some(Route::Bytes(body)) => write_mock_response(
                &mut stream,
                200,
                "application/octet-stream",
                body,
            ),
            Some(Route::Redirect(location)) => {
                let _ = write!(
                    stream,
                    "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
                let _ = stream.flush();
            }
            Some(Route::Truncated { declared, actual }) => {
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {declared}\r\nConnection: close\r\n\r\n"
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(actual);
                let _ = stream.flush();
                // 连接在此 drop：客户端读到的字节数少于声明的 Content-Length（截断）。
            }
            Some(Route::Stall { head, stall_ms }) => {
                let _ = stream.write_all(head);
                let _ = stream.flush();
                std::thread::sleep(Duration::from_millis(*stall_ms));
            }
            Some(Route::NotFound) | None => {
                let _ = write!(
                    stream,
                    "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
                let _ = stream.flush();
            }
        }
    }

    fn write_mock_response(stream: &mut TcpStream, status: u16, content_type: &str, body: &[u8]) {
        let head = format!(
            "HTTP/1.1 {status} OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(body);
        let _ = stream.flush();
    }

    /// 统一的逃生舱注入：只在使用 mock 的用例内生效，用例结束自动还原。
    fn update_env(
        mock: &MockServer,
        install_path: &Path,
        manifest_suffix: &str,
        budget_secs: u64,
    ) -> EnvGuard {
        EnvGuard::acquire(&[
            (
                "PONY_UPDATE_MANIFEST_URL",
                mock.path_of(manifest_suffix),
            ),
            ("PONY_UPDATE_ALLOW_INSECURE", "1".to_string()),
            (
                "PONY_UPDATE_INSTALL_PATH",
                install_path.display().to_string(),
            ),
            (
                "PONY_UPDATE_DOWNLOAD_BUDGET_SECS",
                budget_secs.to_string(),
            ),
        ])
    }

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    // ── C7-19：`pony update` 解析（默认值 + 各旗标） ────────────────────────

    #[test]
    fn parse_cli_action_update_defaults_to_check_install_mode() {
        match parse_cli_action(&args(&["update"])) {
            CliAction::Update {
                check,
                force,
                pin,
                json,
            } => {
                assert!(!check, "裸 `pony update` 必须是「检查并安装」而非 --check");
                assert!(!force, "默认不得允许降级/重装");
                assert!(pin.is_none(), "默认不得指定版本");
                assert!(!json, "默认输出人可读收据而非 JSON");
            }
            other => panic!("`pony update` 必须解析为 CliAction::Update，实际={other:?}"),
        }
    }

    #[test]
    fn parse_cli_action_update_recognises_each_flag() {
        // --check
        match parse_cli_action(&args(&["update", "--check"])) {
            CliAction::Update { check, force, pin, json } => {
                assert!(check, "--check 必须置位 check");
                assert!(!force && pin.is_none() && !json);
            }
            other => panic!("`update --check` 必须解析为 Update，实际={other:?}"),
        }

        // --json
        match parse_cli_action(&args(&["update", "--json"])) {
            CliAction::Update { check, json, .. } => {
                assert!(json, "--json 必须置位 json");
                assert!(!check);
            }
            other => panic!("`update --json` 必须解析为 Update，实际={other:?}"),
        }

        // --force --pin <ver>
        match parse_cli_action(&args(&["update", "--force", "--pin", "0.1.182"])) {
            CliAction::Update { force, pin, .. } => {
                assert!(force, "--force 必须置位 force");
                assert_eq!(
                    pin.as_deref(),
                    Some("0.1.182"),
                    "--pin 必须携带版本号原值"
                );
            }
            other => panic!("`update --force --pin` 必须解析为 Update，实际={other:?}"),
        }
    }

    // ── C7-20：非法用法必须 Invalid 且自带取值口径 ──────────────────────────

    #[test]
    fn parse_cli_action_update_rejects_missing_pin_value_and_unknown_flag() {
        // ① `--pin` 缺值
        match parse_cli_action(&args(&["update", "--pin"])) {
            CliAction::Invalid(message) => {
                assert!(
                    message.contains("--pin"),
                    "缺值错误必须点名 --pin 并给出用法，实际={message}"
                );
            }
            other => panic!("`update --pin`（缺值）必须解析为 Invalid，实际={other:?}"),
        }

        // ② 未知选项
        match parse_cli_action(&args(&["update", "--bogus"])) {
            CliAction::Invalid(message) => {
                assert!(
                    message.contains("--bogus"),
                    "未知选项错误必须点名用户输入，实际={message}"
                );
                assert!(
                    message.contains("--check") && message.contains("--pin"),
                    "未知选项错误必须自带本子命令的取值口径，实际={message}"
                );
            }
            other => panic!("`update --bogus` 必须解析为 Invalid，实际={other:?}"),
        }
    }

    // ── C7-8：正常路径（发现→下载→校验→原子安装→回验） ─────────────────────

    #[test]
    fn update_installs_verified_artifact_end_to_end() {
        // Arrange
        let dir = TempDir::new("e2e-ok");
        let install_path = dir.join("pony");
        let platform = test_platform();
        let asset_name = "pony-tui_0.1.184_test";
        let script = fake_binary_script("0.1.184", &install_path);
        let manifest = manifest_json(
            "0.1.184",
            platform,
            asset_name,
            &sha256_hex(&script),
            script.len() as u64,
        );
        let mock = MockServer::start(vec![
            (format!("/{asset_name}"), Route::Bytes(script.clone())),
            ("/manifest".to_string(), Route::Json(manifest)),
        ]);
        let _env = update_env(&mock, &install_path, "/manifest", 30);

        // Act
        let code = run_update_action(false, false, None, false);

        // Assert
        assert_eq!(code, 0, "正常路径必须成功退出（退出码 0）");
        assert_eq!(
            std::fs::read(&install_path).expect("安装目标必须存在"),
            script,
            "安装结果必须逐字节等于 mock 资产内容"
        );
        let mode = std::fs::metadata(&install_path)
            .expect("目标元数据必须可读")
            .permissions()
            .mode();
        assert_ne!(mode & 0o111, 0, "安装后必须带可执行位，实际 mode={mode:o}");
        let paths = mock.requests_snapshot();
        assert!(
            paths.iter().any(|path| path.ends_with("/manifest")),
            "必须请求清单，实际请求序列={paths:?}"
        );
        assert!(
            paths.iter().any(|path| path.ends_with(asset_name)),
            "必须下载清单声明的资产，实际请求序列={paths:?}"
        );
        let leftovers = dir.leftover_artifacts();
        assert!(
            leftovers.is_empty(),
            "成功路径必须清理临时文件与 lock，实际残留={leftovers:?}"
        );
    }

    // ── C7-9：sha256 不匹配 ⇒ 目标逐字节未变 + 无残留 ───────────────────────

    #[test]
    fn update_rejects_sha256_mismatch_without_touching_existing_binary() {
        // Arrange：既有二进制写哨兵内容；清单声明的 sha256 与实际资产不符。
        let dir = TempDir::new("e2e-sha-mismatch");
        let install_path = dir.join("pony");
        std::fs::write(&install_path, b"OLD-INSTALLED-BINARY").expect("必须能写入哨兵二进制");
        let platform = test_platform();
        let asset_name = "pony-tui_0.1.184_test";
        let script = fake_binary_script("0.1.184", &install_path);
        let wrong_sha = sha256_hex(b"a-different-payload");
        assert_ne!(wrong_sha, sha256_hex(&script), "夹具必须真的不一致");
        let manifest = manifest_json(
            "0.1.184",
            platform,
            asset_name,
            &wrong_sha,
            script.len() as u64,
        );
        let mock = MockServer::start(vec![
            (format!("/{asset_name}"), Route::Bytes(script)),
            ("/manifest".to_string(), Route::Json(manifest)),
        ]);
        let _env = update_env(&mock, &install_path, "/manifest", 30);

        // Act
        let code = run_update_action(false, false, None, false);

        // Assert
        assert_eq!(code, 1, "sha256 不符属校验失败 ⇒ 退出码 1（契约 C2）");
        assert_eq!(
            std::fs::read(&install_path).expect("目标必须仍可读"),
            b"OLD-INSTALLED-BINARY".to_vec(),
            "校验失败必须不触碰既有二进制（逐字节不变）"
        );
        let leftovers = dir.leftover_artifacts();
        assert!(
            leftovers.is_empty(),
            "校验失败路径必须清理临时文件，实际残留={leftovers:?}"
        );
        assert!(
            mock.requests_snapshot()
                .iter()
                .any(|path| path.ends_with(asset_name)),
            "夹具必须真的把资产下载下来（否则本用例退化为清单层失败）"
        );
    }

    // ── C7-10：404（清单 / 资产） ───────────────────────────────────────────

    #[test]
    fn update_reports_http_status_when_manifest_is_missing() {
        let dir = TempDir::new("e2e-manifest-404");
        let install_path = dir.join("pony");
        // 全部路径 404（mock 未注册任何路由）。
        let mock = MockServer::start(Vec::new());
        let _env = update_env(&mock, &install_path, "/manifest", 30);

        let code = run_update_action(false, false, None, false);

        assert_eq!(code, 3, "清单 404 属网络/HTTP 失败 ⇒ 退出码 3（契约 C2）");
        assert!(
            !install_path.exists(),
            "清单 404 不得写盘（不得创建安装目标）"
        );
        assert!(
            !mock.requests_snapshot().is_empty(),
            "必须真的发出过清单请求"
        );
    }

    #[test]
    fn update_reports_http_status_when_asset_is_missing() {
        let dir = TempDir::new("e2e-asset-404");
        let install_path = dir.join("pony");
        let platform = test_platform();
        let asset_name = "pony-tui_0.1.184_test";
        let script = fake_binary_script("0.1.184", &install_path);
        let manifest = manifest_json(
            "0.1.184",
            platform,
            asset_name,
            &sha256_hex(&script),
            script.len() as u64,
        );
        // 只注册清单：资产路径一律 404。
        let mock = MockServer::start(vec![(
            "/manifest".to_string(),
            Route::Json(manifest),
        )]);
        let _env = update_env(&mock, &install_path, "/manifest", 30);

        let code = run_update_action(false, false, None, false);

        assert_eq!(code, 3, "资产 404 属网络/HTTP 失败 ⇒ 退出码 3（契约 C2）");
        assert!(!install_path.exists(), "资产 404 不得写盘");
        let paths = mock.requests_snapshot();
        assert!(
            paths.iter().any(|path| path.ends_with(asset_name)),
            "必须真的尝试下载资产，实际请求序列={paths:?}"
        );
    }

    // ── C7-11：畸形 JSON ────────────────────────────────────────────────────

    #[test]
    fn update_rejects_malformed_manifest_json_without_writing() {
        let dir = TempDir::new("e2e-malformed");
        let install_path = dir.join("pony");
        let mock = MockServer::start(vec![(
            "/manifest".to_string(),
            Route::Json("{\"platforms\":".to_string()),
        )]);
        let _env = update_env(&mock, &install_path, "/manifest", 30);

        let code = run_update_action(false, false, None, false);

        assert_eq!(code, 3, "畸形清单属清单失败 ⇒ 退出码 3（契约 C2）");
        assert!(!install_path.exists(), "畸形清单不得写盘");
        assert!(
            dir.leftover_artifacts().is_empty(),
            "畸形清单不得留下任何临时文件"
        );
    }

    // ── C7-12：平台缺失 ─────────────────────────────────────────────────────

    #[test]
    fn update_reports_unsupported_platform_for_missing_entry() {
        let dir = TempDir::new("e2e-platform-missing");
        let install_path = dir.join("pony");
        // 清单只声明一个「不是本机」的平台。
        let other_platform = if test_platform() == "windows-x86_64" {
            "linux-x86_64"
        } else {
            "windows-x86_64"
        };
        let script = fake_binary_script("0.1.184", &install_path);
        let manifest = manifest_json(
            "0.1.184",
            other_platform,
            "pony-tui_other",
            &sha256_hex(&script),
            script.len() as u64,
        );
        let mock = MockServer::start(vec![
            ("/pony-tui_other".to_string(), Route::Bytes(script)),
            ("/manifest".to_string(), Route::Json(manifest)),
        ]);
        let _env = update_env(&mock, &install_path, "/manifest", 30);

        let code = run_update_action(false, false, None, false);

        assert_eq!(code, 3, "平台缺失属清单失败 ⇒ 退出码 3（契约 C2）");
        assert!(!install_path.exists(), "平台缺失不得写盘");
        let paths = mock.requests_snapshot();
        assert!(
            !paths.iter().any(|path| path.ends_with("/pony-tui_other")),
            "平台缺失时不得下载任何资产，实际请求序列={paths:?}"
        );
    }

    // ── C7-13：下载被截断（声明长度 > 实发） ─────────────────────────────────

    #[test]
    fn update_fails_on_truncated_download_without_writing() {
        let dir = TempDir::new("e2e-truncated");
        let install_path = dir.join("pony");
        let platform = test_platform();
        let asset_name = "pony-tui_0.1.184_test";
        let full_payload = vec![7u8; 1000];
        let manifest = manifest_json(
            "0.1.184",
            platform,
            asset_name,
            &sha256_hex(&full_payload),
            full_payload.len() as u64,
        );
        let mock = MockServer::start(vec![
            (
                format!("/{asset_name}"),
                Route::Truncated {
                    declared: 1000,
                    actual: full_payload[..500].to_vec(),
                },
            ),
            ("/manifest".to_string(), Route::Json(manifest)),
        ]);
        let _env = update_env(&mock, &install_path, "/manifest", 30);

        let code = run_update_action(false, false, None, false);

        assert_ne!(code, 0, "截断下载必须失败（不得以 0 收尾）");
        assert!(
            !install_path.exists(),
            "截断下载不得写盘（不得留下半个可执行文件）"
        );
        assert!(
            dir.leftover_artifacts().is_empty(),
            "截断下载不得残留临时文件，实际残留={:?}",
            dir.leftover_artifacts()
        );
    }

    // ── C7-14：lock 已存在 ⇒ NotWritable 且不删他人 lock ─────────────────────

    #[test]
    fn update_refuses_concurrent_instance_and_preserves_foreign_lock() {
        let dir = TempDir::new("e2e-lock");
        let install_path = dir.join("pony");
        let foreign_lock = dir.join("pony.update.lock");
        std::fs::write(&foreign_lock, b"another-pony-update-running")
            .expect("必须能写入他人 lock 文件");
        let platform = test_platform();
        let asset_name = "pony-tui_0.1.184_test";
        let script = fake_binary_script("0.1.184", &install_path);
        let manifest = manifest_json(
            "0.1.184",
            platform,
            asset_name,
            &sha256_hex(&script),
            script.len() as u64,
        );
        let mock = MockServer::start(vec![
            (format!("/{asset_name}"), Route::Bytes(script)),
            ("/manifest".to_string(), Route::Json(manifest)),
        ]);
        let _env = update_env(&mock, &install_path, "/manifest", 30);

        let code = run_update_action(false, false, None, false);

        assert_eq!(code, 4, "并发实例冲突属安装失败 ⇒ 退出码 4（契约 C2）");
        assert!(
            foreign_lock.exists(),
            "不得删除他人的 lock 文件（否则两个实例会同时进入安装段）"
        );
        assert_eq!(
            std::fs::read(&foreign_lock).expect("他人 lock 必须仍可读"),
            b"another-pony-update-running".to_vec(),
            "他人 lock 内容不得被覆盖"
        );
        assert!(!install_path.exists(), "锁冲突不得写盘");
    }

    // ── C7-15：只读目标目录 ⇒ NotWritable 且无新增文件 ───────────────────────

    #[test]
    fn update_reports_not_writable_for_read_only_install_directory() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new("e2e-readonly");
        let readonly_dir = dir.join("readonly-install");
        std::fs::create_dir_all(&readonly_dir).expect("必须能创建只读目录");
        let install_path = readonly_dir.join("pony");
        let platform = test_platform();
        let asset_name = "pony-tui_0.1.184_test";
        let script = fake_binary_script("0.1.184", &install_path);
        let manifest = manifest_json(
            "0.1.184",
            platform,
            asset_name,
            &sha256_hex(&script),
            script.len() as u64,
        );
        let mock = MockServer::start(vec![
            (format!("/{asset_name}"), Route::Bytes(script)),
            ("/manifest".to_string(), Route::Json(manifest)),
        ]);
        let _env = update_env(&mock, &install_path, "/manifest", 30);
        std::fs::set_permissions(&readonly_dir, std::fs::Permissions::from_mode(0o555))
            .expect("必须能把安装目录设为只读");

        // 收尾保证：无论断言是否失败都恢复 0o755（否则污染临时目录清理）。
        struct RestorePerms(PathBuf);
        impl Drop for RestorePerms {
            fn drop(&mut self) {
                let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
            }
        }
        let _restore = RestorePerms(readonly_dir.clone());

        let code = run_update_action(false, false, None, false);

        assert_eq!(code, 4, "不可写安装目录属安装失败 ⇒ 退出码 4（契约 C2）");
        let entries: Vec<String> = std::fs::read_dir(&readonly_dir)
            .expect("必须能列只读目录")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .collect();
        assert!(
            entries.is_empty(),
            "只读目录内不得产生任何新增文件，实际={entries:?}"
        );
    }

    // ── C7-16：超时注入（预算内失败，不 hang） ───────────────────────────────

    #[test]
    fn update_aborts_stalled_download_within_budget() {
        let dir = TempDir::new("e2e-stall");
        let install_path = dir.join("pony");
        let platform = test_platform();
        let asset_name = "pony-tui_0.1.184_test";
        let script = fake_binary_script("0.1.184", &install_path);
        let manifest = manifest_json(
            "0.1.184",
            platform,
            asset_name,
            &sha256_hex(&script),
            script.len() as u64,
        );
        // 停滞 3000ms：若预算未生效，客户端会一直等到服务端关连接（≈3s），断言即红。
        let stall_head = "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: 4096\r\nConnection: close\r\n\r\n".as_bytes().to_vec();
        let mock = MockServer::start(vec![
            (
                format!("/{asset_name}"),
                Route::Stall {
                    head: stall_head,
                    stall_ms: 3000,
                },
            ),
            ("/manifest".to_string(), Route::Json(manifest)),
        ]);
        let _env = update_env(&mock, &install_path, "/manifest", 1);

        // Act
        let started = Instant::now();
        let code = run_update_action(false, false, None, false);
        let elapsed = started.elapsed();

        // Assert
        assert_ne!(code, 0, "停滞下载必须在预算内失败（不得以 0 收尾）");
        assert!(
            elapsed < Duration::from_millis(2000),
            "必须按 1s 下载预算中止而不是等对端关连接（实测 {elapsed:?}）"
        );
        assert!(elapsed < Duration::from_secs(10), "整体不得 hang（实测 {elapsed:?}）");
        assert!(!install_path.exists(), "停滞下载不得写盘");
    }

    // ── C7-17：体积上限（声明超限） ─────────────────────────────────────────

    #[test]
    fn update_rejects_oversized_artifact_without_writing() {
        let dir = TempDir::new("e2e-too-large");
        let install_path = dir.join("pony");
        let platform = test_platform();
        let asset_name = "pony-tui_0.1.184_test";
        let script = fake_binary_script("0.1.184", &install_path);
        // 清单声明 size = 128MiB + 1（无需真的造 128MB 流量）。
        let oversized = pony_agent_core::agent::updater::MAX_ARTIFACT_BYTES + 1;
        let manifest = manifest_json(
            "0.1.184",
            platform,
            asset_name,
            &sha256_hex(&script),
            oversized,
        );
        let mock = MockServer::start(vec![
            (format!("/{asset_name}"), Route::Bytes(script)),
            ("/manifest".to_string(), Route::Json(manifest)),
        ]);
        let _env = update_env(&mock, &install_path, "/manifest", 30);

        // Act（① CLI 面）：用 `--json` 跑整条命令，锁退出码与「绝不落盘」。
        let code = run_update_action(false, false, None, true);

        // Act（② 错误码真值面）：直接调 CLI 内部同一个入口 `run_self_update`，拿到 `UpdateError`
        // 本体 —— `run_update_action` 的 JSON 收据就是 `error.code()` 的序列化结果。
        //
        // 为什么不在进程内解析 stdout：libtest 用 thread-local 捕获「测试线程」的 stdout，
        // `println!` 被吞进用例缓冲、根本不落 fd 1（本轮实测：dup2 重定向 + 子线程 println 后
        // 捕获文件仍为空串），因此**在门禁命令下无法从 stdout 取 JSON**；改用与 CLI 完全同源的
        // `run_self_update` + `UpdateError::code()`（契约 C1 的真值方法），断言面等价且不依赖
        // `--nocapture`。
        let api_error = pony_agent_core::agent::updater::run_self_update(
            &pony_agent_core::agent::updater::UpdateOptions {
                check: false,
                force: false,
                pin: None,
            },
        )
        .expect_err("超限制品必须被拒绝（run_self_update 不得返回 Ok）");

        // Assert
        assert_ne!(code, 0, "超限制品必须失败（不得以 0 收尾）");
        assert!(
            matches!(
                api_error,
                pony_agent_core::agent::updater::UpdateError::TooLarge(_)
            ),
            "超限制品必须映射为 UpdateError::TooLarge，实际={api_error:?}"
        );
        // 错误码级断言（本用例的区分力来源）：`TooLarge ⇒ code() == "too_large"`（契约 C1，
        // 实测 JSON 收据的 error.code 逐字为 "too_large"）。
        // 该断言用于**杀死「删除 updater.rs 两处 MAX_ARTIFACT_BYTES 判断」这一变异**：
        // 收紧前只锁「非零退出 + 不落盘 + 无残留」时，L3-B 变异实测证明删除判断后本用例仍绿
        // （超限被后续 size/sha 校验路径兜住）⇒ 对「超限」分支零区分力；变异后错误会落到
        // size/sha 校验分支并给出别的 error.code（或直接 Ok），两条断言立即变红。
        assert_eq!(
            api_error.code(),
            "too_large",
            "超限制品必须给出 too_large 错误码（契约 C1：UpdateError::TooLarge ⇒ code()），\
             本断言用于杀死『删除 MAX_ARTIFACT_BYTES 判断』这一变异"
        );
        assert!(
            api_error.exit_code() > 0,
            "TooLarge 必须映射为非零退出码（契约 C2），实际={}",
            api_error.exit_code()
        );
        assert!(
            !install_path.exists(),
            "超限制品不得写盘（必须在写入前按声明 size 拒绝）"
        );
        assert!(
            dir.leftover_artifacts().is_empty(),
            "超限路径不得残留临时文件，实际残留={:?}",
            dir.leftover_artifacts()
        );
    }

    // ── C7-18：重定向（同 host 成功 / 白名单外 host 拒绝） ───────────────────

    #[test]
    fn update_follows_same_host_redirect_when_signed_in_escape_hatch() {
        // Arrange：两段式建路由——先占端口拿到 mock base，才能把地址写进 302 的 Location。
        let dir = TempDir::new("e2e-redirect-ok");
        let install_path = dir.join("pony");
        let platform = test_platform();
        let asset_name = "pony-tui_0.1.184_test";
        let script = fake_binary_script("0.1.184", &install_path);
        let manifest = manifest_json(
            "0.1.184",
            platform,
            asset_name,
            &sha256_hex(&script),
            script.len() as u64,
        );
        let (listener, base) = MockServer::bind();
        let mock = MockServer::serve(
            listener,
            base.clone(),
            vec![
                // 资产请求 ⇒ 302 到同 host 的 /asset-final ⇒ 200 真身。
                (
                    format!("/{asset_name}"),
                    Route::Redirect(format!("{base}/asset-final")),
                ),
                ("/asset-final".to_string(), Route::Bytes(script.clone())),
                ("/manifest".to_string(), Route::Json(manifest)),
            ],
        );
        let _env = update_env(&mock, &install_path, "/manifest", 30);

        // Act
        let code = run_update_action(false, false, None, false);

        // Assert
        assert_eq!(code, 0, "同 host 302 必须被手动跟随并安装成功");
        assert_eq!(
            std::fs::read(&install_path).expect("安装目标必须存在"),
            script,
            "重定向后拿到的资产必须被正确安装"
        );
        let paths = mock.requests_snapshot();
        assert!(
            paths.iter().any(|path| path.ends_with("/asset-final")),
            "必须真的跟随了 302 到 /asset-final，实际请求序列={paths:?}"
        );
        assert!(
            paths.iter().any(|path| path.ends_with(asset_name)),
            "必须先请求清单声明的资产路径，实际请求序列={paths:?}"
        );
    }

    #[test]
    fn update_rejects_redirect_to_host_outside_allow_list() {
        let dir = TempDir::new("e2e-redirect-offwhitelist");
        let install_path = dir.join("pony");
        let platform = test_platform();
        let asset_name = "pony-tui_0.1.184_test";
        let script = fake_binary_script("0.1.184", &install_path);
        let manifest = manifest_json(
            "0.1.184",
            platform,
            asset_name,
            &sha256_hex(&script),
            script.len() as u64,
        );
        // 302 指向白名单外的 host（127.0.0.2 与 mock 的 127.0.0.1 不同）。
        let mock = MockServer::start(vec![
            (
                format!("/{asset_name}"),
                Route::Redirect("http://127.0.0.2:1/steal".to_string()),
            ),
            ("/manifest".to_string(), Route::Json(manifest)),
        ]);
        let _env = update_env(&mock, &install_path, "/manifest", 30);

        let code = run_update_action(false, false, None, false);

        assert_eq!(code, 3, "白名单外重定向属网络失败 ⇒ 退出码 3（契约 C4/C2）");
        assert!(
            !install_path.exists(),
            "白名单外重定向不得写盘（必须逐跳校验 host 后才跟随）"
        );
        assert!(
            !mock
                .requests_snapshot()
                .iter()
                .any(|path| path.ends_with("/steal")),
            "不得真的去请求白名单外 host"
        );
    }
