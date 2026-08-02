// browser-preview domain：浏览器预览兜底模式下的默认工具、capability source 与 capability 定义。
// 仅浏览器预览（无 Tauri 宿主）时使用，帮助渲染 UI 与输入交互。
import type { AvailableTool, CapabilitySourceView, CapabilityView } from "../../types/runtime";

export const defaultAvailableTools: AvailableTool[] = [
  {
    name: "Run",
    canonicalToolName: "Run",
    executionPrimitive: "workspace_run_command",
    description: "在当前工作区内受控执行命令，并委托到内部 RunShell 执行能力；稳定返回 cwd、timeout、exitCode、stdout 和 stderr。",
    kind: "execute",
    exposure: "model_visible",
    displayMetadata: { displayNameZh: "运行" },
    permissionFacts: {
      requiresApproval: false,
      permissionScope: "workspace.execute",
      hostMediated: false,
      approvalMode: "none",
      decisionSource: "runtime",
      permissionProfile: "builtin"
    },
    inputSchema: {
      type: "object",
      properties: {
        command: {
          type: "string",
          description: "要执行的命令文本"
        },
        cwd: {
          type: "string",
          description: "执行命令时的工作区内相对目录，默认 ."
        },
        timeoutMs: {
          type: "integer",
          description: "命令超时毫秒数，默认 10000，最大 120000"
        }
      },
      required: ["command"],
      additionalProperties: false
    }
  },
  {
    name: "Ask",
    canonicalToolName: "Ask",
    executionPrimitive: "echo_input",
    description: "向用户或宿主请求澄清、确认或补充输入；无宿主中介时会回落为受控澄清提示。",
    kind: "interactive",
    exposure: "model_visible",
    displayMetadata: { displayNameZh: "提问" },
    permissionFacts: {
      requiresApproval: false,
      approvalMode: "none",
      decisionSource: "runtime",
      permissionProfile: "builtin"
    },
    inputSchema: {
      type: "object",
      properties: {
        text: {
          type: "string",
          description: "需要向用户展示或确认的文本"
        },
        question: {
          type: "string",
          description: "当 text 缺失时，用于 fallback 的澄清问题"
        }
      },
      additionalProperties: false
    }
  },
  {
    name: "Read",
    canonicalToolName: "Read",
    executionPrimitive: "workspace_gather_context",
    description: "围绕一个路径自动聚合上下文，适合默认读取文件、目录和局部线索的首选入口。",
    kind: "read",
    exposure: "model_visible",
    displayMetadata: { displayNameZh: "读取" },
    permissionFacts: {
      requiresApproval: false,
      permissionScope: "workspace.read",
      hostMediated: false,
      approvalMode: "none",
      decisionSource: "runtime",
      permissionProfile: "builtin"
    },
    inputSchema: {
      type: "object",
      properties: {
        path: {
          type: "string",
          description: "当前工作区内的相对文件路径"
        },
        query: {
          type: "string",
          description: "可选查询词，用于在聚合上下文时补充相关搜索结果"
        },
        limit: {
          type: "integer",
          description: "最多聚合多少个路径，默认使用运行时内置上限"
        },
        lineCount: {
          type: "integer",
          description: "读取文件片段时的目标行数"
        }
      },
      required: ["path"],
      additionalProperties: false
    }
  },
  {
    name: "Search",
    canonicalToolName: "Search",
    executionPrimitive: "workspace_search_text",
    description: "在当前工作区内递归搜索文本内容，返回命中路径、行号和预览片段。",
    kind: "search",
    exposure: "model_visible",
    displayMetadata: { displayNameZh: "搜索" },
    permissionFacts: {
      requiresApproval: false,
      permissionScope: "workspace.read",
      hostMediated: false,
      approvalMode: "none",
      decisionSource: "runtime",
      permissionProfile: "builtin"
    },
    inputSchema: {
      type: "object",
      properties: {
        query: {
          type: "string",
          description: "要搜索的关键字或文本片段"
        },
        path: {
          type: "string",
          description: "可选相对路径，用于缩小搜索范围"
        },
        limit: {
          type: "integer",
          description: "最多返回多少条命中结果"
        },
        regex: {
          type: "boolean",
          description: "是否按增强模式匹配 query；当前 v1 使用通配符式匹配，默认 false"
        }
      },
      required: ["query"],
      additionalProperties: false
    }
  },
  {
    name: "List",
    canonicalToolName: "List",
    executionPrimitive: "workspace_list_files",
    description: "列出当前工作区目录中的文件与子目录，可指定相对路径和返回条数。",
    kind: "read",
    exposure: "model_visible",
    displayMetadata: { displayNameZh: "列表" },
    permissionFacts: {
      requiresApproval: false,
      permissionScope: "workspace.read",
      hostMediated: false,
      approvalMode: "none",
      decisionSource: "runtime",
      permissionProfile: "builtin"
    },
    inputSchema: {
      type: "object",
      properties: {
        path: {
          type: "string",
          description: "当前工作区内的相对目录路径，默认 ."
        },
        limit: {
          type: "integer",
          description: "最多返回多少个条目，默认 40"
        }
      },
      additionalProperties: false
    }
  },
  {
    name: "Glob",
    canonicalToolName: "Glob",
    executionPrimitive: "workspace_glob_files",
    description: "按路径 pattern 递归匹配工作区内文件，适合大代码库中的文件发现。",
    kind: "read",
    exposure: "model_visible",
    displayMetadata: { displayNameZh: "匹配" },
    permissionFacts: {
      requiresApproval: false,
      permissionScope: "capability.discovery",
      hostMediated: false,
      approvalMode: "none",
      decisionSource: "runtime",
      permissionProfile: "builtin"
    },
    inputSchema: {
      type: "object",
      properties: {
        pattern: {
          type: "string",
          description: "要匹配的路径模式，例如 src/*.rs 或 *tool*"
        },
        path: {
          type: "string",
          description: "搜索起点目录，默认为 ."
        },
        limit: {
          type: "integer",
          description: "最多返回多少条路径命中，默认 50"
        }
      },
      required: ["pattern"],
      additionalProperties: false
    }
  },
  {
    name: "WebFetch",
    canonicalToolName: "WebFetch",
    executionPrimitive: "web_fetch_url",
    description: "抓取指定 http/https URL 的正文内容预览，不承担搜索排序职责。",
    kind: "read",
    exposure: "model_visible",
    displayMetadata: { displayNameZh: "抓取" },
    permissionFacts: {
      requiresApproval: false,
      permissionScope: "workspace.read",
      hostMediated: false,
      approvalMode: "none",
      decisionSource: "runtime",
      permissionProfile: "builtin"
    },
    inputSchema: {
      type: "object",
      properties: {
        url: {
          type: "string",
          description: "要抓取的 http/https URL"
        },
        timeoutMs: {
          type: "integer",
          description: "请求超时毫秒数，默认 15000"
        }
      },
      required: ["url"],
      additionalProperties: false
    }
  },
  {
    name: "WebSearch",
    canonicalToolName: "WebSearch",
    executionPrimitive: "web_search_query",
    description: "执行外部搜索并返回结构化结果列表，不把抓取和搜索混为一个工具。",
    kind: "search",
    exposure: "model_visible",
    displayMetadata: { displayNameZh: "外搜" },
    permissionFacts: {
      requiresApproval: false,
      permissionScope: "workspace.read",
      hostMediated: false,
      approvalMode: "none",
      decisionSource: "runtime",
      permissionProfile: "builtin"
    },
    inputSchema: {
      type: "object",
      properties: {
        query: {
          type: "string",
          description: "外部搜索关键词"
        },
        limit: {
          type: "integer",
          description: "最多返回多少条搜索结果，默认 5"
        },
        timeoutMs: {
          type: "integer",
          description: "请求超时毫秒数，默认 15000"
        }
      },
      required: ["query"],
      additionalProperties: false
    }
  },
  {
    name: "MCPResource",
    canonicalToolName: "MCPResource",
    executionPrimitive: "mcp_resource_read",
    description: "通过 capability registry 读取指定 MCP 资源 capability 的只读内容，不混入普通工具执行。",
    kind: "read",
    exposure: "model_visible",
    displayMetadata: { displayNameZh: "资源" },
    permissionFacts: {
      requiresApproval: false,
      permissionScope: "workspace.read",
      hostMediated: false,
      approvalMode: "none",
      decisionSource: "runtime",
      permissionProfile: "builtin"
    },
    inputSchema: {
      type: "object",
      properties: {
        capabilityId: {
          type: "string",
          description: "目标 resource capability id，例如 mcp:resource:repo-index"
        },
        arguments: {
          type: "object",
          description: "传给 resource capability 的结构化参数"
        }
      },
      required: ["capabilityId"],
      additionalProperties: false
    }
  },
  {
    name: "ToolSearch",
    canonicalToolName: "ToolSearch",
    executionPrimitive: "tool_search",
    description: "搜索 capability registry 中可用的工具候选，返回结构化候选项，作为 deferred / dynamic tool discovery 入口。",
    kind: "search",
    exposure: "deferred",
    displayMetadata: { displayNameZh: "找工具" },
    permissionFacts: {
      requiresApproval: false,
      permissionScope: "capability.discovery",
      hostMediated: false,
      approvalMode: "none",
      decisionSource: "runtime",
      permissionProfile: "builtin"
    },
    inputSchema: {
      type: "object",
      properties: {
        query: {
          type: "string",
          description: "可选查询词；为空时返回默认候选列表"
        },
        sourceId: {
          type: "string",
          description: "可选 source id，用于缩小 discovery 范围"
        },
        limit: {
          type: "integer",
          description: "最多返回多少条候选，默认 8"
        }
      },
      additionalProperties: false
    }
  },
  {
    name: "Write",
    canonicalToolName: "Write",
    executionPrimitive: "workspace_write_file",
    description: "在当前工作区内新建或整文件覆写文本文件，可控制是否允许覆盖现有文件。",
    kind: "write",
    exposure: "model_visible",
    displayMetadata: { displayNameZh: "写入" },
    permissionFacts: {
      requiresApproval: false,
      permissionScope: "workspace.write",
      hostMediated: false,
      approvalMode: "none",
      decisionSource: "runtime",
      permissionProfile: "builtin"
    },
    inputSchema: {
      type: "object",
      properties: {
        path: {
          type: "string",
          description: "当前工作区内的相对文件路径"
        },
        content: {
          type: "string",
          description: "要写入文件的完整文本内容"
        },
        overwrite: {
          type: "boolean",
          description: "是否允许覆盖已存在文件，默认 true"
        }
      },
      required: ["path", "content"],
      additionalProperties: false
    }
  },
  {
    name: "Edit",
    canonicalToolName: "Edit",
    executionPrimitive: "workspace_edit_file",
    description: "在当前工作区内按 oldText/newText 对文本文件做受控替换；默认只允许单一匹配。",
    kind: "write",
    exposure: "model_visible",
    displayMetadata: { displayNameZh: "编辑" },
    permissionFacts: {
      requiresApproval: false,
      permissionScope: "workspace.write",
      hostMediated: false,
      approvalMode: "none",
      decisionSource: "runtime",
      permissionProfile: "builtin"
    },
    inputSchema: {
      type: "object",
      properties: {
        path: {
          type: "string",
          description: "当前工作区内的相对文件路径"
        },
        oldText: {
          type: "string",
          description: "需要被替换的原始文本"
        },
        newText: {
          type: "string",
          description: "替换后的新文本"
        },
        replaceAll: {
          type: "boolean",
          description: "是否允许替换全部匹配，默认 false"
        }
      },
      required: ["path", "oldText", "newText"],
      additionalProperties: false
    }
  },
  {
    name: "BatchExecute",
    canonicalToolName: "BatchExecute",
    executionPrimitive: "workspace_batch",
    description: "批量执行多个工具子调用，可选并发和失败继续选项，用于一次性收集多个上下文片段。",
    kind: "composite",
    exposure: "model_visible",
    displayMetadata: { displayNameZh: "批量执行" },
    permissionFacts: {
      requiresApproval: false,
      permissionScope: "workspace.read",
      hostMediated: false,
      approvalMode: "none",
      decisionSource: "runtime",
      permissionProfile: "builtin"
    },
    inputSchema: {
      type: "object",
      properties: {
        calls: {
          type: "array",
          description: "待执行的子调用数组"
        },
        parallel: {
          type: "boolean",
          description: "是否并发执行子调用"
        },
        continueOnError: {
          type: "boolean",
          description: "子调用失败后是否继续执行剩余步骤"
        }
      },
      required: ["calls"],
      additionalProperties: false
    }
  }
];

