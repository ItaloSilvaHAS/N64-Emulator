use crate::bus::Bus;

pub struct Cpu {
    pub gpr: [u64; 32],
    pub pc: u64,
    pub hi: u64,
    pub lo: u64,
}

impl Cpu {
    pub fn new(entry_point: u32) -> Self {
        Self {
            gpr: [0; 32],
            pc: entry_point as u64,
            hi: 0,
            lo: 0,
        }
    }

    pub fn read_gpr(&self, idx: usize) -> u64 {
        if idx == 0 {
            0
        } else {
            self.gpr[idx & 31]
        }
    }

    pub fn write_gpr(&mut self, idx: usize, val: u64) {
        let reg = idx & 31;
        if reg != 0 {
            self.gpr[reg] = val;
        }
    }

    pub fn step(&mut self, bus: &mut Bus) {
        let instr = bus.read_u32(self.pc as u32);
        self.pc = self.pc.wrapping_add(4);

        let opcode = (instr >> 26) & 0x3F;
        let rs = ((instr >> 21) & 0x1F) as usize;
        let rt = ((instr >> 16) & 0x1F) as usize;
        let imm = (instr & 0xFFFF) as u16;
        let imm_sign_ext = (imm as i16) as i64 as u64;

        match opcode {
            // NOP / SPECIAL (0x00)
            0x00 => {
                if instr == 0 {
                    // NOP
                }
            }
            // ADDIU (0x09)
            0x09 => {
                let res = (self.read_gpr(rs) as i32).wrapping_add(imm_sign_ext as i32) as i64 as u64;
                self.write_gpr(rt, res);
            }
            // ORI (0x0D)
            0x0D => {
                let res = self.read_gpr(rs) | (imm as u64);
                self.write_gpr(rt, res);
            }
            // LUI (0x0F)
            0x0F => {
                let res = ((imm as i16 as i32) << 16) as i64 as u64;
                self.write_gpr(rt, res);
            }
            _ => {
                eprintln!(
                    "Opcode não implementado: 0x{:02X} (instr=0x{:08X}) em PC=0x{:08X}",
                    opcode,
                    instr,
                    self.pc.wrapping_sub(4)
                );
            }
        }
    }
}
