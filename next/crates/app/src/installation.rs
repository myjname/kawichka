//! Installation identity uses complete normalized paths, never display spelling.
use std::path::Path;

pub fn same_data_path(stored: &Path, running: &Path) -> bool {
    stored.is_absolute()
        && running.is_absolute()
        && kavychka_storage::transfer::owned_tail(stored, running)
            .is_some_and(|tail| tail.as_os_str().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[cfg(windows)]
    fn ordinary_extended_drive_and_unc_spellings_identify_the_same_data() {
        for (normal, extended) in [
            (
                r"E:\Кавычка\data\.kavychka-next",
                r"\\?\E:\Кавычка\data\.kavychka-next",
            ),
            (
                r"\\server\share\Кавычка\data",
                r"\\?\UNC\server\share\Кавычка\data",
            ),
            (
                r"e:\Кавычка\DATA\.kavychka-next",
                r"\\?\E:\Кавычка\data\.kavychka-next",
            ),
        ] {
            assert!(same_data_path(Path::new(normal), Path::new(extended)));
            assert!(same_data_path(Path::new(extended), Path::new(normal)));
        }
    }
    #[test]
    fn matching_requires_an_absolute_exact_directory() {
        let base = std::env::temp_dir().join("kavychka-identity-test");
        assert!(same_data_path(&base, &base));
        for other in [
            base.join("child"),
            base.with_file_name("kavychka-identity-test-other"),
            Path::new("relative").to_path_buf(),
        ] {
            assert!(!same_data_path(&base, &other));
            assert!(!same_data_path(&other, &base));
        }
    }
}
