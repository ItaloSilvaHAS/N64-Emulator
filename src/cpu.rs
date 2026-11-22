use crate::bus::{Bus, BusError};
use std::fmt;

pub struct Cpu {
    pub gpr: [u64; 32],
    pub pc: u64,
    pub next_pc: u64,
    pub hi: u64,
    pub lo: u64,
    pub cp0: Cp0,
    in_delay_slot: bool,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Cp0 {
    pub bad_vaddr: u64,
    pub status: u64,
    pub cause: u64,
    pub epc: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ExceptionCode {
    AddressLoad = 4,
    AddressStore = 5,
    InstructionBusError = 6,
    DataBusError = 7,
    Syscall = 8,
    Breakpoint = 9,
    ReservedInstruction = 10,
    ArithmeticOverflow = 12,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepOutcome {
    Executed,
    Exception { code: ExceptionCode, vector: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusAccessKind {
    InstructionFetch,
    Load,
    Store,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CpuError {
    UnsupportedInstruction {
        pc: u64,
        instruction: u32,
    },
    ArithmeticOverflow {
        pc: u64,
        instruction: u32,
    },
    MisalignedMemoryAccess {
        pc: u64,
        address: u32,
        bytes: u8,
        write: bool,
    },
    MisalignedInstructionFetch {
        pc: u64,
    },
    BusAccess {
        pc: u64,
        source: BusError,
        kind: BusAccessKind,
    },
    GuestException {
        pc: u64,
        code: ExceptionCode,
    },
}

impl fmt::Display for CpuError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedInstruction { pc, instruction } => write!(
                formatter,
                "instrução não implementada 0x{instruction:08X} no PC 0x{pc:08X}"
            ),
            Self::ArithmeticOverflow { pc, instruction } => write!(
                formatter,
                "overflow aritmético na instrução 0x{instruction:08X} no PC 0x{pc:08X}"
            ),
            Self::MisalignedMemoryAccess {
                pc,
                address,
                bytes,
                write: is_write,
            } => write!(
                formatter,
                "acesso de {} de {bytes} bytes desalinhado no endereço 0x{address:08X}, PC 0x{pc:08X}",
                if *is_write { "escrita" } else { "leitura" }
            ),
            Self::MisalignedInstructionFetch { pc } => {
                write!(formatter, "busca de instrução desalinhada no PC 0x{pc:08X}")
            }
            Self::BusAccess { pc, source, .. } => {
                write!(formatter, "falha de barramento no PC 0x{pc:08X}: {source}")
            }
            Self::GuestException { pc, code } => {
                write!(formatter, "exceção {code:?} no PC 0x{pc:08X}")
            }
        }
    }
}

impl std::error::Error for CpuError {}

impl Cpu {
    pub fn new(entry_point: u32) -> Self {
        let pc = entry_point as u64;
        Self {
            gpr: [0; 32],
            pc,
            next_pc: pc.wrapping_add(4),
            hi: 0,
            lo: 0,
            cp0: Cp0::default(),
            in_delay_slot: false,
        }
    }

    pub fn read_gpr(&self, idx: usize) -> u64 {
        if idx == 0 { 0 } else { self.gpr[idx & 31] }
    }

    pub fn write_gpr(&mut self, idx: usize, val: u64) {
        let reg = idx & 31;
        if reg != 0 {
            self.gpr[reg] = val;
        }
    }

    pub fn step(&mut self, bus: &mut Bus) -> Result<StepOutcome, CpuError> {
        let instruction_pc = self.pc;
        let was_in_delay_slot = self.in_delay_slot;
        match self.execute_instruction(bus) {
            Ok(()) => Ok(StepOutcome::Executed),
            Err(error) => {
                let (code, bad_vaddr) = match &error {
                    CpuError::UnsupportedInstruction { .. } => {
                        (ExceptionCode::ReservedInstruction, None)
                    }
                    CpuError::ArithmeticOverflow { .. } => {
                        (ExceptionCode::ArithmeticOverflow, None)
                    }
                    CpuError::MisalignedInstructionFetch { pc } => {
                        (ExceptionCode::AddressLoad, Some(*pc))
                    }
                    CpuError::MisalignedMemoryAccess { address, write, .. } => (
                        if *write {
                            ExceptionCode::AddressStore
                        } else {
                            ExceptionCode::AddressLoad
                        },
                        Some(*address as u64),
                    ),
                    CpuError::BusAccess { kind, .. } => (
                        if *kind == BusAccessKind::InstructionFetch {
                            ExceptionCode::InstructionBusError
                        } else {
                            ExceptionCode::DataBusError
                        },
                        None,
                    ),
                    CpuError::GuestException { code, .. } => (*code, None),
                };
                Ok(self.enter_exception(instruction_pc, was_in_delay_slot, code, bad_vaddr))
            }
        }
    }

    fn enter_exception(
        &mut self,
        instruction_pc: u64,
        was_in_delay_slot: bool,
        code: ExceptionCode,
        bad_vaddr: Option<u64>,
    ) -> StepOutcome {
        const STATUS_EXL: u64 = 1 << 1;
        const STATUS_BEV: u64 = 1 << 22;
        const CAUSE_BD: u64 = 1 << 31;
        const CAUSE_EXCCODE: u64 = 0x1F << 2;

        if self.cp0.status & STATUS_EXL == 0 {
            self.cp0.epc = if was_in_delay_slot {
                instruction_pc.wrapping_sub(4)
            } else {
                instruction_pc
            };
        }

        if let Some(address) = bad_vaddr {
            self.cp0.bad_vaddr = address;
        }

        self.cp0.cause &= !(CAUSE_BD | CAUSE_EXCCODE);
        self.cp0.cause |= (code as u64) << 2;
        if was_in_delay_slot {
            self.cp0.cause |= CAUSE_BD;
        }

        self.cp0.status |= STATUS_EXL;
        let vector = if self.cp0.status & STATUS_BEV != 0 {
            0xBFC0_0380
        } else {
            0x8000_0180
        };
        self.pc = vector as u64;
        self.next_pc = self.pc.wrapping_add(4);
        self.in_delay_slot = false;

        StepOutcome::Exception { code, vector }
    }

    fn execute_instruction(&mut self, bus: &mut Bus) -> Result<(), CpuError> {
        let instruction_pc = self.pc;
        let address = instruction_pc as u32;
        if address % 4 != 0 {
            return Err(CpuError::MisalignedInstructionFetch { pc: instruction_pc });
        }
        let instruction = bus.read_u32(address).map_err(|source| {
            bus_access(instruction_pc, source, BusAccessKind::InstructionFetch)
        })?;
        let next_pc = self.next_pc.wrapping_add(4);
        let mut branch_target = None;
        let mut has_delay_slot = false;

        let opcode = instruction >> 26;
        let rs = ((instruction >> 21) & 0x1F) as usize;
        let rt = ((instruction >> 16) & 0x1F) as usize;
        let immediate = instruction as u16;
        let signed_immediate = immediate as i16 as i64;

        match opcode {
            0x00 if instruction == 0 => {}
            0x00 => {
                has_delay_slot = matches!(instruction & 0x3F, 0x08 | 0x09);
                self.execute_special(instruction, instruction_pc, &mut branch_target)?;
            }
            0x01 => {
                has_delay_slot = true;
                let signed_value = self.read_gpr(rs) as i64;
                let (take_branch, link) = match rt {
                    0x00 => (signed_value < 0, false),  // BLTZ
                    0x01 => (signed_value >= 0, false), // BGEZ
                    0x10 => (signed_value < 0, true),   // BLTZAL
                    0x11 => (signed_value >= 0, true),  // BGEZAL
                    _ => return Err(unsupported(instruction_pc, instruction)),
                };
                if take_branch {
                    if link {
                        self.write_gpr(31, instruction_pc.wrapping_add(8));
                    }
                    branch_target = Some(branch_address(instruction_pc, immediate));
                }
            }
            0x02 | 0x03 => {
                has_delay_slot = true;
                let target = instruction & 0x03FF_FFFF;
                branch_target =
                    Some(((instruction_pc.wrapping_add(4)) & 0xF000_0000) | ((target as u64) << 2));
                if opcode == 0x03 {
                    self.write_gpr(31, instruction_pc.wrapping_add(8));
                }
            }
            0x04 | 0x05 => {
                has_delay_slot = true;
                let equal = self.read_gpr(rs) == self.read_gpr(rt);
                if equal == (opcode == 0x04) {
                    branch_target = Some(branch_address(instruction_pc, immediate));
                }
            }
            0x06 | 0x07 => {
                has_delay_slot = true;
                if rt != 0 {
                    return Err(unsupported(instruction_pc, instruction));
                }
                let value = self.read_gpr(rs) as i64;
                let take_branch = if opcode == 0x06 {
                    value <= 0
                } else {
                    value > 0
                };
                if take_branch {
                    branch_target = Some(branch_address(instruction_pc, immediate));
                }
            }
            0x08 => {
                let lhs = self.read_gpr(rs) as u32 as i32;
                let rhs = signed_immediate as i32;
                let result = lhs
                    .checked_add(rhs)
                    .ok_or_else(|| overflow(instruction_pc, instruction))?;
                self.write_gpr(rt, sign_extend_word(result as u32));
            }
            0x09 => {
                let lhs = self.read_gpr(rs) as u32 as i32;
                let result = lhs.wrapping_add(signed_immediate as i32);
                self.write_gpr(rt, sign_extend_word(result as u32));
            }
            0x0A => {
                let value = (self.read_gpr(rs) as i64) < signed_immediate;
                self.write_gpr(rt, u64::from(value));
            }
            0x0B => {
                let value = self.read_gpr(rs) < signed_immediate as u64;
                self.write_gpr(rt, u64::from(value));
            }
            0x0C => self.write_gpr(rt, self.read_gpr(rs) & immediate as u64),
            0x0D => self.write_gpr(rt, self.read_gpr(rs) | immediate as u64),
            0x0E => self.write_gpr(rt, self.read_gpr(rs) ^ immediate as u64),
            0x0F => {
                self.write_gpr(rt, sign_extend_word((immediate as u32) << 16));
            }
            0x20 | 0x21 | 0x23 | 0x24 | 0x25 | 0x27 | 0x37 => {
                let address = effective_address(self.read_gpr(rs), signed_immediate);
                let bytes = match opcode {
                    0x20 | 0x24 => 1,
                    0x21 | 0x25 => 2,
                    0x23 | 0x27 => 4,
                    0x37 => 8,
                    _ => unreachable!(),
                };
                check_alignment(instruction_pc, address, bytes, false)?;
                let value =
                    match opcode {
                        0x20 => bus.read_u8(address).map_err(|source| {
                            bus_access(instruction_pc, source, BusAccessKind::Load)
                        })? as i8 as i64 as u64, // LB
                        0x21 => bus.read_u16(address).map_err(|source| {
                            bus_access(instruction_pc, source, BusAccessKind::Load)
                        })? as i16 as i64 as u64, // LH
                        0x23 => bus.read_u32(address).map_err(|source| {
                            bus_access(instruction_pc, source, BusAccessKind::Load)
                        })? as i32 as i64 as u64, // LW
                        0x24 => bus.read_u8(address).map_err(|source| {
                            bus_access(instruction_pc, source, BusAccessKind::Load)
                        })? as u64, // LBU
                        0x25 => bus.read_u16(address).map_err(|source| {
                            bus_access(instruction_pc, source, BusAccessKind::Load)
                        })? as u64, // LHU
                        0x27 => bus.read_u32(address).map_err(|source| {
                            bus_access(instruction_pc, source, BusAccessKind::Load)
                        })? as u64, // LWU
                        0x37 => bus.read_u64(address).map_err(|source| {
                            bus_access(instruction_pc, source, BusAccessKind::Load)
                        })?, // LD
                        _ => unreachable!(),
                    };
                self.write_gpr(rt, value);
            }
            0x28 | 0x29 | 0x2B | 0x3F => {
                let address = effective_address(self.read_gpr(rs), signed_immediate);
                let bytes = match opcode {
                    0x28 => 1, // SB
                    0x29 => 2, // SH
                    0x2B => 4, // SW
                    0x3F => 8, // SD
                    _ => unreachable!(),
                };
                check_alignment(instruction_pc, address, bytes, true)?;
                let value = self.read_gpr(rt);
                let result = match opcode {
                    0x28 => bus.write_u8(address, value as u8),
                    0x29 => bus.write_u16(address, value as u16),
                    0x2B => bus.write_u32(address, value as u32),
                    0x3F => bus.write_u64(address, value),
                    _ => unreachable!(),
                };
                result
                    .map_err(|source| bus_access(instruction_pc, source, BusAccessKind::Store))?;
            }
            0x18 => {
                let lhs = self.read_gpr(rs) as i64;
                let result = lhs
                    .checked_add(signed_immediate)
                    .ok_or_else(|| overflow(instruction_pc, instruction))?;
                self.write_gpr(rt, result as u64);
            }
            0x19 => {
                self.write_gpr(rt, self.read_gpr(rs).wrapping_add(signed_immediate as u64));
            }
            _ => return Err(unsupported(instruction_pc, instruction)),
        }

        self.pc = self.next_pc;
        self.next_pc = branch_target.unwrap_or(next_pc);
        self.in_delay_slot = has_delay_slot;
        Ok(())
    }

    fn execute_special(
        &mut self,
        instruction: u32,
        instruction_pc: u64,
        branch_target: &mut Option<u64>,
    ) -> Result<(), CpuError> {
        let rs = ((instruction >> 21) & 0x1F) as usize;
        let rt = ((instruction >> 16) & 0x1F) as usize;
        let rd = ((instruction >> 11) & 0x1F) as usize;
        let shift = (instruction >> 6) & 0x1F;
        let function = instruction & 0x3F;
        let lhs = self.read_gpr(rs);
        let rhs = self.read_gpr(rt);

        match function {
            0x00 => self.write_gpr(rd, sign_extend_word((rhs as u32) << shift)),
            0x02 => self.write_gpr(rd, sign_extend_word((rhs as u32) >> shift)),
            0x03 => {
                let result = ((rhs as u32 as i32) >> shift) as u32;
                self.write_gpr(rd, sign_extend_word(result));
            }
            0x04 => self.write_gpr(rd, sign_extend_word((rhs as u32) << (lhs as u32 & 0x1F))),
            0x06 => self.write_gpr(rd, sign_extend_word((rhs as u32) >> (lhs as u32 & 0x1F))),
            0x07 => {
                let result = ((rhs as u32 as i32) >> (lhs as u32 & 0x1F)) as u32;
                self.write_gpr(rd, sign_extend_word(result));
            }
            0x08 => *branch_target = Some(lhs),
            0x09 => {
                *branch_target = Some(lhs);
                self.write_gpr(rd, instruction_pc.wrapping_add(8));
            }
            0x10 => self.write_gpr(rd, self.hi),
            0x11 => self.hi = lhs,
            0x12 => self.write_gpr(rd, self.lo),
            0x13 => self.lo = lhs,
            0x20 | 0x22 => {
                let left = lhs as u32 as i32;
                let right = rhs as u32 as i32;
                let result = if function == 0x20 {
                    left.checked_add(right)
                } else {
                    left.checked_sub(right)
                }
                .ok_or_else(|| overflow(instruction_pc, instruction))?;
                self.write_gpr(rd, sign_extend_word(result as u32));
            }
            0x21 => {
                let result = (lhs as u32).wrapping_add(rhs as u32);
                self.write_gpr(rd, sign_extend_word(result));
            }
            0x23 => {
                let result = (lhs as u32).wrapping_sub(rhs as u32);
                self.write_gpr(rd, sign_extend_word(result));
            }
            0x24 => self.write_gpr(rd, lhs & rhs),
            0x25 => self.write_gpr(rd, lhs | rhs),
            0x26 => self.write_gpr(rd, lhs ^ rhs),
            0x27 => self.write_gpr(rd, !(lhs | rhs)),
            0x2A => self.write_gpr(rd, u64::from((lhs as i64) < (rhs as i64))),
            0x2B => self.write_gpr(rd, u64::from(lhs < rhs)),
            0x2C | 0x2E => {
                let left = lhs as i64;
                let right = rhs as i64;
                let result = if function == 0x2C {
                    left.checked_add(right)
                } else {
                    left.checked_sub(right)
                }
                .ok_or_else(|| overflow(instruction_pc, instruction))?;
                self.write_gpr(rd, result as u64);
            }
            0x2D => self.write_gpr(rd, lhs.wrapping_add(rhs)),
            0x2F => self.write_gpr(rd, lhs.wrapping_sub(rhs)),
            0x38 => self.write_gpr(rd, rhs << shift),
            0x3A => self.write_gpr(rd, rhs >> shift),
            0x3B => self.write_gpr(rd, ((rhs as i64) >> shift) as u64),
            0x3C => self.write_gpr(rd, rhs << (shift + 32)),
            0x3E => self.write_gpr(rd, rhs >> (shift + 32)),
            0x3F => self.write_gpr(rd, ((rhs as i64) >> (shift + 32)) as u64),
            0x0C => {
                return Err(CpuError::GuestException {
                    pc: instruction_pc,
                    code: ExceptionCode::Syscall,
                });
            }
            0x0D => {
                return Err(CpuError::GuestException {
                    pc: instruction_pc,
                    code: ExceptionCode::Breakpoint,
                });
            }
            _ => return Err(unsupported(instruction_pc, instruction)),
        }

        Ok(())
    }
}

fn sign_extend_word(value: u32) -> u64 {
    (value as i32 as i64) as u64
}

fn effective_address(base: u64, signed_immediate: i64) -> u32 {
    base.wrapping_add(signed_immediate as u64) as u32
}

fn check_alignment(pc: u64, address: u32, bytes: u8, write: bool) -> Result<(), CpuError> {
    if address % bytes as u32 == 0 {
        Ok(())
    } else {
        Err(CpuError::MisalignedMemoryAccess {
            pc,
            address,
            bytes,
            write,
        })
    }
}

fn bus_access(pc: u64, source: BusError, kind: BusAccessKind) -> CpuError {
    CpuError::BusAccess { pc, source, kind }
}

fn branch_address(instruction_pc: u64, immediate: u16) -> u64 {
    instruction_pc
        .wrapping_add(4)
        .wrapping_add(((immediate as i16 as i64) << 2) as u64)
}

fn unsupported(pc: u64, instruction: u32) -> CpuError {
    CpuError::UnsupportedInstruction { pc, instruction }
}

fn overflow(pc: u64, instruction: u32) -> CpuError {
    CpuError::ArithmeticOverflow { pc, instruction }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rom::{Rom, RomFormat};

    const ENTRY: u32 = 0x8000_0000;

    fn system(instructions: &[u32]) -> (Cpu, Bus) {
        let rom = Rom {
            data: vec![0; 0x40],
            format: RomFormat::Z64,
            entry_point: ENTRY,
        };
        let mut bus = Bus::new(rom);
        for (index, instruction) in instructions.iter().enumerate() {
            bus.write_u32(ENTRY + (index as u32 * 4), *instruction)
                .unwrap();
        }
        (Cpu::new(ENTRY), bus)
    }

    fn r_type(rs: u32, rt: u32, rd: u32, shift: u32, function: u32) -> u32 {
        (rs << 21) | (rt << 16) | (rd << 11) | (shift << 6) | function
    }

    fn i_type(opcode: u32, rs: u32, rt: u32, immediate: u16) -> u32 {
        (opcode << 26) | (rs << 21) | (rt << 16) | immediate as u32
    }

    fn j_type(opcode: u32, target: u32) -> u32 {
        (opcode << 26) | ((target >> 2) & 0x03FF_FFFF)
    }

    #[test]
    fn addu_sign_extends_word_result_and_preserves_zero_register() {
        let instructions = [
            r_type(1, 2, 3, 0, 0x21), // ADDU $3, $1, $2
            r_type(1, 2, 0, 0, 0x21), // ADDU $0, $1, $2
        ];
        let (mut cpu, mut bus) = system(&instructions);
        cpu.write_gpr(1, 0x7FFF_FFFF);
        cpu.write_gpr(2, 1);

        cpu.step(&mut bus).unwrap();
        cpu.step(&mut bus).unwrap();

        assert_eq!(cpu.read_gpr(3), 0xFFFF_FFFF_8000_0000);
        assert_eq!(cpu.read_gpr(0), 0);
    }

    #[test]
    fn beq_executes_the_delay_slot_before_branching() {
        let instructions = [
            i_type(0x04, 1, 1, 2), // BEQ $1, $1, +2
            i_type(0x09, 0, 2, 7), // delay slot: ADDIU $2, $0, 7
            i_type(0x09, 0, 2, 9), // skipped
            i_type(0x09, 0, 3, 5), // branch target
        ];
        let (mut cpu, mut bus) = system(&instructions);
        cpu.write_gpr(1, 42);

        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.pc, ENTRY as u64 + 4);
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.read_gpr(2), 7);
        assert_eq!(cpu.pc, ENTRY as u64 + 12);
        cpu.step(&mut bus).unwrap();

        assert_eq!(cpu.read_gpr(2), 7);
        assert_eq!(cpu.read_gpr(3), 5);
    }

    #[test]
    fn jal_sets_the_return_address_and_runs_its_delay_slot() {
        let instructions = [
            j_type(0x03, ENTRY + 12), // JAL target
            i_type(0x09, 0, 2, 7),    // delay slot
            i_type(0x09, 0, 2, 9),    // skipped
            i_type(0x09, 0, 3, 5),    // target
        ];
        let (mut cpu, mut bus) = system(&instructions);

        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.read_gpr(31), ENTRY as u64 + 8);
        cpu.step(&mut bus).unwrap();
        cpu.step(&mut bus).unwrap();

        assert_eq!(cpu.read_gpr(2), 7);
        assert_eq!(cpu.read_gpr(3), 5);
    }

