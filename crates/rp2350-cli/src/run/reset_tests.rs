//! `AIRCR.SYSRESETREQ` replay, restart policy and fault-enable tests
//! `AIRCR.SYSRESETREQ` 回放、重启策略与故障使能测试
//!
//! Every fixture is a synthetic Arm ELF with a flash vector table, so the whole
//! path runs: guest `str` → PPB replay log → `CpuCore::system_write` → core
//! fault/reset state → `Machine` host API.
//! 所有夹具都是带 Flash 向量表的合成 Arm ELF，因此整条路径都是真实的：客户机
//! `str` → PPB 回放日志 → `CpuCore::system_write` → 核内故障/复位状态 →
//! `Machine` 宿主 API。

use std::path::PathBuf;

use super::*;
use crate::run::reset::{MAX_SYSTEM_RESETS, SystemResetPolicy, execute};
use crate::test_support::build_elf;
use rp2350_core::{AddressBus, FLASH_XIP_BASE};

/// `SCB_AIRCR` / `SCB_AIRCR` 绝对地址
const SCB_AIRCR: u32 = 0xE000_ED0C;

/// `SCB_SHCSR` / `SCB_SHCSR` 绝对地址
const SCB_SHCSR: u32 = 0xE000_ED24;

/// `SCB_CFSR` / `SCB_CFSR` 绝对地址
const SCB_CFSR: u32 = 0xE000_ED28;

/// `SCB_HFSR` / `SCB_HFSR` 绝对地址
const SCB_HFSR: u32 = 0xE000_ED2C;

/// `SCB_CPACR` / `SCB_CPACR` 绝对地址
const SCB_CPACR: u32 = 0xE000_ED88;

/// `VECTKEY = 0x05FA` value / `VECTKEY = 0x05FA` 值
const AIRCR_VECTKEY: u32 = 0x05FA_0000;

/// `AIRCR.SYSRESETREQ` bit / `AIRCR.SYSRESETREQ` 位
const AIRCR_SYSRESETREQ: u32 = 1 << 2;

/// `SHCSR.USGFAULTENA` bit / `SHCSR.USGFAULTENA` 位
const SHCSR_USGFAULTENA: u32 = 1 << 18;

/// `CFSR.UFSR.NOCP` bit / `CFSR.UFSR.NOCP` 位
const CFSR_NOCP: u32 = 1 << 19;

/// `HFSR.FORCED` bit / `HFSR.FORCED` 位
const HFSR_FORCED: u32 = 1 << 30;

/// Reset entry offset inside the fixture image / 夹具映像中的复位入口偏移
const ENTRY_OFFSET: u32 = 0x100;

/// Handler offset inside the fixture image / 夹具映像中的处理程序偏移
const HANDLER_OFFSET: u32 = 0x180;

/// Initial stack pointer used by the fixture / 夹具使用的初始栈指针
const FIXTURE_SP: u32 = 0x2008_2000;

