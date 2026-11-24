# Agent Note: ACP 文本读取把「非 UTF-8」错报成「读取失败」，并修掉不可达分支（B177 ⑥）

Status: implemented

## Problem

`acp/handlers.rs` 的 `read_slice` 有两条问题，复审 r2 记作 ADVISORY ⑥：

1. **错误归类错位**：`reader.read_line(&mut String)` 遇到非 UTF-8 字节时返回的是
   `io::ErrorKind::InvalidData`（`stream did not contain valid UTF-8`），却被
   `.map_err(SliceReadError::Io)?` 一律当成 IO 失败 ⇒ 用户看到
   「读取文件失败（…）：stream did not contain valid UTF-8」，
   与真实原因（文件不是文本）不符，而且 `SliceReadError::NotUtf8` 对应的准确文案
   「文件不是合法 UTF-8，拒绝以文本读取」**永远不会出现**。
2. **`NotUtf8` 的那条分支本身不可达**：函数尾部
   ```rust
   if index == 0 && !selected.is_empty() { return Err(SliceReadError::NotUtf8); }
   ```
   `index == 0` 只可能在循环第一次就 `read == 0`（空文件/立即 EOF）时成立，而那种情况下
   循环体里的 `selected.push_str` 从未执行过 ⇒ `selected` 必为空 ⇒ 条件恒假。
   这是一条纯粹的死分支。

## Decision

1. 把 `read_line` 的结果显式分流：`ErrorKind::InvalidData` ⇒ `SliceReadError::NotUtf8`，
   其余错误 ⇒ `SliceReadError::Io`。这样「不是文本」与「读不动」在用户面前区分开。
2. **删掉尾部那条恒假分支**，并在 `read_line` 分流处留下注释说明「归到 NotUtf8 才是准确文案」。

行为变化仅限**错误归类与文案**：读取成功的路径、分片语义、上限判定逐字未变。

## Verification

- `bash scripts/cargo-lane.sh check -p pony-agent-core --lib` → Finished（exit 0）
- `读取文件失败` / `不是合法 UTF-8` 两个文案在 crates 内只有本文件产出，无既有测试断言依赖
  其中任一条（已 grep 确认），改动不会打破既有断言
- **未在本地跑 `--lib` 单测**：当前会话的测试执行约束是「只允许 `--test <单个解耦测试>`」，
  而 `read_slice` 是私有函数、只能由 lib 单测覆盖。故本条以 CI 的 verify（跑 `--lib` 全量）
  作为回归门——CI 红即回退，不以此 PR 的单次本地结果充当证据。

## Alternatives considered

- **保留死分支、只改错误分类**：死分支留着会让后来者以为"空文件会返回 NotUtf8"，
  并且下次有人删 `selected.is_empty()` 判断时行为会突然改变。既然它不可能为真，就该删掉。
- **把 `read_slice` 提升为 `pub` 以便写集成测试**：为了测试而扩大 API 面，会让 `#[test]` 的需要
  驱动生产可见性，属于典型倒置；lib 单测才是它该待的地方。
- **不动、只登记**：这条错报会让外部 agent 以为文件读不动（IO 问题）而不是"这不是文本"，
  直接影响它下一步的决策（重试 vs 换策略），值得改。
