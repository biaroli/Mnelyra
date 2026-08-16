use tokio::process::Child;

/// Owns the OS-level process-tree containment associated with a managed child.
///
/// On Windows this is a kill-on-close Job Object, so descendants created by a
/// `.cmd` shim or a provider/tunnel binary cannot survive the owning Mnelyra
/// process. The stored representation is an integer rather than HANDLE so the
/// guard remains Send/Sync inside Tauri state.
#[cfg(target_os = "windows")]
pub(crate) struct ProcessTreeGuard(usize);

#[cfg(target_os = "windows")]
impl ProcessTreeGuard {
    pub(crate) fn attach(child: &Child) -> Result<Self, String> {
        use std::ffi::c_void;
        use std::mem::size_of;
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };
        use windows::Win32::System::Threading::{
            OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
        };

        let pid = child
            .id()
            .ok_or_else(|| "managed child has no process id".to_string())?;
        unsafe {
            let job = CreateJobObjectW(None, PCWSTR::null())
                .map_err(|error| format!("failed to create process job: {error}"))?;
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if let Err(error) = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const c_void,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ) {
                let _ = CloseHandle(job);
                return Err(format!("failed to configure process job: {error}"));
            }
            let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, false, pid)
                .map_err(|error| format!("failed to open managed child {pid}: {error}"))?;
            let assigned = AssignProcessToJobObject(job, process);
            let _ = CloseHandle(process);
            if let Err(error) = assigned {
                let _ = CloseHandle(job);
                return Err(format!("failed to attach process tree to job: {error}"));
            }
            Ok(Self(job.0 as usize))
        }
    }
}

#[cfg(target_os = "windows")]
impl Drop for ProcessTreeGuard {
    fn drop(&mut self) {
        use std::ffi::c_void;
        use windows::Win32::Foundation::{CloseHandle, HANDLE};

        unsafe {
            let handle = HANDLE(self.0 as *mut c_void);
            let _ = CloseHandle(handle);
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub(crate) struct ProcessTreeGuard;

#[cfg(not(target_os = "windows"))]
impl ProcessTreeGuard {
    pub(crate) fn attach(_child: &Child) -> Result<Self, String> {
        Ok(Self)
    }
}
