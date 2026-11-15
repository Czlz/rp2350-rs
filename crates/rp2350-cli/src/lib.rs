//! Library target of the RP2350 command-line front end
//! RP2350 命令行前端的库目标
//!
//! The binary (`src/main.rs`) only parses arguments and dispatches; everything
//! reusable lives here so `rp2350-egui` and other hosts can drive the same
//! machine. [`run`] owns [`run::Machine`] plus the subcommands that predate the
//! split; [`play`] owns the background frame and input worker.
//! 可执行文件（`src/main.rs`）只解析参数并分发；所有可复用部分都放在这里，使
//! `rp2350-egui` 等宿主能驱动同一台机器。[`run`] 持有 [`run::Machine`] 以及拆分
//! 前就存在的各个子命令；[`play`] 持有后台帧/输入 worker。

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

pub mod play;
pub mod run;

// 测试夹具只在测试构建中编译：`run` 的测试仍按 `crate::test_support` 引用它。
// Test fixtures are test-build only: `run`'s tests still import them as
// `crate::test_support`.
#[cfg(test)]
mod test_support;
