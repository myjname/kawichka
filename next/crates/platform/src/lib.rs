//! Platform queries, kept out of the domain and storage layers.
pub mod audio;
pub mod desktop;
pub mod dialogs;
pub mod input;
pub mod process;
pub mod time;
pub mod ui_scheduling;
pub fn on_ac_power() -> bool {
    power_source() == kavychka_core::engine::PowerSource::Ac
}
pub fn power_source() -> kavychka_core::engine::PowerSource {
    #[cfg(windows)]
    unsafe {
        let mut status: windows_sys::Win32::System::Power::SYSTEM_POWER_STATUS = std::mem::zeroed();
        if windows_sys::Win32::System::Power::GetSystemPowerStatus(&mut status) != 0 {
            return match status.ACLineStatus {
                0 => kavychka_core::engine::PowerSource::Battery,
                1 => kavychka_core::engine::PowerSource::Ac,
                _ => kavychka_core::engine::PowerSource::Unknown,
            };
        }
    }
    kavychka_core::engine::PowerSource::Unknown
}
/// None means the OS preference could not be read. The UI retains its last value.
pub fn dark_preference() -> Option<bool> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::{
            Foundation::ERROR_SUCCESS,
            System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW},
        };
        let key: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let name: Vec<u16> = "AppsUseLightTheme".encode_utf16().chain(Some(0)).collect();
        let mut value = 1u32;
        let mut size = 4u32;
        // SAFETY: terminated strings, four-byte writable value buffer with matching size.
        let result = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_DWORD,
                std::ptr::null_mut(),
                (&mut value as *mut u32).cast(),
                &mut size,
            )
        };
        if result == ERROR_SUCCESS {
            Some(value == 0)
        } else {
            None
        }
    }
    #[cfg(not(windows))]
    {
        None
    }
}
