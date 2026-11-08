# Agent Note: 会话目录包导出/导入（ADR 0099 阶段 3-A）

Status: implemented

## Problem

阶段 1–2 之后，会话的真源已经是一份**每会话一个 append-only 日志文件**。但用户拿不走它：

- 换机、重装、发给同事审计——没有导出入口；
- 未来远程同步需要一个明确的「可传输单元」与线格式，而现在只有内部文件布局；
- 直接复制目录虽可行，但**没有完整性校验**，也没有版本声明，坏包无法识别。

阶段 3 的第一块因此不是「先起服务端」，而是**把传输单元定义清楚**——它能立刻兑现用户价值，
也是将来 HTTP 端点唯一要复用的格式。

## Decision

**包格式（唯一真源形态，不另造）**：

```
<包根>/
  manifest.json      # 不参与自校验（否则自引用哈希无解）
  events.jsonl       # header 行 + {"seq","ts","cmd"} 行，与 session.jsonl 同格式
  attachments/       # 附件实体（可降级）
```

`manifest.json` 必填：`format`(`pony.session.bundle`) / `version`(1) / `sessionId` / `cwd` /
`createdAtMs` / `eventCount` / `lastSeq` / `files[]`(`path`+`sha256`+`bytes`) / `source`
(`kind`+`appVersion`+`exportedAtMs`)。`files[].path` 为**包根相对的 POSIX 路径**，
禁绝对路径与 `..` 段。

**单文件形态 = 手写最小 ustar `.tar`（不压缩）**：512 字节头 + 内容 512 对齐 + 两个全零块收尾；
>100 字节路径由写入侧切进 ustar `prefix` 字段（自家写入 ⇒ 自家读取必有解）；
读侧校验每个头的 chksum，遇 GNU 长名 `L` **明确报错**而不猜。
CLI `--format dir|tar`，默认 `dir`；`--out -` 明确拒绝（stdout 字节流留待后续）。

**退出码契约**（让脚本能区分三态，符合「不得静默」）：

| code | 含义 |
|---|---|
| 0 | 全成功（含附件全部校验通过） |
| 2 | 硬失败（会话不存在 / manifest 非法 / sha256 或 bytes 不符 / header 或 seq 不符 / 路径越界）——**零落盘** |
| 3 | 目标已存在且未加 `--force` |
| 4 | **事件导入成功、但附件降级**（缺失或哈希不符），报告逐条列出文件与原因 |
| 1 | IO / tar 结构失败 |

**严格度分层**：`events.jsonl` 严格（真源）；`attachments/*` 可降级（缺失或不符只警告 + EXIT=4）。

**`manifest.json` 无自校验 ⇒ 结构性兜底**：逐字段校验「存在 + 类型 + 语义非空」，
sha256 必须 64 位十六进制；再由 `events.jsonl` header（`sessionId`/`cwd`/`createdAtMs` 逐一比对）
+ `eventCount` / `lastSeq` / seq 连续性三重兜底。

**导入只落日志目录与附件，不写 SQLite 索引**——索引由既有 `ponycli sessions rebuild-index` 补。
`--force` 覆盖前把旧目录改名 `<id>.bak-<ts>`，不直接删。

## Verification

- `[1] dir bundle：events=13 lastSeq=Some(12) → 往返 4 条消息 / 2 节点 / 1 trace 完全一致`
- `[2] tar bundle：8704B / events=13 → 往返逐字段一致（history=4 nodes=2 traces=1）`
- `[3] 篡改一个字节 → EXIT=2「events.jsonl sha256 不符（manifest ddd043… / 实际 e1f2cf…）」，目标根零落盘`
- `[4] 截断事件流：sha256 拦截；改对哈希后由结构化校验拦截「eventCount=13 lastSeq=Some(12)，实际 12/11」`
- `[5] version=99 → EXIT=2「包格式版本 99 不受支持」`
- `[6] 重复导入 → EXIT=3；--force 后旧目录 `…pa_3a_bundle.bak-1794142309392``
- `[7] dry-run：日志根 0 个文件前后指纹一致`
- `[8] 导入后 sessions 行=0；rebuild-index 后 `rebuilt=["pa_3a_bundle"]` 且内容一致`
- `[9] CLI：export EXIT=0 / import EXIT=4（附件降级逐条报告），事件仍完整往返`
- 套件：`session_bundle_export_import` 9 passed；`session_read_from_log` 8 / `session_fold_rebuild` 8 /
  `session_log_wiring` 7 / `session_log_store` 6 / `session_stop_blob_write` 7；`--lib` 1393；
  `cargo check` 0 warning；`check-declaration.sh` PASS

## 两个已知限制（记录在案，未隐藏）

1. **压缩未做**：`.tar.zst` 需要新增依赖（tar/zstd/flate2 均不在 `Cargo.toml`），按「不得新增第三方依赖」
   的约束降级为不压缩 ustar。留作独立 ADR 事项。
2. **附件来源不完整**：导出收集两处——`<会话目录>/attachments/**`（规范位置，当前写路径尚未往里写）
   + 尽力从 SQLite `store_metadata` 的 `attachment_assets` / `session_attachment_index` 拷实体
   （库不可读或键缺失只 warning）。导入把附件落到目标会话目录的 `attachments/`，**不并入 DB 附件索引**。
   这一层要在附件写入路径迁移到会话目录后收口。

## Alternatives considered

1. **复制目录 + 外部 `tar` 命令** —— 否决。没有完整性校验、没有版本声明，坏包无法识别，
   也无法作为将来 HTTP 端点的线格式。
2. **`.tar.zst`（对齐 DSH 的 zstd 帧）** —— 否决（本期）。需要新增依赖；而「传输格式」的价值在
   结构与校验，不在压缩率，压缩可后加且不破坏兼容（`manifest.files[].sha256` 对内容而非容器）。
3. **单文件形态直接用自定义二进制容器** —— 否决。`.tar` 是外部工具（`bsdtar`/`unzip` 之外的最通用格式）
   可直接查看，互操作性优于自定义容器，且 ustar 手写成本只有约 100 行。
4. **manifest 也放进 `files` 自校验** —— 不可能（自引用哈希无解）。改为结构性校验 + 事件流三重兜底。
5. **附件与事件同等严格（附件坏则整包拒绝）** —— 否决。附件是可降级的展示资产，事件是真源；
   用 EXIT=4 区分「成功但降级」，既不静默也不过度拒绝。
6. **导入时顺带写 SQLite 索引** —— 否决。那会让「导入」变成两件事，且索引本就可由 `rebuild-index`
   从日志重建；让导入只做一件事，边界更清晰。
