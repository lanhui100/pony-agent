#!/usr/bin/env node
/**
 * gen-extension-points.mjs —— 扩展点归属表生成器（ADR 0068 决策 3 的骨架，W2 前置）。
 *
 * 归属表是**生成物**：手写副本必然与 registry source 枚举 / trait 清单漂移
 * （ADR 0068 已把「手写归属表」列为候选方案中的否决项）。因此本脚本是
 * `docs/architecture/extension-points.md` 的**唯一真值来源**。
 *
 * 事实源（全部读源码，不读人工维护的第二份清单）：
 *   - crates/pony-agent-core/src/agent/tools.rs        → `ToolDescriptorSource` 枚举变体
 *   - crates/pony-agent-core/src/agent/tool_runtime.rs → 已定义的运行期端口 trait
 *   - crates/pony-agent-core/src/agent/hooks.rs        → `TurnHookPoint` 钩子点数量
 *   - docs/contracts/*.v1.schema.json                  → 五份契约版本串
 *   - 各扩展点载体的**存在性**与 MCP 配置面关键词命中 → 状态列
 *
 * 用法：
 *   node scripts/gen-extension-points.mjs            # 打印到 stdout（不落盘）
 *   node scripts/gen-extension-points.mjs --write    # 写入 docs/architecture/extension-points.md
 *   node scripts/gen-extension-points.mjs --check    # 一致性校验：不一致则 exit 1
 *
 * 退出码：0 通过 / 1 生成物与仓库内文件不一致（仅 --check）/ 2 用法或事实源读取失败。
 */

