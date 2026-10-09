# Release Notes 生成器 验收契约矩阵（Protocol Agent 冻结，Test Agent / Executor 对齐基准）

> 需求：GitHub Release 页面展示本次发布的变更信息。
> 现状：`.github/workflows/release.yml` 每次打 `vX.Y.Z` tag 发布时，release notes 硬编码为
> `--notes "Automated signed release (Windows desktop)"`（当前第 111 行）；`latest.json` 的 `notes`
> 字段硬编码为空串（当前第 145 行 `notes = ""`），导致 Release 页面无本次更新信息。
> 修复方向：新增跨平台 Node 脚本 `scripts/release-notes.mjs`，从 git 历史（conventional commits）
> 生成分组 markdown；`release.yml` 改用 `--notes-file` 传参，并把同一份 notes 同步填入 `latest.json` 的
> `notes` 字段。workflow 运行于 `windows-latest` + pwsh，Node 22 可用。

## 0. 已冻结的边界事实（All Frozen Ground Truth）

- 生成器**只读本地 git 仓库**（`git log` / `git tag` / 读取 `package.json`），**禁止任何网络调用**
  （不调 GitHub API、不调 `gh`、不访问 npm registry）。
- 空桩阶段验收契约见第 8 节（含物理形态与机器校验证据）；正式实现阶段由 Executor 以常规 ESM 实现
  整体替换空桩（导出签名与行为以本文为准，空桩形态与正式实现无关）。
- **非目标（Non-Goals）**：不支持 breaking change 后缀（`feat!:` / `fix!:` 形式不识别，落入
  maintenance 组）；不做 GitHub Release 页面渲染定制；不生成 CHANGELOG 文件；不改 Tauri 更新器逻辑。

---

## 1. 模块导出签名与类型（Module Surface Contract）

文件：`scripts/release-notes.mjs`（ESM，`"type": "module"`）。导出 4 个命名函数，全部为
**纯函数：无副作用、确定性、零网络调用**（`main` 除外——它执行 CLI 侧 I/O，见第 4 节）。

| 导出 | 签名 | 返回 | 说明 |
|---|---|---|---|
| `parseCommit` | `(subject: string) => { type: string \| null, scope: string \| null, description: string }` | 结构化提交对象 | 解析单条 conventional commit subject |
| `groupCommits` | `(subjects: CommitSubject[]) => GroupedCommits` | 五组有序对象 | 按分组规则分组，排除规则先行 |
| `buildReleaseNotes` | `({ subjects, from, to, repoName }: ReleaseNotesInput) => string` | markdown 全文 | 不写文件、不打日志，纯字符串返回 |
| `main` | `(argv: string[]) => number` | 0=成功 | CLI 入口；失败抛异常（stderr 可诊断 + 非 0 退出） |

类型定义（冻结，`//` 为文档注释，运行时以形状校验为准）：

```ts
type CommitSubject = { subject: string; sha: string }; // sha 为 git %h 缩写（原样，不截断）
type ReleaseNotesInput = { subjects: CommitSubject[]; from: string | null; to: string; repoName: string }; // from=null 表示全量历史；repoName 由调用方显式入参（CLI 读取 package.json.name），buildReleaseNotes 保持纯函数
type GroupKey = "features" | "bugFixes" | "performance" | "docs" | "maintenance";
type GroupedCommits = Record<GroupKey, CommitSubject[]>; // 键序固定：features→bugFixes→performance→docs→maintenance
```

**parseCommit 解析规则（冻结）**：

- 匹配 `type(scope): description`（type 大小写不敏感识别，**识别后归一化为小写**：
  `FEAT(x): y` → `type: "feat"`），归一化后 type 限定 `[a-z]+`，scope 可为空：
  `feat: x` → `{ type: "feat", scope: null, description: "x" }`；
  `fix(ui): handle null` → `{ type: "fix", scope: "ui", description: "handle null" }`。
- 不匹配（无冒号、无 description、type 含非法字符、breaking `!` 后缀等）→
  `{ type: null, scope: null, description: <subject 原样 trim 后> }`。
- description 两侧空白 trim；subject 为空串 → 视为不匹配。

**groupCommits 规则（冻结）**：

1. **排除先行**：跳过 subject 匹配 `/^chore\(release\):/` 的条目（bump 提交，如
   `chore(release): bump version to 0.1.114`）——不进入任何组、不出现在输出。
2. 其余条目按 `parseCommit(subject).type` 落组，未匹配（type=null）与
   `chore/test/refactor/build/ci/` 之外的任何 type → maintenance。
3. 组内**严格保留输入顺序**；分组**不改变条目内部字段**。
4. 返回对象仅含五组键（空组以空数组存在），键序固定 `features→bugFixes→performance→docs→maintenance`。

---

## 2. 分组规则映射（Grouping Rule, Frozen）

| type | 组键 | 输出小节标题（冻结，含前缀空格） |
|---|---|---|
| `feat` | features | `## ✨ Features/新功能` |
| `fix` | bugFixes | `## 🐛 Bug Fixes/修复` |
| `perf` | performance | `## ⚡ Performance/性能` |
| `docs` | docs | `## 📝 Docs/文档` |
| 其余全部（chore/test/refactor/build/ci/其他/无前缀/`feat!:` 等） | maintenance | `## 🧹 Maintenance/维护` |

