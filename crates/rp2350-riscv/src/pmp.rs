//! Physical memory protection (PMP) / 物理内存保护
//!
//! Hazard3 implements 16 PMP entries. RV32 exposes them through the
//! even-numbered `pmpcfg0/2/4/6` registers (four 8-bit descriptors each) and
//! `pmpaddr0..15`. This module decodes the descriptors and enforces them on the
//! fetch, load and store paths.
//! Hazard3 实现 16 个 PMP 条目。RV32 通过偶数编号的 `pmpcfg0/2/4/6`（每个含四个
//! 8 位描述符）与 `pmpaddr0..15` 暴露它们。本模块解析描述符并在取指、加载、存储
//! 路径上执行检查。
//!
//! # Privilege rules / 特权级规则
//!
//! The RISC-V privileged specification applies PMP checks to every access whose
//! effective privilege mode is S or U, and to M-mode accesses only when the
//! matching entry has `L = 1` (locked). M-mode data accesses use `MPP` as their
//! effective privilege when `mstatus.MPRV` is set; instruction fetches always
//! use the current privilege mode.
//! RISC-V 特权规范要求对所有有效特权级为 S/U 的访问做 PMP 检查；M 模式访问只有
//! 在匹配条目的 `L = 1`（锁定）时才受限。`mstatus.MPRV` 置位时，M 模式数据访问
//! 按其 `MPP` 的有效特权级检查；取指始终使用当前特权级。
//!
//! # Locking / 锁定
//!
//! A locked entry ignores writes to its descriptor byte and to `pmpaddr_i`.
//! A locked TOR entry additionally protects `pmpaddr_{i-1}`, because that word
//! forms the lower bound of the range. Ignored writes raise no exception.
//! 锁定条目忽略对其描述符字节与 `pmpaddr_i` 的写入；锁定的 TOR 条目还会保护
//! `pmpaddr_{i-1}`（该字构成区间下界）。被忽略的写入不会触发异常。

use crate::csr::CsrFile;

/// Number of PMP entries implemented / 实现的 PMP 条目数
pub(crate) const PMP_ENTRIES: usize = 16;

/// PMP access type / PMP 访问类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PmpAccess {
    /// Instruction fetch / 取指
    Fetch,
    /// Data load / 数据载入
    Load,
    /// Data store / 数据存储
    Store,
}

/// `R` permission bit in a descriptor byte / 描述符中的 `R` 权限位
const PMP_R: u8 = 1 << 0;

/// `W` permission bit in a descriptor byte / 描述符中的 `W` 权限位
const PMP_W: u8 = 1 << 1;

/// `X` permission bit in a descriptor byte / 描述符中的 `X` 权限位
const PMP_X: u8 = 1 << 2;

/// Shift of the `A` matching-mode field / `A` 匹配模式字段的位移
const PMP_A_SHIFT: u8 = 3;

/// Mask of the `A` matching-mode field / `A` 匹配模式字段的掩码
const PMP_A_MASK: u8 = 0b11;

/// `L` lock bit in a descriptor byte / 描述符中的 `L` 锁定位
const PMP_L: u8 = 1 << 7;

/// `A = TOR`: top of range / `A = TOR`：区间上界
const PMP_A_TOR: u8 = 1;

/// `A = NA4`: naturally aligned four-byte range / `A = NA4`：自然对齐 4 字节区间
const PMP_A_NA4: u8 = 2;

/// `A = NAPOT`: naturally aligned power-of-two range / `A = NAPOT`：自然对齐 2 的幂区间
const PMP_A_NAPOT: u8 = 3;

impl CsrFile {
    /// Return descriptor byte `index` from the packed `pmpcfg` registers
    /// 从打包的 `pmpcfg` 寄存器返回第 `index` 个描述符字节
    pub(crate) fn pmp_cfg_byte(&self, index: usize) -> u8 {
        (self.pmpcfg[index / 4] >> (8 * (index % 4))) as u8
    }

    /// Report whether entry `index` is locked / 报告第 `index` 个条目是否锁定
    fn pmp_locked(&self, index: usize) -> bool {
        self.pmp_cfg_byte(index) & PMP_L != 0
    }

    /// Return the `A` matching mode of entry `index` / 返回条目的 `A` 匹配模式
    fn pmp_mode(&self, index: usize) -> u8 {
        (self.pmp_cfg_byte(index) >> PMP_A_SHIFT) & PMP_A_MASK
    }

    /// Write one `pmpcfg` register, honouring per-entry locks
    /// 写入一个 `pmpcfg` 寄存器，遵守逐条目锁定
    ///
    /// Reserved bits 6:5 are kept as written (WARL storage), matching the
    /// register's role as configuration storage.
    /// 保留位 6:5 按写入值保留（WARL 存储），与寄存器作为配置存储的定位一致。
    pub(crate) fn pmp_write_cfg(&mut self, register: usize, value: u32) {
        let mut merged = self.pmpcfg[register];
        for entry in 0..4 {
            let shift = 8 * entry;
            if (merged >> shift) & u32::from(PMP_L) != 0 {
                continue;
            }
            merged = (merged & !(0xFF << shift)) | (value & (0xFF << shift));
        }
        self.pmpcfg[register] = merged;
    }

