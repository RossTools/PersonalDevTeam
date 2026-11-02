//! Detached processes (PDT-32): a run command started outside any PTY, so it keeps running when pdtd exits (a
//! restart or an update). Its output goes to a log file; it has no input.
//!
//! The command and everything it starts are kept together so they can be found, watched and ended as one, also by
//! the next pdtd:
//! - unix: its own session and process group (`setsid`). Alive while any process is in the group. Found again by
//!   the group id (`pgid:<n>`).
//! - Windows: a named job object (`job:<name>`), with no kill-on-close, so it outlives pdtd. The command starts
//!   suspended, joins the job, then resumes, so nothing it starts escapes. Alive while the job has an active process.
//!
//! Either way an app that restarts itself (starts a copy of itself and exits) is still tracked: the copy inherits
//! the group or job.
//!
//! A job object's name disappears once no process has a handle to it, even while processes are still in it (they
//! hold references, not handles). So that the next pdtd can open it by name, pdtd copies a handle to the job into
//! every process in it (at start and on every check while it watches). The only gap: a process that starts while
//! no pdtd is running, after every process holding a handle has exited.

use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::oneshot;

/// How often a watcher checks whether the processes are still alive.
const WATCH_EVERY: Duration = Duration::from_millis(500);
/// How long the processes get to act on SIGTERM before they are killed (unix).
#[cfg(unix)]
const TERM_GRACE: Duration = Duration::from_secs(3);

#[derive(Debug, Clone)]
pub struct DetachedSpec {
    /// Names the Windows job (with the start time, so a new start never finds an old job).
    pub id: String,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    /// Removed from the inherited environment before `env` is applied.
    pub env_remove: Vec<String>,
    pub env: Vec<(String, String)>,
    /// stdout and stderr go here (truncated first).
    pub log: PathBuf,
}

/// A started command.
pub struct Detached {
    /// How to find its processes again (store it; see [`alive`], [`terminate`], [`watch`]).
    pub process: String,
    /// The process this pdtd started (the shell running the command).
    pub main: std::process::Child,
}

/// Starts `spec` detached. Blocking.
pub fn spawn(spec: &DetachedSpec) -> Result<Detached, String> {
    if !spec.cwd.is_dir() {
        return Err(format!("working directory does not exist: {}", spec.cwd.display()));
    }
    if let Some(dir) = spec.log.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    }
    let log = std::fs::File::create(&spec.log).map_err(|e| format!("creating {}: {e}", spec.log.display()))?;
    let command = |log: &std::fs::File| -> Result<std::process::Command, String> {
        let mut cmd = std::process::Command::new(&spec.program);
        cmd.args(&spec.args).current_dir(&spec.cwd);
        for k in &spec.env_remove {
            cmd.env_remove(k);
        }
        cmd.envs(spec.env.iter().map(|(k, v)| (k, v)));
        let out = log.try_clone().map_err(|e| e.to_string())?;
        let err = log.try_clone().map_err(|e| e.to_string())?;
        cmd.stdin(std::process::Stdio::null()).stdout(out).stderr(err);
        Ok(cmd)
    };
    sys::spawn(spec, &log, command)
}

/// Whether any process of `process` is alive. Blocking. An unknown form counts as gone.
pub fn alive(process: &str) -> bool {
    sys::alive(process)
}

/// Ends every process of `process`. Blocking (on unix it returns once SIGTERM is sent; SIGKILL follows later).
pub fn terminate(process: &str) {
    sys::terminate(process)
}

/// Watches `process` until all its processes have exited, then resolves with the exit code of `main` when it was
/// the last to go (`None` when other processes outlived it, or when there is no `main`: a run found again after a
/// pdtd restart). Dropping the receiver ends the watch at its next check.
pub fn watch(process: String, mut main: Option<std::process::Child>) -> oneshot::Receiver<Option<i32>> {
    let (tx, rx) = oneshot::channel();
    std::thread::spawn(move || {
        let mut tracker = sys::Tracker::new(&process);
        // The main process's code while nothing else has outlived it.
        let mut code: Option<i32> = None;
        loop {
            if tx.is_closed() {
                return;
            }
            let main_exited = match main.as_mut().map(|c| c.try_wait()) {
                Some(Ok(Some(status))) => {
                    code = Some(status.code().unwrap_or(-1));
                    main = None;
                    true
                }
                Some(Err(_)) => {
                    main = None;
                    false
                }
                _ => false,
            };
            if !tracker.alive() {
                let _ = tx.send(code);
                return;
            }
            if !main_exited {
                // Something outlived the main process (or there never was one): its code is not the run's.
                code = code.filter(|_| main.is_some());
            }
            std::thread::sleep(WATCH_EVERY);
        }
    });
    rx
}

