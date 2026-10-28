//! Local file selection; no shell command construction or network activity.
pub fn profile_zip(
    save: bool,
    initial: &str,
    owner: Option<isize>,
) -> Result<Option<std::path::PathBuf>, String> {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::UI::Controls::Dialogs::*;
        let mut name = vec![0u16; 32768];
        let initial: Vec<_> = initial.encode_utf16().collect();
        if initial.len() >= name.len() {
            return Err("File path is too long".into());
        }
        name[..initial.len()].copy_from_slice(&initial);
        let filter: Vec<_> = "Kavychka profile ZIP\0*.zip\0\0".encode_utf16().collect();
        let extension: Vec<_> = "zip\0".encode_utf16().collect();
        let mut dialog: OPENFILENAMEW = std::mem::zeroed();
        dialog.lStructSize = std::mem::size_of_val(&dialog) as u32;
        dialog.hwndOwner = owner.map(|n| n as _).unwrap_or(std::ptr::null_mut());
        dialog.lpstrFile = name.as_mut_ptr();
        dialog.nMaxFile = name.len() as u32;
        dialog.lpstrFilter = filter.as_ptr();
        dialog.lpstrDefExt = extension.as_ptr();
        dialog.Flags = OFN_PATHMUSTEXIST
            | OFN_NOCHANGEDIR
            | OFN_EXPLORER
            | OFN_DONTADDTORECENT
            | if save {
                OFN_OVERWRITEPROMPT
            } else {
                OFN_FILEMUSTEXIST
            };
        let ok = if save {
            GetSaveFileNameW(&mut dialog)
        } else {
            GetOpenFileNameW(&mut dialog)
        };
        if ok != 0 {
            return Ok(Some(
                String::from_utf16_lossy(&name[..name.iter().position(|c| *c == 0).unwrap_or(0)])
                    .into(),
            ));
        }
        let error = CommDlgExtendedError();
        if error != 0 {
            return Err(format!("File dialog failed: {error}"));
        }
        Ok(None)
    }
    #[cfg(not(windows))]
    {
        let _ = (save, initial, owner);
        Err("Native file selection is not qualified on this platform".into())
    }
}
pub fn open() -> Option<std::path::PathBuf> {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::UI::Controls::Dialogs::*;
        let mut name = vec![0u16; 32768];
        let filter:Vec<u16>="Персонаж, пресет или медиа / Character, preset or media\0*.kavychka;*.zip;*.json;*.txt;*.png;*.jpg;*.jpeg;*.gif;*.webp;*.wav;*.flac;*.mp3;*.ogg;*.m4a;*.mp4\0Все файлы / All files\0*.*\0\0".encode_utf16().collect();
        let mut dialog: OPENFILENAMEW = std::mem::zeroed();
        dialog.lStructSize = std::mem::size_of_val(&dialog) as u32;
        dialog.lpstrFile = name.as_mut_ptr();
        dialog.nMaxFile = name.len() as u32;
        dialog.lpstrFilter = filter.as_ptr();
        dialog.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;
        if GetOpenFileNameW(&mut dialog) != 0 {
            return Some(
                String::from_utf16_lossy(&name[..name.iter().position(|c| *c == 0).unwrap_or(0)])
                    .into(),
            );
        }
    }
    None
}
