// [B064] Windows ConPTY 隔离驱动空桩
// 计划：通过 ConPTY API 实现真实的伪终端采集，替换当前的 Pipe 模拟。
// 目前：只包含基础特征定义的 Interface 与占位实现，确保编译通过。

pub trait ConPtyDriver {
    fn create_pseudo_console(width: u16, height: u16) -> Result<Self, String> where Self: Sized;
    fn spawn_process(command: &str) -> Result<u32, String>;
}

pub struct WindowsPtyDriver;

impl ConPtyDriver for WindowsPtyDriver {
    fn create_pseudo_console(_width: u16, _height: u16) -> Result<Self, String> {
        // TODO: 实现 ConPTY API (CreatePseudoConsole) 的调用
        Err("Not implemented".to_string())
    }

    fn spawn_process(_command: &str) -> Result<u32, String> {
        // TODO: 实现进程生成并挂载至 PseudoConsole
        Err("Not implemented".to_string())
    }
}
