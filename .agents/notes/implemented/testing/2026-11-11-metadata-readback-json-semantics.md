# Agent Note: 元数据读回判定改按 JSON 语义（B072：Windows 上裸子串断言恒假）

Status: implemented

## Problem

`tests/storage_stage0_write_amplification.rs::authorize_path_does_not_trigger_full_store_commit`
在 Windows CI 恒红（run 43003138264：该二进制 3 passed / 1 failed，失败点为 :483
「元数据必须已落盘到 store_metadata 的 path_authorizations.v1」），而 Linux 本地恒绿。

排障一度把方向指向生产侧「`workspace_bootstrap` 派生修复延后吃掉了授权首帧提交」——
但该假设是错的。真实机理只有一句：**断言用裸子串比对 JSON**。

`store_metadata.value` 存的是 `serde_json` 序列化结果。Windows 路径分隔符是 `\`，
JSON 会把它转义成 `\\`，于是「条目已落盘」与「裸子串命中」在 Windows 上恒为假；
Linux 临时目录不含反斜杠、转义不发生，同一错误口径恰好绿——**平台差异掩盖了判定口径错误**，
而不是暴露了平台缺陷。

本地 hermetic 复现（把反斜杠写进 Linux 目录名，构造同一形态）：

```
canonical     = /tmp/b072-repro-932593/RUNNER~1\AppData\Local\authorized-dir
stored        = [{"path":"/tmp/.../RUNNER~1\\AppData\\Local\\authorized-dir",...}]
substring hit = false        ← 生产已落盘，仅断言口径错
```

危害不止一条用例：这类「拿序列化后的文本做子串断言」的写法把**编码细节**混进了
**语义断言**，任何含转义字符的字段（反斜杠、引号、非 ASCII）都会踩，且踩到与否
取决于运行平台/临时目录命名——红灯既不指向缺陷，绿灯也不证明正确。

## Decision

1. **读回判定按 JSON 语义**：新增 `store_metadata_authorized_paths()`，把
   `path_authorizations.v1` 反序列化为 `Vec<AuthorizedPathEntry>`，断言用
   `entry.path == canonical` 逐字段比对。序列化形态（转义、字段序、空格）不再参与判定。
2. **生产代码零改动**：`authorize_path → save_path_authorizations_to_backend →
   PersistCommand::PublishMetadata → INSERT OR REPLACE store_metadata` 链路本身正确，
   病灶纯在测试口径。修测试不改被测实现，避免为迁就断言而污染生产契约。
3. **补一条跨平台回归守卫**
   （`path_authorization_readback_compares_json_not_raw_substring`）：在**任意平台**
   把反斜杠路径写进同一张表的同一 key，同时断言两件事——裸子串**必然落空**（旧口径的
   Windows 红相）、JSON 语义读回**必须命中**。这样旧口径一旦回潮立即红，不再依赖
   「跑在 Windows 上才失败」的运气。守卫已用变异验证：把语义读回替换为裸子串口径后
   该用例立即 FAILED（`left: 0, right: 1`），恢复后 5 例全绿。
4. **`store_metadata_contains` 保留**：`workspaces` 等 key 的断言比的是不含转义字符的
   简单标识（如 `amp-ws`），语义上就是「文本里有没有这个串」。只在路径/含转义字段的
   场景改用语义读回，不做无差别的全局替换（那是超出本缺陷的改动）。

## Alternatives considered

- **把生产侧序列化改成不做转义**（如 `serde_json::to_string` 后手工还原 `\\`）：
  否决——那是把「测试断言写错」变成「磁盘格式破坏」。JSON 转义是格式契约的一部分，
  为迁就一条断言去改落盘格式，会让所有既有库的读回路径一起承压。
- **在断言前对 needle 做转义**（`needle.replace('\\', "\\\\")` 后比对）：
  否决——把转义的**规则**复制进测试，等于测试自己实现一遍序列化；`serde_json` 一旦
  调整转义策略（如 `\u` 转义非 ASCII）测试就静默失真。反序列化再比对没有这个耦合。
- **给该用例加 `#[cfg(unix)]` 平台 gating**：否决——病灶不是「Windows 特有语义」，
  而是判定口径错误；gating 会把一条本可在全平台验证的契约永久丢掉，且掩盖同一写法
  在其他 key 上的潜在踩雷。
- **把 `store_metadata_contains` 全量替换为语义读回**：否决——见决策 4。对
  `workspaces` 这类「断言某个标识串在不在」的用法，语义读回需要为每种 key 写一份
  反序列化，收益为零而耦合上升。

## Consequences

- Windows 侧该用例的红相消失，且红/绿不再取决于临时目录是否含反斜杠。
- 新增一条**不依赖平台**的判定口径守卫；旧口径回潮会在任意平台立刻失败。
- 后续新增 `store_metadata` 读回断言时，含转义字符的字段一律走反序列化比对，
  不再引入同类平台运气。
- 遗留：`tests/storage_stage0_write_amplification.rs` 仍是 5 例，本改动只加 1 例守卫；
  B072 的「Windows 真机全绿」验收仍需 CI 实跑确认（本地无法替代）。
