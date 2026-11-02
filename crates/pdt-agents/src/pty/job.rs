//! Windows: each terminal session's process tree in a job object of its own, created with kill-on-close.
//!
//! pdtd holds the only handle, so when pdtd ends for any reason (a crash, being killed, an update) Windows ends every
//! process in the job: what a session started (a dev server under `cmd.exe`, `node` under an npm shim, an agent's MCP
//! servers) cannot outlive it holding ports and files. A process that asks to break away (`CREATE_BREAKAWAY_FROM_JOB`,
//! as a detached run does) may: it means to outlive its parent.
//!
//! The child joins the job just after it starts (portable-pty cannot start it suspended), so what it starts in that
//! moment escapes. A shell or agent starts nothing that fast, and under ConPTY its first output waits for the cursor
//! reply anyway.

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_BREAKAWAY_OK, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject,
    TerminateJobObject,
};
use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE};

/// A session's job. Dropping it closes the handle, which ends every process still in it.
pub(super) struct Job(HANDLE);

// A kernel handle may be used from any thread.
unsafe impl Send for Job {}
unsafe impl Sync for Job {}

impl Drop for Job {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

impl Job {
    /// Puts process `pid` (just started) in a new kill-on-close job. A process already in a job (pdtd's own, from
    /// pdt-host) joins a nested one.
    pub(super) fn assign(pid: u32) -> std::io::Result<Job> {
        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if job.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let job = Job(job);
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_BREAKAWAY_OK;
        let ok = unsafe {
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let process = unsafe { OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid) };
        if process.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let assigned = unsafe { AssignProcessToJobObject(job.0, process) } != 0;
        let err = std::io::Error::last_os_error();
        unsafe {
            CloseHandle(process);
        }
        if !assigned {
            return Err(err);
        }
        Ok(job)
    }

    /// Ends every process in the job now.
    pub(super) fn terminate(&self) {
        unsafe {
            TerminateJobObject(self.0, 1);
        }
    }
}
