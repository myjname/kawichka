//! UI-thread scheduling via Windows CPU Sets, never a hard affinity mask.
//! DPC load during GPU inference can monopolize CPU0 even when ML has lower priority.
//! Keep CPU0's interrupt load away from UI on eligible single-group systems;
//! preserve explicit user CPU sets/masks and OS power management.

pub fn current_processor() -> Option<u32> {
    #[cfg(windows)]
    {
        Some(unsafe { windows_sys::Win32::System::Threading::GetCurrentProcessorNumber() })
    }
    #[cfg(not(windows))]
    {
        None
    }
}

#[derive(Clone, Copy)]
struct Candidate {
    id: u32,
    group: u16,
    index: u8,
    efficiency: u8,
    flags: u8,
}
fn choose(cpus: &[Candidate], group: u16, mask: usize) -> Option<u32> {
    if mask.count_ones() < 2 {
        return None;
    }
    cpus.iter()
        .filter(|c| {
            c.group == group
                && (c.index as u32) < usize::BITS
                && mask & (1usize << c.index) != 0
                && c.flags & 1 == 0 // parked
                && (c.flags & 2 == 0 || c.flags & 4 != 0) // not reserved for others
        })
        .max_by_key(|c| (c.efficiency, c.index != 0, c.index))
        .map(|c| c.index as u32)
}

