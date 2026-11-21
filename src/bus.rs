use crate::rom::Rom;

const RDRAM_SIZE: usize = 8 * 1024 * 1024; // 8 MB

pub struct Bus {
    pub rdram: Vec<u8>,
    pub rom: Rom,
}

impl Bus {
    pub fn new(rom: Rom) -> Self {
        Self {
            rdram: vec![0; RDRAM_SIZE],
            rom,
        }
    }

    fn translate_addr(&self, addr: u32) -> u32 {
        // Remove bits de segmento MIPS comuns (KSEG0 0x80000000 / KSEG1 0xA0000000 -> endereços físicos de 29 bits)
        match addr {
            0x80000000..=0x9FFFFFFF | 0xA0000000..=0xBFFFFFFF => addr & 0x1FFF_FFFF,
            _ => addr,
        }
    }

    pub fn read_u32(&self, addr: u32) -> u32 {
        let paddr = self.translate_addr(addr) as usize;

        // RDRAM física: 0x0000_0000 .. 0x007F_FFFF
        if paddr + 3 < RDRAM_SIZE {
            return u32::from_be_bytes([
                self.rdram[paddr],
                self.rdram[paddr + 1],
                self.rdram[paddr + 2],
                self.rdram[paddr + 3],
            ]);
        }

        // Cartucho / ROM física: 0x1000_0000 ..
        const ROM_BASE: usize = 0x1000_0000;
        if paddr >= ROM_BASE {
            let rom_offset = paddr - ROM_BASE;
            if rom_offset + 3 < self.rom.data.len() {
                return u32::from_be_bytes([
                    self.rom.data[rom_offset],
                    self.rom.data[rom_offset + 1],
                    self.rom.data[rom_offset + 2],
                    self.rom.data[rom_offset + 3],
                ]);
            }
        }

        0
    }

    pub fn write_u32(&mut self, addr: u32, value: u32) {
        let paddr = self.translate_addr(addr) as usize;

        if paddr + 3 < RDRAM_SIZE {
            let bytes = value.to_be_bytes();
            self.rdram[paddr] = bytes[0];
            self.rdram[paddr + 1] = bytes[1];
            self.rdram[paddr + 2] = bytes[2];
            self.rdram[paddr + 3] = bytes[3];
        }
    }
}
