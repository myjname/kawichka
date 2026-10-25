//! Process-tree lifetime boundary. Assign before sending any work to the child.
pub struct Job {
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
}
unsafe impl Send for Job {}
impl Job {
    pub fn assign(child: &std::process::Child) -> Result<Self, String> {
        Self::assign_policy(child, false)
    }
    /// Inference may use idle CPU while normal-priority UI/audio stays responsive.
    pub fn assign_background(child: &std::process::Child) -> Result<Self, String> {
        Self::assign_policy(child, true)
    }
    fn assign_policy(child: &std::process::Child, background: bool) -> Result<Self, String> {
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::{Foundation::CloseHandle, System::JobObjects::*};
            unsafe {
                let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if handle.is_null() {
                    return Err(std::io::Error::last_os_error().to_string());
                }
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                if background {
                    info.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_PRIORITY_CLASS;
                    info.BasicLimitInformation.PriorityClass =
                        windows_sys::Win32::System::Threading::BELOW_NORMAL_PRIORITY_CLASS;
                }
                if SetInformationJobObject(
                    handle,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as _,
                    std::mem::size_of_val(&info) as u32,
                ) == 0
                    || AssignProcessToJobObject(handle, child.as_raw_handle() as _) == 0
                {
                    let error = std::io::Error::last_os_error().to_string();
                    CloseHandle(handle);
                    return Err(error);
                }
                Ok(Self { handle })
            }
        }
        #[cfg(not(windows))]
        {
            let _ = (child, background);
            Err("process-tree adapter unavailable on this platform".into())
        }
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

#[derive(Clone, Copy)]
pub struct Sample {
    pub private_bytes: u64,
    pub cpu_100ns: u64,
    pub age_millis: u64,
    pub page_faults: u32,
    pub read_bytes: u64,
    pub available_physical_bytes: u64,
    pub handle_count: u32,
    pub gdi_objects: u32,
    pub user_objects: u32,
}
pub fn sample() -> Result<Sample, String> {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::{
            Foundation::FILETIME,
            System::{ProcessStatus::*, Threading::*},
        };
        let mut memory: PROCESS_MEMORY_COUNTERS_EX = std::mem::zeroed();
        memory.cb = std::mem::size_of_val(&memory) as u32;
        if GetProcessMemoryInfo(GetCurrentProcess(), &mut memory as *mut _ as _, memory.cb) == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let mut creation: FILETIME = std::mem::zeroed();
        let mut exit: FILETIME = std::mem::zeroed();
        let mut kernel: FILETIME = std::mem::zeroed();
        let mut user: FILETIME = std::mem::zeroed();
        if GetProcessTimes(
            GetCurrentProcess(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        ) == 0
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let value = |t: FILETIME| ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64;
        let mut io: IO_COUNTERS = std::mem::zeroed();
        let mut status: windows_sys::Win32::System::SystemInformation::MEMORYSTATUSEX =
            std::mem::zeroed();
        status.dwLength = std::mem::size_of_val(&status) as u32;
        if GetProcessIoCounters(GetCurrentProcess(), &mut io) == 0
            || windows_sys::Win32::System::SystemInformation::GlobalMemoryStatusEx(&mut status) == 0
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_millis() as u64
            + 11_644_473_600_000;
        let mut handle_count = 0;
        if GetProcessHandleCount(GetCurrentProcess(), &mut handle_count) == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(Sample {
            private_bytes: memory.PrivateUsage as u64,
            cpu_100ns: value(kernel) + value(user),
            age_millis: now.saturating_sub(value(creation) / 10_000),
            page_faults: memory.PageFaultCount,
            read_bytes: io.ReadTransferCount,
            available_physical_bytes: status.ullAvailPhys,
            handle_count,
            gdi_objects: GetGuiResources(GetCurrentProcess(), 0),
            user_objects: GetGuiResources(GetCurrentProcess(), 1),
        })
    }
    #[cfg(not(windows))]
    {
        Err("Windows measurement adapter unavailable".into())
    }
}

/// Read-only handle to the calling thread; usable by an owned watchdog. Never
/// suspends the thread, changes priorities, or inspects other applications.
pub struct ThreadObserver {
    id: u32,
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
}
unsafe impl Send for ThreadObserver {}
impl ThreadObserver {
    pub fn current() -> Result<Self, String> {
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::System::Threading::*;
            let handle = OpenThread(THREAD_QUERY_INFORMATION, 0, GetCurrentThreadId());
            if handle.is_null() {
                return Err(std::io::Error::last_os_error().to_string());
            }
            Ok(Self {
                handle,
                id: GetCurrentThreadId(),
            })
        }
        #[cfg(not(windows))]
        {
            Err("Thread measurement adapter unavailable".into())
        }
    }
    pub fn id(&self) -> u32 {
        self.id
    }
    pub fn sample(&self) -> Result<(u64, bool), String> {
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::{Foundation::FILETIME, System::Threading::*};
            let mut creation: FILETIME = std::mem::zeroed();
            let mut exit: FILETIME = std::mem::zeroed();
            let mut kernel: FILETIME = std::mem::zeroed();
            let mut user: FILETIME = std::mem::zeroed();
            let mut pending = 0;
            if GetThreadTimes(
                self.handle,
                &mut creation,
                &mut exit,
                &mut kernel,
                &mut user,
            ) == 0
                || GetThreadIOPendingFlag(self.handle, &mut pending) == 0
            {
                return Err(std::io::Error::last_os_error().to_string());
            }
            let value = |t: FILETIME| ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64;
            Ok((value(kernel) + value(user), pending != 0))
        }
        #[cfg(not(windows))]
        {
            Err("Thread measurement adapter unavailable".into())
        }
    }
}
impl Drop for ThreadObserver {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

#[cfg(all(test, windows))]
mod measurement_tests {
    use super::*;
    #[test]
    fn observer_queries_the_captured_thread_from_another_thread() {
        let observer = ThreadObserver::current().unwrap();
        let first = observer.sample().unwrap().0;
        let second = std::thread::spawn(move || observer.sample().unwrap().0)
            .join()
            .unwrap();
        assert!(second >= first);
        let process = sample().unwrap();
        assert!(process.private_bytes > 0 && process.available_physical_bytes > 0);
    }
}

/// Qualification only: pin the calling test thread and its owned load child to
/// one allowed CPU. Never changes a system mask; guard cannot move threads.
pub struct TestAffinity {
    #[cfg(windows)]
    previous: usize,
    mask: usize,
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
}
impl TestAffinity {
    pub fn one_core() -> Result<Self, String> {
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::System::Threading::*;
            let mut allowed = 0;
            let mut system = 0;
            if GetProcessAffinityMask(GetCurrentProcess(), &mut allowed, &mut system) == 0
                || allowed == 0
            {
                return Err(std::io::Error::last_os_error().to_string());
            }
            let mask = 1usize << (usize::BITS - 1 - allowed.leading_zeros());
            let previous = SetThreadAffinityMask(GetCurrentThread(), mask);
            if previous == 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
            Ok(Self {
                previous,
                mask,
                _thread: Default::default(),
            })
        }
        #[cfg(not(windows))]
        {
            Err("Windows-only qualification adapter".into())
        }
    }
    pub fn child_priority(&self, child: &std::process::Child) -> Result<u32, String> {
        #[cfg(windows)]
        unsafe {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::System::Threading::*;
            if SetProcessAffinityMask(child.as_raw_handle() as _, self.mask) == 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
            let priority = GetPriorityClass(child.as_raw_handle() as _);
            if priority == 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
            Ok(priority)
        }
        #[cfg(not(windows))]
        {
            let _ = child;
            Err("Windows-only qualification adapter".into())
        }
    }
    pub fn mask(&self) -> usize {
        self.mask
    }
    pub fn restore(mut self) -> Result<(), String> {
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::System::Threading::*;
            if SetThreadAffinityMask(GetCurrentThread(), self.previous) == 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
            self.previous = 0;
        }
        Ok(())
    }
}
impl Drop for TestAffinity {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::System::Threading::*;
            if self.previous != 0 {
                SetThreadAffinityMask(GetCurrentThread(), self.previous);
            }
        }
    }
}
