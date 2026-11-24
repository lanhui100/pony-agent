// ACP P2（B163）：外部 agent 清单的只读前端 API。
//
// 宿主侧语义与 `ponycli acp --list` 一致：清单文件缺省 ⇒ 内置默认两条；解析/校验失败 ⇒
// 显式错误（fail-closed，不伪造清单），由调用方呈现错误态而非空列表冒充「无 agent」。
import { safeInvoke } from "@/lib/tauri";
import type { AcpAgentSpec } from "@/types/acp";

export async function listAcpAgents(): Promise<AcpAgentSpec[]> {
  return safeInvoke<AcpAgentSpec[]>("acp_list_agents");
}
