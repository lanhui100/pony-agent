//! L2-AT 确定性破坏注入（红蓝对抗）——中断轮续跑。
//!
//! 三类注入，全部确定性（固定种子 / 固定迭代 / Barrier 同步，**零 sleep 盲等**）：
//! - `at1_oversized_malformed_event_sequence`：超长 + 畸形步骤序列（空名/换行/NUL/重复名/8KB 名/
//!   4KB 结果 ×500），断言闭合结构不截断、明细逐行有界、输出确定；
//! - `at2_repeated_recovery_race`：重复恢复竞态（固定种子 LCG + 固定线程数/迭代数 + Barrier），
//!   断言请求逐字确定、无重复注入、无跨线程污染；
//! - `at3_dirty_dsml_body_injection`：脏 DSML 正文注入（未闭合块 / 多块 / 半角变体 / 参数残片），
//!   断言协议片段绝不出现在持久化正文/展示文本/下一轮请求里。
//!
//! 失败原语一律是 SUT 栈（`pony_agent_core::agent::session::interrupted_turn`）的 panic/断言，
//! 不是编译错误、不是 harness 超时。

mod at1_oversized_malformed_event_sequence;
mod at2_repeated_recovery_race;
mod at3_dirty_dsml_body_injection;
