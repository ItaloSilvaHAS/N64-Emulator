use crate::rom::{Rom, RomFormat};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct GameEntry {
    pub title: String,
    pub rom_path: PathBuf,
    pub cover_path: Option<PathBuf>,
    pub format: RomFormat,
    pub file_size: u64,
}

#[derive(Debug)]
pub struct GameLibrary {
    pub games: Vec<GameEntry>,
}

impl GameLibrary {
    pub fn scan<P: AsRef<Path>>(directory: P) -> Result<Self, String> {
        let directory = directory.as_ref();
        let entries = fs::read_dir(directory).map_err(|error| {
            format!("Não foi possível abrir '{}': {error}", directory.display())
        })?;
        let mut rom_paths = Vec::new();
        let mut cover_paths = Vec::new();

        for entry in entries {
            let entry =
                entry.map_err(|error| format!("Falha ao ler a pasta da biblioteca: {error}"))?;
            let path = entry.path();
            if path.is_dir()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.eq_ignore_ascii_case("capas"))
            {
                let cover_entries = fs::read_dir(&path).map_err(|error| {
                    format!(
                        "Não foi possível abrir a pasta de capas '{}': {error}",
                        path.display()
                    )
                })?;
                for cover_entry in cover_entries {
                    let cover_path = cover_entry
                        .map_err(|error| format!("Falha ao ler a pasta de capas: {error}"))?
                        .path();
                    if cover_path.is_file() && is_cover_file(&cover_path) {
                        cover_paths.push(cover_path);
                    }
                }
                continue;
            }
            if !path.is_file() {
                continue;
            }

            match path
                .extension()
                .and_then(|extension| extension.to_str())
                .map(str::to_ascii_lowercase)
                .as_deref()
            {
                Some("z64" | "v64" | "n64") => rom_paths.push(path),
                Some("png" | "jpg" | "jpeg" | "webp") => cover_paths.push(path),
                _ => {}
            }
        }

        rom_paths.sort();
        cover_paths.sort();

        let mut games = Vec::with_capacity(rom_paths.len());
        for rom_path in rom_paths {
            let rom = Rom::from_file(&rom_path)
                .map_err(|error| format!("{}: {error}", rom_path.display()))?;
            let header_title = rom.title();
            let file_title = rom_path
                .file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .split(" (")
                .next()
                .unwrap_or_default()
                .trim();
            let display_title = if file_title.is_empty() {
                if header_title.is_empty() {
                    "Jogo sem título".to_string()
                } else {
                    header_title
                }
            } else {
                file_title.to_string()
            };
            let cover_path = find_cover(&display_title, &cover_paths);
            let file_size = fs::metadata(&rom_path)
                .map_err(|error| format!("Não foi possível ler '{}': {error}", rom_path.display()))?
                .len();

            games.push(GameEntry {
                title: display_title,
                rom_path,
                cover_path,
                format: rom.format,
                file_size,
            });
        }

        Ok(Self { games })
    }
}

fn is_cover_file(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("png" | "jpg" | "jpeg" | "webp")
    )
}

fn find_cover(title: &str, cover_paths: &[PathBuf]) -> Option<PathBuf> {
    let title_words = meaningful_words(title);
    if title_words.is_empty() {
        return None;
    }

    let mut best_match: Option<(&PathBuf, usize)> = None;
    let mut tied = false;

    for cover_path in cover_paths {
        let cover_name = cover_path.file_stem()?.to_str()?;
        let cover_words = meaningful_words(cover_name);
        let overlap = title_words
            .iter()
            .filter(|word| cover_words.contains(word))
            .count();

        if overlap < 2 {
            continue;
        }

        match best_match {
            None => {
                best_match = Some((cover_path, overlap));
                tied = false;
            }
            Some((_, best_overlap)) if overlap > best_overlap => {
                best_match = Some((cover_path, overlap));
                tied = false;
            }
            Some((_, best_overlap)) if overlap == best_overlap => tied = true,
            _ => {}
        }
    }

    if tied {
        None
    } else {
        best_match.map(|(path, _)| path.clone())
    }
}

fn meaningful_words(value: &str) -> Vec<String> {
    value
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| {
            !word.is_empty()
                && !matches!(
                    word.to_ascii_lowercase().as_str(),
                    "the" | "of" | "a" | "an" | "and" | "for" | "to"
                )
        })
        .map(str::to_ascii_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let id = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "n64-emulator-library-test-{}-{id}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn sample_rom(title: &str) -> Vec<u8> {
        let mut data = vec![0; 0x40];
        data[0..4].copy_from_slice(&[0x80, 0x37, 0x12, 0x40]);
        let title_bytes = title.as_bytes();
        let length = title_bytes.len().min(20);
        data[0x20..0x20 + length].copy_from_slice(&title_bytes[..length]);
        data
    }

    #[test]
    fn scan_reads_game_title_and_matches_cover() {
        let directory = TestDirectory::new();
        fs::write(
            directory.0.join("Super Mario 64 (Europe).z64"),
            sample_rom("SUPER MARIO 64"),
        )
        .unwrap();
        let covers = directory.0.join("Capas");
        fs::create_dir(&covers).unwrap();
        fs::write(covers.join("Super-Mario-64.png"), b"test image").unwrap();

        let library = GameLibrary::scan(&directory.0).unwrap();

        assert_eq!(library.games.len(), 1);
        assert_eq!(library.games[0].title, "Super Mario 64");
        assert_eq!(
            library.games[0]
                .cover_path
                .as_ref()
                .unwrap()
                .file_name()
                .unwrap(),
            "Super-Mario-64.png"
        );
        assert_eq!(library.games[0].format, RomFormat::Z64);
        assert_eq!(library.games[0].file_size, 0x40);
    }

    #[test]
    fn scan_reports_roms_without_a_matching_cover() {
        let directory = TestDirectory::new();
        fs::write(
            directory.0.join("Super Mario 64.z64"),
            sample_rom("SUPER MARIO 64"),
        )
        .unwrap();

        let library = GameLibrary::scan(&directory.0).unwrap();

        assert!(library.games[0].cover_path.is_none());
    }

    #[test]
    fn scan_rejects_invalid_n64_rom_files() {
        let directory = TestDirectory::new();
        fs::write(directory.0.join("broken.z64"), b"not a rom").unwrap();

        let error = GameLibrary::scan(&directory.0).unwrap_err();

        assert!(error.contains("broken.z64"));
        assert!(error.contains("muito pequeno"));
    }
}
