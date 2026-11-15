//! Cached instruction execution (decode-cache hit path)
//! 缓存指令执行（译码缓存命中路径）
//!
//! [`CortexM33Core::step_cached`] takes one [`decode_cache::Entry`] and executes
//! the instruction it describes. It lives in its own module so the crate root
//! keeps only the fetch/decode path, and so the two paths are easy to compare
//! side by side.
//! [`CortexM33Core::step_cached`] 接受一个 [`decode_cache::Entry`] 并执行它所描述的
//! 指令。它单独成模块，使 crate 根只保留取指/译码路径，两条路径也便于并排对照。
//!
//! The executors called here are the same ones the generic decoder uses
//! ([`crate::execute_thumb16::fast`] and
//! [`crate::execute_thumb32::execute_group_tag`]), so the cache only changes
//! *how* an instruction is dispatched, never what it does. Cycle accounting is
//! reproduced exactly, including the quirk that a **taken** branch books no
//! extra cycle because its generic arm returns before the shared tail.
//! 这里调用的执行器与通用译码器完全相同（[`crate::execute_thumb16::fast`] 与
//! [`crate::execute_thumb32::execute_group_tag`]），因此缓存只改变指令的**分派方式**，
//! 从不改变它做什么。周期记账被完整复现，包括「**已执行**的分支不额外记周期」这一
//! 细节——通用分支在共享尾部之前就返回了。

use rp2350_core::bus::AddressBus;
use rp2350_core::error::CoreError;

use crate::CortexM33Core;
use crate::conditions::Condition;
use crate::decode_cache::{self, Kind};
use crate::execute_thumb16::StepResult;
use crate::execute_thumb16::{execute_thumb16, fast};
use crate::execute_thumb32::{execute_group_tag, execute_thumb32};

impl CortexM33Core {
    /// Execute one instruction from a decoded cache entry
    /// 依据译码缓存条目执行一条指令
    ///
    /// The entry carries the instruction length, the raw word and the dispatch
    /// tag, so `PC` advancement, the operand fields and the branch targets are
    /// not recomputed. Cycle accounting matches the generic path exactly: the
    /// cache books the same two cycles per fetched halfword plus one per fetch.
    /// 条目携带指令长度、原始指令字与分派标签，因此 `PC` 推进、操作数字段与分支目标
    /// 都无需重算。周期记账与通用路径完全一致：缓存按每个取指半字记两个周期，外加
    /// 每次取指一个。
    #[inline]
    pub(crate) fn step_cached(
        &mut self,
        bus: &mut dyn AddressBus,
        cycles: &mut u64,
        entry: decode_cache::Entry,
    ) -> Result<StepResult, CoreError> {
        *cycles += entry.fetch_cycles();
        let old_pc = self.registers.pc;
        self.registers.pc = old_pc.wrapping_add(u32::from(entry.len));
        let word = entry.word;

        let result = match entry.kind {
            Kind::B16 => {
                // 分支指令在通用路径里提前返回，因此不额外记周期。
                // The branch arms return early in the generic decoder, so they
                // book no extra cycle.
                self.registers.pc = entry.aux;
                StepResult::BranchTaken
            }
            Kind::BCond16 => {
                let cond = Condition::from_bits(entry.group);
                if cond.evaluate(&self.registers) {
                    // 条件成立时通用路径提前返回，不记周期。
                    // A taken conditional branch returns early in the generic
                    // path and books no cycle.
                    self.registers.pc = entry.aux;
                    StepResult::BranchTaken
                } else {
                    *cycles += 1;
                    StepResult::Ok
                }
            }
            Kind::LdrLiteral16 => {
                let rd = ((word >> 8) & 0x7) as usize;
                self.registers.r[rd] = bus.read_u32(entry.aux, cycles)?;
                *cycles += 1;
                StepResult::Ok
            }
            Kind::LdrWordImm5 | Kind::StrWordImm5 => fast::load_store_word_imm5(
                word as u16,
                entry.kind == Kind::StrWordImm5,
                &mut self.registers,
                bus,
                cycles,
            )?,
            Kind::DpRegister16 => {
                fast::dp_register(word as u16, &mut self.registers);
                *cycles += 1;
                StepResult::Ok
            }
            Kind::Hint16 => {
                // 提示空间按原路径记账：它不额外计入周期。
                // The hint space books no extra cycle, exactly as before.
                fast::hint_space(word as u16, &mut self.it_state).unwrap_or(StepResult::Ok)
            }
            Kind::Cps16 => {
                fast::cps(word as u16, &mut self.registers);
                *cycles += 1;
                StepResult::Ok
            }
            Kind::Group32 => {
                let result =
                    execute_group_tag(entry.group, word, &mut self.registers, bus, cycles)?;
                *cycles += 1;
                result
            }
            Kind::Slow32 => {
                self.prepare_fpu_instruction(bus, cycles, word)?;
                execute_thumb32(
                    word,
                    &mut self.registers,
                    bus,
                    &mut self.nvic,
                    &mut self.systick,
                    &mut self.it_state,
                    cycles,
                )?
            }
            Kind::Slow16 | Kind::Empty => execute_thumb16(
                word as u16,
                &mut self.registers,
                bus,
                &mut self.nvic,
                &mut self.systick,
                &mut self.it_state,
                cycles,
            )?,
        };

        self.finish_instruction(bus, cycles, old_pc, word, entry.len, result)
    }
}