    /// Write one `pmpaddr` register, honouring entry and TOR locks
    /// 写入一个 `pmpaddr` 寄存器，遵守条目锁定与 TOR 锁定
    pub(crate) fn pmp_write_addr(&mut self, index: usize, value: u32) {
        if self.pmp_locked(index) {
            return;
        }
        // A locked TOR entry owns the preceding address register as its lower
        // bound. / 锁定的 TOR 条目把前一个地址寄存器作为下界一并保护。
        if index + 1 < PMP_ENTRIES
            && self.pmp_locked(index + 1)
            && self.pmp_mode(index + 1) == PMP_A_TOR
        {
            return;
        }
        self.pmpaddr[index] = value;
    }

    /// Return the physical address range of entry `index`, if it is enabled
    /// 返回条目的物理地址区间（若条目启用）
    ///
    /// The range is expressed in 64-bit units because the highest NAPOT entry
    /// can cover the full 34-bit physical address space.
    /// 区间以 64 位表示，因为最高的 NAPOT 条目可覆盖完整的 34 位物理地址空间。
    fn pmp_region(&self, index: usize) -> Option<(u64, u64)> {
        match self.pmp_mode(index) {
            0 => None,
            PMP_A_TOR => {
                let start = if index == 0 {
                    0
                } else {
                    u64::from(self.pmpaddr[index - 1]) << 2
                };
                let end = u64::from(self.pmpaddr[index]) << 2;
                // An empty or inverted range matches nothing.
                // 空区间或反向区间不匹配任何地址。
                (end > start).then_some((start, end - start))
            }
            PMP_A_NA4 => Some((u64::from(self.pmpaddr[index]) << 2, 4)),
            PMP_A_NAPOT => {
                // `n` trailing ones encode a `2^(n+3)`-byte range.
                // `n` 个尾部 1 编码 `2^(n+3)` 字节的区间。
                let ones = self.pmpaddr[index].trailing_ones().min(31);
                let low_mask = (1u64 << (ones + 1)) - 1;
                let base = (u64::from(self.pmpaddr[index]) & !low_mask) << 2;
                Some((base, 1u64 << (ones + 3)))
            }
            _ => None,
        }
    }

