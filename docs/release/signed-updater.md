# Pony Agent 签名桌面更新 · 发布运行手册（release-owner gate）

状态：**草案（DRAFT）**。真实密钥未生成、endpoint 未启用、workflow 未运行。完成下列步骤后本卡才可标记"发布 gate 完成"。

## 1. 信任锚（固定，运行时不接受环境覆盖）

- endpoint（固定）：
  `https://github.com/lanhui100/pony-agent/releases/latest/download/latest.json`
- 公钥：由发布方生成（见 §2），`pubkey` 值写入 `src-tauri/tauri.conf.json` 的 `plugins.updater.pubkey`，随启用提交进入仓库。
- 私钥：**永不入库、不入 bundle、不入日志**。仅存在于生成者的安全存储与 CI secrets。

## 2. 密钥生成与保管（发布方执行）

在本地（有安全存储的机器）执行：

```powershell
npx --no-install tauri signer generate --ci -w .\tauri-signing.key
# 生成 tauri-signing.key（私钥）+ 屏幕输出公钥（PUBLIC KEY / minisign public key 字符串）
```

- 私钥文件与密码立即移入密码管理器 / Windows Credential Manager，删除工作区副本。
- 屏幕输出的公钥字符串交给开发侧写入 `tauri.conf.json`（仅公钥入库）。
- CI secrets（GitHub 仓库 Settings → Secrets and variables → Actions）：
  - `TAURI_SIGNING_PRIVATE_KEY`：私钥内容
  - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`：私钥口令
  - secrets 缺失时 release workflow 必须 **hard fail**（不允许无签名产物）。

## 3. 启用改动（拿到公钥后由开发侧落地，一次提交）

1. `src-tauri/tauri.conf.json`：
   - `plugins.updater`: `{ "endpoints": ["https://github.com/lanhui100/pony-agent/releases/latest/download/latest.json"], "pubkey": "<真实公钥>" }`
   - `bundle.createUpdaterArtifacts: true`
2. `src/lib/tauri-updater.ts`：`SIGNED_UPDATER_ENABLED = true`（注释保留发布 gate 说明）。
3. 重新生成 capability schemas（`npm run schemas:normalize`），跑 `npm run version:check` / typecheck / vitest / build / cargo check。
4. 更新 ADR 0019 事实、任务卡 release gate 勾选、会话日志。

## 4. 发布流程（每次发版）

1. 按项目惯例 bump 版本（`npm run version:patch` 或等价），四处版本同步 + `.version.json`。
2. 提交并 push 到 `main`；随后 push 匹配 tag（`vX.Y.Z`）。
3. `.github/workflows/release.yml` 触发（见 §5）：
   - 校验：tag == 四处版本、version:check、typecheck、vitest、cargo check。
   - 构建：Windows x64 NSIS bundle + updater artifacts（.tar.gz/.sig）。
   - 组装 latest.json（manifest），draft release → 上传全部资产（安装器 + tar.gz + sig）→ **最后上传 latest.json** → publish（确保 `/releases/latest/download/latest.json` 指向新 manifest）。
   - 任一步失败 → 删除 draft release、job fail；不产生半成品 manifest。

### manifest（latest.json）契约

```json
{
  "version": "0.1.95",
  "notes": "release notes（可空字符串）",
  "pub_date": "2026-10-02T17:00:00Z",
  "platforms": {
    "windows-x86_64": {
      "signature": "<minisign signature base64，来自产物 .sig>",
      "url": "https://github.com/lanhui100/pony-agent/releases/download/v0.1.95/pony-agent_0.1.95_x64-setup.nsis.tar.gz"
    }
  }
}
```

- `signature` 必须与产物 `*.sig` 完全一致（workflow 内做字节比对，防止手抄漂移）。
- `url` 使用静态 `releases/download/<tag>/<asset>` 形式，资产名与上传名必须一致。

## 5. release workflow（.github/workflows/release.yml）

见仓库内该文件草案。要点：

- 触发：`push` tag `v*`；`permissions: contents: write`（仅该 job 需要），`pull-requests: read`。
- secrets 缺失即 fail；签名失败（tauri build 报错）即 fail。
- 当前草案覆盖 Windows x64；macOS/Linux 目标与 notarization 属后续扩展，未启用前 UI 仍不展示安装按钮（平台矩阵见 PA-103 design）。

## 6. 发布后 smoke（必须留证据）

- 已安装旧版 → 手动安装器引导（预 updater 二进制无法自举，首个版本需用户手动下载安装）。
- 新版本内点"立即升级"：下载→验签→安装→重启；记录前后版本号。
- 篡改/错误 key 负向：改 latest.json 或替换 artifact 后点升级必须失败且不重启。
- 断网/部分下载：失败文案安全、无 relaunch、候选清除。
- 用户数据保留：升级前后工作区/会话数据不变。
- 崩溃/断电恢复：安装中断后下次启动不损坏。

## 7. 回滚 / 撤下 / key rotation

- 回滚：不发"降级" manifest（客户端拒绝非递增版本）；撤下即删除/替换 GitHub release 的 latest.json 与资产，旧客户端保持 fail-closed。
- key rotation：公钥烧进旧二进制，轮换需发布新版引导 + 双公钥过渡协议（当前不支持，属未来决策）；私钥泄露立即撤下 latest.json、发布含新公钥的新版并引导手动升级。
- 日志：CI 与客户端日志不得含私钥/口令/凭据；CI 日志打码 secret。

## 8. 启用检查清单（完成即 gate 通过）

- [ ] 私钥已生成并仅存于安全处；CI secrets 已配置
- [ ] 公钥已写入 `tauri.conf.json`；`SIGNED_UPDATER_ENABLED=true`；createUpdaterArtifacts 开启
- [ ] 全部门禁绿；schemas 重新生成
- [ ] release.yml 就绪；一次 tag 全流程跑通并上传资产
- [ ] smoke 正/负向证据留档（§6）
- [ ] ADR 0019 / 任务卡 / 会话日志更新；OpenSpec tasks release gate 勾选
