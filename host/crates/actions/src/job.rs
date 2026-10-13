//! A process and everything it starts, ended together.
//!
//! On Windows, killing a plugin's process leaves the programs it started running (a
//! soundboard's players keep playing). [`ProcessTree::spawn`] and [`ProcessTree::adopt`]
//! put the process in a Windows job object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`:
//! when the tree is ended or dropped, or the host dies, Windows ends every process
//! still in the job. Elsewhere they do nothing.
//!
//! Processes started inside the job stay in it, with one way out: the job allows
//! *explicit* breakaway (`JOB_OBJECT_LIMIT_BREAKAWAY_OK`), so a program started with
//! `CREATE_BREAKAWAY_FROM_JOB` leaves it and outlives the plugin. That's for programs a
//! plugin opens for the user (an app from a Focus key, a browser for signing in); the
//! Python SDK's `open_for_user` and `start_for_user` do it. Nothing leaves silently:
//! the `py.exe` launcher, for one, puts Python in a job of its own that lets children
//! break away without asking, but per Microsoft's "Nested Jobs" a child only breaks
//! away up to the first job that doesn't allow it, and a job that allows only explicit
//! breakaway keeps a child that didn't ask (checked on Windows 11).

/// How long [`ProcessTree::end`] waits for the processes it ended to be gone.
#[cfg(windows)]
const END_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

/// Dropping it ends every process left in the tree (Windows only).
pub struct ProcessTree {
    #[cfg(windows)]
    job: imp::Job,
}

impl ProcessTree {
    /// Start `cmd` (with these Windows process creation flags) in a tree of its own.
    /// On Windows the process starts suspended and only runs once it's in the job, so
    /// nothing it starts can slip out first (`py.exe`, say, starting Python before an
    /// [`adopt`](Self::adopt) right after the spawn). The tree is `None` when Windows
    /// refused a job; the process then runs as before.
    pub fn spawn(
        cmd: &mut tokio::process::Command,
        creation_flags: u32,
    ) -> std::io::Result<(tokio::process::Child, Option<ProcessTree>)> {
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::Threading::CREATE_SUSPENDED;
            cmd.creation_flags(creation_flags | CREATE_SUSPENDED);
            let mut child = cmd.spawn()?;
            let tree = Self::adopt(&child);
            let resumed = match child.id() {
                Some(pid) => imp::resume(pid),
                None => Err(std::io::Error::other("it exited before it started")),
            };
            if let Err(e) = resumed {
                // Left suspended it would never run, so end it and say why.
                let _ = child.start_kill();
                return Err(std::io::Error::new(
                    e.kind(),
                    format!("it couldn't be resumed: {e}"),
                ));
            }
            Ok((child, tree))
        }
        #[cfg(not(windows))]
        {
            let _ = creation_flags;
            Ok((cmd.spawn()?, None))
        }
    }

    /// Put a running `child`, and whatever it starts from now on, in a tree of its
    /// own. `None` when that isn't possible (not Windows, or Windows refused): the
    /// process then runs as before, and only it is ended when stopped.
    pub fn adopt(child: &tokio::process::Child) -> Option<ProcessTree> {
        #[cfg(windows)]
        {
            let handle = child.raw_handle()?;
            match imp::Job::adopt(handle) {
                Ok(job) => Some(ProcessTree { job }),
                Err(e) => {
                    tracing::warn!(
                        "couldn't put the process in a job object, so programs it starts \
                         may outlive it: {e}"
                    );
                    None
                }
            }
        }
        #[cfg(not(windows))]
        {
            let _ = child;
            None
        }
    }

    /// End every process still in the tree, and wait (up to two seconds) until they're
    /// gone: closing the job only starts ending them, and a folder one of them runs
    /// from, or works in, can't be deleted until they have.
    pub async fn end(self) {
        #[cfg(windows)]
        {
            self.job.terminate();
            let until = std::time::Instant::now() + END_WAIT;
            while self.job.active_processes() > 0 && std::time::Instant::now() < until {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        }
    }
}