/// Scoped to the calling UI thread; cannot be moved to another thread.
pub struct UiScheduling {
    previous: u32,
    pub selected_count: u32,
    pub preferred: u32,
    pub group: u16,
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
}
impl UiScheduling {
    /// Unsupported/failed queries leave the OS policy intact. Callers may
    /// continue without a hint; this optimization must not prevent app startup.
    pub fn prefer_responsive() -> Result<Option<Self>, String> {
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::System::{SystemInformation::*, Threading::*};
            let mut affinity = GROUP_AFFINITY::default();
            if GetThreadGroupAffinity(GetCurrentThread(), &mut affinity) == 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
            // Explicit assignments made by the user/tool take precedence.
            let mut thread_count = 0;
            let mut process_count = 0;
            let thread_ok = GetThreadSelectedCpuSets(
                GetCurrentThread(),
                std::ptr::null_mut(),
                0,
                &mut thread_count,
            );
            let thread_error = std::io::Error::last_os_error();
            let process_ok = GetProcessDefaultCpuSets(
                GetCurrentProcess(),
                std::ptr::null_mut(),
                0,
                &mut process_count,
            );
            if thread_count != 0 || process_count != 0 {
                return Ok(None);
            }
            if thread_ok == 0 {
                return Err(thread_error.to_string());
            }
            if process_ok == 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
            let mut required = 0;
            GetSystemCpuSetInformation(
                std::ptr::null_mut(),
                0,
                &mut required,
                GetCurrentProcess(),
                0,
            );
            if required == 0 || required > 1024 * 1024 {
                return Err("CPU set size unavailable".into());
            }
            // u64 storage provides alignment; records carry their own size.
            let mut storage = vec![0u64; (required as usize).div_ceil(8)];
            if GetSystemCpuSetInformation(
                storage.as_mut_ptr().cast(),
                (storage.len() * 8) as u32,
                &mut required,
                GetCurrentProcess(),
                0,
            ) == 0
            {
                return Err(std::io::Error::last_os_error().to_string());
            }
            let length = required as usize;
            if length > storage.len() * 8 {
                return Err("CPU set buffer changed".into());
            }
            let mut offset = 0;
            let mut cpus = vec![];
            while offset + 8 <= length {
                let at = storage.as_ptr().cast::<u8>().add(offset);
                let size = at.cast::<u32>().read_unaligned() as usize;
                if size < 8 || size > length - offset {
                    return Err("Invalid CPU set record".into());
                }
                let kind = at.add(4).cast::<i32>().read_unaligned();
                if kind == CpuSetInformation
                    && size >= std::mem::size_of::<SYSTEM_CPU_SET_INFORMATION>()
                {
                    let info = at.cast::<SYSTEM_CPU_SET_INFORMATION>().read_unaligned();
                    let cpu = info.Anonymous.CpuSet;
                    cpus.push(Candidate {
                        id: cpu.Id,
                        group: cpu.Group,
                        index: cpu.LogicalProcessorIndex,
                        efficiency: cpu.EfficiencyClass,
                        flags: cpu.Anonymous1.AllFlags,
                    });
                }
                offset += size;
            }
            if offset != length {
                return Err("Truncated CPU set record".into());
            }
            let Some(preferred) = choose(&cpus, affinity.Group, affinity.Mask) else {
                return Ok(None);
            };
            // CPU Sets preserve hard masks and let Windows manage parking.
            // Skip multi-group systems rather than accidentally narrowing an
            // automatically cross-group thread to its current primary group.
            let selected: Vec<u32> = if GetActiveProcessorGroupCount() == 1
                && affinity.Group == 0
                && affinity.Mask.count_ones() >= 4
                && affinity.Mask & 1 != 0
                && preferred != 0
            {
                cpus.iter()
                    .filter(|c| {
                        c.group == 0
                            && c.index != 0
                            && (c.index as u32) < usize::BITS
                            && affinity.Mask & (1usize << c.index) != 0
                            && (c.flags & 2 == 0 || c.flags & 4 != 0)
                    })
                    .map(|c| c.id)
                    .collect()
            } else {
                vec![]
            };
            let selected_count = if selected.len() >= 2 {
                selected.len() as u32
            } else {
                0
            };
            if selected_count != 0
                && SetThreadSelectedCpuSets(GetCurrentThread(), selected.as_ptr(), selected_count)
                    == 0
            {
                return Err(std::io::Error::last_os_error().to_string());
            }
            let previous = SetThreadIdealProcessor(GetCurrentThread(), preferred);
            if previous == u32::MAX {
                let error = std::io::Error::last_os_error().to_string();
                if selected_count != 0 {
                    SetThreadSelectedCpuSets(GetCurrentThread(), std::ptr::null(), 0);
                }
                return Err(error);
            }
            Ok(Some(Self {
                previous,
                selected_count,
                preferred,
                group: affinity.Group,
                _thread: Default::default(),
            }))
        }
        #[cfg(not(windows))]
        {
            Ok(None)
        }
    }
}
impl Drop for UiScheduling {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            if self.selected_count != 0 {
                windows_sys::Win32::System::Threading::SetThreadSelectedCpuSets(
                    windows_sys::Win32::System::Threading::GetCurrentThread(),
                    std::ptr::null(),
                    0,
                );
            }
            windows_sys::Win32::System::Threading::SetThreadIdealProcessor(
                windows_sys::Win32::System::Threading::GetCurrentThread(),
                self.previous,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cpu(group: u16, index: u8, efficiency: u8, flags: u8) -> Candidate {
        Candidate {
            id: index as u32,
            group,
            index,
            efficiency,
            flags,
        }
    }
    #[test]
    fn respects_group_mask_reservations_and_performance_class() {
        let cpus = [
            cpu(0, 0, 9, 0),
            cpu(0, 1, 9, 0),
            cpu(0, 2, 1, 0),
            cpu(0, 3, 9, 1),
            cpu(0, 4, 9, 2),
            cpu(0, 5, 9, 6),
            cpu(1, 6, 99, 0),
        ];
        assert_eq!(choose(&cpus, 0, 0b111111), Some(5));
        assert_eq!(choose(&cpus, 0, 0b11111), Some(1));
        assert_eq!(choose(&cpus, 0, 0b101), Some(0)); // do not prefer an E-core just to avoid CPU0
        assert_eq!(choose(&cpus, 0, 1), None); // explicit one-CPU constraint
        assert_eq!(choose(&cpus, 2, 0xff), None);
    }
    #[test]
    fn ignores_out_of_mask_and_out_of_range_processors() {
        assert_eq!(
            choose(
                &[cpu(0, 0, 0, 0), cpu(0, 1, 0, 0), cpu(0, 127, 99, 0)],
                0,
                3
            ),
            Some(1)
        );
        assert_eq!(choose(&[cpu(0, 5, 99, 0)], 0, 3), None);
    }
    #[cfg(windows)]
    #[test]
    fn native_hint_does_not_change_thread_affinity() {
        unsafe {
            use windows_sys::Win32::System::{SystemInformation::*, Threading::*};
            let mut before = GROUP_AFFINITY::default();
            assert_ne!(GetThreadGroupAffinity(GetCurrentThread(), &mut before), 0);
            let hint = UiScheduling::prefer_responsive().unwrap();
            let mut custom_id = None;
            if let Some(hint) = &hint
                && hint.selected_count != 0
            {
                let mut count = 0;
                let mut ids = vec![0; hint.selected_count as usize];
                assert_ne!(
                    GetThreadSelectedCpuSets(
                        GetCurrentThread(),
                        ids.as_mut_ptr(),
                        hint.selected_count,
                        &mut count
                    ),
                    0
                );
                assert_eq!(count, hint.selected_count);
                custom_id = ids.first().copied();
            }
            let mut during = GROUP_AFFINITY::default();
            assert_ne!(GetThreadGroupAffinity(GetCurrentThread(), &mut during), 0);
            assert_eq!((before.Group, before.Mask), (during.Group, during.Mask));
            drop(hint);
            let mut after = GROUP_AFFINITY::default();
            assert_ne!(GetThreadGroupAffinity(GetCurrentThread(), &mut after), 0);
            assert_eq!((before.Group, before.Mask), (after.Group, after.Mask));
            if let Some(id) = custom_id {
                let mut count = 0;
                assert_ne!(
                    GetThreadSelectedCpuSets(
                        GetCurrentThread(),
                        std::ptr::null_mut(),
                        0,
                        &mut count
                    ),
                    0
                );
                assert_eq!(count, 0); // guard restored inheritance
                assert_ne!(SetThreadSelectedCpuSets(GetCurrentThread(), &id, 1), 0);
                let skipped = UiScheduling::prefer_responsive().unwrap();
                let mut actual = 0;
                let query =
                    GetThreadSelectedCpuSets(GetCurrentThread(), &mut actual, 1, &mut count);
                // Restore before assertions, including on regression failures.
                assert_ne!(
                    SetThreadSelectedCpuSets(GetCurrentThread(), std::ptr::null(), 0),
                    0
                );
                assert!(skipped.is_none());
                assert_ne!(query, 0);
                assert_eq!((count, actual), (1, id));
            }
        }
    }
}