#[cfg(unix)]
mod sys {
    use super::{DetachedSpec, TERM_GRACE};
    use std::os::unix::process::CommandExt;

    fn pgid(process: &str) -> Option<libc::pid_t> {
        process.strip_prefix("pgid:")?.parse().ok().filter(|p: &libc::pid_t| *p > 1)
    }

    pub(super) fn spawn(
        _spec: &DetachedSpec,
        log: &std::fs::File,
        command: impl Fn(&std::fs::File) -> Result<std::process::Command, String>,
    ) -> Result<super::Detached, String> {
        let mut cmd = command(log)?;
        // Its own session: no controlling terminal, and its own process group whose id is its pid.
        unsafe {
            cmd.pre_exec(|| if libc::setsid() == -1 { Err(std::io::Error::last_os_error()) } else { Ok(()) });
        }
        let main = cmd.spawn().map_err(|e| e.to_string())?;
        Ok(super::Detached { process: format!("pgid:{}", main.id()), main })
    }

    pub(super) fn alive(process: &str) -> bool {
        // Signal 0 checks without sending anything. A group id is reused only once every process in it has gone
        // and a new process becomes leader of a group with that id, which within a pdtd restart is unlikely.
        pgid(process).is_some_and(|g| unsafe { libc::kill(-g, 0) } == 0)
    }

    pub(super) fn terminate(process: &str) {
        let Some(g) = pgid(process) else { return };
        if unsafe { libc::kill(-g, libc::SIGTERM) } != 0 {
            return;
        }
        std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + TERM_GRACE;
            while std::time::Instant::now() < deadline {
                if unsafe { libc::kill(-g, 0) } != 0 {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            unsafe {
                libc::kill(-g, libc::SIGKILL);
            }
        });
    }

    pub(super) struct Tracker(String);

    impl Tracker {
        pub(super) fn new(process: &str) -> Self {
            Self(process.to_string())
        }
        pub(super) fn alive(&mut self) -> bool {
            alive(&self.0)
        }
    }
}

