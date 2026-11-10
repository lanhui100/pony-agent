//! 内置工具 JSON schema 装配：`builtin_tools()`、`with_description`、
//! `ToolDefinition` contract view 与展示优先级。
use super::*;

impl ToolDefinition {
    pub fn contract_view(&self) -> ToolDefinitionContractView {
        ToolDefinitionContractView {
            name: product_visible_tool_name(self.name),
            canonical_tool_name: product_visible_tool_name(self.name),
            execution_primitive: canonical_tool_name(self.name)
                .unwrap_or(self.name)
                .to_string(),
            description: self.description.to_string(),
            input_schema: self.input_schema.clone(),
            kind: tool_kind_for_name(self.name).as_str().to_string(),
            exposure: tool_exposure_for_name(self.name).as_str().to_string(),
            display_metadata: tool_display_metadata_for_name(self.name),
            permission_facts: default_permission_facts_for_name(self.name),
        }
    }
}

pub(super) fn with_description(schema: Value) -> Value {
    let desc = json!({
        "type": "string",
        "description": "用中文极简描述本次工具调用的目的，用于用户界面展示。每次调用工具时必须提供此字段。例如「读取 config.json」「搜索 TokenManager」「运行单元测试」。"
    });
    if let Some(properties) = schema
        .as_object()
        .and_then(|o| o.get("properties"))
        .and_then(|p| p.as_object())
    {
        let mut props = properties.clone();
        props.insert("description".to_string(), desc);
        let mut schema = Value::Object(schema.as_object().unwrap().clone());
        let object = schema.as_object_mut().unwrap();
        object.insert("properties".to_string(), Value::Object(props));
        let mut required = object
            .get("required")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if !required
            .iter()
            .any(|value| value.as_str() == Some("description"))
        {
            required.push(Value::String("description".to_string()));
        }
        object.insert("required".to_string(), Value::Array(required));
        schema
    } else {
        schema
    }
}