/// Put one 16-bit Thumb halfword into an image / 在映像中写入一个 16 位 Thumb 半字
fn put_half(image: &mut [u8], offset: usize, value: u16) {
    image[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

/// Put one 32-bit word into an image / 在映像中写入一个 32 位字
fn put_word(image: &mut [u8], offset: usize, value: u32) {
    image[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

/// Build an ELF whose program runs at `ENTRY_OFFSET` and whose fault vectors
/// all point at a single `b .` handler
/// 构建一个程序位于 `ENTRY_OFFSET`、故障向量都指向单个 `b .` 处理程序的 ELF
fn fixture_image(program: &[u16], literals: &[(usize, u32)]) -> Vec<u8> {
    let mut data = vec![0u8; 0x200];
    put_word(&mut data, 0x00, FIXTURE_SP);
    put_word(&mut data, 0x04, FLASH_XIP_BASE + ENTRY_OFFSET + 1);
    // HardFault (3) and UsageFault (6) both land in the spinning handler.
    // HardFault（3）与 UsageFault（6）都进入自旋处理程序。
    put_word(&mut data, 0x0C, (FLASH_XIP_BASE + HANDLER_OFFSET) | 1);
    put_word(&mut data, 0x18, (FLASH_XIP_BASE + HANDLER_OFFSET) | 1);
    for (index, halfword) in program.iter().enumerate() {
        put_half(&mut data, ENTRY_OFFSET as usize + index * 2, *halfword);
    }
    for (offset, value) in literals {
        put_word(&mut data, *offset, *value);
    }
    put_half(&mut data, HANDLER_OFFSET as usize, 0xE7FE);
    let entry = (FLASH_XIP_BASE + ENTRY_OFFSET) | 1;
    let memsz = data.len() as u32;
    build_elf(entry, &[(FLASH_XIP_BASE, &data, memsz)])
}

/// ELF that stores `value` into `SCB_AIRCR` and then spins
/// 向 `SCB_AIRCR` 写入 `value` 后自旋的 ELF
fn aircr_elf(value: u32) -> Vec<u8> {
    // ldr r0,[pc,#0x18] / ldr r1,[pc,#0x1C] / str r1,[r0] / b .
    // 字面量：0x11C = AIRCR 地址，0x120 = 写入值。
    // Literals: 0x11C = AIRCR address, 0x120 = value.
    fixture_image(
        &[0x4806, 0x4907, 0x6001, 0xE7FE],
        &[(0x11C, SCB_AIRCR), (0x120, value)],
    )
}

/// ELF that programs `SHCSR.USGFAULTENA`, clears `CPACR` and executes `VLDR`
/// 编程 `SHCSR.USGFAULTENA`、清 `CPACR` 后执行 `VLDR` 的 ELF
///
/// The `VLDR` encoding is the same used by the `rp2350-arm` NOCP tests, so a
/// denied coprocessor access is the only fault source in this image.
/// `VLDR` 编码与 `rp2350-arm` 的 NOCP 测试相同，因此本映像唯一的故障源就是协处理器
/// 访问被拒绝。
fn nocp_elf(enable_usagefault: bool) -> Vec<u8> {
    let shcsr = if enable_usagefault {
        SHCSR_USGFAULTENA
    } else {
        0
    };
    fixture_image(
        &[
            0x4806, // ldr r0,[pc,#0x18] -> SHCSR
            0x4907, // ldr r1,[pc,#0x1C] -> enable value
            0x6001, // str r1,[r0]
            0x4807, // ldr r0,[pc,#0x1C] -> CPACR
            0x4907, // ldr r1,[pc,#0x1C] -> 0
            0x6001, // str r1,[r0]
            0xED90, 0x0A00, // vldr (denied by CPACR) / 被 CPACR 拒绝
            0xE7FE, // b .
        ],
        &[
            (0x11C, SCB_SHCSR),
            (0x120, shcsr),
            (0x124, SCB_CPACR),
            (0x128, 0),
        ],
    )
}

/// Machine options for the synthetic Arm fixtures / 合成 Arm 夹具的机器选项
fn options(max_cycles: u64) -> MachineOptions {
    MachineOptions {
        arch: Some(ArchArg::Arm),
        max_cycles,
        strict_decode: false,
        psram: None,
    }
}

/// `run` options for the synthetic fixtures / 合成夹具的 `run` 选项
fn run_options(max_cycles: u64) -> RunOptions {
    RunOptions {
        file: PathBuf::from("synthetic.elf"),
        arch: Some(ArchArg::Arm),
        max_cycles,
        trace_steps: 0,
        strict_decode: false,
        sleep_fast_forward: None,
        core_threads: CoreThreadOptions::default(),
        psram: None,
        dumps: Vec::new(),
    }
}

/// Write one word through the machine bus / 通过机器总线写入一个字
fn write_word(machine: &mut Machine, addr: u32, value: u32) {
    let mut cycles = 0u64;
    machine
        .bus
        .write_u32(addr, value, &mut cycles)
        .expect("bus write");
}

#[test]
fn guest_aircr_write_reaches_the_core_and_latches_a_reset() {
    let elf = aircr_elf(AIRCR_VECTKEY | AIRCR_SYSRESETREQ);
    let mut machine = Machine::new(&elf, &options(64)).expect("machine");
    machine.run_cycles(16);

    assert!(
        machine.system_reset_pending(),
        "SYSRESETREQ must stop the machine / SYSRESETREQ 必须让机器停止推进"
    );
    // AIRCR 读视图：VECTKEYSTAT 恒为 0xFA05，只写位读 0。
    // AIRCR read view: VECTKEYSTAT is always 0xFA05, write-only bits read 0.
    assert_eq!(machine.system_register(SCB_AIRCR), Some(0xFA05_0000));
    assert!(machine.take_system_reset_request(), "first take sees it");
    assert!(
        !machine.take_system_reset_request(),
        "the latch is one-shot / 锁存为一次性"
    );
}

#[test]
fn aircr_write_without_vectkey_is_ignored_by_the_machine() {
    let elf = aircr_elf(AIRCR_SYSRESETREQ);
    let mut machine = Machine::new(&elf, &options(64)).expect("machine");
    machine.run_cycles(16);

    assert!(
        !machine.system_reset_pending(),
        "a write without VECTKEY must be ignored / 无 VECTKEY 的写入必须被忽略"
    );
    assert_eq!(machine.system_register(SCB_AIRCR), Some(0xFA05_0000));
    assert!(!machine.take_system_reset_request());
    // 机器未被复位请求拦停，仍能继续执行（自旋）。
    // The machine was not stopped by a reset request and keeps running.
    machine.run_cycles(16);
    assert_eq!(machine.steps(), 32);
}

#[test]
fn shcsr_replay_routes_nocp_to_usagefault() {
    let elf = nocp_elf(true);
    let mut machine = Machine::new(&elf, &options(128)).expect("machine");
    machine.run_cycles(32);

    assert_eq!(
        machine.xpsr() & 0x1FF,
        6,
        "USGFAULTENA must route NOCP to UsageFault / USGFAULTENA 必须把 NOCP 送入 UsageFault"
    );
    assert_ne!(
        machine.system_register(SCB_CFSR).unwrap_or(0) & CFSR_NOCP,
        0,
        "CFSR.NOCP is recorded / 记录 CFSR.NOCP"
    );
    assert_eq!(
        machine.system_register(SCB_HFSR).unwrap_or(0) & HFSR_FORCED,
        0,
        "an enabled UsageFault is not escalated / 已使能的 UsageFault 不升级"
    );
    assert_eq!(
        machine.pc(),
        FLASH_XIP_BASE + HANDLER_OFFSET,
        "the UsageFault vector is taken / 取 UsageFault 向量"
    );
}

#[test]
fn cfsr_and_hfsr_clears_match_between_ppb_and_core_paths() {
    // 先用未使能的 UsageFault 触发升级，HFSR.FORCED 与 CFSR.NOCP 同时置位；
    // 然后分别经 PPB 回放与直接 system_write 写 1 清零，结果必须一致。
    // First trigger an escalated fault so HFSR.FORCED and CFSR.NOCP are set;
    // then clear them through the PPB replay and through a direct system_write;
    // the results must match.
    let elf = nocp_elf(false);

    let mut via_ppb = Machine::new(&elf, &options(128)).expect("ppb machine");
    via_ppb.run_cycles(32);
    assert_ne!(
        via_ppb.system_register(SCB_HFSR).unwrap_or(0) & HFSR_FORCED,
        0
    );
    assert_ne!(
        via_ppb.system_register(SCB_CFSR).unwrap_or(0) & CFSR_NOCP,
        0
    );
    write_word(&mut via_ppb, SCB_HFSR, HFSR_FORCED);
    write_word(&mut via_ppb, SCB_CFSR, CFSR_NOCP);
    // 一步即可在 `after_running_step` 中回放 PPB 写入。
    // One step replays the PPB writes in `after_running_step`.
    via_ppb.step().expect("replay step");

    let mut via_core = Machine::new(&elf, &options(128)).expect("core machine");
    via_core.run_cycles(32);
    assert_ne!(
        via_core.system_register(SCB_HFSR).unwrap_or(0) & HFSR_FORCED,
        0
    );
    assert_ne!(
        via_core.system_register(SCB_CFSR).unwrap_or(0) & CFSR_NOCP,
        0
    );
    via_core.core.system_write(SCB_HFSR, HFSR_FORCED);
    via_core.core.system_write(SCB_CFSR, CFSR_NOCP);

    let ppb_hfsr = via_ppb.system_register(SCB_HFSR).expect("HFSR");
    let core_hfsr = via_core.system_register(SCB_HFSR).expect("HFSR");
    assert_eq!(ppb_hfsr, core_hfsr, "HFSR write-one-to-clear parity");
    assert_eq!(ppb_hfsr & HFSR_FORCED, 0);
    assert_eq!(
        via_ppb.system_register(SCB_CFSR),
        via_core.system_register(SCB_CFSR),
        "CFSR write-one-to-clear parity"
    );
    assert_eq!(
        via_ppb.system_register(SCB_CFSR).unwrap_or(0) & CFSR_NOCP,
        0
    );

    // 写 0 不清除任何位：两条路径再次保持一致。
    // Writing zero clears nothing: both paths stay identical again.
    write_word(&mut via_ppb, SCB_HFSR, 0);
    write_word(&mut via_ppb, SCB_CFSR, 0);
    via_ppb.step().expect("zero-write replay step");
    via_core.core.system_write(SCB_HFSR, 0);
    via_core.core.system_write(SCB_CFSR, 0);
    assert_eq!(
        via_ppb.system_register(SCB_HFSR),
        via_core.system_register(SCB_HFSR)
    );
    assert_eq!(
        via_ppb.system_register(SCB_CFSR),
        via_core.system_register(SCB_CFSR)
    );
}

#[test]
fn reboot_restores_counters_and_reruns_the_same_image() {
    let elf = aircr_elf(AIRCR_VECTKEY | AIRCR_SYSRESETREQ);
    let machine_options = options(64);
    let mut machine = Machine::new(&elf, &machine_options).expect("machine");
    machine.run_cycles(16);
    assert!(machine.take_system_reset_request());
    let before = machine.steps();
    assert!(before > 0);

    machine.reboot(&elf, &machine_options).expect("reboot");
    assert_eq!(machine.steps(), 0, "the step counter restarts at zero");
    assert_eq!(
        machine.pc(),
        FLASH_XIP_BASE + ENTRY_OFFSET,
        "PC is back at the image entry / PC 回到映像入口"
    );
    assert!(machine.irqs().is_empty(), "diagnostics are reset");
    assert!(!machine.notices_pending());
    assert_eq!(machine.sleep_counters(), ([0, 0], [0, 0]));
    assert!(!machine.system_reset_pending());

    // 同一映像再次运行并再次请求复位，证明复位后确实从头执行。
    // The same image runs again and requests the reset again, proving execution
    // really restarted from the beginning.
    machine.run_cycles(16);
    assert!(machine.take_system_reset_request());
}

#[test]
fn run_cycles_stops_advancing_once_a_reset_is_requested() {
    // `repl` 与 `play` 的停止策略依赖这一点：锁存后 `run_cycles` 不再前进。
    // The `repl`/`play` stop policy relies on this: after the latch,
    // `run_cycles` makes no further progress.
    let elf = aircr_elf(AIRCR_VECTKEY | AIRCR_SYSRESETREQ);
    let mut machine = Machine::new(&elf, &options(1_000)).expect("machine");
    machine.run_cycles(16);
    assert!(machine.system_reset_pending());
    let frozen = machine.steps();
    machine.run_cycles(1_000);
    assert_eq!(machine.steps(), frozen);
    assert!(machine.system_reset_pending(), "the latch survives polling");
    assert!(machine.take_system_reset_request());
}

#[test]
fn reset_policy_grants_three_restarts_then_reports_a_storm() {
    let mut policy = SystemResetPolicy::new();
    assert_eq!(policy.record().expect("first"), 1);
    assert_eq!(policy.record().expect("second"), 2);
    assert_eq!(policy.record().expect("third"), MAX_SYSTEM_RESETS);
    let error = policy.record().expect_err("fourth request is a storm");
    let message = error.to_string();
    assert!(message.contains("reset storm"), "{message}");
    assert!(message.contains("AIRCR.SYSRESETREQ"), "{message}");
    assert!(
        message.contains(&(MAX_SYSTEM_RESETS + 1).to_string()),
        "{message}"
    );
}

#[test]
fn run_restarts_the_machine_and_stops_a_reset_storm() {
    // 一个每次启动都请求复位的映像：`run` 必须重启 3 次，第 4 次请求报错停止。
    // An image that requests a reset on every boot: `run` must restart three
    // times and fail on the fourth request.
    let elf = aircr_elf(AIRCR_VECTKEY | AIRCR_SYSRESETREQ);
    let options = run_options(64);
    let mut output = Vec::new();
    let error = execute(&elf, &options, &mut output).expect_err("reset storm");
    let text = String::from_utf8(output).expect("utf-8 output");

    assert_eq!(
        text.matches(SYSTEM_RESET_MESSAGE).count(),
        4,
        "one diagnostic per request / 每次请求一条诊断:\n{text}"
    );
    assert!(text.contains("[run] reboot 1/3"), "{text}");
    assert!(text.contains("[run] reboot 3/3"), "{text}");
    assert!(error.to_string().contains("reset storm"), "{error}");
}