- 小节输出顺序固定：features → bugFixes → performance → docs → maintenance。
- 空组**不输出**小节头（避免空标题）；全部组为空时输出整体占位说明（见第 5 节）。

---

## 3. 输出格式（Markdown, Frozen）

`buildReleaseNotes` 返回字符串，**确定性、字节级可复现**（同输入必同输出）：

1. 标题（首行）：区间模式 `# {repoName} release notes ({from} → {to})`；
   全量回退模式（第 4 节无前一 tag 场景，`from=null`）`# {repoName} release notes ({to} / 全量历史)`。
   `{repoName}` = `--repo-root/package.json` 的 `name` 字段（读取失败视为致命错误）；
   `{from}`/`{to}` 为 tag 字符串原样（如 `v0.1.113` / `v0.1.114`）。
2. 非空分组：小节头（第 2 节表格）+ 条目，条目格式冻结：`- {escapedSubject} ([{sha}])`，
   `{sha}` 为 `CommitSubject.sha` 原样（git `%h` 缩写）。
3. 全部为空（含排除后无任何条目）：标题下直接输出占位行，文案**精确**为
   `本版本无用户可见变更`（不再输出任何小节头）。
4. **subject markdown 转义（冻结执行顺序，逐字符映射）**：
   ① `\` → `\\`；② `` ` `` → `` \` ``；③ `#` → `\#`；④ `\n` / `\r`（及 CRLF）→ 单个空格。
5. 输出以恰好一个换行符 `\n` 结尾；全程 UTF-8；输出中除第 4 条外不出现未被转义的换行。

---

## 4. CLI 契约（main, Frozen）

用法（冻结）：

```
node scripts/release-notes.mjs --repo-root <dir> --to <tag> [--from <tag>] [--out <file>]
```

- `--repo-root <dir>`：必填；git 仓库根目录（既有测试/工作流均从仓库根调用）。
- `--to <tag>`：必填；目标 tag（含 `v` 前缀，如 `v0.1.114`）。
- `--from <tag>`：可选；起始 tag（范围 `from..to`，from 端不含）。传入则直接采用，不做探测。
- `--out <file>`：可选；输出文件路径。缺省向 stdout 打印 markdown；
  提供时写入文件（UTF-8），写失败（路径不存在/无权限）→ 抛异常 → 非 0 退出 + stderr 可诊断。
- **无 `--from` 时自动探测前一 tag（冻结算法）**：
  1. `git tag -l`（cwd=`--repo-root`，60s 超时）取全部 tag；
  2. 过滤形如 `^v\d+\.\d+\.\d+$` 的 tag，按 semver 升序排序（非法版本跳过）；
  3. 取**严格小于** `--to` 的最大者作为 from；不存在 → `from = null`（全量历史回退，输出注明，见第 3 节）。
- **git 子命令契约（completed 实现冻结）**，一律 `execFileSync("git", args, { cwd, encoding: "utf8", timeout: 60_000, stdio: ["ignore","pipe","pipe"] })`：
  - 区间：`git log --format=%h%x00%s <from>..<to>`
  - 全量：`git log --format=%h%x00%s <to>`（自 `--to` 起全量历史，即"回退为 --to 起全量历史"）
  - 探测：`git tag -l`、`git rev-parse --verify <tag>`（tag 存在性校验，`--to` 必须可解析）
- **失败契约**：任何 git exec 失败 / 参数缺失 / 读取失败 → `main` 抛 `Error`（消息含可诊断上下文：
  失败命令、repo-root、tag 或 git stderr 尾部），未捕获 → Node 默认 stderr 输出 + **exit code 非 0**。
  成功 → 返回 `0`。
- **退出码契约**：成功 `0`；一切失败 `非 0`（Node 未捕获异常默认 `1`），stderr 必须可诊断。

---

## 5. NFR（CLI 特定，明确不写入 `.dev-team/nfr-baseline.json`）

> 以下为本生成器专用 NFR，与既有应用级基线文件正交；**禁止**改动/追加 `nfr-baseline.json`。

| NFR | 冻结值 |
|---|---|
| git exec 超时 | 60_000 ms（超时 → 失败非 0 + stderr 诊断） |
| 零网络 | 不发起任何 HTTP/网络请求；仅本地 git + fs + stdout |
| 确定性输出 | 同一仓库状态 + 同一参数 → 输出字节级一致（无时间戳/无随机/无并发依赖） |
| 编码 | 全程 UTF-8（读入/写出/子进程输出） |
| exit code 契约 | 成功 0；失败非 0 且 stderr 可诊断 |
| 并发 | 一次性 CI CLI，无共享可变状态、无需加锁 |

---

## 6. latest.json 接线契约（workflow, Frozen）

`release.yml`（`build-and-release` job，发布步骤）必须按下述接线，删除硬编码：

