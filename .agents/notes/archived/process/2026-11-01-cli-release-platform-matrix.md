# Agent Note: CLI 发布平台矩阵、清单单点装配与词表对齐门禁

Status: implemented

> **归档冻结说明**：本文档为原历史决策 `docs/archive/decisions/0108-cli-release-platform-matrix.md` 之归档快照，仅供溯源。

## 背景

`pony update` 的客户端侧（[ADR 0087](0087-cli-self-update.md)）与发布侧长期不一致，且**没有任何命令会红**：

- 客户端协议词表认识 5 个平台（`crates/pony-agent-core/src/agent/updater.rs:80-88` 的 `current_platform()`：
  `linux-x86_64` / `linux-aarch64` / `macos-x86_64` / `macos-aarch64` / `windows-x86_64`），
  契约矩阵 `.dev-team/contract-matrix-cli-self-update.md` C1 还把这句写成「发布侧与客户端唯一真值」；
- 发布侧只有一个 job：`.github/workflows/release.yml` 的 `build-cli-linux` 硬编码 `runs-on: ubuntu-latest` +
  `pony-tui_<ver>_linux-x86_64`，清单 `platforms` 永远只有一项。

结果是 macOS / ARM 机器上 `pony update` 稳定得到 `UnsupportedPlatform` —— 错误串虽然诚实列出「清单已有平台」，
但**承诺与交付的缺口在治理层面无人负责**。同一段流水线还有三处并发缺陷：

1. **命名与事实错位**：清单资产叫 `pony-cli-latest.json`，装的却是 TUI 二进制 `pony-tui_*`；`ponycli` 根本不发布，
   而仓库里也没有一句成文的「不发布」说明（ADR 0087 只在备选方案里顺带提过 Windows）。
2. **清单装配逻辑长在 YAML 里**：`pony-cli-latest.json` 由 job 内联 python 生成、`sha256sum` 由 shell 生成。
   本机没有可跑的等价实现，只有推 `v*` tag 才知道对不对；且 mac/linux 的摘要命令不同（`sha256sum` vs `shasum`），
   多平台一上来就会踩差异。
3. **矩阵化的必然竞态**：若每个平台 job 各自生成并 `gh release upload --clobber` 同一份清单，最终只剩最后一个
   跑完的平台 —— 比现状更糟。

## 决策

1. **发布平台矩阵有且只有一个真值源：`scripts/cli-platforms.mjs`**。本轮发布 3 个平台
   （`linux-x86_64` 必需；`linux-aarch64`、`macos-aarch64` 尽力），`windows-x86_64` 与 `macos-x86_64`
   显式登记在 `NOT_PUBLISHED_PLATFORMS` 并附结构性理由（前者：运行中的 `.exe` 无法被自身替换，走桌面 NSIS 包，
   沿用 ADR 0087 决定 7；后者：GitHub 已无 Intel macOS runner 稳定供给，存量 Intel Mac 趋零）。
   词表（客户端认识什么）与发布集合（我们真的发什么）从此是两个概念，注释与文档不再混写。
2. **构建与装配分两个 job**：`build-cli` 是平台矩阵（`strategy.matrix.include`，原生 runner），只做
   `cargo build --release --bin pony` + core 版本链断言 + 上传 workflow artifact；`publish-cli` 单点下载全部 artifact，
   跑 `scripts/build-cli-manifest.mjs` 装配清单与 `<asset>.sha256` 旁证文件（摘要口径自此唯一实现，可在本机跑单测），
   再一次 `gh release upload --clobber`。清单版本递增门禁（ADR 0097）随装配步骤前移到 `publish-cli`，位置语义不变：
   **生成清单之前**。
3. **必需平台失败即发版失败，边缘平台失败只让该平台缺席**：`continue-on-error: ${{ !matrix.required }}`；
   装配脚本对「必需平台无制品」非零退出，对「本轮缺边缘平台」在 stderr 打 `[WARN] 本轮缺失平台…` 并说明用户侧后果。
   收尾断言新增一条：清单声明的每个平台必须在 release 上真的取得同名资产（防「清单有、资产无」）。
4. **三方对齐是机器门禁**：`node scripts/check-cli-release-contract.mjs`（已接入 `npm run verify`，也有一条 vitest
   用例覆盖）校验 ① workflow 矩阵集合 == 真值源集合；② 发布的每个平台必须在客户端 `current_platform()` 词表内
   （否则该制品永远无人选中）；③ `windows-x86_64` 不得出现在矩阵；④ 客户端 `CLI_MANIFEST_ASSET` 字面量未被改写；
   ⑤ release.yml 必须调用装配脚本而不是内联 python。