impl Drop for ProcessTree {
    /// The job object (Windows) is closed right after this, which ends the processes
    /// still in it. Spelled out so callers can `drop` a tree to end it at a set point.
    fn drop(&mut self) {}
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, HANDLE, INVALID_HANDLE_VALUE},
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD,
                THREADENTRY32,
            },
            JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicAccountingInformation,
                JobObjectExtendedLimitInformation, QueryInformationJobObject,
                SetInformationJobObject, TerminateJobObject,
                JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_BASIC_LIMIT_INFORMATION,
                JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_BREAKAWAY_OK,
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            },
            Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME},
        },
    };

    /// An owned job object handle, closed (ending its processes) on drop.
    pub struct Job(HANDLE);

    // SAFETY: a job handle is a kernel handle, usable from any thread.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    impl Job {
        pub fn adopt(process: HANDLE) -> std::io::Result<Job> {
            // SAFETY: plain Win32 calls. The job handle is owned by `Job`, which closes
            // it on every path; `info` outlives the call that reads it, and its size is
            // passed with it.
            unsafe {
                let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if handle.is_null() {
                    return Err(std::io::Error::last_os_error());
                }
                let job = Job(handle);
                let info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
                    BasicLimitInformation: JOBOBJECT_BASIC_LIMIT_INFORMATION {
                        LimitFlags: JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
                            | JOB_OBJECT_LIMIT_BREAKAWAY_OK,
                        ..Default::default()
                    },
                    ..Default::default()
                };
                if SetInformationJobObject(
                    job.0,
                    JobObjectExtendedLimitInformation,
                    &info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION
                        as *const core::ffi::c_void,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                ) == 0
                {
                    return Err(std::io::Error::last_os_error());
                }
                if AssignProcessToJobObject(job.0, process) == 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(job)
            }
        }

        /// Start ending every process in the job (this doesn't wait for them).
        pub fn terminate(&self) {
            // SAFETY: the handle is ours and still open.
            unsafe {
                TerminateJobObject(self.0, 1);
            }
        }

        /// How many processes in the job are still running (0 if Windows won't say).
        pub fn active_processes(&self) -> u32 {
            let mut info = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
            // SAFETY: `info` outlives the call that fills it, and its size is passed
            // with it.
            let ok = unsafe {
                QueryInformationJobObject(
                    self.0,
                    JobObjectBasicAccountingInformation,
                    &mut info as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION
                        as *mut core::ffi::c_void,
                    std::mem::size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                0
            } else {
                info.ActiveProcesses
            }
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY: the handle is ours and closed only here.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    /// Let a process started with `CREATE_SUSPENDED` run by resuming its threads. std
    /// keeps the main thread's handle to itself, so this finds them by process id.
    ///
    /// Every thread is resumed, not only the first one found: security software can
    /// add a thread of its own before the process runs, and resuming only that one
    /// would leave the main thread suspended for good. Resuming a thread that isn't
    /// suspended does nothing. An error means the process may not run, so the caller
    /// ends it.
    pub fn resume(pid: u32) -> std::io::Result<()> {
        let threads = threads_of(pid)?;
        if threads.is_empty() {
            return Err(std::io::Error::other("its thread wasn't found"));
        }
        for thread in threads {
            // SAFETY: plain Win32 calls; the thread handle is closed before the next.
            unsafe {
                let handle = OpenThread(THREAD_SUSPEND_RESUME, 0, thread);
                if handle.is_null() {
                    let error = std::io::Error::last_os_error();
                    // A thread that has ended since the snapshot needs no resuming.
                    if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) {
                        continue;
                    }
                    return Err(error);
                }
                let resumed = ResumeThread(handle);
                let error = std::io::Error::last_os_error();
                CloseHandle(handle);
                if resumed == u32::MAX {
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    /// The ids of the threads process `pid` has now.
    fn threads_of(pid: u32) -> std::io::Result<Vec<u32>> {
        // SAFETY: plain Win32 calls. The snapshot is closed before returning, and
        // `entry` outlives the calls that fill it, with its size set.
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
            if snapshot == INVALID_HANDLE_VALUE {
                return Err(std::io::Error::last_os_error());
            }
            let mut entry = THREADENTRY32 {
                dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
                ..Default::default()
            };
            let mut threads = Vec::new();
            let mut more = Thread32First(snapshot, &mut entry) != 0;
            while more {
                if entry.th32OwnerProcessID == pid {
                    threads.push(entry.th32ThreadID);
                }
                more = Thread32Next(snapshot, &mut entry) != 0;
            }
            CloseHandle(snapshot);
            Ok(threads)
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::time::Duration;

    fn alive(pid: u32) -> bool {
        let out = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).contains(&format!("\"{pid}\""))
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_spawned_tree_runs_and_ends_together() {
        // `cmd /c` starts ping as a child process of its own.
        let mut cmd = tokio::process::Command::new("cmd");
        cmd.args(["/c", "ping -n 60 127.0.0.1 >nul"])
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true);
        let (mut child, tree) = ProcessTree::spawn(&mut cmd, 0x0800_0000).unwrap(); // CREATE_NO_WINDOW
        let tree = tree.expect("a job object");
        // It was resumed: cmd runs and starts ping, so the job soon holds two.
        let mut running = 0;
        for _ in 0..200 {
            running = tree.job.active_processes();
            if running >= 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(running >= 2, "cmd and its ping run in the job ({running})");
        let pid = child.id().unwrap();
        tree.end().await;
        assert!(!alive(pid), "ending the tree ended cmd");
        assert!(child.wait().await.is_ok());
    }

    #[test]
    #[allow(unsafe_code)]
    fn resume_wakes_every_suspended_thread_not_only_the_first() {
        use std::sync::{
            atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
            Arc,
        };
        use windows_sys::Win32::{
            Foundation::CloseHandle,
            System::Threading::{
                GetCurrentThreadId, OpenThread, ResumeThread, SuspendThread, THREAD_SUSPEND_RESUME,
            },
        };

        // Two threads of this process stand in for a main thread and one added before
        // it, both suspended. Each counts while it runs, and holds no lock while it does,
        // so suspending it can't block anything else here.
        let stop = Arc::new(AtomicBool::new(false));
        let workers: Vec<_> = (0..2)
            .map(|_| {
                let (count, id) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU32::new(0)));
                let (c, i, s) = (count.clone(), id.clone(), stop.clone());
                let join = std::thread::spawn(move || {
                    // SAFETY: no arguments; it only reads this thread's id.
                    i.store(unsafe { GetCurrentThreadId() }, Ordering::SeqCst);
                    while !s.load(Ordering::Relaxed) {
                        c.fetch_add(1, Ordering::Relaxed);
                        std::hint::spin_loop();
                    }
                });
                (count, id, join)
            })
            .collect();
        let counting = |w: &(Arc<AtomicU64>, Arc<AtomicU32>, _)| {
            let before = w.0.load(Ordering::Relaxed);
            std::thread::sleep(Duration::from_millis(50));
            w.0.load(Ordering::Relaxed) != before
        };
        let mut handles = Vec::new();
        for w in &workers {
            while w.0.load(Ordering::Relaxed) == 0 {
                std::thread::sleep(Duration::from_millis(1));
            }
            // SAFETY: the thread is ours and still running; its handle is closed below.
            let handle =
                unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, w.1.load(Ordering::SeqCst)) };
            assert!(!handle.is_null());
            // SAFETY: a valid thread handle with suspend access.
            assert_eq!(unsafe { SuspendThread(handle) }, 0);
            handles.push(handle);
        }
        // Suspending takes effect a moment later.
        std::thread::sleep(Duration::from_millis(50));
        let were_suspended = workers.iter().all(|w| !counting(w));

        let resumed = imp::resume(std::process::id());
        let all_run = workers.iter().all(counting);

        // Let them go whatever happened, so a failure doesn't hang the test.
        for handle in handles {
            // SAFETY: the handles opened above, each closed once here.
            unsafe {
                ResumeThread(handle);
                CloseHandle(handle);
            }
        }
        stop.store(true, Ordering::Relaxed);
        for (_, _, join) in workers {
            join.join().unwrap();
        }
        assert!(were_suspended, "both threads were suspended");
        resumed.unwrap();
        assert!(all_run, "resume let every thread of the process run");
    }
}