1. 生成步骤（`pwsh` 或 node 直调）：
   `node scripts/release-notes.mjs --repo-root <checkout根> --to $tag --out <notesFile>`，
   `<notesFile>` 建议位于产物目录（如 `target/release/bundle/nsis/release-notes.md`）；
   生成失败即整步 fail（失败即 fail，不得发布空 notes）。
2. `gh release create` 改用 notes 文件传参，删除第 111 行硬编码
   `--notes "Automated signed release (Windows desktop)"`：
   `gh release create "$tag" --draft --title "$tag" --notes-file <notesFile>`。
3. `latest.json` 的 `notes` 字段（当前第 145 行 `notes = ""`）必须从 notes 文件全文读取：
   `$manifest.notes = (Get-Content <notesFile> -Raw)`；**JSON 转义安全**由 `ConvertTo-Json`
   保证，禁止手工字符串拼接/内插 notes 文本。
4. `latest.json` 与 Release 页 notes 为**同一真源**（同一文件），不得出现两处不一致的文本。
5. 回归校验基准：Release 页展示本次变更分组；`latest.json` 内 `notes` 为非空字符串且与文件内容一致。

---

## 7. 机器校验证据（双方门禁, Frozen）

- 空桩语法门禁：`node --check scripts/release-notes.mjs` → Exit 0。
- 空桩纯度门禁：`python3 /tmp/dev-team-sandbox/scripts/dev-stub-lint.py scripts/release-notes.mjs`
  → Exit 0（本轮实测：Exit 0，输出 `STUB LINT PASSED`）。
- 正式实现门禁：`node --check` Exit 0；Test Agent 红相/验收测试全绿（`npm test`）。

---

## 8. 空桩物理形态与红相接缝（Protocol Frozen）

**为什么是这种形态**：`dev-stub-lint.py` 是 Python AST 白名单校验器，对 `scripts/release-notes.mjs`
运行要求该文件**本身同时是合法 Python 源**；而 ESM 所需的 `export`/`function`/`throw` 等关键字
在 token 级无法构成合法 Python 语句（已实测验证：普通 ESM 空桩 lint 必然 SyntaxError）。
因此空桩以 **JS/Python 双语法（polyglot）表达式**落盘：module 级仅 4 条
`globalThis.<name> = Function(<参数>, '<body>')` 常量赋值，函数体以字符串字面量
`throw new Error("NotImplemented");` 存在。**该形态仅为空桩冻结，正式实现由 Executor 替换为
常规 ESM 导出（`export function parseCommit...` 等），不受本形态约束。**

- 空桩运行时行为：模块可被副作用导入（`await import(...)` 成功，无导出报错）；
  `globalThis.parseCommit / groupCommits / buildReleaseNotes / main` 为真实可调用函数，
  **任何调用即抛 `Error`，message 精确等于 `"NotImplemented"`**（不校验参数形状）。
- **红相测试接缝（Test Agent 使用）**：
  1. 单元层：`await import("../scripts/release-notes.mjs")` 后从 `globalThis` 取函数，
     断言 `toThrow("NotImplemented")`；
  2. CLI 子进程黑盒（`node scripts/release-notes.mjs ...`）**不在红相阶段针对空桩运行**
     （空桩无 CLI 分发，直接执行会静默 exit 0），该黑盒由绿相对 Executor 正式实现验证。
- 空桩文件内**不存在任何注释**（JS 与 Python 注释语法不相交，polyglot 无法承载注释），非缺陷。

---

## 9. 角色与红线（Role & Red Lines）

- **Protocol Agent（唯一编写方）**：冻结本契约矩阵、空桩、`docs/decisions/0025-…md`；
  不改 `nfr-baseline.json`（只读）；不改测试；不做 git 写操作。
- **Test Agent**：仅依据本矩阵编写红相（L2-T/L2-AT）与验收测试（红相文件
  `tests/acceptance/stage-*-release-notes.spec.ts` 一类）；对业务代码只读。
- **Executor**：仅实现业务代码——`scripts/release-notes.mjs` 正式 ESM 实现（导出 4 函数 +
  CLI 分发 + git 调用 + 分组/转义/格式化）与 `release.yml` 接线（第 6 节）；**严禁修改任何测试文件**；
  空桩文件整体替换，不残留 globalThis 桩。
- **门禁顺序**：红相锚定 commit → Executor 实现 → `npm test` 全绿 + `node --check` Exit 0 →
  Lead 逐项核对契约（第 1–6 节）→ 审查（≥2 路）→ 阶段提交 + `.dev-team/state.json` 更新。

---

## 10. 验收标准汇总（Acceptance Summary）

1. `parseCommit`/`groupCommits`/`buildReleaseNotes`/`main` 导出与行为符合第 1、2 节；
2. CLI 探测/回退/失败契约符合第 4 节（含 60s 超时、exit code 0/非 0、stderr 可诊断）；
3. 输出格式符合第 3 节（标题/小节/条目/转义/占位/换行，确定性）；
4. NFR（第 5 节）全部满足，`nfr-baseline.json` 未被修改；
5. `release.yml` 按第 6 节接线：`--notes-file` 生效、`latest.json.notes` 非空且同源、
   失败即 fail；无新增 secret。