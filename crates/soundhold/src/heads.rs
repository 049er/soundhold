use crate::settings::user_folder;
use anyhow::{anyhow, bail, Context, Result};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

pub const DEFAULT: &str = "HRIR_FULL2DEG.sofa";
const INSTALLED_FOLDER: &str = "/usr/share/soundhold";
const EXTENSION: &str = "sofa";
const HDF5_SIGNATURE: &[u8; 8] = b"\x89HDF\r\n\x1a\n";

fn shipped_folder() -> PathBuf {
    let installed = PathBuf::from(INSTALLED_FOLDER);
    if !cfg!(debug_assertions) || installed.join(DEFAULT).exists() {
        installed
    } else {
        PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../sofa"))
    }
}

fn user_heads_folder() -> Result<PathBuf> {
    Ok(user_folder("XDG_DATA_HOME", ".local/share")?.join("soundhold/heads"))
}

fn folders() -> Vec<PathBuf> {
    std::iter::once(shipped_folder()).chain(user_heads_folder().ok()).collect()
}

fn names_in(folders: &[PathBuf]) -> Vec<String> {
    let mut names: Vec<String> = folders
        .iter()
        .filter_map(|folder| std::fs::read_dir(folder).ok())
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e == EXTENSION))
        .filter_map(|path| Some(path.file_name()?.to_string_lossy().into_owned()))
        .collect();
    names.sort_by_key(|name| (name != DEFAULT, name.clone()));
    names.dedup();
    names
}

fn find_in(folders: &[PathBuf], name: &str) -> Option<PathBuf> {
    if Path::new(name).file_name()? != name {
        return None;
    }
    folders.iter().map(|folder| folder.join(name)).find(|path| path.is_file())
}

/// The file for `name`, or the default when no name is given or the file is gone.
pub fn resolve(name: Option<&str>) -> Result<PathBuf> {
    let folders = folders();
    name.and_then(|n| find_in(&folders, n)).or_else(|| find_in(&folders, DEFAULT)).ok_or_else(|| anyhow!("{DEFAULT} not found"))
}

pub fn list() -> Vec<String> {
    names_in(&folders())
}

fn safe_name(name: &str) -> bool {
    name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ' '))
}

fn add_to(source: &Path, shipped: &Path, destination: &Path) -> Result<String> {
    let name = source.file_name().ok_or_else(|| anyhow!("the file has no name"))?.to_string_lossy().into_owned();
    if source.extension().is_none_or(|e| e != EXTENSION) {
        bail!("{name} is not a .sofa file");
    }
    if !safe_name(&name) {
        bail!("{name} has characters other than letters, digits, spaces, dots, dashes and underscores");
    }
    let mut signature = [0u8; 8];
    File::open(source).and_then(|mut file| file.read_exact(&mut signature)).with_context(|| format!("could not read {name}"))?;
    if signature != *HDF5_SIGNATURE {
        bail!("{name} is not a valid .sofa file");
    }
    if shipped.join(&name).exists() {
        bail!("{name} is already included");
    }
    std::fs::create_dir_all(destination)?;
    std::fs::copy(source, destination.join(&name))?;
    Ok(name)
}

/// Copies a head data file into the user's folder and returns its name.
pub fn add(source: &Path) -> Result<String> {
    add_to(source, &shipped_folder(), &user_heads_folder()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_folder(label: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("soundhold-test-{}-{label}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    #[test]
    fn add_to_accepts_only_sofa_files() {
        let source = empty_folder("source");
        let shipped = empty_folder("shipped");
        let destination = empty_folder("destination");
        std::fs::write(shipped.join("included.sofa"), HDF5_SIGNATURE).unwrap();
        let cases: [(&str, &[u8], Result<&str, &str>); 5] = [
            ("mine.sofa", HDF5_SIGNATURE, Ok("mine.sofa")),
            ("mine.wav", HDF5_SIGNATURE, Err("mine.wav is not a .sofa file")),
            ("text.sofa", b"not a sofa file", Err("text.sofa is not a valid .sofa file")),
            ("included.sofa", HDF5_SIGNATURE, Err("included.sofa is already included")),
            ("a\"b.sofa", HDF5_SIGNATURE, Err("a\"b.sofa has characters other than letters, digits, spaces, dots, dashes and underscores")),
        ];
        for (name, content, expected) in cases {
            let file = source.join(name);
            std::fs::write(&file, content).unwrap();
            match (add_to(&file, &shipped, &destination), expected) {
                (Ok(found), Ok(wanted)) => {
                    assert_eq!(found, wanted);
                    assert!(destination.join(wanted).is_file());
                }
                (Err(found), Err(wanted)) => assert_eq!(found.to_string(), wanted),
                (found, wanted) => panic!("{name}: got {found:?}, expected {wanted:?}"),
            }
        }
    }

    #[test]
    fn default_is_listed_first_and_paths_cannot_leave_the_folders() {
        let folder = empty_folder("list");
        for name in ["a.sofa", DEFAULT, "ignored.txt"] {
            std::fs::write(folder.join(name), b"").unwrap();
        }
        let folders = [folder.clone()];
        assert_eq!(names_in(&folders), vec![DEFAULT.to_string(), "a.sofa".to_string()]);
        assert_eq!(find_in(&folders, "a.sofa"), Some(folder.join("a.sofa")));
        assert_eq!(find_in(&folders, "../list/a.sofa"), None);
    }
}
