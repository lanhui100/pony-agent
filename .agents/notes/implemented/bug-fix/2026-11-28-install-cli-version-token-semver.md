# Agent Note: install-cli 版本探测只认 semver 形状 token，不认「行尾 token」

Status: implemented

## Problem

`scripts/install-cli.mjs` 的 `probeVersion` 在 `npm run cli:install` 收尾时探测两个 bin 的自报版本
（`pony` / `ponycli`），原实现取 `--version` **首行的行尾 token** 当版本号，并只校验「含数字」。

但两个 bin 的首行格式本来就不同：

```
$ pony --version
pony 0.1.179 · linux-x86_64        ← 行尾 token 是平台三元组（含数字！）
binary: /home/dm/.local/bin/pony
$ ponycli --version
ponycli 0.1.179                     ← 行尾 token 恰好是版本号
```

于是 `pony` 被判成 `linux-x86_64`、`ponycli` 被判成 `0.1.179`，**每次安装都误报
「警告：两 bin 版本漂移」**（B1795832157497）。误报的代价不是噪音：漂移告警是
「同包两 bin 静默不同步」的唯一可见信号（B261），假警会让真警一起被忽略。

## Decision

1. **只认 semver 形状**：`extractVersionToken(line)` 取首行里第一个 `\d+\.\d+\.\d+` 匹配，
   取不到即返回 `null`（视为「该二进制不支持/不提供版本」）。
2. **移植到结构化位置**：解析逻辑抽成可导入的纯函数（`scripts/install-cli.mjs` 导出），
   `main()` 用「被直接执行才跑」的守卫包起来（`realpath(process.argv[1]) === realpath(import.meta.url)`），
   以便 spec 直接对解析函数做正负样本，而不是靠伪造二进制。
3. **保守的失败面**：解析不到 ⇒ 仍是「无法探测版本」的**警告而非失败**——安装本身已成功，
   不能把「读不到版本」当成安装失败（沿用 ADR `2026-11-22-cli-install-version-probe` 的宽容语义）。
4. **误报与漏报都锁住**：spec 补三条端到端样本（同版本不告警 / 真漂移仍告警 / 解析样本表），
   防止「为了消掉假警把判定改宽成永不告警」。

## Alternatives considered

- **取第二 token（`<name> <version>` 形态）**：`pony` 与 `ponycli` 都是两 token，看似可行，
  但一旦未来加平台后缀或 `pony dev-build` 这类无版本输出，第二 token 就会变成「dev-build」；
  semver 形状是更强的判据，且与 `npm run version:check` 的四处同步口径一致（只认三段式）。
- **让 `pony --version` 改成只打印版本号（去掉平台段）**：会丢掉用户排查时最需要的
  「装的是哪个平台的二进制」信息，且改的是 CLI 对外契约，代价远大于改探测。
- **加 `--version-only` 之类的第二旗标**：给一个信息查询接口加参数分叉，等于让小脚本承担
  CLI 版本协议的稳定性，属把复杂度放错地方。
- **把 `linux-x86_64` 当版本号、只在两者都为三元组时跳过告警**：治标——下一个首行格式变化
  （如加 git sha）又会误判；判据应当锚在「版本号长什么样」而不是「哪些不是版本号」。

## Consequences

- 安装收尾的漂移告警重新可信：`pony 0.1.179 · linux-x86_64` 与 `ponycli 0.1.179` 判为同版本。
- 解析规则单点收口（`extractVersionToken`），任何新 bin 只需遵循「首行含三段式版本」即可被识别。
- 守卫：`npm run install-cli:selftest` 的 P5（解析样本表）/ P6（不误报）/ P7（真漂移仍告警）。

## Verification

- `node scripts/install-cli.spec.mjs` → 25 项全绿（含 P5 七条解析样本、P6/P7 端到端）
- 实测输入：`pony --version` = `pony 0.1.179 · linux-x86_64`，`ponycli --version` = `ponycli 0.1.179`