5. **术语与资产名冻结**：清单资产名 `pony-cli-latest.json` **不改名**——它已硬编码在存量已发布二进制里，
   改名等于让所有老客户端静默断更。现行口径：`pony-cli-*` 中的「cli」指**命令行形态**（含 TUI 二进制），
   不是 `ponycli` 那个 bin。确有改名需求时，CI 必须同批上传新旧两个清单、旧清单保留到覆盖周期结束，且走新 ADR。
6. **`ponycli` 的定位成文**：`crates/pony-agent-core/README.md` 新增两个入口的定位表（面向对象、stdout 契约、
   有无公开制品），并写明「不拆 crate：cargo 无 per-`[[bin]]` 依赖，`ponycli` 必然链接 `ratatui`/`image`，
   拆分收益只有体积」。

## 候选方案

- **A. 发布矩阵扩到 3 平台 + 单点装配（选中）**：交付与承诺对齐；客户端零改动（`platforms` 本来就是
  `BTreeMap` 查表）；清单逻辑搬回可测脚本。代价：release 多 2 个 job 的 runner 分钟数，且首次接入的两个平台
  未经本机验证。
- **B. 收窄客户端词表到 `linux-x86_64`**（否决）：这会让客户端对 macOS/ARM 用户说「你的平台不支持」，而事实是
  「我们没发」；同时推翻 ADR 0087 C1 已冻结的 API 面。用「少承诺」掩盖「少交付」是把缺口藏起来而不是补上。
- **C. 单个 linux job 交叉编译全部目标（`cross` / zigbuild）**（否决）：`native-tls`、`keyring`、`libdbus-sys`、
  `rusqlite/bundled` 的 C 依赖在交叉目标上需要成体系 sysroot 与目标专属包，本机无法验证、失败率高；
  原生 runner 反而更接近用户机器（`ubuntu-24.04-arm` 是原生 aarch64）。
- **D. 每个平台 job 各自生成并上传清单**（否决）：同一份 `pony-cli-latest.json` 被多 job `--clobber` 互相覆盖，
  最终清单只剩最后完成的平台——这正是本 ADR 要消灭的「清单与发布事实错位」。
- **E. 人工上传 mac/arm 二进制补空洞**（否决）：ADR 0087 已裁决「清单由 CI 生成，不由人手写」；
  历史上 `v0.1.182` 里人工上传的 `pony-tui_0.1.155_linux-x86_64` 落后 tag，就是这种补法的下场。
- **F. 为 `ponycli` 拆独立 crate 甩掉 TUI 依赖**（否决）：见决策 6；收益只有二进制体积（~30MB 已随 ADR 0087 定价），
  代价是新增一层跨 crate 契约与版本锁步。

## 影响

- **用户可见**：macOS(Apple Silicon) 与 Linux(aarch64) 机器上 `pony update` 从「必然 UnsupportedPlatform」变为
  可正常升级；Windows 用户仍然走桌面安装包（口径未变）。客户端代码零改动。
- **CI 面**：`release.yml` 的 `build-cli-linux` 被 `build-cli`（矩阵）+ `publish-cli` 取代；资产数量从 3 件
  （二进制 + `.sha256` + 清单）增至每成功平台 2 件 + 1 份清单。**不可逆面**：`gh release upload --clobber`
  会覆盖同 tag 上的同名资产；清单 version 由 ADR 0097 的严格递增门禁守住，不会出现「同号不同内容」的新清单
  覆盖旧清单却不通知任何人。
- **回滚**：把 `build-cli` 的 `matrix.include` 退回 `linux-x86_64` 单条 + 同批改 `scripts/cli-platforms.mjs`
  即可回到本轮之前的行为；只改一处会被 `contract:cli` 拦红（这是设计意图）。
- **可观测性**：不新增运行时接口、出网路径或后台任务（改动面是 CI + 文档 + 门禁脚本），
  `docs/observability.yaml` 中 ADR 0087 已登记的四维归属保持不变。
- **靠 review 的部分（机器验不到，如实登记）**：① `ubuntu-24.04-arm` 与 `macos-latest` runner 对本仓依赖树的
  可编译性（含 `-D warnings` 下 macOS 是否有平台专属告警）只能在下一个 `v*` tag 的真实 run 上验收；
  ② `macos-x86_64` 永久缺席是可接受还是需补，取决于是否还有 Intel Mac 用户，当前判断为不需补。
  失败时的表现是**显式的**：非必需平台 job 红但不阻断发版，`publish-cli` 打印缺失平台，用户侧得到
  `UnsupportedPlatform` + 清单已有平台清单。