pub fn builtin_tools() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: TOOL_TIME_NOW,
            description: "返回当前本机 UNIX 时间戳，适合最小时间查询演示。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_ECHO_INPUT,
            description: "把传入的 text 原样返回，适合验证 tool roundtrip。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "text": {
                        "type": "string",
                        "description": "需要原样回显给用户的文本"
                    }
                },
                "required": ["text"],
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_ASK_USER,
            description:
                "向用户提问并等待其回答，适合需要用户确认、选择或补充信息后才能继续的场景；返回用户的回答（options 提供可选快捷回答；multiSelect=true 时用户可以勾选多项，回答里会一并给出全部选中项）。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "question": {
                        "type": "string",
                        "description": "要向用户提出的问题"
                    },
                    "options": {
                        "type": "array",
                        "items": {
                            "type": "string"
                        },
                        "description": "可选的预设回答选项，用户可直接点选"
                    },
                    "multiSelect": {
                        "type": "boolean",
                        "description": "仅当问题需要用户同时选择多个选项时置 true；默认 false（单选，选一项即提交）"
                    }
                },
                "required": ["question"],
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_WORKSPACE_READ_FILE,
            description: "读取当前工作区内的文本文件内容预览，需要提供相对路径；大文件会被拒绝并引导改用分段读取。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "当前工作区内的相对文件路径"
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_WORKSPACE_READ_DOCUMENT,
            description: "把当前工作区内的办公文档（Word/PPT/Excel/ODF/RTF/EPUB/CSV/PDF）本地转换为 GitHub 风格 Markdown 文本返回，需要提供相对路径；扫描版 PDF（无内嵌文本）不支持，转换结果超出 maxOutputBytes 时截断并在 truncated 字段给出证据。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "当前工作区内的相对文档路径（.doc/.docx/.ppt/.pptx/.xls/.xlsx/.odt/.ods/.odp/.rtf/.epub/.csv/.pdf 等）"
                    },
                    "maxOutputBytes": {
                        "type": "integer",
                        "description": "转换结果 Markdown 输出上限（字节），默认 524288"
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_WORKSPACE_READ_FILE_SEGMENT,
            description: "按行读取当前工作区文件的一段内容，适合大文件局部查看。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "当前工作区内的相对文件路径"
                    },
                    "startLine": {
                        "type": "integer",
                        "description": "从第几行开始读取，最小为 1；分页参数以 gather 回显为准"
                    },
                    "lineCount": {
                        "type": "integer",
                        "description": "读取多少行，默认 40，最大 400"
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_WORKSPACE_LIST_FILES,
            description: "列出当前工作区目录下的文件和子目录，可指定相对路径和返回条数。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "当前工作区内的相对目录路径，默认为 ."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "最多返回多少个条目，默认 40"
                    }
                },
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_WORKSPACE_PATH_INFO,
            description: "返回工作区内文件或目录的路径元信息，适合快速判断它是什么、大小和层级。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "当前工作区内的相对路径，默认为 ."
                    }
                },
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_WORKSPACE_SEARCH_TEXT,
            description: "递归搜索工作区目录内的文本内容，返回命中的路径、行号和预览。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "要搜索的关键字或文本片段"
                    },
                    "path": {
                        "type": "string",
                        "description": "搜索起点目录，默认为 ."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "最多返回多少条命中，默认 20"
                    },
                    "ignoreCase": {
                        "type": "boolean",
                        "description": "是否忽略大小写，默认 true"
                    },
                    "regex": {
                        "type": "boolean",
                        "description": "是否按增强模式匹配 query；当前 v1 使用通配符式匹配，默认 false"
                    },
                    "filePattern": {
                        "type": "string",
                        "description": "可选的路径子串过滤，例如 .rs 或 src/agent"
                    }
                },
                "required": ["query"],
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_WORKSPACE_GLOB_FILES,
            description: "按路径 pattern 递归匹配工作区内文件，适合大代码库中的文件发现。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "要匹配的路径模式，例如 src/*.rs 或 *tool*"
                    },
                    "path": {
                        "type": "string",
                        "description": "搜索起点目录，默认为 ."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "最多返回多少条路径命中，默认 50"
                    }
                },
                "required": ["pattern"],
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_WEB_FETCH_URL,
            description: "抓取指定 http/https URL 的正文内容预览，不承担搜索排序职责。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "要抓取的 http/https URL"
                    },
                    "timeoutMs": {
                        "type": "integer",
                        "description": "请求超时毫秒数，默认 15000"
                    }
                },
                "required": ["url"],
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_WEB_SEARCH_QUERY,
            description: "执行外部搜索并返回结构化结果列表，不把抓取和搜索混为一个工具；每次调用必须同时给出 query 与 objective（本轮搜索目标）。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "外部搜索关键词"
                    },
                    "objective": {
                        "type": "string",
                        "description": "Goal for this search turn; say which documents should rank first, which should be excluded, and what specific facts or figures to pull from them."
                    },
                    "timeoutMs": {
                        "type": "integer",
                        "description": "请求超时毫秒数，默认 15000"
                    }
                },
                "required": ["query", "objective"],
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            // ── Boundary decision (PA-076 P2-6): MCP resource read stays in the capability
            // registry path (runtime/tool_exec.rs:execute_resource_registry_tool_call). The
            // builtin registry entry is discovery metadata only; actual execution goes through
            // the source-bound `McpTransport` + `CapabilityRegistry`, not the governed
            // dispatcher. Adding a governed handler for this descriptor WOULD create a double
            // execution path, which is explicitly forbidden. ──
            name: TOOL_MCP_RESOURCE_READ,
            description: "通过 capability registry 读取指定 MCP 资源 capability 的只读内容，不混入普通工具执行。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "capabilityId": {
                        "type": "string",
                        "description": "目标 resource capability id，例如 mcp:resource:repo-index"
                    },
                    "arguments": {
                        "type": "object",
                        "description": "传给 resource capability 的结构化参数"
                    }
                },
                "required": ["capabilityId"],
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            // ── Boundary decision (PA-076 P2-6): ToolSearch stays in the capability registry
            // path (runtime/tool_exec.rs:execute_tool_search_registry_tool_call). The builtin
            // registry entry is discovery metadata only; actual search and elevation go through
            // the `ToolSearchElevator` + `TurnToolView` + `CapabilityRegistry`, not the
            // governed dispatcher. Adding a governed handler WOULD create a double execution
            // path, which is explicitly forbidden. ──
            name: TOOL_TOOL_SEARCH,
            description: "搜索 capability registry 中可用的工具候选，作为 deferred / dynamic tool discovery 入口。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "可选查询词；为空时返回默认候选列表"
                    },
                    "sourceId": {
                        "type": "string",
                        "description": "可选 source id，用于缩小 discovery 范围"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "最多返回多少条候选，默认 8"
                    }
                },
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_WORKSPACE_WRITE_FILE,
            description: "在当前工作区内新建或整文件覆写文本文件，可控制是否允许覆盖现有文件。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "当前工作区内的相对文件路径"
                    },
                    "content": {
                        "type": "string",
                        "description": "要写入文件的完整文本内容"
                    },
                    "overwrite": {
                        "type": "boolean",
                        "description": "是否允许覆盖已存在文件，默认 true"
                    },
                    "expectedHash": {
                        "type": "string",
                        "description": "可选。CAS 乐观并发守卫：当前文件的预期 SHA-256 哈希值。若磁盘文件哈希不匹配，则拒绝写入并提供自愈指引。"
                    }
                },
                "required": ["path", "content"],
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_WORKSPACE_EDIT_FILE,
            description: "在当前工作区内按 oldText/newText 对文本文件做受控替换；默认只允许单一匹配。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "当前工作区内的相对文件路径"
                    },
                    "oldText": {
                        "type": "string",
                        "description": "需要被替换的原始文本"
                    },
                    "newText": {
                        "type": "string",
                        "description": "替换后的新文本"
                    },
                    "replaceAll": {
                        "type": "boolean",
                        "description": "是否允许替换全部匹配，默认 false"
                    },
                    "expectedHash": {
                        "type": "string",
                        "description": "可选。CAS 乐观并发守卫：当前文件的预期 SHA-256 哈希值。若磁盘文件哈希不匹配，则拒绝编辑并提供自愈指引。"
                    }
                },
                "required": ["path", "oldText", "newText"],
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_WORKSPACE_RUN_COMMAND,
            description: "在当前工作区内受控执行命令，返回 cwd、timeout、exitCode、stdout 和 stderr。提示：严禁执行 rmdir /s、remove-item 或 rm -rf；清理工作区临时文件时优先使用 git clean 或修改 .gitignore；在 Windows 下避免使用 ';' 复合多条命令，请使用 '&&'。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "要执行的命令文本"
                    },
                    "cwd": {
                        "type": "string",
                        "description": "执行命令时的工作区内相对目录，默认 ."
                    },
                    "timeoutMs": {
                        "type": "integer",
                        "description": "命令超时毫秒数，默认 120000，最大 600000；长任务请改用后台执行 `job_start`（`job_output` 取增量输出、`job_kill` 终止，无超时上限）"
                    }
                },
                "required": ["command"],
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_WORKSPACE_BATCH,
            description: "批量执行多个工具子调用，可选并发和 continueOnError，用于一次性收集多个上下文片段。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "parallel": {
                        "type": "boolean",
                        "description": "是否并发执行；当 continueOnError=false 时会自动退回串行"
                    },
                    "continueOnError": {
                        "type": "boolean",
                        "description": "某个子调用失败后是否继续执行其余子调用"
                    },
                    "calls": {
                        "type": "array",
                        "description": "子调用列表，每个元素包含 name 和可选 arguments",
                        "items": {
                            "type": "object",
                            "properties": {
                                "name": { "type": "string" },
                                "arguments": { "type": "object" }
                            },
                            "required": ["name"],
                            "additionalProperties": false
                        }
                    }
                },
                "required": ["calls"],
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_WORKSPACE_GATHER_CONTEXT,
            description: "围绕一个路径自动收集最合适的上下文：文件会拿 path info 和首段内容，目录会拿 path info 和文件列表，带 query 时会连同搜索结果一起返回。大文件可用 startLine 分页续读。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "目标路径，默认为 ."
                    },
                    "paths": {
                        "type": "array",
                        "description": "可选的多路径聚合输入；提供后会依次对每个路径执行单路径 gather 并汇总结果",
                        "items": {
                            "type": "string"
                        }
                    },
                    "query": {
                        "type": "string",
                        "description": "可选搜索词；提供后会进入搜索模式"
                    },
                    "startLine": {
                        "type": "integer",
                        "description": "从第几行开始读取（最小 1）。显式提供时对所有模式生效，多路径下逐路径生效；未提供时文件模式从第 1 行、搜索模式自动定位命中附近片段。requestedStartLine/startLineFallback 为 output-only 聚合回显，模型不得回填（回填触发 invalid_arguments）"
                    },
                    "lineCount": {
                        "type": "integer",
                        "description": "文件模式下读取多少行，默认 80，最大 400"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "目录模式和搜索模式下的条数限制，默认 40"
                    }
                },
                "additionalProperties": false
            })),
        },
        // ── PA-076 phase 7 P2-6: Plan + ViewImage added after all existing definitions
        // so the stable prefix invariant (design Decision 10) is preserved. ──
        ToolDefinition {
            name: TOOL_PLAN_CONTROL,
            description: "控制会话级 Plan 状态：create / replace / merge / complete_step / start_execution / abort，不执行任意子调用。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "op": {
                        "type": "string",
                        "description": "plan 控制操作：create / replace / merge / complete_step / start_execution / abort"
                    },
                    "session_id": {
                        "type": "string",
                        "description": "拥有该 plan 的 session id"
                    },
                    "plan_id": {
                        "type": "string",
                        "description": "要修改的 plan id（replace / merge / complete_step / start_execution / abort 需要；abort 省略时撤销本会话最早的 Draft 计划）"
                    },
                    "revision": {
                        "type": "integer",
                        "description": "期望的 plan revision，用于 CAS 冲突检测（replace / merge / complete_step / start_execution 需要；abort 省略时取当前 revision）"
                    },
                    "step_id": {
                        "type": "string",
                        "description": "要标记完成的 step id（complete_step 需要）"
                    },
                    "payload": {
                        "type": "object",
                        "description": "plan 内容（create / replace 需要）",
                        "properties": {
                            "kind": { "type": "string", "description": "plan 分类标签" },
                            "summary": { "type": "string", "description": "plan 概要描述" },
                            "steps": {
                                "type": "array",
                                "description": "step 规范列表",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "name": { "type": "string" },
                                        "summary": { "type": "string" }
                                    }
                                }
                            }
                        }
                    },
                    "step": {
                        "type": "object",
                        "description": "merge 操作要追加的 step 规范",
                        "properties": {
                            "name": { "type": "string" },
                            "summary": { "type": "string" }
                        }
                    }
                },
                "required": ["op"],
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_VIEW_IMAGE,
            description: "读取工作区内图片并返回规范化 image artifact（受控引用 + magic-byte MIME/尺寸元数据）；默认不嵌入文件字节，是否将字节发送给模型由 provider modality 决定（design Decision 11）。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "当前工作区内的相对图片路径"
                    },
                    "includeBytes": {
                        "type": "boolean",
                        "description": "是否在 artifact 中嵌入字节；默认 false（仅返回受控引用和元数据）"
                    },
                    "maxWidth": {
                        "type": "integer",
                        "description": "最大允许的图片宽度，超出标记 truncated（默认 8192）"
                    },
                    "maxHeight": {
                        "type": "integer",
                        "description": "最大允许的图片高度，超出标记 truncated（默认 8192）"
                    },
                    "maxBytes": {
                        "type": "integer",
                        "description": "嵌入字节时的最大文件字节数，超出标记 truncated（默认 2 MiB）"
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_SUBAGENT,
            description: "派生聚焦的子智能体执行独立子任务，隔离上下文并返回聚合结果。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "prompt": {
                        "type": "string",
                        "description": "传递给子智能体的具体任务描述"
                    },
                    "description": {
                        "type": "string",
                        "description": "本次子任务简述，用于跟踪与显示"
                    },
                    "run_in_background": {
                        "type": "boolean",
                        "description": "是否在后台执行"
                    },
                    "fork_context": {
                        "type": "boolean",
                        "description": "是否继承当前会话上下文"
                    }
                },
                "required": ["prompt", "description"],
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_WORKFLOW,
            description: "根据定义的步骤依赖图（DAG）编排执行多步骤工作流。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "工作流名称"
                    },
                    "steps": {
                        "type": "array",
                        "description": "工作流步骤列表",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": { "type": "string" },
                                "action": { "type": "string" },
                                "depends_on": {
                                    "type": "array",
                                    "items": { "type": "string" }
                                }
                            },
                            "required": ["id", "action"]
                        }
                    }
                },
                "required": ["name", "steps"],
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_SPAWN_TEAMMATE,
            description: "在当前 Agent Team 中派生具有独立角色和职责的持久化队友智能体。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "队友名称（唯一标识）"
                    },
                    "role": {
                        "type": "string",
                        "description": "队友角色，如 tester, reviewer, executor"
                    },
                    "description": {
                        "type": "string",
                        "description": "队友职责描述"
                    },
                    "prompt": {
                        "type": "string",
                        "description": "队友系统角色提示词与初始任务"
                    },
                    "context": {
                        "type": "string",
                        "description": "fresh 或 fork，默认 fresh"
                    }
                },
                "required": ["name", "description", "prompt"],
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_LIST_AGENTS,
            description: "列出当前团队中所有活跃及持久化智能体花名册及其运行状态。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_SEND_MESSAGE,
            description: "向团队内目标智能体发送协同消息，支持普通排队 (queue) 或紧急插队 (steer)。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "target": {
                        "type": "string",
                        "description": "目标智能体名称或标识"
                    },
                    "message": {
                        "type": "string",
                        "description": "要投递的消息文本"
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["queue", "steer"],
                        "description": "消息投递模式，默认为 queue"
                    }
                },
                "required": ["target", "message"],
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_INTERRUPT_AGENT,
            description: "把指定队友标记为中断并保留其待办收件箱：当前回合会在下一次收件箱 drain 边界前停止（引擎无回合内 stop API，不会立即中止正在执行的 turn）。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "target": {
                        "type": "string",
                        "description": "目标智能体名称"
                    },
                    "reason": {
                        "type": "string",
                        "description": "中断原因说明"
                    }
                },
                "required": ["target"],
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_DRAIN_INBOX,
            description: "读取并消费调用者自己信箱中的所有待处理消息队列（target 必须等于调用者自身；队友信箱由驱动循环消费，跨成员 drain 会被拒绝）。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "target": {
                        "type": "string",
                        "description": "目标智能体名称"
                    }
                },
                "required": ["target"],
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_TEAM_TASK_CREATE,
            description: "在团队任务看板上创建待认领任务，支持设置写域范围与依赖阻塞。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "subject": {
                        "type": "string",
                        "description": "任务简明标题"
                    },
                    "description": {
                        "type": "string",
                        "description": "任务详细内容与验收标准"
                    },
                    "write_scopes": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "该任务预期的写操作作用域"
                    },
                    "blocked_by": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "依赖的前序任务 ID 列表"
                    }
                },
                "required": ["subject", "description"],
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_TEAM_TASK_UPDATE,
            description: "通过 CAS 乐观锁推进或更新团队看板上的任务状态（认领/完成/释放/删除）。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "task_id": {
                        "type": "string",
                        "description": "目标任务 ID"
                    },
                    "expected_revision": {
                        "type": "integer",
                        "description": "期望的 CAS 版本号，防并发冲突"
                    },
                    "action": {
                        "type": "string",
                        "enum": ["claim", "complete", "release", "delete"],
                        "description": "要执行的流转操作"
                    },
                    "owner": {
                        "type": "string",
                        "description": "认领人名称（action 为 claim 时使用）"
                    }
                },
                "required": ["task_id", "expected_revision", "action"],
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_TEAM_TASK_LIST,
            description: "列出团队共享看板上所有的任务项、归属人、写域及流转状态。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_TODO_WRITE,
            description: "记录并全量覆写当前任务列表以规划多步工作并展示进度；单步简单任务可跳过。在开始前为每个具体步骤添加一条 todo；当仍有工作剩余时，将正在执行的任务置为 in_progress（支持并发多个）；某个步骤经物理验证完成后，立即更新置为 completed。参数直接传入全量 todos 列表。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "todos": {
                        "type": "array",
                        "description": "全量任务列表，替换之前的列表",
                        "items": {
                            "type": "object",
                            "properties": {
                                "content": {
                                    "type": "string",
                                    "description": "任务内容，简短的祈使句描述"
                                },
                                "status": {
                                    "type": "string",
                                    "enum": ["pending", "in_progress", "completed"],
                                    "description": "pending (未开始) | in_progress (进行中) | completed (已完成)"
                                }
                            },
                            "required": ["content", "status"]
                        }
                    },
                    "session_id": {
                        "type": "string",
                        "description": "可选 session id，默认当前 session"
                    }
                },
                "required": ["todos"],
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_TODO_LIST,
            description: "获取当前 session 记录的 todo 任务列表及状态。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "可选 session id"
                    }
                },
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_SPILL_READ,
            description: "按 content_ref 回读大输出落盘溢出（spill）的内容，支持 offset/limit 分段读取。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "ref_id": {
                        "type": "string",
                        "description": "spilled 结果里返回的 content_ref（64 位十六进制内容哈希）"
                    },
                    "offset": {
                        "type": "integer",
                        "description": "起始字节偏移，默认 0"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "本次读取的最大字节数，默认 8000，上限 200000"
                    }
                },
                "required": ["ref_id"],
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_CREATE_GOAL,
            description: "创建会话目标，使 Agent 在同会话内支持跨多轮自动续行追求持久目标。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "objective": {
                        "type": "string",
                        "description": "目标的具体描述与验收要求"
                    },
                    "max_goal_rounds": {
                        "type": "integer",
                        "description": "允许的最大续行轮次上限，默认 50"
                    },
                    "session_id": {
                        "type": "string",
                        "description": "可选 session id，默认当前 session"
                    }
                },
                "required": ["objective"],
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_GET_GOAL,
            description: "获取当前会话的持久目标信息、当前阶段 phase 与修订号 revision。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "可选 session id"
                    }
                },
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_UPDATE_GOAL,
            description: "使用 CAS 乐观锁机制更新当前会话的持久目标状态（complete, pause, resume, edit, blocked）。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "goal_id": {
                        "type": "string",
                        "description": "目标 ID"
                    },
                    "revision": {
                        "type": "integer",
                        "description": "当前目标版本号，必须与服务端一致"
                    },
                    "action": {
                        "type": "string",
                        "enum": ["complete", "pause", "resume", "edit", "blocked"],
                        "description": "更新动作"
                    },
                    "objective": {
                        "type": "string",
                        "description": "编辑时的全新目标描述"
                    },
                    "blocked_reason": {
                        "type": "string",
                        "description": "阻断时的具体原因说明（action 为 blocked 时必填）"
                    },
                    "max_goal_rounds": {
                        "type": "integer",
                        "description": "编辑时的全新轮次上限"
                    },
                    "session_id": {
                        "type": "string",
                        "description": "可选 session id"
                    }
                },
                "required": ["goal_id", "revision", "action"],
                "additionalProperties": false
            })),
        },
        // W37-C1：Present —— 按引用向用户呈报文件卡片（最多 4 个）。
        ToolDefinition {
            name: TOOL_PRESENT,
            description: "向用户呈报工作区内的文件（最多 4 个，按引用展示卡片，含大小与类型）；适合交付成果、展示截图或文档时使用，不会把文件内容读进上下文。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "files": {
                        "type": "array",
                        "maxItems": 4,
                        "items": {
                            "type": "object",
                            "properties": {
                                "path": {
                                    "type": "string",
                                    "description": "工作区内的相对文件路径"
                                },
                                "description": {
                                    "type": "string",
                                    "description": "该文件的一句话说明（可选）"
                                }
                            },
                            "required": ["path"],
                            "additionalProperties": false
                        },
                        "description": "要呈报的文件列表（1–4 项）"
                    }
                },
                "required": ["files"],
                "additionalProperties": false
            })),
        },
        // W37-C1：Find —— 模糊文件名检索。
        ToolDefinition {
            name: TOOL_FUZZY_FILE_SEARCH,
            description: "在工作区内按模糊关键词检索文件路径（大小写不敏感的包含匹配并打分），适合只记得文件名片段时定位文件。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "文件名模糊查询关键词"
                    },
                    "maxResults": {
                        "type": "integer",
                        "description": "返回匹配数量上限（可选，默认 20）"
                    }
                },
                "required": ["query"],
                "additionalProperties": false
            })),
        },
        // W37-C1：GitDiff —— 本地与远端分支的差异。
        ToolDefinition {
            name: TOOL_GIT_DIFF_REMOTE,
            description: "查看当前工作区 git 仓库与远端 upstream 分支的差异（未设置 targetRef 时默认与 @{upstream} 比较），返回 diff 文本与变更文件清单。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "targetRef": {
                        "type": "string",
                        "description": "可选的目标分支或引用（默认远端 upstream）"
                    }
                },
                "additionalProperties": false
            })),
        },
        // W37-C1：JobStart —— 后台长任务启动。
        ToolDefinition {
            name: TOOL_JOB_START,
            description: "在当前工作区后台启动一个长任务进程（如 dev server、watch 构建），立即返回 jobId，可用 JobOutput 轮询输出、JobKill 终止。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "要执行的可执行命令"
                    },
                    "args": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "命令参数列表（可选）"
                    },
                    "cwd": {
                        "type": "string",
                        "description": "工作目录（可选，默认当前工作区 root）"
                    },
                    "timeoutMs": {
                        "type": "integer",
                        "description": "超时上限毫秒数（可选）"
                    }
                },
                "required": ["command"],
                "additionalProperties": false
            })),
        },
        // W37-C1：JobOutput —— 读取后台任务输出。
        ToolDefinition {
            name: TOOL_JOB_OUTPUT,
            description: "按 jobId 读取后台任务的累计输出与状态，可选择性等待任务结束（wait+timeoutMs）或从指定 offset 增量读取。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "jobId": {
                        "type": "string",
                        "description": "JobStart 返回的任务 ID"
                    },
                    "wait": {
                        "type": "boolean",
                        "description": "是否阻塞等待任务结束（可选，默认 false）"
                    },
                    "timeoutMs": {
                        "type": "integer",
                        "description": "等待超时毫秒数（可选）"
                    },
                    "offset": {
                        "type": "integer",
                        "description": "起始读取游标 offset（可选，默认 0）"
                    }
                },
                "required": ["jobId"],
                "additionalProperties": false
            })),
        },
        // W37-C1：JobKill —— 终止后台任务。
        ToolDefinition {
            name: TOOL_JOB_KILL,
            description: "按 jobId 终止一个仍在运行的后台任务进程。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "jobId": {
                        "type": "string",
                        "description": "JobStart 返回的任务 ID"
                    }
                },
                "required": ["jobId"],
                "additionalProperties": false
            })),
        },
        // W37-C1：JobList —— 列出后台任务。
        ToolDefinition {
            name: TOOL_JOB_LIST,
            description: "列出当前会话全部后台任务（含运行中与历史保留项）及其状态。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            })),
        },
        // W42/PA-149：SkillLoad —— 按需加载已安装技能正文。
        ToolDefinition {
            name: TOOL_SKILL_LOAD,
            description: "按名加载已安装技能的 SKILL.md 正文（先看可用技能列表，需要该技能的具体步骤时再按需读取）。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "要加载正文的技能名（与可用技能列表里的名字一致）"
                    }
                },
                "required": ["name"],
                "additionalProperties": false
            })),
        },
    ]
}

pub(super) fn contract_priority(view: &ToolDefinitionContractView) -> u8 {
    match view.execution_primitive.as_str() {
        TOOL_TIME_NOW => 10,
        TOOL_ECHO_INPUT => 100,
        // PA-114: `ask_user` must outrank the legacy `echo_input` placeholder for the shared
        // product name `Ask`, so the model surface winner switches to ask_user and echo_input
        // drops to Internal exposure (winner logic in `from_builtin_definitions`).
        TOOL_ASK_USER => 110,
        TOOL_WORKSPACE_LIST_FILES => 100,
        TOOL_WORKSPACE_SEARCH_TEXT => 100,
        TOOL_WORKSPACE_GLOB_FILES => 100,
        TOOL_WEB_FETCH_URL => 100,
        TOOL_WEB_SEARCH_QUERY => 100,
        TOOL_MCP_RESOURCE_READ => 100,
        TOOL_TOOL_SEARCH => 100,
        TOOL_WORKSPACE_WRITE_FILE => 100,
        TOOL_WORKSPACE_EDIT_FILE => 100,
        TOOL_WORKSPACE_RUN_COMMAND => 100,
        TOOL_WORKSPACE_BATCH => 100,
        TOOL_WORKSPACE_GATHER_CONTEXT => 100,
        TOOL_WORKSPACE_READ_FILE => 40,
        TOOL_WORKSPACE_READ_FILE_SEGMENT => 30,
        TOOL_WORKSPACE_PATH_INFO => 20,
        TOOL_PLAN_CONTROL => 50,
        TOOL_VIEW_IMAGE => 50,
        // W37-C1：七项新 primitive 与内置工具同级。
        TOOL_PRESENT => 100,
        TOOL_FUZZY_FILE_SEARCH => 100,
        TOOL_GIT_DIFF_REMOTE => 100,
        TOOL_JOB_START => 100,
        TOOL_JOB_OUTPUT => 100,
        TOOL_JOB_KILL => 100,
        TOOL_JOB_LIST => 100,
        // W42/PA-149：技能正文加载与内置只读工具同级。
        TOOL_SKILL_LOAD => 100,
        _ => 50,
    }
}
