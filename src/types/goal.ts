export type GoalPhase = "active" | "paused" | "blocked" | "completed";
export type GoalActivation = "armed" | "disarmed";

export interface GoalData {
  id: string;
  session_id: string;
  objective: string;
  phase: GoalPhase;
  activation: GoalActivation;
  revision: number;
  roundsStarted: number;
  maxGoalRounds: number;
  blocked_reason?: string | null;
}

export type UpdateGoalAction = "complete" | "pause" | "resume" | "edit" | "blocked";

export interface CreateGoalPayload {
  sessionId?: string;
  objective: string;
  maxGoalRounds?: number;
}

export interface UpdateGoalPayload {
  sessionId?: string;
  goalId: string;
  revision: number;
  action: UpdateGoalAction;
  objective?: string;
  blockedReason?: string;
  maxGoalRounds?: number;
}
