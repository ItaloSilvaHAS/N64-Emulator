mod bus;
mod cpu;
mod library;
mod rom;

use bus::Bus;
use cpu::{Cpu, StepOutcome};
use library::GameLibrary;
use rom::Rom;
use std::env;

fn main() {
    let mut args = env::args().skip(1);
    let Some(argument) = args.next() else {
        print_usage();
        return;
    };

    if argument == "--library" {
        let directory = args.next().unwrap_or_else(|| "Room".to_string());
        if let Err(error) = print_library(&directory) {
            eprintln!("Erro: {error}");
            std::process::exit(1);
        }
        return;
    }

    match Rom::from_file(&argument) {
        Ok(rom) => {
            println!("Formato: {:?}", rom.format);
            println!("Título: {}", rom.title());
            println!("Tamanho: {} bytes", rom.data.len());
            println!("Entry point: 0x{:08X}", rom.entry_point);

            let mut bus = Bus::new(rom);
            let mut cpu = Cpu::new(bus.rom.entry_point);

            for step in 0..10 {
                match cpu.step(&mut bus) {
                    Ok(StepOutcome::Executed) => {}
                    Ok(StepOutcome::Exception { code, vector }) => {
                        println!(
                            "CPU entrou na exceção {code:?} no passo {}, vetor 0x{vector:08X}",
                            step + 1
                        );
                        break;
                    }
                    Err(error) => {
                        eprintln!("Falha do emulador no passo {}: {error}", step + 1);
                        break;
                    }
                }
            }
        }
        Err(err) => eprintln!("Erro: {err}"),
    }
}

fn print_library(directory: &str) -> Result<(), String> {
    let library = GameLibrary::scan(directory)?;
    println!("Biblioteca: {}", directory);
    println!("Jogos encontrados: {}", library.games.len());

    for game in library.games {
        let cover = game
            .cover_path
            .as_deref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "não encontrada".to_string());
        println!(
            "\n{}\n  ROM: {}\n  Capa: {}\n  Formato: {:?} | {} bytes",
            game.title,
            game.rom_path.display(),
            cover,
            game.format,
            game.file_size
        );
    }

    Ok(())
}

fn print_usage() {
    eprintln!("Utilização:");
    eprintln!("  cargo run -- <caminho_da_rom>");
    eprintln!("  cargo run -- --library [pasta_da_biblioteca]");
}
