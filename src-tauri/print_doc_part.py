with open("event-streaming-doc.md", "r", encoding="utf-8") as f:
    lines = f.readlines()

# 打印 40-160 行（投影表格 + 工具调用 + 子代理 + 状态部分）
for i in range(39, min(170, len(lines))):
    print(f"{i+1:4d}| {lines[i]}", end="")