import { readFileSync, writeFileSync, existsSync, readdirSync, statSync } from 'node:fs';
import { join, dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(HERE, '..');
const TARGET = join(ROOT, 'docs/architecture/extension-points.md');

const SRC = 'crates/pony-agent-core/src/agent';
const CONTRACTS = [
  ['tool-descriptor', 'docs/contracts/tool-descriptor.v1.schema.json'],
  ['hooks', 'docs/contracts/hooks.v1.schema.json'],
  ['mcp-config', 'docs/contracts/mcp-config.v1.schema.json'],
  ['skill-manifest', 'docs/contracts/skill-manifest.v1.schema.json'],
  ['plugin', 'docs/contracts/plugin.v1.schema.json'],
];

function fail(message) {
  console.error(`[gen-extension-points] ${message}`);
  process.exit(2);
}

function read(relativePath) {
  const absolute = join(ROOT, relativePath);
  if (!existsSync(absolute)) fail(`事实源缺失: ${relativePath}`);
  return readFileSync(absolute, 'utf8');
}

function exists(relativePath) {
  return existsSync(join(ROOT, relativePath));
}

/** 解析枚举体里的变体名（跳过空行、注释、属性）。 */
function enumVariants(source, enumName) {
  const match = source.match(new RegExp(`pub enum ${enumName}\\s*\\{([\\s\\S]*?)\\n\\}`));
  if (!match) fail(`未找到枚举 ${enumName}（事实源结构变化，需同步生成器）`);
  return match[1]
    .split('\n')
    .map((line) => line.trim())
    .filter((line) => line && !line.startsWith('//') && !line.startsWith('#'))
    .map((line) => line.replace(/,?\s*$/, ''))
    .filter((line) => /^[A-Za-z][A-Za-z0-9_]*$/.test(line));
}

function snake(name) {
  return name.replace(/([a-z0-9])([A-Z])/g, '$1_$2').toLowerCase();
}

/** 递归搜索源码目录里的字面量。 */
function sourceContains(directory, needle) {
  const absolute = join(ROOT, directory);
  if (!existsSync(absolute)) return false;
  for (const entry of readdirSync(absolute)) {
    const full = join(absolute, entry);
    const stats = statSync(full);
    if (stats.isDirectory()) {
      if (sourceContains(join(directory, entry), needle)) return true;
    } else if (entry.endsWith('.rs') && readFileSync(full, 'utf8').includes(needle)) {
      return true;
    }
  }
  return false;
}

function main() {
  const args = process.argv.slice(2);
  const unknown = args.filter((arg) => !['--write', '--check', '--help', '-h'].includes(arg));
  if (unknown.length > 0) fail(`未知参数: ${unknown.join(' ')}`);
  if (args.includes('--help') || args.includes('-h')) {
    console.log('用法: node scripts/gen-extension-points.mjs [--write|--check]');
    return;
  }

  const tools = read(`${SRC}/tools.rs`);
  const runtime = read(`${SRC}/tool_runtime.rs`);
  const hooks = read(`${SRC}/hooks.rs`);

  const sources = enumVariants(tools, 'ToolDescriptorSource');
  const hookPoints = enumVariants(hooks, 'TurnHookPoint');
  const traits = [...runtime.matchAll(/pub trait (\w+)/g)].map((match) => match[1]);
  const uniqueTraits = [...new Set(traits)].sort();

  const versions = CONTRACTS.map(([name, path]) => {
    const schema = JSON.parse(read(path));
    if (!schema.version) fail(`契约 schema 缺少 version: ${path}`);
    return [name, schema.version];
  });

  const pluginHost = `${SRC}/plugin_host/mod.rs`;
  const mcpConfigLoader = sourceContains(SRC, 'mcpServers');

  const rows = [
    {
      point: '内建工具',
      layer: 'L1 编译期接缝',
      how: '改 Rust：实现 handler + 注册 descriptor',
      contract: 'pony-tool-descriptor-v1',
      carrier: '`governed_executor.rs`',
      status: exists(`${SRC}/governed_executor.rs`) ? '载体已存在（`governed_executor.rs`）' : '载体未落地',
    },
    {
      point: 'MCP server',
      layer: 'L2 进程外插件',
      how: '写 MCP server + `.mcp.json`（v1 只 stdio）',
      contract: 'mcp-config-v1 + pony-tool-descriptor-v1',
      carrier: '`mcp_client.rs`',
      status: mcpConfigLoader
        ? '客户端已存在（`mcp_client.rs`）；配置面关键词 `mcpServers` 已命中'
        : '客户端已存在（`mcp_client.rs`）；**未命中 `mcpServers` 读取 → 配置加载未落地**',
    },
    {
      point: 'Skill',
      layer: 'L1 编译期接缝',
      how: '写 `SKILL.md`（frontmatter）',
      contract: 'pony-skill-manifest-v1',
      carrier: '`workspace_hotloader.rs`',
      status: exists(`${SRC}/workspace_hotloader.rs`) ? '载体已存在（`workspace_hotloader.rs`）' : '载体未落地',
    },
    {
      point: 'Hook',
      layer: 'L1 编译期接缝',
      how: '写 `.pony/hooks.json`（`command` 或 `argv` 形态）',
      contract: 'agent-hooks-v1',
      carrier: '`hook_commands.rs`',
      status: exists(`${SRC}/hook_commands.rs`) ? '载体已存在（`hook_commands.rs`）' : '载体未落地',
    },
    {
      point: 'Plugin',
      layer: 'L2 进程外插件',
      how: '写 `plugin.json` + 可执行体，stdio JSON-RPC 2.0',
      contract: 'pony-plugin-v1',
      carrier: '`plugin_host/`',
      status: exists(pluginHost) ? '载体已存在（`plugin_host/`）' : '载体未落地（ADR 0067 待 T3）',
    },
  ];

  const lines = [];
  lines.push('# 扩展点归属表（生成物）');
  lines.push('');
  lines.push('> 本文件由 `scripts/gen-extension-points.mjs` 从源码事实生成，**禁止手改**（手写副本即第二真值源，ADR 0068 决策 3 已否决）。');
  lines.push('> 重生成：`node scripts/gen-extension-points.mjs --write`；一致性校验：`node scripts/gen-extension-points.mjs --check`。');
  lines.push('> 事实源：`tools.rs`（registry source 枚举）、`tool_runtime.rs`（已定义 trait 端口）、`hooks.rs`（钩子点）、`docs/contracts/*.v1.schema.json`（契约版本）、各载体文件是否存在。');
  lines.push('');
  lines.push('## 1. 想加什么，该挂哪（ADR 0064 三层模型）');
  lines.push('');
  lines.push('| 扩展点 | 层 | 扩展方式 | 契约 | 载体 | 当前状态（由文件事实推导） |');
  lines.push('|---|---|---|---|---|---|');
  for (const row of rows) {
    lines.push(`| ${row.point} | ${row.layer} | ${row.how} | \`${row.contract}\` | ${row.carrier} | ${row.status} |`);
  }
  lines.push('');
  lines.push(`## 2. registry source 枚举（\`tools.rs\` → \`ToolDescriptorSource\`，${sources.length} 个变体）`);
  lines.push('');
  lines.push(sources.map((variant) => `\`${snake(variant)}\``).join(' · '));
  lines.push('');
  lines.push(`## 3. 已定义的运行期端口 trait（\`tool_runtime.rs\`，${uniqueTraits.length} 个）`);
  lines.push('');
  lines.push(uniqueTraits.map((name) => `\`${name}\``).join(' · '));
  lines.push('');
  lines.push(`## 4. 契约版本（\`docs/contracts/*.v1.schema.json\`）`);
  lines.push('');
  lines.push('| 契约 | version |');
  lines.push('|---|---|');
  for (const [name, version] of versions) {
    lines.push(`| ${name} | \`${version}\` |`);
  }
  lines.push('');
  lines.push(`## 5. Hook 钩子点（\`hooks.rs\` → \`TurnHookPoint\`，${hookPoints.length} 个）`);
  lines.push('');
  lines.push(hookPoints.map((point) => `\`${snake(point)}\``).join(' · '));
  lines.push('');
  const generated = lines.join('\n');

  if (args.includes('--check')) {
    if (!existsSync(TARGET)) fail(`仓库内缺少生成物: ${TARGET}`);
    const current = readFileSync(TARGET, 'utf8');
    if (current !== generated) {
      console.error('[gen-extension-points] FAIL 生成物与仓库内文件不一致。');
      console.error('  修复：node scripts/gen-extension-points.mjs --write 后复核 diff。');
      process.exit(1);
    }
    console.log('[gen-extension-points] PASS 生成物与仓库内文件一致。');
    return;
  }

  if (args.includes('--write')) {
    writeFileSync(TARGET, generated, 'utf8');
    console.log(`[gen-extension-points] 已写入 ${TARGET}`);
    return;
  }

  process.stdout.write(generated);
}

main();
