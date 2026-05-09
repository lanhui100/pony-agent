#[derive(Clone)]
pub struct ToolDefinition {
    pub name: &'static str,
    pub description: &'static str,
}

pub fn builtin_tools() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "terminal.exec",
            description: "执行本地终端命令",
        },
        ToolDefinition {
            name: "fs.read",
            description: "读取工作区文件",
        },
        ToolDefinition {
            name: "mcp.call",
            description: "调用外部 MCP 工具",
        },
    ]
}
