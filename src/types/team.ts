export type TeammateStatus =
  | "provisioning"
  | "active"
  | "running"
  | "inactive"
  | "failed"
  | "interrupted";

export interface TeammateMember {
  target: string;
  name: string;
  role: string;
  status: TeammateStatus;
  description: string;
  /** 读时快照：是否有后台驱动线程正在跑该 target。 */
  is_driving?: boolean;
  /** 上一轮驱动失败的错误文本；成功收口后缺席。 */
  last_error?: string | null;
}

export interface TeamTaskItem {
  task_id: string;
  subject: string;
  description: string;
  status: "pending" | "in_progress" | "completed" | "deleted";
  owner?: string | null;
  revision: number;
  created_at_ms: number;
  write_scopes: string[];
  blocked_by: string[];
}