    /// Report whether an access is permitted by the PMP unit
    /// 报告 PMP 单元是否允许一次访问
    ///
    /// `size` is the access width in bytes. The lowest-numbered entry that
    /// covers the whole access decides; a partial overlap fails. `machine` is
    /// the effective privilege (`false` for S/U, including M-mode data accesses
    /// redirected by `mstatus.MPRV`).
    /// `size` 为访问宽度（字节）。覆盖整个访问的最低编号条目起决定作用；仅部分
    /// 重叠视为失败。`machine` 为有效特权级（S/U 为 `false`，包括由
    /// `mstatus.MPRV` 重定向的 M 模式数据访问）。
    pub(crate) fn pmp_permits(
        &self,
        addr: u32,
        size: u32,
        access: PmpAccess,
        machine: bool,
    ) -> bool {
        // Fast path: all descriptors at their reset value means no entry is
        // enabled, so no check can fail. / 快速路径：所有描述符均为复位值时没有
        // 任何条目启用，检查不可能失败。
        if self.pmpcfg == [0; 4] {
            return machine;
        }
        let start = u64::from(addr);
        let end = start + u64::from(size);
        for index in 0..PMP_ENTRIES {
            let Some((base, length)) = self.pmp_region(index) else {
                continue;
            };
            let region_end = base + length;
            if start >= region_end || base >= end {
                continue;
            }
            // The lowest-numbered entry matching any byte decides; it must
            // cover the whole access or the access fails (privileged spec
            // 3.7.1.3). / 匹配任一字节的最低编号条目起决定作用；它必须覆盖整个
            // 访问，否则访问失败（特权规范 3.7.1.3）。
            if start < base || end > region_end {
                return false;
            }
            let descriptor = self.pmp_cfg_byte(index);
            // An unlocked entry never restricts M-mode but still grants
            // permissions to S/U. / 未锁定条目不限制 M 模式，但仍向 S/U 授予权限。
            if machine && descriptor & PMP_L == 0 {
                return true;
            }
            return match access {
                PmpAccess::Fetch => descriptor & PMP_X != 0,
                PmpAccess::Load => descriptor & PMP_R != 0,
                PmpAccess::Store => descriptor & PMP_W != 0,
            };
        }
        // No entry matched: M-mode keeps full access, S/U has none.
        // 无条目匹配：M 模式保持完全访问权限，S/U 无任何权限。
        machine
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a CSR file with one NAPOT entry covering `[base, base + size)`
    /// 构建一个 CSR 文件，其 NAPOT 条目覆盖 `[base, base + size)`
    fn with_napot(base: u32, size: u32, descriptor: u8) -> CsrFile {
        let mut csr = CsrFile::new();
        let ones = size.trailing_zeros() - 3;
        csr.pmpaddr[0] = (base >> 2) | ((1 << ones) - 1);
        csr.pmpcfg[0] = u32::from(descriptor);
        csr
    }

    #[test]
    fn tor_and_napot_ranges_are_decoded() {
        let mut csr = CsrFile::new();
        // TOR region [0x2000_0400, 0x2000_0800). TOR [0x2000_0400, 0x2000_0800)。
        csr.pmpaddr[0] = 0x2000_0400 >> 2;
        csr.pmpaddr[1] = 0x2000_0800 >> 2;
        csr.pmpcfg[0] = u32::from(PMP_L | PMP_R | (PMP_A_TOR << PMP_A_SHIFT)) << 8;
        assert!(csr.pmp_permits(0x2000_0400, 4, PmpAccess::Load, false));
        assert!(csr.pmp_permits(0x2000_07FC, 4, PmpAccess::Load, false));
        assert!(!csr.pmp_permits(0x2000_0800, 4, PmpAccess::Load, false));
        assert!(!csr.pmp_permits(0x2000_03FC, 4, PmpAccess::Load, false));
        // A four-byte access straddling the upper bound must fail.
        // 跨过上界的 4 字节访问必须失败。
        assert!(!csr.pmp_permits(0x2000_07FE, 4, PmpAccess::Load, false));

        let napot = with_napot(
            0x2000_0800,
            256,
            PMP_L | PMP_R | (PMP_A_NAPOT << PMP_A_SHIFT),
        );
        assert!(napot.pmp_permits(0x2000_0800, 4, PmpAccess::Load, false));
        assert!(napot.pmp_permits(0x2000_08FC, 4, PmpAccess::Load, false));
        assert!(!napot.pmp_permits(0x2000_0900, 4, PmpAccess::Load, false));
    }

    #[test]
    fn unlocked_entries_do_not_restrict_m_mode() {
        // An unlocked entry ignores M-mode permissions but still governs U-mode.
        // 未锁定条目不限制 M 模式，但仍约束 U 模式。
        let csr = with_napot(0x2000_0800, 256, PMP_R | (PMP_A_NAPOT << PMP_A_SHIFT));
        assert!(csr.pmp_permits(0x2000_0800, 4, PmpAccess::Store, true));
        assert!(!csr.pmp_permits(0x2000_0800, 4, PmpAccess::Store, false));
        // Locked without the W bit: a store is denied even in M-mode.
        // 锁定但无 W 位：即使 M 模式存储也被拒绝。
        let locked = with_napot(
            0x2000_0800,
            256,
            PMP_L | PMP_R | (PMP_A_NAPOT << PMP_A_SHIFT),
        );
        assert!(!locked.pmp_permits(0x2000_0800, 4, PmpAccess::Store, true));
        assert!(locked.pmp_permits(0x2000_0800, 4, PmpAccess::Load, true));
    }

    #[test]
    fn partial_overlap_with_a_lower_entry_fails() {
        // Entry 0 (NA4 at 0x804) matches one byte of a 4-byte access at 0x802;
        // entry 1 (16-byte NAPOT at 0x800) would cover all of it, but the
        // lowest-numbered partial match must fail the access.
        // 条目 0（0x804 的 NA4）只匹配 0x802 处 4 字节访问的一部分；条目 1
        // （0x800 的 16 字节 NAPOT）可完整覆盖，但最低编号的部分匹配必须使访问失败。
        let mut csr = CsrFile::new();
        csr.pmpaddr[0] = 0x804 >> 2;
        csr.pmpaddr[1] = (0x800 >> 2) | 1;
        csr.pmpcfg[0] = u32::from(PMP_R | (PMP_A_NA4 << PMP_A_SHIFT))
            | (u32::from(PMP_R | (PMP_A_NAPOT << PMP_A_SHIFT)) << 8);
        assert!(csr.pmp_permits(0x800, 4, PmpAccess::Load, true));
        assert!(!csr.pmp_permits(0x802, 4, PmpAccess::Load, true));
    }

    #[test]
    fn locked_entries_ignore_descriptor_and_address_writes() {
        let mut csr = with_napot(0x2000_0800, 256, PMP_L | (PMP_A_NAPOT << PMP_A_SHIFT));
        let cfg = csr.pmpcfg[0];
        let addr = csr.pmpaddr[0];
        csr.pmp_write_cfg(0, 0);
        csr.pmp_write_addr(0, 0);
        assert_eq!(csr.pmpcfg[0], cfg);
        assert_eq!(csr.pmpaddr[0], addr);
    }

    #[test]
    fn locked_tor_entry_protects_the_preceding_address() {
        let mut csr = CsrFile::new();
        csr.pmpaddr[0] = 0x1234;
        csr.pmpaddr[1] = 0x5678;
        csr.pmpcfg[0] = u32::from(PMP_L | (PMP_A_TOR << PMP_A_SHIFT)) << 8;
        csr.pmp_write_addr(0, 0xDEAD);
        csr.pmp_write_addr(1, 0xDEAD);
        assert_eq!(csr.pmpaddr[0], 0x1234);
        assert_eq!(csr.pmpaddr[1], 0x5678);
    }
}
