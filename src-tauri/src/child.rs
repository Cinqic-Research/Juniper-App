//! Native subprocess ownership. Launch fails if lifecycle ownership cannot be established.
use tokio::process::{Child, Command};

pub fn spawn_owned(command: &mut Command) -> std::io::Result<Child> {
    #[cfg(windows)]
    return windows::spawn(command);
    #[cfg(not(windows))]
    command.kill_on_drop(true).spawn()
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::io;
    use std::sync::OnceLock;
    use windows_sys::Win32::Foundation::{
        CloseHandle, DUPLICATE_SAME_ACCESS, DuplicateHandle, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
    };
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };
    use windows_sys::Win32::System::Threading::{
        CREATE_SUSPENDED, GetCurrentProcess, INFINITE, OpenThread, ResumeThread,
        THREAD_SUSPEND_RESUME, WaitForSingleObject,
    };

    struct Handle(HANDLE);
    // Kernel handles can be used and closed from another thread.
    unsafe impl Send for Handle {}
    unsafe impl Sync for Handle {}
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }

    fn checked(handle: HANDLE) -> io::Result<Handle> {
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            Err(io::Error::last_os_error())
        } else {
            Ok(Handle(handle))
        }
    }

    fn new_job() -> io::Result<Handle> {
        // The handle is deliberately non-inheritable.
        unsafe {
            let job = checked(CreateJobObjectW(std::ptr::null(), std::ptr::null()))?;
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&limits).cast(),
                std::mem::size_of_val(&limits) as u32,
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(job)
        }
    }

    fn parent_job() -> io::Result<()> {
        static PARENT: OnceLock<Result<Handle, i32>> = OnceLock::new();
        // Join before CreateProcess: inherited ownership is atomic even if the
        // parent crashes between creating the suspended child and assigning it.
        match PARENT.get_or_init(|| {
            let job = new_job().map_err(|e| e.raw_os_error().unwrap_or(1))?;
            if unsafe { AssignProcessToJobObject(job.0, GetCurrentProcess()) } == 0 {
                return Err(io::Error::last_os_error().raw_os_error().unwrap_or(1));
            }
            Ok(job)
        }) {
            Ok(_) => Ok(()),
            Err(code) => Err(io::Error::from_raw_os_error(*code)),
        }
    }

    fn initial_thread(pid: u32) -> io::Result<Handle> {
        unsafe {
            let snapshot = checked(CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0))?;
            let mut entry: THREADENTRY32 = std::mem::zeroed();
            entry.dwSize = std::mem::size_of_val(&entry) as u32;
            let mut found = Thread32First(snapshot.0, &mut entry);
            while found != 0 {
                if entry.th32OwnerProcessID == pid {
                    return checked(OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID));
                }
                found = Thread32Next(snapshot.0, &mut entry);
            }
        }
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Child thread unavailable",
        ))
    }

    pub fn spawn(command: &mut Command) -> io::Result<Child> {
        parent_job()?;
        let job = new_job()?;
        let mut child = command
            .creation_flags(0x0800_0000 | CREATE_SUSPENDED)
            .kill_on_drop(true)
            .spawn()?;
        let result = (|| {
            // No user code runs before assignment. Incompatible enclosing jobs
            // fail closed rather than silently weakening cleanup guarantees.
            let process = child
                .raw_handle()
                .ok_or_else(|| io::Error::other("Child handle unavailable"))?;
            if unsafe { AssignProcessToJobObject(job.0, process.cast()) } == 0 {
                return Err(io::Error::last_os_error());
            }
            let thread = initial_thread(
                child
                    .id()
                    .ok_or_else(|| io::Error::other("Child PID unavailable"))?,
            )?;
            let mut duplicate = std::ptr::null_mut();
            if unsafe {
                DuplicateHandle(
                    GetCurrentProcess(),
                    process.cast(),
                    GetCurrentProcess(),
                    &mut duplicate,
                    0,
                    0,
                    DUPLICATE_SAME_ACCESS,
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            let process = checked(duplicate)?;
            // Close this child's job when its root exits, killing descendants
            // after cancellation too. A native waiter does not occupy an async
            // worker or keep Tokio shutdown waiting for a live child.
            std::thread::Builder::new()
                .name("juniper-child-owner".into())
                .spawn(move || {
                    unsafe { WaitForSingleObject(process.0, INFINITE) };
                    drop(job);
                    drop(process);
                })?;
            if unsafe { ResumeThread(thread.0) } == u32::MAX {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        })();
        if let Err(error) = result {
            let _ = child.start_kill();
            return Err(error);
        }
        Ok(child)
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_ACTIVE_PROCESS,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
    };

    struct Process(HANDLE);
    impl Process {
        fn open(pid: u32) -> Self {
            let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
            assert!(
                !handle.is_null(),
                "open fixture process {pid}: {}",
                std::io::Error::last_os_error()
            );
            Self(handle)
        }
        fn assert_running(&self) {
            assert_eq!(unsafe { WaitForSingleObject(self.0, 0) }, WAIT_TIMEOUT);
        }
        fn assert_exited(&self) {
            assert_eq!(
                unsafe { WaitForSingleObject(self.0, 10_000) },
                WAIT_OBJECT_0
            );
        }
    }
    impl Drop for Process {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }

    fn python() -> String {
        if let Ok(path) = std::env::var("JUNIPER_MCP_TEST_PYTHON") {
            return path;
        }
        let result = std::process::Command::new("python")
            .args(["-c", "import sys; print(sys.executable)"])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "Python is required for process-tree fixtures"
        );
        String::from_utf8(result.stdout).unwrap().trim().to_owned()
    }

    fn marker() -> PathBuf {
        std::env::temp_dir().join(format!("juniper-child-tree-{}.json", uuid::Uuid::new_v4()))
    }

    fn tree_command(marker: &Path) -> Command {
        let mut command = Command::new(python());
        command.args(["-c", "import json,os,pathlib,subprocess,sys,time; child=subprocess.Popen([sys.executable,'-c','import time; time.sleep(120)']); pathlib.Path(sys.argv[1]).write_text(json.dumps([os.getpid(),child.pid])); time.sleep(120)"])
            .arg(marker)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        command
    }

    fn tree_processes(marker: &Path) -> (Process, Process) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Ok(bytes) = std::fs::read(marker)
                && let Ok(ids) = serde_json::from_slice::<Vec<u32>>(&bytes)
            {
                return (Process::open(ids[0]), Process::open(ids[1]));
            }
            assert!(
                Instant::now() < deadline,
                "fixture must publish its process tree"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn owned_children_kill_descendants_without_stopping_concurrent_roots() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let first_marker = marker();
            let second_marker = marker();
            let mut first = spawn_owned(&mut tree_command(&first_marker)).unwrap();
            let mut second = spawn_owned(&mut tree_command(&second_marker)).unwrap();
            let (first_root, first_descendant) = tree_processes(&first_marker);
            let (second_root, second_descendant) = tree_processes(&second_marker);
            first.start_kill().unwrap();
            first.wait().await.unwrap();
            first_root.assert_exited();
            first_descendant.assert_exited();
            second_root.assert_running();
            second_descendant.assert_running();
            second.start_kill().unwrap();
            second.wait().await.unwrap();
            second_root.assert_exited();
            second_descendant.assert_exited();
            std::fs::remove_file(first_marker).unwrap();
            std::fs::remove_file(second_marker).unwrap();
        });
    }

    // Invoked in an isolated copy of the test harness. This is deliberately
    // inert in ordinary suite runs; the outer tests set both variables.
    #[test]
    fn owned_process_helper() {
        let Ok(mode) = std::env::var("JUNIPER_CHILD_HELPER_MODE") else {
            return;
        };
        let marker = PathBuf::from(std::env::var_os("JUNIPER_CHILD_HELPER_MARKER").unwrap());
        if mode == "limited" {
            unsafe {
                let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                assert!(!job.is_null());
                let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
                limits.BasicLimitInformation.ActiveProcessLimit = 1;
                assert_ne!(
                    SetInformationJobObject(
                        job,
                        JobObjectExtendedLimitInformation,
                        std::ptr::from_ref(&limits).cast(),
                        std::mem::size_of_val(&limits) as u32
                    ),
                    0
                );
                assert_ne!(AssignProcessToJobObject(job, GetCurrentProcess()), 0);
                let runtime = tokio::runtime::Runtime::new().unwrap();
                let _entered = runtime.enter();
                assert!(
                    spawn_owned(&mut tree_command(&marker)).is_err(),
                    "restrictive job must fail closed"
                );
                CloseHandle(job);
            }
            assert!(!marker.exists(), "unowned fixture user code must never run");
        } else {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            let _entered = runtime.enter();
            let _child = spawn_owned(&mut tree_command(&marker)).unwrap();
            std::thread::sleep(Duration::from_secs(120));
        }
    }

    fn helper(mode: &str, marker: &Path) -> std::process::Child {
        std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "child::tests::owned_process_helper",
                "--nocapture",
            ])
            .env("JUNIPER_CHILD_HELPER_MODE", mode)
            .env("JUNIPER_CHILD_HELPER_MARKER", marker)
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap()
    }

    #[test]
    fn forced_parent_death_ends_the_owned_process_tree() {
        let marker = marker();
        let mut parent = helper("tree", &marker);
        let (root, descendant) = tree_processes(&marker);
        parent.kill().unwrap();
        parent.wait().unwrap();
        root.assert_exited();
        descendant.assert_exited();
        std::fs::remove_file(marker).unwrap();
    }

    #[test]
    fn restrictive_enclosing_job_refuses_unowned_launch() {
        let marker = marker();
        assert!(helper("limited", &marker).wait().unwrap().success());
        assert!(!marker.exists());
    }
}
