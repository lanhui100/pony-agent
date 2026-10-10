use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UpdateGoalAction {
    Complete,
    Pause,
    Resume,
    Edit,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GoalActivation {
    Armed,
    Disarmed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GoalData {
    pub id: String,
    pub session_id: String,
    pub objective: String,
    pub phase: String,
    pub activation: GoalActivation,
    pub revision: u64,
    #[serde(rename = "roundsStarted")]
    pub rounds_started: u32,
    #[serde(rename = "maxGoalRounds")]
    pub max_goal_rounds: u32,
    pub blocked_reason: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CreateGoalArgs {
    #[serde(default)]
    pub session_id: Option<String>,
    pub objective: String,
    pub max_goal_rounds: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateGoalResult {
    pub goal: GoalData,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GetGoalArgs {
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetGoalResult {
    pub goal: Option<GoalData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateGoalArgs {
    #[serde(default)]
    pub session_id: Option<String>,
    pub goal_id: String,
    pub revision: u64,
    pub action: UpdateGoalAction,
    pub objective: Option<String>,
    pub blocked_reason: Option<String>,
    pub max_goal_rounds: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateGoalResult {
    pub goal: GoalData,
}

/// Map: session_id -> GoalData
static SESSION_GOALS: Mutex<Option<HashMap<String, GoalData>>> = Mutex::new(None);

fn resolve_session(session_id: Option<String>) -> String {
    session_id
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "default".to_string())
}

pub fn create_goal(args: CreateGoalArgs) -> Result<CreateGoalResult, String> {
    let session = resolve_session(args.session_id);
    let goal_id = format!("goal-{}", Uuid::new_v4());
    let goal = GoalData {
        id: goal_id,
        session_id: session.clone(),
        objective: args.objective,
        phase: "active".to_string(),
        activation: GoalActivation::Armed,
        revision: 1,
        rounds_started: 0,
        max_goal_rounds: args.max_goal_rounds.unwrap_or(50),
        blocked_reason: None,
    };

    let mut lock = SESSION_GOALS.lock().unwrap();
    if lock.is_none() {
        *lock = Some(HashMap::new());
    }
    lock.as_mut().unwrap().insert(session, goal.clone());

    Ok(CreateGoalResult { goal })
}

pub fn get_goal(args: GetGoalArgs) -> Result<GetGoalResult, String> {
    let session = resolve_session(args.session_id);
    let lock = SESSION_GOALS.lock().unwrap();
    if let Some(ref map) = *lock {
        return Ok(GetGoalResult {
            goal: map.get(&session).cloned(),
        });
    }
    Ok(GetGoalResult { goal: None })
}

pub fn update_goal(args: UpdateGoalArgs) -> Result<UpdateGoalResult, String> {
    let session = resolve_session(args.session_id);
    let mut lock = SESSION_GOALS.lock().unwrap();
    let map = lock.as_mut().ok_or_else(|| "No active goal found".to_string())?;
    let current = map
        .get_mut(&session)
        .ok_or_else(|| "No goal found for this session".to_string())?;

    if current.id != args.goal_id {
        return Err("Goal ID mismatch".to_string());
    }

    if current.revision != args.revision {
        return Err(format!(
            "Stale revision: expected {}, got {}",
            current.revision, args.revision
        ));
    }

    match args.action {
        UpdateGoalAction::Pause => {
            current.phase = "paused".to_string();
            current.activation = GoalActivation::Disarmed;
        }
        UpdateGoalAction::Resume => {
            current.phase = "active".to_string();
            current.activation = GoalActivation::Armed;
            current.blocked_reason = None;
        }
        UpdateGoalAction::Complete => {
            current.phase = "completed".to_string();
            current.activation = GoalActivation::Disarmed;
            current.blocked_reason = None;
        }
        UpdateGoalAction::Blocked => {
            let reason = args
                .blocked_reason
                .ok_or_else(|| "Blocked action requires blocked_reason".to_string())?;
            current.phase = "blocked".to_string();
            current.activation = GoalActivation::Disarmed;
            current.blocked_reason = Some(reason);
        }
        UpdateGoalAction::Edit => {
            if let Some(obj) = args.objective {
                current.objective = obj;
            }
            if let Some(max) = args.max_goal_rounds {
                current.max_goal_rounds = max;
            }
        }
    }

    current.revision += 1;
    Ok(UpdateGoalResult {
        goal: current.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_goal_lifecycle_and_cas() {
        let session = "test_session_goal_1".to_string();
        let create_res = create_goal(CreateGoalArgs {
            session_id: Some(session.clone()),
            objective: "Test Objective".to_string(),
            max_goal_rounds: Some(10),
        })
        .unwrap();

        assert_eq!(create_res.goal.revision, 1);
        assert_eq!(create_res.goal.phase, "active");
        assert_eq!(create_res.goal.activation, GoalActivation::Armed);

        let get_res = get_goal(GetGoalArgs {
            session_id: Some(session.clone()),
        })
        .unwrap();
        assert_eq!(get_res.goal.unwrap().id, create_res.goal.id);

        // Stale revision test
        let stale_err = update_goal(UpdateGoalArgs {
            session_id: Some(session.clone()),
            goal_id: create_res.goal.id.clone(),
            revision: 99,
            action: UpdateGoalAction::Pause,
            objective: None,
            blocked_reason: None,
            max_goal_rounds: None,
        });
        assert!(stale_err.is_err());

        // Pause
        let pause_res = update_goal(UpdateGoalArgs {
            session_id: Some(session.clone()),
            goal_id: create_res.goal.id.clone(),
            revision: 1,
            action: UpdateGoalAction::Pause,
            objective: None,
            blocked_reason: None,
            max_goal_rounds: None,
        })
        .unwrap();
        assert_eq!(pause_res.goal.revision, 2);
        assert_eq!(pause_res.goal.phase, "paused");
        assert_eq!(pause_res.goal.activation, GoalActivation::Disarmed);

        // Blocked requires reason
        let block_without_reason = update_goal(UpdateGoalArgs {
            session_id: Some(session.clone()),
            goal_id: create_res.goal.id.clone(),
            revision: 2,
            action: UpdateGoalAction::Blocked,
            objective: None,
            blocked_reason: None,
            max_goal_rounds: None,
        });
        assert!(block_without_reason.is_err());

        // Blocked with reason
        let block_res = update_goal(UpdateGoalArgs {
            session_id: Some(session.clone()),
            goal_id: create_res.goal.id.clone(),
            revision: 2,
            action: UpdateGoalAction::Blocked,
            objective: None,
            blocked_reason: Some("Rate limited".to_string()),
            max_goal_rounds: None,
        })
        .unwrap();
        assert_eq!(block_res.goal.phase, "blocked");
        assert_eq!(block_res.goal.blocked_reason.as_deref(), Some("Rate limited"));
        assert_eq!(block_res.goal.activation, GoalActivation::Disarmed);

        // Resume
        let resume_res = update_goal(UpdateGoalArgs {
            session_id: Some(session.clone()),
            goal_id: create_res.goal.id.clone(),
            revision: 3,
            action: UpdateGoalAction::Resume,
            objective: None,
            blocked_reason: None,
            max_goal_rounds: None,
        })
        .unwrap();
        assert_eq!(resume_res.goal.phase, "active");
        assert_eq!(resume_res.goal.activation, GoalActivation::Armed);
        assert!(resume_res.goal.blocked_reason.is_none());

        // Complete
        let complete_res = update_goal(UpdateGoalArgs {
            session_id: Some(session.clone()),
            goal_id: create_res.goal.id.clone(),
            revision: 4,
            action: UpdateGoalAction::Complete,
            objective: None,
            blocked_reason: None,
            max_goal_rounds: None,
        })
        .unwrap();
        assert_eq!(complete_res.goal.phase, "completed");
        assert_eq!(complete_res.goal.activation, GoalActivation::Disarmed);
    }
}
