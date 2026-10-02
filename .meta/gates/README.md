# gates/ —— 工程门禁集合

本项目将关键验证逻辑收敛为非零退出（non-zero exit）命令，与 `.githooks` 及 GitHub Actions CI 协同运作。

## 门禁索引

1. **版本一致性门禁**：`scripts/check-version-sync.ps1`
   - 验证 `package.json`、`src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json`、`.version.json` 严格一致。
2. **提交卫生门禁 (Git Guard)**：`scripts/git-guard.ps1`
   - 拦截构建物（target, dist, coverage, tmp 等）、二进制文件（.rlib, .pdb 等）及超大文件提交。
3. **决策格式门禁**：`scripts/verify-decisions.ps1`
   - 验证 ADR 的 Status 行合法性与目录归属。