#[cfg(windows)]
mod sys {
    use super::DetachedSpec;
    use std::collections::HashSet;
    use std::os::windows::io::AsRawHandle;
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::Foundation::{CloseHandle, DUPLICATE_SAME_ACCESS, DuplicateHandle, HANDLE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
    };
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
        JobObjectBasicAccountingInformation, JobObjectBasicProcessIdList, OpenJobObjectW, QueryInformationJobObject,
        TerminateJobObject,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcess, OpenThread, ResumeThread};

    const CREATE_SUSPENDED: u32 = 0x0000_0004;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const JOB_OBJECT_QUERY: u32 = 0x0004;
    const JOB_OBJECT_TERMINATE: u32 = 0x0008;
    const PROCESS_DUP_HANDLE: u32 = 0x0040;
    const THREAD_SUSPEND_RESUME: u32 = 0x0002;
    const ERROR_ACCESS_DENIED: i32 = 5;

    /// A handle closed on drop.
    struct Owned(HANDLE);
    // A kernel handle may be used from any thread.
    unsafe impl Send for Owned {}
    impl Drop for Owned {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn job_name(process: &str) -> Option<&str> {
        process.strip_prefix("job:").filter(|n| !n.is_empty())
    }

    fn open_job(process: &str, access: u32) -> Option<Owned> {
        let name = wide(job_name(process)?);
        let h = unsafe { OpenJobObjectW(access, 0, name.as_ptr()) };
        (!h.is_null()).then(|| Owned(h))
    }

    fn active_processes(job: &Owned) -> u32 {
        let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { std::mem::zeroed() };
        let ok = unsafe {
            QueryInformationJobObject(
                job.0,
                JobObjectBasicAccountingInformation,
                (&mut info as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                std::mem::size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 { 0 } else { info.ActiveProcesses }
    }

    /// The ids of the job's processes (up to 1024).
    fn process_ids(job: &Owned) -> Vec<u32> {
        // JOBOBJECT_BASIC_PROCESS_ID_LIST: two u32 counts, then the ids as usize.
        const MAX: usize = 1024;
        let mut buf = vec![0usize; 1 + MAX];
        let ok = unsafe {
            QueryInformationJobObject(
                job.0,
                JobObjectBasicProcessIdList,
                buf.as_mut_ptr().cast(),
                (buf.len() * std::mem::size_of::<usize>()) as u32,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Vec::new();
        }
        let in_list = (buf[0] >> 32) as usize;
        buf[1..1 + in_list.min(MAX)].iter().map(|&p| p as u32).collect()
    }

    /// Copies a handle to `job` into `process`, which keeps the job's name while that process lives.
    fn give_handle(job: &Owned, process: HANDLE) -> bool {
        let mut out: HANDLE = std::ptr::null_mut();
        unsafe { DuplicateHandle(GetCurrentProcess(), job.0, process, &mut out, 0, 0, DUPLICATE_SAME_ACCESS) != 0 }
    }

    fn give_handle_to_pid(job: &Owned, pid: u32) -> bool {
        let h = unsafe { OpenProcess(PROCESS_DUP_HANDLE, 0, pid) };
        if h.is_null() {
            return false;
        }
        let h = Owned(h);
        give_handle(job, h.0)
    }

    /// Resumes every thread of `pid` (a process started suspended has one).
    fn resume(pid: u32) -> bool {
        let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        if snap.is_null() || snap as isize == -1 {
            return false;
        }
        let snap = Owned(snap);
        let mut entry: THREADENTRY32 = unsafe { std::mem::zeroed() };
        entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
        let mut resumed = false;
        let mut more = unsafe { Thread32First(snap.0, &mut entry) } != 0;
        while more {
            if entry.th32OwnerProcessID == pid {
                let t = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
                if !t.is_null() {
                    let t = Owned(t);
                    resumed |= unsafe { ResumeThread(t.0) } != u32::MAX;
                }
            }
            more = unsafe { Thread32Next(snap.0, &mut entry) } != 0;
        }
        resumed
    }

    pub(super) fn spawn(
        spec: &DetachedSpec,
        log: &std::fs::File,
        command: impl Fn(&std::fs::File) -> Result<std::process::Command, String>,
    ) -> Result<super::Detached, String> {
        let stamp =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        let name = format!("Local\\{}-run-{}-{stamp}", pdt_core::brand::SLUG, spec.id);
        let wname = wide(&name);
        // No limits, so no kill-on-close: the job (and its processes) outlive this handle and pdtd.
        let job = unsafe { CreateJobObjectW(std::ptr::null(), wname.as_ptr()) };
        if job.is_null() {
            return Err(format!("could not create a job object: {}", std::io::Error::last_os_error()));
        }
        let job = Owned(job);
        let flags = CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP | CREATE_SUSPENDED;
        // Out of any job pdtd itself is in (one with kill-on-close would end the run with pdtd). A job that forbids
        // breaking away refuses the start; then the command starts inside it (nested in ours).
        let mut main = match command(log)?.creation_flags(flags | CREATE_BREAKAWAY_FROM_JOB).spawn() {
            Err(e) if e.raw_os_error() == Some(ERROR_ACCESS_DENIED) => {
                tracing::warn!("pdtd's job forbids breaking away; the detached run may end with pdtd");
                command(log)?.creation_flags(flags).spawn().map_err(|e| e.to_string())?
            }
            r => r.map_err(|e| e.to_string())?,
        };
        let process = main.as_raw_handle() as HANDLE;
        let fail = |main: &mut std::process::Child, what: &str| {
            let e = std::io::Error::last_os_error();
            let _ = main.kill();
            let _ = main.wait();
            format!("{what}: {e}")
        };
        if unsafe { AssignProcessToJobObject(job.0, process) } == 0 {
            return Err(fail(&mut main, "could not put the command in its job"));
        }
        give_handle(&job, process);
        if !resume(main.id()) {
            unsafe {
                TerminateJobObject(job.0, 1);
            }
            return Err(fail(&mut main, "could not resume the command"));
        }
        Ok(super::Detached { process: format!("job:{name}"), main })
    }

    pub(super) fn alive(process: &str) -> bool {
        open_job(process, JOB_OBJECT_QUERY).is_some_and(|j| active_processes(&j) > 0)
    }

    pub(super) fn terminate(process: &str) {
        if let Some(j) = open_job(process, JOB_OBJECT_TERMINATE) {
            unsafe {
                TerminateJobObject(j.0, 1);
            }
        }
    }

    /// Holds the job open while watching and hands a job handle to each new process in it.
    pub(super) struct Tracker {
        process: String,
        job: Option<Owned>,
        given: HashSet<u32>,
    }

    impl Tracker {
        pub(super) fn new(process: &str) -> Self {
            Self { process: process.to_string(), job: None, given: HashSet::new() }
        }

        pub(super) fn alive(&mut self) -> bool {
            if self.job.is_none() {
                self.job = open_job(&self.process, JOB_OBJECT_QUERY);
            }
            let Some(job) = &self.job else { return false };
            if active_processes(job) == 0 {
                return false;
            }
            let pids = process_ids(job);
            for pid in &pids {
                if !self.given.contains(pid) && give_handle_to_pid(job, *pid) {
                    self.given.insert(*pid);
                }
            }
            self.given.retain(|p| pids.contains(p));
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(dir: &std::path::Path, id: &str, command: &str) -> DetachedSpec {
        let (program, args) = if cfg!(windows) {
            (
                PathBuf::from(std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".into())),
                vec!["/d".into(), "/c".into(), command.into()],
            )
        } else {
            (PathBuf::from("/bin/sh"), vec!["-c".into(), command.into()])
        };
        DetachedSpec {
            id: id.into(),
            program,
            args,
            cwd: dir.to_path_buf(),
            env_remove: vec![],
            env: vec![("PDT_TEST_VALUE".into(), "hello".into())],
            log: dir.join("logs").join("command.log"),
        }
    }

    async fn wait_until(mut f: impl FnMut() -> bool) -> bool {
        for _ in 0..100 {
            if f() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        false
    }

    /// A command that prints and then waits a long time (until it is ended).
    fn long_command() -> &'static str {
        if cfg!(windows) {
            "echo started %PDT_TEST_VALUE% && ping -n 60 127.0.0.1 >nul"
        } else {
            "echo started $PDT_TEST_VALUE; sleep 60"
        }
    }

    #[tokio::test]
    async fn runs_logs_and_is_ended_as_a_whole() {
        let tmp = tempfile::tempdir().unwrap();
        let s = spec(tmp.path(), "t1", long_command());
        let d = spawn(&s).unwrap();
        assert!(alive(&d.process));
        let log = s.log.clone();
        assert!(
            wait_until(|| std::fs::read_to_string(&log).is_ok_and(|t| t.contains("started hello"))).await,
            "output goes to the log, with the spec's environment"
        );
        let done = watch(d.process.clone(), Some(d.main));
        terminate(&d.process);
        let code = tokio::time::timeout(Duration::from_secs(10), done).await.expect("ended in time").unwrap();
        assert_ne!(code, Some(0), "ended, not a clean exit");
        assert!(!alive(&d.process));
    }

    /// Without its handle in this process (as after a pdtd restart), the processes are still found and ended.
    #[tokio::test]
    async fn found_again_by_name_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let d = spawn(&spec(tmp.path(), "t2", long_command())).unwrap();
        let process = d.process.clone();
        // Reaped in the background, as a new pdtd would not be its parent.
        let mut main = d.main;
        std::thread::spawn(move || main.wait());
        assert!(alive(&process));
        let done = watch(process.clone(), None);
        terminate(&process);
        let code = tokio::time::timeout(Duration::from_secs(10), done).await.expect("ended in time").unwrap();
        assert_eq!(code, None, "no main process: the code is unknown");
        assert!(!alive(&process));
    }

    /// An app that restarts itself (starts a copy and exits) is still alive; its code is no longer the run's.
    #[tokio::test]
    async fn a_child_that_outlives_the_main_process_keeps_it_alive() {
        let tmp = tempfile::tempdir().unwrap();
        // The child outlives the check by far: a loaded Windows runner is slow to start one.
        let command =
            if cfg!(windows) { "start /b ping -n 6 127.0.0.1 >nul && exit /b 0" } else { "(sleep 5) & exit 0" };
        let d = spawn(&spec(tmp.path(), "t3", command)).unwrap();
        let process = d.process.clone();
        let done = watch(process.clone(), Some(d.main));
        tokio::time::sleep(Duration::from_millis(700)).await;
        assert!(alive(&process), "the child is still running after the main process exited");
        let code = tokio::time::timeout(Duration::from_secs(15), done).await.expect("ended in time").unwrap();
        assert_eq!(code, None);
        assert!(!alive(&process));
    }

    #[tokio::test]
    async fn the_main_process_code_when_it_is_the_last() {
        let tmp = tempfile::tempdir().unwrap();
        let d = spawn(&spec(tmp.path(), "t4", "exit 3")).unwrap();
        let done = watch(d.process.clone(), Some(d.main));
        let code = tokio::time::timeout(Duration::from_secs(10), done).await.expect("ended in time").unwrap();
        assert_eq!(code, Some(3));
    }

    #[test]
    fn unknown_forms_are_gone() {
        for p in ["", "pgid:", "pgid:1", "pgid:-5", "job:", "nonsense"] {
            assert!(!alive(p), "{p:?}");
            terminate(p);
        }
    }
}
