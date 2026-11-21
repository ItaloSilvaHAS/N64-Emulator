use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RomFormat {
    Z64,
    V64,
    N64,
}

#[derive(Debug)]
pub struct Rom {
    pub data: Vec<u8>,
    pub format: RomFormat,
    pub entry_point: u32,
}

impl Rom {
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        let mut raw = fs::read(path).map_err(|e| format!("Falha ao ler ROM: {e}"))?;
        if raw.len() < 0x40 {
            return Err("Arquivo muito pequeno para ser uma ROM de N64".to_string());
        }

        let format = match (raw[0], raw[1], raw[2], raw[3]) {
            (0x80, 0x37, 0x12, 0x40) => RomFormat::Z64,
            (0x37, 0x80, 0x40, 0x12) => {
                for chunk in raw.chunks_exact_mut(2) {
                    chunk.swap(0, 1);
                }
                RomFormat::V64
            }
            (0x40, 0x12, 0x37, 0x80) => {
                for chunk in raw.chunks_exact_mut(4) {
                    chunk.reverse();
                }
                RomFormat::N64
            }
            (a, b, c, d) => {
                return Err(format!(
                    "Formato de ROM desconhecido: {:02X} {:02X} {:02X} {:02X}",
                    a, b, c, d
                ));
            }
        };

        let entry_point = u32::from_be_bytes([raw[0x08], raw[0x09], raw[0x0A], raw[0x0B]]);

        Ok(Self {
            data: raw,
            format,
            entry_point,
        })
    }
}