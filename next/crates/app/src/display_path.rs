//! Presentation only: keep extended Win32/UNC paths unchanged for filesystem IO.
use std::path::Path;

pub fn display(path: &Path) -> String {
    readable(&path.to_string_lossy())
}

pub fn readable(value: &str) -> String {
    let bytes = value.as_bytes();
    let extended = bytes
        .get(..4)
        .is_some_and(|p| p == br"\\?\" || p == b"//?/");
    if !extended {
        return value.to_owned();
    }
    let tail = &value[4..];
    if tail
        .as_bytes()
        .get(..4)
        .is_some_and(|p| p.eq_ignore_ascii_case(br"UNC\") || p.eq_ignore_ascii_case(b"UNC/"))
    {
        return format!(r"\\{}", tail[4..].replace('/', r"\"));
    }
    if tail
        .as_bytes()
        .get(..3)
        .is_some_and(|p| p[0].is_ascii_alphabetic() && p[1] == b':' && matches!(p[2], b'\\' | b'/'))
    {
        return tail.to_owned();
    }
    // Namespaces such as GLOBALROOT have no equivalent ordinary file path.
    value.to_owned()
}

/// Status/detail messages may contain several paths in otherwise ordinary text.
pub fn text(value: &str) -> String {
    let mut remaining = value;
    let mut result = String::with_capacity(value.len());
    while let Some(at) = remaining
        .as_bytes()
        .windows(4)
        .position(|p| p == br"\\?\" || p == b"//?/")
    {
        result.push_str(&remaining[..at]);
        let candidate = &remaining[at..];
        let tail = &candidate[4..];
        if tail
            .as_bytes()
            .get(..4)
            .is_some_and(|p| p.eq_ignore_ascii_case(br"UNC\") || p.eq_ignore_ascii_case(b"UNC/"))
        {
            result.push_str(r"\\");
            remaining = &candidate[8..];
        } else if tail.as_bytes().get(..3).is_some_and(|p| {
            p[0].is_ascii_alphabetic() && p[1] == b':' && matches!(p[2], b'\\' | b'/')
        }) {
            remaining = tail;
        } else {
            result.push_str(&candidate[..4]);
            remaining = tail;
        }
    }
    result.push_str(remaining);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_extended_paths_have_readable_drive_and_unc_forms() {
        for (input, expected) in [
            (r"\\?\C:\Users\Example\Models", r"C:\Users\Example\Models"),
            (r"\\?\UNC\server\share\Models", r"\\server\share\Models"),
            (r"\\?\unc\server\share\Модели", r"\\server\share\Модели"),
            ("//?/UNC/server/share/Models", r"\\server\share\Models"),
            ("//?/D:/Models", "D:/Models"),
            (r"\\server\share\Models", r"\\server\share\Models"),
            (r"C:\Models", r"C:\Models"),
            (
                r"\\?\GLOBALROOT\Device\HarddiskVolume1",
                r"\\?\GLOBALROOT\Device\HarddiskVolume1",
            ),
            ("", ""),
            ("/tmp/models", "/tmp/models"),
        ] {
            assert_eq!(readable(input), expected);
            assert_eq!(display(Path::new(input)), expected);
        }
    }
    #[test]
    fn status_text_cleans_embedded_paths_without_changing_other_text() {
        assert_eq!(
            text("Путь: \\\\?\\C:\\Модели\\voice\nСеть: \\\\?\\UNC\\server\\share"),
            "Путь: C:\\Модели\\voice\nСеть: \\\\server\\share"
        );
        assert_eq!(text("No paths or updates"), "No paths or updates");
        assert_eq!(
            text(r"Unknown: \\?\GLOBALROOT\Device"),
            r"Unknown: \\?\GLOBALROOT\Device"
        );
    }
}
