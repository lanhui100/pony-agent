// [B070] 稳健数据目录隔离实现
// 计划：统一封装测试环境的数据路径注入，通过 `PONY_AGENT_DATA_DIR` 旁路实现。

use std::env;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

pub struct PonyAgentTestContext {
    pub data_dir: TempDir,
}

impl PonyAgentTestContext {
    pub fn new() -> Result<Self, std::io::Error> {
        let temp_dir = tempfile::tempdir()?;
        env::set_var("PONY_AGENT_DATA_DIR", temp_dir.path());
        Ok(Self { data_dir: temp_dir })
    }

    pub fn get_data_dir(&self) -> &Path {
        self.data_dir.path()
    }
}
