// ACP P2（B163）：外部 agent 清单的只读前端 API。
//
// 宿主侧语义与 `ponycli acp --list` 一致：清单文件缺省 ⇒ 内置默认两条；解析/校验失败 ⇒
// 显式错误（fail-closed，不伪造清单），由调用方呈现错误态而非空列表冒充「无 agent」。
//
// 返回项是**摘要**（`AcpAgentSummary`）：只回 env 键名，env 值永不出宿主。
import { safeInvoke } from "@/lib/tauri";
import type { AcpAgentSummary } from "@/types/acp";

export async function listAcpAgents(): Promise<AcpAgentSummary[]> {
  return safeInvoke<AcpAgentSummary[]>("acp_list_agents");
}