    #[test]
    fn immediate_instructions_use_their_correct_extension_rules() {
        let instructions = [
            i_type(0x09, 0, 1, u16::MAX), // ADDIU $1, $0, -1
            i_type(0x0C, 1, 2, 0x8001),   // ANDI $2, $1, 0x8001
            i_type(0x19, 4, 3, u16::MAX), // DADDIU $3, $4, -1
        ];
        let (mut cpu, mut bus) = system(&instructions);
        cpu.write_gpr(4, 0x1_0000_0000);

        for _ in 0..instructions.len() {
            cpu.step(&mut bus).unwrap();
        }

        assert_eq!(cpu.read_gpr(1), u64::MAX);
        assert_eq!(cpu.read_gpr(2), 0x8001);
        assert_eq!(cpu.read_gpr(3), 0xFFFF_FFFF);
    }

    #[test]
    fn jalr_sets_its_selected_link_register_and_runs_its_delay_slot() {
        let target = ENTRY + 12;
        let instructions = [
            r_type(1, 0, 5, 0, 0x09), // JALR $5, $1
            i_type(0x09, 0, 2, 7),    // delay slot
            i_type(0x09, 0, 2, 9),    // skipped
            i_type(0x09, 0, 3, 5),    // target
        ];
        let (mut cpu, mut bus) = system(&instructions);
        cpu.write_gpr(1, target as u64);

        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.read_gpr(5), ENTRY as u64 + 8);
        cpu.step(&mut bus).unwrap();
        cpu.step(&mut bus).unwrap();

