use crate::error::AppError;

/// Windows launchers and their descendants must end with their harness session,
/// including when the Desktop process exits before async cleanup can run.
pub struct ChildJob {
    #[cfg(windows)]
    _handle: std::os::windows::io::OwnedHandle,
}

impl ChildJob {
    pub fn attach(child: &tokio::process::Child) -> Result<Self, AppError> {
        #[cfg(windows)]
        {
            use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
            use windows_sys::Win32::System::JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
                SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            };
            let process = child
                .raw_handle()
                .ok_or_else(|| AppError::Agent("Harness process has already exited".into()))?;
            // The returned handle is owned here; closing it is the kernel's kill signal.
            let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if handle.is_null() {
                return Err(AppError::Agent(std::io::Error::last_os_error().to_string()));
            }
            let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let configured = unsafe {
                SetInformationJobObject(
                    handle.as_raw_handle(),
                    JobObjectExtendedLimitInformation,
                    std::ptr::from_ref(&limits).cast(),
                    std::mem::size_of_val(&limits) as u32,
                )
            };
            if configured == 0
                || unsafe { AssignProcessToJobObject(handle.as_raw_handle(), process) } == 0
            {
                return Err(AppError::Agent(format!(
                    "Could not contain the harness process tree: {}",
                    std::io::Error::last_os_error()
                )));
            }
            Ok(Self { _handle: handle })
        }
        #[cfg(not(windows))]
        {
            let _ = child;
            Ok(Self {})
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn closing_the_job_stops_a_live_child() {
        let mut command = tokio::process::Command::new("cmd.exe");
        command
            .args(["/d", "/c", "ping -n 30 127.0.0.1 >nul"])
            .creation_flags(0x08000000)
            .kill_on_drop(true);
        let mut child = command.spawn().unwrap();
        let job = ChildJob::attach(&child).unwrap();
        assert!(child.try_wait().unwrap().is_none());
        drop(job);
        tokio::time::timeout(std::time::Duration::from_secs(3), child.wait())
            .await
            .unwrap()
            .unwrap();
        // Closing a Windows job may terminate members with exit code zero.
    }
}
