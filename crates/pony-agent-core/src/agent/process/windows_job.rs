//! Windows Job Object lifecycle ownership for managed processes (PA-077).
//!
//! A private, non-inheritable Job with kill-on-close and neither breakaway flag. Native handle
//! access and explicit close are serialized, including when an operation retains the entry Arc.

use std::io;
use std::mem::size_of;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
use std::sync::Mutex;
use windows_sys::Win32::Foundation::{
    GetLastError, SetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, IsProcessInJob, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct FailureInjection {
    pub create: bool,
    pub configure: bool,
    pub assign: bool,
    pub verify: bool,
    pub terminate: bool,
}

#[derive(Debug)]
pub(super) struct Job {
    // OwnedHandle supplies Send/Sync and closes exactly once. The mutex prevents close racing
    // a native operation; Option permits shutdown's kill-on-close fallback despite retained Arcs.
    handle: Mutex<Option<OwnedHandle>>,
    #[cfg(test)]
    failure: Mutex<FailureInjection>,
}

impl Job {
    pub(super) fn create(#[cfg(test)] failure: Option<FailureInjection>) -> Result<Self, String> {
        #[cfg(test)]
        let failure = failure.unwrap_or_default();
        #[cfg(test)]
        if failure.create {
            return Err("job create injected failure".to_string());
        }
        let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if raw.is_null() || raw == INVALID_HANDLE_VALUE {
            return Err(last_error("CreateJobObjectW"));
        }
        // SAFETY: CreateJobObjectW returned a valid, exclusively owned kernel handle.
        let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
        let job = Self {
            handle: Mutex::new(Some(handle)),
            #[cfg(test)]
            failure: Mutex::new(failure),
        };
        job.configure()?;
        Ok(job)
    }

    #[cfg(test)]
    pub(super) fn set_failure(&self, failure: FailureInjection) {
        *self.failure.lock().unwrap() = failure;
    }

    fn configure(&self) -> Result<(), String> {
        #[cfg(test)]
        if self.failure.lock().unwrap().configure {
            return Err("job configure injected failure".to_string());
        }
        let guard = self.handle.lock().expect("job handle poisoned");
        let raw = raw_handle(&guard)?;
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ok = unsafe {
            SetInformationJobObject(
                raw,
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&info).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if ok == 0 {
            return Err(last_error("SetInformationJobObject"));
        }
        if unsafe { SetHandleInformation(raw, HANDLE_FLAG_INHERIT, 0) } == 0 {
            return Err(last_error("SetHandleInformation"));
        }
        Ok(())
    }

    pub(super) fn assign_and_verify(&self, process: RawHandle) -> Result<(), String> {
        #[cfg(test)]
        if self.failure.lock().unwrap().assign {
            return Err("job assign injected failure".to_string());
        }
        let guard = self.handle.lock().expect("job handle poisoned");
        let raw = raw_handle(&guard)?;
        if unsafe { AssignProcessToJobObject(raw, process as HANDLE) } == 0 {
            return Err(last_error("AssignProcessToJobObject"));
        }
        #[cfg(test)]
        if self.failure.lock().unwrap().verify {
            return Err("job verify injected failure".to_string());
        }
        let mut in_job = 0;
        if unsafe { IsProcessInJob(process as HANDLE, raw, &mut in_job) } == 0 {
            return Err(last_error("IsProcessInJob"));
        }
        if in_job == 0 {
            return Err("IsProcessInJob returned false (process not contained in job)".to_string());
        }
        Ok(())
    }

    pub(super) fn terminate(&self, exit_code: u32) -> Result<(), String> {
        let guard = self.handle.lock().expect("job handle poisoned");
        // A previous shutdown may have closed this Job while another operation held the entry.
        let Some(handle) = guard.as_ref() else {
            return Ok(());
        };
        #[cfg(test)]
        if self.failure.lock().unwrap().terminate {
            return Err("job terminate injected failure".to_string());
        }
        if unsafe { TerminateJobObject(handle.as_raw_handle(), exit_code) } == 0 {
            return Err(last_error("TerminateJobObject"));
        }
        Ok(())
    }

    /// Close now, not just when the last ManagedProcess Arc drops. Kill-on-close remains the
    /// fallback if explicit termination fails during failed-start cleanup or session shutdown.
    pub(super) fn close(&self) {
        drop(self.handle.lock().expect("job handle poisoned").take());
    }

    #[cfg(test)]
    pub(super) fn query_limit_flags(&self) -> Result<u32, String> {
        use windows_sys::Win32::System::JobObjects::QueryInformationJobObject;
        let guard = self.handle.lock().unwrap();
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        let mut return_len = 0;
        if unsafe {
            QueryInformationJobObject(
                raw_handle(&guard)?,
                JobObjectExtendedLimitInformation,
                std::ptr::from_mut(&mut info).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                &mut return_len,
            )
        } == 0
        {
            return Err(last_error("QueryInformationJobObject"));
        }
        Ok(info.BasicLimitInformation.LimitFlags)
    }

    #[cfg(test)]
    pub(super) fn is_process_in_job(&self, process: RawHandle) -> Result<bool, String> {
        let guard = self.handle.lock().unwrap();
        let mut in_job = 0;
        if unsafe { IsProcessInJob(process as HANDLE, raw_handle(&guard)?, &mut in_job) } == 0 {
            return Err(last_error("IsProcessInJob"));
        }
        Ok(in_job != 0)
    }

    #[cfg(test)]
    pub(super) fn handle_flags(&self) -> Result<u32, String> {
        use windows_sys::Win32::Foundation::GetHandleInformation;
        let guard = self.handle.lock().unwrap();
        let mut flags = 0;
        if unsafe { GetHandleInformation(raw_handle(&guard)?, &mut flags) } == 0 {
            return Err(last_error("GetHandleInformation"));
        }
        Ok(flags)
    }
}

fn raw_handle(handle: &Option<OwnedHandle>) -> Result<HANDLE, String> {
    handle
        .as_ref()
        .map(AsRawHandle::as_raw_handle)
        .ok_or_else(|| "job object already closed".to_string())
}

fn last_error(operation: &str) -> String {
    let error = io::Error::from_raw_os_error(unsafe { GetLastError() } as i32);
    format!(
        "{operation} failed: {error} (win32={})",
        error.raw_os_error().unwrap_or(0)
    )
}
