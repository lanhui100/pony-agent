//! 工具触达即载子目录指令（PA-152）：只读发现，按目录浅→深排序。

use std::collections::BTreeSet;
use std::path::Path;

/// 一条待注入的子目录指令。
pub struct TouchedInstruction {
    pub display_path: String,
    pub content: String,
}

/// 单步触达注入的字节上限。
pub const MAX_TOUCH_STEP_BYTES: usize = 16_384;

/// 发现 workspace_root→touched_path 父目录链上各级的指令文件（首命中），跳过已注入路径。
#[allow(unused_variables)]
pub fn instructions_for_touched_path(
    workspace_root: &Path,
    touched_path: &Path,
    already_injected: &BTreeSet<String>,
) -> Vec<TouchedInstruction> {
    unimplemented!("wave42 stub: instructions_for_touched_path")
}
