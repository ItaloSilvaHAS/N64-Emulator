use crate::rom::Rom;
use std::fmt;

const RDRAM_SIZE: usize = 8 * 1024 * 1024; // 8 MB

#[derive(Debug, PartialEq, Eq)]
pub enum BusError {
    UnmappedRead { address: u32, bytes: u8 },
    UnmappedWrite { address: u32, bytes: u8 },
    ReadOnlyWrite { address: u32, bytes: u8 },
}

impl fmt::Display for BusError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnmappedRead { address, bytes } => write!(
                formatter,
                "leitura de {bytes} bytes em endereço não mapeado 0x{address:08X}"
            ),
            Self::UnmappedWrite { address, bytes } => write!(
                formatter,
                "escrita de {bytes} bytes em endereço não mapeado 0x{address:08X}"
            ),
            Self::ReadOnlyWrite { address, bytes } => write!(
                formatter,
                "tentativa de escrita de {bytes} bytes na ROM somente leitura em 0x{address:08X}"
            ),
        }
    }
}

impl std::error::Error for BusError {}

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

    fn read_bytes(&self, addr: u32, length: usize) -> Result<&[u8], BusError> {
        let paddr = self.translate_addr(addr) as usize;
        let width = length as u8;
        let end = paddr.checked_add(length).ok_or(BusError::UnmappedRead {
            address: addr,
            bytes: width,
        })?;

        if paddr < self.rdram.len() {
            return self.rdram.get(paddr..end).ok_or(BusError::UnmappedRead {
                address: addr,
                bytes: width,
            });
        }

        // Cartucho / ROM física começa em 0x1000_0000.
        const ROM_BASE: usize = 0x1000_0000;
        if paddr >= ROM_BASE {
            let rom_offset = paddr - ROM_BASE;
            let rom_end = rom_offset
                .checked_add(length)
                .ok_or(BusError::UnmappedRead {
                    address: addr,
                    bytes: width,
                })?;
            return self
                .rom
                .data
                .get(rom_offset..rom_end)
                .ok_or(BusError::UnmappedRead {
                    address: addr,
                    bytes: width,
                });
        }

        Err(BusError::UnmappedRead {
            address: addr,
            bytes: width,
        })
    }

    pub fn read_u8(&self, addr: u32) -> Result<u8, BusError> {
        Ok(self.read_bytes(addr, 1)?[0])
    }

    pub fn read_u16(&self, addr: u32) -> Result<u16, BusError> {
        let bytes = self.read_bytes(addr, 2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    pub fn read_u32(&self, addr: u32) -> Result<u32, BusError> {
        let bytes = self.read_bytes(addr, 4)?;
        Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    pub fn read_u64(&self, addr: u32) -> Result<u64, BusError> {
        let bytes = self.read_bytes(addr, 8)?;
        Ok(u64::from_be_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    fn write_bytes(&mut self, addr: u32, bytes: &[u8]) -> Result<(), BusError> {
        let paddr = self.translate_addr(addr) as usize;
        let width = bytes.len() as u8;
        let end = paddr
            .checked_add(bytes.len())
            .ok_or(BusError::UnmappedWrite {
                address: addr,
                bytes: width,
            })?;

        if paddr < self.rdram.len() {
            if end > self.rdram.len() {
                return Err(BusError::UnmappedWrite {
                    address: addr,
                    bytes: width,
                });
            }
            self.rdram[paddr..end].copy_from_slice(bytes);
            return Ok(());
        }

        const ROM_BASE: usize = 0x1000_0000;
        if paddr >= ROM_BASE {
            let rom_offset = paddr - ROM_BASE;
            if rom_offset
                .checked_add(bytes.len())
                .is_some_and(|rom_end| rom_end <= self.rom.data.len())
            {
                return Err(BusError::ReadOnlyWrite {
                    address: addr,
                    bytes: width,
                });
            }
        }

        Err(BusError::UnmappedWrite {
            address: addr,
            bytes: width,
        })
    }

    pub fn write_u8(&mut self, addr: u32, value: u8) -> Result<(), BusError> {
        self.write_bytes(addr, &[value])
    }

    pub fn write_u16(&mut self, addr: u32, value: u16) -> Result<(), BusError> {
        self.write_bytes(addr, &value.to_be_bytes())
    }

    pub fn write_u32(&mut self, addr: u32, value: u32) -> Result<(), BusError> {
        self.write_bytes(addr, &value.to_be_bytes())
    }

    pub fn write_u64(&mut self, addr: u32, value: u64) -> Result<(), BusError> {
        self.write_bytes(addr, &value.to_be_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rom::RomFormat;

    fn test_bus() -> Bus {
        Bus::new(Rom {
            data: vec![0; 0x40],
            format: RomFormat::Z64,
            entry_point: 0,
        })
    }

    #[test]
    fn ram_reads_and_writes_use_big_endian_order() {
        let mut bus = test_bus();
        let address = 0x8000_0100;

        bus.write_u64(address, 0x0123_4567_89AB_CDEF).unwrap();

        assert_eq!(bus.read_u8(address).unwrap(), 0x01);
        assert_eq!(bus.read_u16(address).unwrap(), 0x0123);
        assert_eq!(bus.read_u32(address).unwrap(), 0x0123_4567);
        assert_eq!(bus.read_u64(address).unwrap(), 0x0123_4567_89AB_CDEF);
    }

    #[test]
    fn rom_is_readable_but_not_writable_through_the_bus() {
        let mut bus = test_bus();
        bus.rom.data[0x10..0x14].copy_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);

        assert_eq!(bus.read_u32(0x1000_0010).unwrap(), 0xDEAD_BEEF);
        assert_eq!(
            bus.write_u32(0x1000_0010, 0),
            Err(BusError::ReadOnlyWrite {
                address: 0x1000_0010,
                bytes: 4,
            })
        );
        assert_eq!(bus.read_u32(0x1000_0010).unwrap(), 0xDEAD_BEEF);
    }

    #[test]
    fn unmapped_accesses_return_errors_instead_of_zero_or_silent_writes() {
        let mut bus = test_bus();

        assert_eq!(
            bus.read_u32(0x0800_0000),
            Err(BusError::UnmappedRead {
                address: 0x0800_0000,
                bytes: 4,
            })
        );
        assert_eq!(
            bus.write_u8(0x0800_0000, 0xFF),
            Err(BusError::UnmappedWrite {
                address: 0x0800_0000,
                bytes: 1,
            })
        );
    }
}
