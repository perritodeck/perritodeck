//! A process and everything it starts, ended together.
//!
//! On Windows, killing a plugin's process leaves the programs it started running (a
//! soundboard's players keep playing). [`ProcessTree::adopt`] puts the process in a
//! Windows job object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`: when the tree is
//! dropped, or the host dies, Windows ends every process still in the job. Processes
//! started inside it stay in it (the job allows no breakaway, and per Microsoft's
//! "Nested Jobs" a child only breaks away up to the first job that doesn't allow it, so
//! `py.exe`'s own job doesn't let them escape this one). Elsewhere it does nothing.

/// Dropping it ends every process left in the tree (Windows only).
pub struct ProcessTree {
    #[cfg(windows)]
    _job: imp::Job,
}

impl ProcessTree {
    /// Put `child`, and whatever it starts from now on, in a tree of its own. `None`
    /// when that isn't possible (not Windows, or Windows refused): the process then
    /// runs as before, and only it is ended when stopped.
    pub fn adopt(child: &tokio::process::Child) -> Option<ProcessTree> {
        #[cfg(windows)]
        {
            let handle = child.raw_handle()?;
            match imp::Job::adopt(handle) {
                Ok(job) => Some(ProcessTree { _job: job }),
                Err(e) => {
                    tracing::debug!("couldn't put the process in a job object: {e}");
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
        Foundation::{CloseHandle, HANDLE},
        System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_BASIC_LIMIT_INFORMATION,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
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
                        LimitFlags: JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
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
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY: the handle is ours and closed only here.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}
