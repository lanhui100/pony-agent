use serde::{Deserialize, Serialize};
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalData {
    pub id: String,
    pub objective: String,
    pub phase: String,
    pub revision: u64,
    #[serde(rename = "roundsStarted")]
    pub rounds_started: u32,
    #[serde(rename = "maxGoalRounds")]
    pub max_goal_rounds: u32,
    pub blocked_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateGoalArgs {
    pub objective: String,
    pub max_goal_rounds: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateGoalResult {
    pub goal: GoalData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetGoalResult {
    pub goal: Option<GoalData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateGoalArgs {
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

static GOALS_MAP: Mutex<Option<std::collections::HashMap<String, GoalData>>> = Mutex::new(None);
static LATEST_GOAL_ID: Mutex<Option<String>> = Mutex::new(None);

pub fn create_goal(args: CreateGoalArgs) -> Result<CreateGoalResult, String> {
    let goal_id = format!("goal-{}", Uuid::new_v4());
    let goal = GoalData {
        id: goal_id.clone(),
        objective: args.objective,
        phase: "active".to_string(),
        revision: 1,
        rounds_started: 0,
        max_goal_rounds: args.max_goal_rounds.unwrap_or(50),
        blocked_reason: None,
    };

    let mut map_lock = GOALS_MAP.lock().unwrap();
    if map_lock.is_none() {
        *map_lock = Some(std::collections::HashMap::new());
    }
    map_lock.as_mut().unwrap().insert(goal_id.clone(), goal.clone());

    let mut latest_lock = LATEST_GOAL_ID.lock().unwrap();
    *latest_lock = Some(goal_id);

    Ok(CreateGoalResult { goal })
}

pub fn get_goal() -> Result<GetGoalResult, String> {
    let latest_lock = LATEST_GOAL_ID.lock().unwrap();
    if let Some(ref id) = *latest_lock {
        let map_lock = GOALS_MAP.lock().unwrap();
        if let Some(ref map) = *map_lock {
            return Ok(GetGoalResult {
                goal: map.get(id).cloned(),
            });
        }
    }
    Ok(GetGoalResult { goal: None })
}

pub fn update_goal(args: UpdateGoalArgs) -> Result<UpdateGoalResult, String> {
    let mut map_lock = GOALS_MAP.lock().unwrap();
    let map = map_lock.as_mut().ok_or_else(|| "No active goal found".to_string())?;
    let current = map.get_mut(&args.goal_id).ok_or_else(|| "Goal ID mismatch".to_string())?;
    if current.revision != args.revision {
        return Err(format!(
            "Stale revision: expected {}, got {}",
            current.revision, args.revision
        ));
    }

    match args.action {
        UpdateGoalAction::Pause => {
            current.phase = "paused".to_string();
        }
        UpdateGoalAction::Resume => {
            current.phase = "active".to_string();
        }
        UpdateGoalAction::Complete => {
            current.phase = "completed".to_string();
        }
        UpdateGoalAction::Blocked => {
            if args.blocked_reason.is_none() {
                return Err("Blocked action requires blocked_reason".to_string());
            }
            current.phase = "blocked".to_string();
            current.blocked_reason = args.blocked_reason;
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