        assert_eq!(cpu.read_gpr(2), 7);
        assert_eq!(cpu.read_gpr(3), 5);
    }

    #[test]
    fn loads_and_stores_handle_sign_extension_and_64_bit_values() {
        let instructions = [
            i_type(0x2B, 1, 2, 0),   // SW
            i_type(0x23, 1, 3, 0),   // LW
            i_type(0x28, 1, 4, 4),   // SB
            i_type(0x20, 1, 5, 4),   // LB
            i_type(0x24, 1, 6, 4),   // LBU
            i_type(0x3F, 1, 7, 8),   // SD
            i_type(0x37, 1, 8, 8),   // LD
            i_type(0x29, 1, 9, 18),  // SH
            i_type(0x21, 1, 10, 18), // LH
            i_type(0x25, 1, 11, 18), // LHU
        ];
        let (mut cpu, mut bus) = system(&instructions);
        cpu.write_gpr(1, 0x8000_0100);
        cpu.write_gpr(2, 0x1234_5678);
        cpu.write_gpr(4, 0x80);
        cpu.write_gpr(7, 0x0102_0304_0506_0708);
        cpu.write_gpr(9, 0x80FF);

        for _ in 0..instructions.len() {
            cpu.step(&mut bus).unwrap();
        }

        assert_eq!(cpu.read_gpr(3), 0x1234_5678);
        assert_eq!(cpu.read_gpr(5), 0xFFFF_FFFF_FFFF_FF80);
        assert_eq!(cpu.read_gpr(6), 0x80);
        assert_eq!(cpu.read_gpr(8), 0x0102_0304_0506_0708);
        assert_eq!(cpu.read_gpr(10), 0xFFFF_FFFF_FFFF_80FF);
        assert_eq!(cpu.read_gpr(11), 0x80FF);
    }

    #[test]
    fn misaligned_word_load_raises_address_error_and_records_bad_address() {
        let (mut cpu, mut bus) = system(&[i_type(0x23, 1, 2, 1)]);
        cpu.write_gpr(1, 0x8000_0100);

        assert_eq!(
            cpu.step(&mut bus).unwrap(),
            StepOutcome::Exception {
                code: ExceptionCode::AddressLoad,
                vector: 0x8000_0180,
            }
        );
        assert_eq!(cpu.cp0.bad_vaddr, 0x8000_0101);
        assert_eq!(cpu.cp0.epc, ENTRY as u64);
        assert_eq!((cpu.cp0.cause >> 2) & 0x1F, 4);
        assert_ne!(cpu.cp0.status & (1 << 1), 0);
        assert_eq!(cpu.pc, 0x8000_0180);
    }

    #[test]
    fn unmapped_load_raises_data_bus_error() {
        let (mut cpu, mut bus) = system(&[i_type(0x23, 1, 2, 0)]);
        cpu.write_gpr(1, 0x0800_0000);

        assert_eq!(
            cpu.step(&mut bus).unwrap(),
            StepOutcome::Exception {
                code: ExceptionCode::DataBusError,
                vector: 0x8000_0180,
            }
        );
        assert_eq!((cpu.cp0.cause >> 2) & 0x1F, 7);
        assert_eq!(cpu.cp0.epc, ENTRY as u64);
        assert_eq!(cpu.pc, 0x8000_0180);
    }

    #[test]
    fn store_to_cartridge_rom_raises_data_bus_error() {
        let (mut cpu, mut bus) = system(&[i_type(0x2B, 1, 2, 0)]);
        cpu.write_gpr(1, 0x1000_0010);
        cpu.write_gpr(2, 0x1234_5678);

        assert_eq!(
            cpu.step(&mut bus).unwrap(),
            StepOutcome::Exception {
                code: ExceptionCode::DataBusError,
                vector: 0x8000_0180,
            }
        );
        assert_eq!((cpu.cp0.cause >> 2) & 0x1F, 7);
        assert_eq!(cpu.cp0.epc, ENTRY as u64);
    }

    #[test]
    fn instruction_fetch_from_unmapped_address_raises_instruction_bus_error() {
        let (mut cpu, mut bus) = system(&[]);
        cpu.pc = 0x0800_0000;
        cpu.next_pc = cpu.pc + 4;

        assert_eq!(
            cpu.step(&mut bus).unwrap(),
            StepOutcome::Exception {
                code: ExceptionCode::InstructionBusError,
                vector: 0x8000_0180,
            }
        );
        assert_eq!((cpu.cp0.cause >> 2) & 0x1F, 6);
        assert_eq!(cpu.cp0.epc, 0x0800_0000);
    }

    #[test]
    fn misaligned_instruction_fetch_raises_address_error() {
        let (mut cpu, mut bus) = system(&[]);
        cpu.pc = ENTRY as u64 + 2;

        assert_eq!(
            cpu.step(&mut bus).unwrap(),
            StepOutcome::Exception {
                code: ExceptionCode::AddressLoad,
                vector: 0x8000_0180,
            }
        );
        assert_eq!(cpu.cp0.bad_vaddr, ENTRY as u64 + 2);
        assert_eq!(cpu.cp0.epc, ENTRY as u64 + 2);
    }

    #[test]
    fn signed_add_overflow_raises_overflow_exception() {
        let (mut cpu, mut bus) = system(&[r_type(1, 2, 3, 0, 0x20)]);
        cpu.write_gpr(1, i32::MAX as u64);
        cpu.write_gpr(2, 1);

        assert_eq!(
            cpu.step(&mut bus).unwrap(),
            StepOutcome::Exception {
                code: ExceptionCode::ArithmeticOverflow,
                vector: 0x8000_0180,
            }
        );
        assert_eq!((cpu.cp0.cause >> 2) & 0x1F, 12);
        assert_eq!(cpu.cp0.epc, ENTRY as u64);
    }

    #[test]
    fn unsupported_instruction_raises_reserved_instruction_exception() {
        let instruction = 0x4000_0000; // Coprocessador ainda não implementado
        let (mut cpu, mut bus) = system(&[instruction]);

        assert_eq!(
            cpu.step(&mut bus).unwrap(),
            StepOutcome::Exception {
                code: ExceptionCode::ReservedInstruction,
                vector: 0x8000_0180,
            }
        );
        assert_eq!((cpu.cp0.cause >> 2) & 0x1F, 10);
        assert_eq!(cpu.cp0.epc, ENTRY as u64);
        assert_eq!(cpu.pc, 0x8000_0180);
    }

    #[test]
    fn exception_in_branch_delay_slot_records_branch_pc_and_bd_bit() {
        let instructions = [
            i_type(0x04, 1, 1, 2), // BEQ
            0x4000_0000,           // instrução inválida no delay slot
            0,
            0,
        ];
        let (mut cpu, mut bus) = system(&instructions);
        cpu.write_gpr(1, 7);

        cpu.step(&mut bus).unwrap();
        let outcome = cpu.step(&mut bus).unwrap();

        assert_eq!(
            outcome,
            StepOutcome::Exception {
                code: ExceptionCode::ReservedInstruction,
                vector: 0x8000_0180,
            }
        );
        assert_eq!(cpu.cp0.epc, ENTRY as u64);
        assert_ne!(cpu.cp0.cause & (1 << 31), 0);
    }

    #[test]
    fn exception_keeps_existing_epc_when_exl_is_already_set() {
        let (mut cpu, mut bus) = system(&[0x4000_0000]);
        cpu.cp0.status |= 1 << 1;
        cpu.cp0.epc = 0x8123_4560;

        cpu.step(&mut bus).unwrap();

        assert_eq!(cpu.cp0.epc, 0x8123_4560);
    }

    #[test]
    fn bootstrap_vector_is_used_when_bev_is_set() {
        let (mut cpu, mut bus) = system(&[0x4000_0000]);
        cpu.cp0.status |= 1 << 22;

        let outcome = cpu.step(&mut bus).unwrap();

        assert_eq!(
            outcome,
            StepOutcome::Exception {
                code: ExceptionCode::ReservedInstruction,
                vector: 0xBFC0_0380,
            }
        );
    }

    #[test]
    fn syscall_and_break_raise_their_architectural_exceptions() {
        for (function, expected) in [
            (0x0C, ExceptionCode::Syscall),
            (0x0D, ExceptionCode::Breakpoint),
        ] {
            let (mut cpu, mut bus) = system(&[r_type(0, 0, 0, 0, function)]);

            assert_eq!(
                cpu.step(&mut bus).unwrap(),
                StepOutcome::Exception {
                    code: expected,
                    vector: 0x8000_0180,
                }
            );
            assert_eq!((cpu.cp0.cause >> 2) & 0x1F, expected as u64);
            assert_eq!(cpu.cp0.epc, ENTRY as u64);
        }
    }
}