export function createAvailableTools() {
  return defaultAvailableTools.map((tool) => ({
    ...tool,
    inputSchema: {
      ...tool.inputSchema,
      properties: tool.inputSchema.properties ? { ...tool.inputSchema.properties } : {}
    }
  }));
}

const defaultCapabilitySources: CapabilitySourceView[] = [
  {
    sourceId: "builtin-tools",
    sourceKind: "builtin",
    displayName: "Builtin Tools",
    transportKind: "in_process",
    serverIdentity: "pony-agent:builtin-tools",
    availability: "available",
    declaredCapabilities: ["tool"],
    permissionProfile: "host-mediated",
    updatedAtMs: 0,
    lastIngressObservation: null
  }
];

export function createCapabilitySources() {
  return defaultCapabilitySources.map((source) => ({
    ...source,
    declaredCapabilities: [...source.declaredCapabilities]
  }));
}

function canonicalizeBuiltinCapabilityName(toolName: string) {
  return toolName.replace(/\./g, "_");
}

export function createCapabilities() {
  return defaultAvailableTools.map((tool): CapabilityView => ({
    capabilityId: `builtin:${canonicalizeBuiltinCapabilityName(tool.executionPrimitive)}`,
    sourceId: "builtin-tools",
    sourceKind: "builtin",
    kind: "tool",
    label: canonicalizeBuiltinCapabilityName(tool.executionPrimitive),
    canonicalToolName: tool.canonicalToolName,
    displayNameZh: tool.displayMetadata.displayNameZh ?? null,
    description: tool.description,
    invocationMode: "direct_tool_call",
    inputSchemaSummary: tool.inputSchema.type ?? "object",
    safetyClass: "host_tool",
    visibility: "default",
    observabilityTags: ["builtin", "tool"],
    requiresApproval: tool.permissionFacts.requiresApproval ?? false,
    hostMediated: tool.permissionFacts.hostMediated ?? false,
    permissionScope: tool.permissionFacts.permissionScope ?? "--",
    permissionFacts: { ...tool.permissionFacts }
  }));
}
