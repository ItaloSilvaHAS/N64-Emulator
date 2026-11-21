mod bus;
mod cpu;
mod rom;

use bus::Bus;
use cpu::Cpu;
use rom::Rom;
use std::env;

fn main() {
    let mut args = env::args().skip(1);
    let Some(rom_path) = args.next() else {
        eprintln!("Utilização: cargo run -- <caminho_da_rom>");
        return;
    };

    match Rom::from_file(&rom_path) {
        Ok(rom) => {
            println!("Formato: {:?}", rom.format);
            println!("Tamanho: {} bytes", rom.data.len());
            println!("Entry point: 0x{:08X}", rom.entry_point);

            let mut bus = Bus::new(rom);
            let mut cpu = Cpu::new(bus.rom.entry_point);

            for _ in 0..10 {
                cpu.step(&mut bus);
            }
        }
        Err(err) => eprintln!("Erro: {err}"),
    }
}