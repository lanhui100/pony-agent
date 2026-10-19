export type TeammateStatus = "provisioning" | "active" | "running" | "inactive" | "failed";

export interface TeammateMember {
  target: string;
  name: string;
  role: string;
  status: TeammateStatus;
  description: string;
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
