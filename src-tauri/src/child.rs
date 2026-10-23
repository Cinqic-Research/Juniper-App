//! Conventions for the processes Juniper starts: the local runtime, Ollama
//! for a GGUF import, and local MCP servers.

use tokio::process::{Child, Command};

/// Prepares a child of this process so it opens no console window.
///
/// Windows gives a console-subsystem child a console of its own whenever the
/// parent has none, and release Juniper has none (`windows_subsystem =
/// "windows"`): that is the terminal window that appeared beside the
/// application while `llama-server.exe` ran. Redirecting the child's
/// standard streams does not prevent it; `CREATE_NO_WINDOW` does. A no-op
/// elsewhere.
pub fn without_console(command: &mut Command) -> &mut Command {
    #[cfg(windows)]
    command.creation_flags(windows::CREATE_NO_WINDOW);
    command
}

/// Ends the child when this process ends, however it ends.
///
/// The Linux local runtime asks for `PR_SET_PDEATHSIG`. Windows has no
/// parent-death signal; a job object with `KILL_ON_JOB_CLOSE` is closed by
/// the kernel when Juniper exits or is killed from Task Manager, and every
/// process assigned to it is terminated with it. Failure is not fatal: the
/// child runs as before and exit cleanup still stops it on a normal quit.
pub fn end_with_parent(child: &Child) {
    #[cfg(windows)]
    windows::assign_to_job(child);
    #[cfg(not(windows))]
    let _ = child;
}

#[cfg(windows)]
mod windows {
    use std::sync::OnceLock;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };

    /// `CREATE_NO_WINDOW` from `processthreadsapi.h`.
    pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    /// The job every child joins. Its handle is deliberately kept for the
    /// life of the process: closing it is what ends the children.
    struct Job(HANDLE);
    // A job handle is a kernel object reference; it is valid from any thread.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    fn job() -> Option<HANDLE> {
        static JOB: OnceLock<Option<Job>> = OnceLock::new();
        JOB.get_or_init(|| {
            // SAFETY: plain Win32 calls with a zeroed, correctly sized
            // information block; the handle is closed on failure.
            unsafe {
                let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if handle.is_null() {
                    return None;
                }
                let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let set = SetInformationJobObject(
                    handle,
                    JobObjectExtendedLimitInformation,
                    std::ptr::from_ref(&limits).cast(),
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
                if set == 0 {
                    CloseHandle(handle);
                    return None;
                }
                Some(Job(handle))
            }
        })
        .as_ref()
        .map(|job| job.0)
    }

    pub fn assign_to_job(child: &tokio::process::Child) {
        if let (Some(job), Some(process)) = (job(), child.raw_handle()) {
            // SAFETY: both handles are live; the call only fails when the
            // child is already in an incompatible job, which leaves it as it was.
            unsafe {
                AssignProcessToJobObject(job, process.cast());
            }
        }
    }
}
