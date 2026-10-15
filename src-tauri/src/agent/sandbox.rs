//! Sandboxed code execution for agent tasks. On Linux, programs run under
//! bubblewrap in fresh user, mount, PID, network, IPC, and UTS namespaces:
//! the system's `/usr` read-only, a private writable working folder, an
//! optional project folder read-only, no network interfaces but loopback,
//! a cleared environment, and CPU, memory, file-size, and wall-time limits.
//! A working folder or virtual environment alone is not a sandbox, so on
//! platforms without a verified sandbox this capability is unavailable.

use super::tools::{Execution, Outcome, ToolError, create_artifact};
use crate::providers::TranscriptToolCall;
use serde_json::json;

#[cfg(target_os = "linux")]
const WALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
#[cfg(target_os = "linux")]
const MAX_STREAM_BYTES: usize = 64 * 1024;
#[cfg(target_os = "linux")]
const MAX_OUTPUT_FILES: usize = 5;
#[cfg(target_os = "linux")]
const MAX_OUTPUT_FILE_BYTES: u64 = 1024 * 1024;

fn fail(code: &str, message: impl Into<String>) -> ToolError {
    (code.to_owned(), message.into())
}

#[cfg(target_os = "linux")]
fn bubblewrap() -> Option<&'static str> {
    ["/usr/bin/bwrap", "/bin/bwrap"]
        .into_iter()
        .find(|path| std::path::Path::new(path).is_file())
}

/// Whether sandboxed execution works here. On Linux this runs a probe
/// sandbox once and caches the answer.
pub fn availability() -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        static PROBE: std::sync::OnceLock<Result<(), String>> = std::sync::OnceLock::new();
        PROBE
            .get_or_init(|| {
                let bwrap = bubblewrap().ok_or("Install bubblewrap (bwrap) to run code in a sandbox.")?;
                if !std::path::Path::new("/usr/bin/python3").is_file() {
                    return Err("Python 3 (/usr/bin/python3) is not installed.".into());
                }
                let status = std::process::Command::new(bwrap)
                    .args(["--unshare-all", "--die-with-parent", "--ro-bind", "/usr", "/usr", "--proc", "/proc", "--dev", "/dev"])
                    .args(system_links())
                    .args(["/usr/bin/true"])
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .map_err(|_| "bubblewrap could not start.".to_owned())?;
                if status.success() {
                    Ok(())
                } else {
                    Err("This system does not allow unprivileged sandboxes (user namespaces are disabled).".into())
                }
            })
            .clone()
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(
            "Sandboxed code runs on Linux in this beta. This platform has no verified sandbox yet."
                .into(),
        )
    }
}

/// `/lib`, `/bin`, and friends: recreated as the same links on merged-/usr
/// systems, or bound read-only where they are real folders.
#[cfg(target_os = "linux")]
fn system_links() -> Vec<String> {
    let mut args = Vec::new();
    for name in ["/lib", "/lib64", "/lib32", "/bin", "/sbin"] {
        match std::fs::symlink_metadata(name) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                if let Ok(target) = std::fs::read_link(name) {
                    args.extend([
                        "--symlink".to_owned(),
                        target.to_string_lossy().into_owned(),
                        name.to_owned(),
                    ]);
                }
            }
            Ok(metadata) if metadata.is_dir() => {
                args.extend(["--ro-bind".to_owned(), name.to_owned(), name.to_owned()]);
            }
            _ => {}
        }
    }
    for file in ["/etc/ld.so.cache", "/etc/alternatives", "/etc/localtime"] {
        args.extend(["--ro-bind-try".to_owned(), file.to_owned(), file.to_owned()]);
    }
    args
}

#[cfg(target_os = "linux")]
pub struct Limits {
    pub cpu_seconds: u64,
    pub address_space: u64,
    pub file_size: u64,
}

#[cfg(target_os = "linux")]
pub struct Finished {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    pub cancelled: bool,
}

/// Tasks (threads included) that count against RLIMIT_NPROC for this user.
#[cfg(target_os = "linux")]
fn user_task_count() -> u64 {
    // SAFETY: getuid has no preconditions.
    let uid = unsafe { libc::getuid() };
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return 4096;
    };
    entries
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .chars()
                .all(|character| character.is_ascii_digit())
        })
        .filter(|entry| {
            use std::os::unix::fs::MetadataExt;
            entry.metadata().is_ok_and(|metadata| metadata.uid() == uid)
        })
        .map(|entry| {
            std::fs::read_to_string(entry.path().join("status"))
                .ok()
                .and_then(|status| {
                    status.lines().find_map(|line| {
                        line.strip_prefix("Threads:")
                            .and_then(|value| value.trim().parse::<u64>().ok())
                    })
                })
                .unwrap_or(1)
        })
        .sum()
}

/// Runs `program` inside the sandbox with `work` as its only writable
/// folder (mounted at /work) and an optional read-only `/project`.
#[cfg(target_os = "linux")]
pub async fn run(
    work: &std::path::Path,
    project: Option<&std::path::Path>,
    cwd: &str,
    program: &[String],
    limits: Limits,
    cancellation: &crate::commands::Cancellation,
) -> Result<Finished, ToolError> {
    use std::os::unix::process::CommandExt;
    use tokio::io::AsyncReadExt;
    availability().map_err(|reason| fail("SANDBOX_UNAVAILABLE", reason))?;
    let bwrap =
        bubblewrap().ok_or_else(|| fail("SANDBOX_UNAVAILABLE", "bubblewrap is not installed."))?;
    let mut args: Vec<String> = [
        "--unshare-all",
        "--die-with-parent",
        "--new-session",
        "--clearenv",
        "--setenv",
        "PATH",
        "/usr/bin:/bin",
        "--setenv",
        "HOME",
        "/work",
        "--setenv",
        "LANG",
        "C.UTF-8",
        "--setenv",
        "PYTHONDONTWRITEBYTECODE",
        "1",
        "--setenv",
        "PYTHONNOUSERSITE",
        "1",
        "--ro-bind",
        "/usr",
        "/usr",
        "--proc",
        "/proc",
        "--dev",
        "/dev",
        "--tmpfs",
        "/tmp",
    ]
    .iter()
    .map(|value| (*value).to_owned())
    .collect();
    args.extend(system_links());
    args.extend([
        "--bind".into(),
        work.to_string_lossy().into_owned(),
        "/work".into(),
    ]);
    if let Some(project) = project {
        args.extend([
            "--ro-bind".into(),
            project.to_string_lossy().into_owned(),
            "/project".into(),
        ]);
    }
    args.extend(["--chdir".into(), cwd.to_owned(), "--".into()]);
    args.extend(program.iter().cloned());
    // A fork bomb inside the sandbox stops at this many extra tasks.
    let process_limit = user_task_count() + 256;
    let mut command = tokio::process::Command::new(bwrap);
    command
        .args(&args)
        .env_clear()
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let (cpu, memory, file_size) = (limits.cpu_seconds, limits.address_space, limits.file_size);
    // SAFETY: only async-signal-safe calls (setpgid, setrlimit) run between
    // fork and exec.
    unsafe {
        command.as_std_mut().pre_exec(move || {
            libc::setpgid(0, 0);
            let set = |resource, value: u64| {
                let limit = libc::rlimit {
                    rlim_cur: value,
                    rlim_max: value,
                };
                libc::setrlimit(resource, &limit);
            };
            set(libc::RLIMIT_CPU, cpu);
            set(libc::RLIMIT_AS, memory);
            set(libc::RLIMIT_FSIZE, file_size);
            set(libc::RLIMIT_NOFILE, 256);
            set(libc::RLIMIT_NPROC, process_limit);
            set(libc::RLIMIT_CORE, 0);
            Ok(())
        });
    }
    let mut child = command
        .spawn()
        .map_err(|_| fail("SANDBOX_FAILED", "The sandbox could not start."))?;
    let process_group = child.id().map(|id| id as i32);
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| fail("SANDBOX_FAILED", "No output stream."))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| fail("SANDBOX_FAILED", "No error stream."))?;
    let read_bounded = |mut stream: Box<dyn tokio::io::AsyncRead + Unpin + Send>| async move {
        let mut collected = Vec::new();
        let mut buffer = [0u8; 8192];
        loop {
            match stream.read(&mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(count) => {
                    if collected.len() < MAX_STREAM_BYTES {
                        let room = MAX_STREAM_BYTES - collected.len();
                        collected.extend_from_slice(&buffer[..count.min(room)]);
                    }
                }
            }
        }
        collected
    };
    let out_task = tokio::spawn(read_bounded(Box::new(stdout)));
    let err_task = tokio::spawn(read_bounded(Box::new(stderr)));
    let kill_group = |group: Option<i32>| {
        if let Some(group) = group {
            // SAFETY: signalling our own child's process group.
            unsafe {
                libc::kill(-group, libc::SIGKILL);
            }
        }
    };
    let (status, timed_out, cancelled) = tokio::select! {
        status = child.wait() => (status.ok(), false, false),
        _ = tokio::time::sleep(WALL_TIMEOUT) => {
            kill_group(process_group);
            let _ = child.kill().await;
            (None, true, false)
        }
        _ = cancellation.wait() => {
            kill_group(process_group);
            let _ = child.kill().await;
            (None, false, true)
        }
    };
    kill_group(process_group);
    let stdout = out_task.await.unwrap_or_default();
    let stderr = err_task.await.unwrap_or_default();
    Ok(Finished {
        exit_code: status.and_then(|status| status.code()),
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        timed_out,
        cancelled,
    })
}

pub async fn run_tool(
    execution: &Execution<'_>,
    call: &TranscriptToolCall,
) -> Result<Outcome, ToolError> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (execution, call);
        Err(fail(
            "SANDBOX_UNAVAILABLE",
            availability().err().unwrap_or_default(),
        ))
    }
    #[cfg(target_os = "linux")]
    {
        run_tool_linux(execution, call).await
    }
}

#[cfg(target_os = "linux")]
async fn run_tool_linux(
    execution: &Execution<'_>,
    call: &TranscriptToolCall,
) -> Result<Outcome, ToolError> {
    let work = execution
        .paths
        .sandboxes()
        .join(&execution.task.root_id)
        .join(uuid::Uuid::new_v4().simple().to_string());
    std::fs::create_dir_all(&work)
        .map_err(|_| fail("SANDBOX_FAILED", "The sandbox folder could not be created."))?;
    let root_for = |id: &str| -> Result<std::path::PathBuf, ToolError> {
        let root = execution
            .run
            .project
            .as_ref()
            .and_then(|project| project.roots.iter().find(|root| root.id == id))
            .ok_or_else(|| {
                fail(
                    "ROOT_NOT_GRANTED",
                    "That folder is not granted to this task's project.",
                )
            })?;
        super::workspace::open_root(&root.path)?;
        Ok(std::path::PathBuf::from(&root.path))
    };
    let (program, project, cwd, limits, label) = if call.name == "code.python" {
        std::fs::write(
            work.join("main.py"),
            call.arguments["code"].as_str().unwrap_or_default(),
        )
        .map_err(|_| fail("SANDBOX_FAILED", "The script could not be written."))?;
        let project = call.arguments["projectRootId"]
            .as_str()
            .map(root_for)
            .transpose()?;
        (
            vec![
                "/usr/bin/python3".to_owned(),
                "-I".to_owned(),
                "main.py".to_owned(),
            ],
            project,
            "/work".to_owned(),
            Limits {
                cpu_seconds: 30,
                address_space: 2 << 30,
                file_size: 16 << 20,
            },
            "Python".to_owned(),
        )
    } else {
        let root = root_for(call.arguments["rootId"].as_str().unwrap_or_default())?;
        let relative =
            super::workspace::relative_path(call.arguments["path"].as_str().unwrap_or(""))?;
        let program = match call.arguments["program"].as_str() {
            Some("python3") => "/usr/bin/python3",
            Some("node") if std::path::Path::new("/usr/bin/node").is_file() => "/usr/bin/node",
            Some("node") => {
                return Err(fail(
                    "PROGRAM_UNAVAILABLE",
                    "Node.js is not installed system-wide (/usr/bin/node).",
                ));
            }
            _ => return Err(fail("PROGRAM_REJECTED", "Only python3 and node can run.")),
        };
        let mut argv = vec![program.to_owned()];
        argv.extend(
            call.arguments["args"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|value| value.as_str())
                .map(str::to_owned),
        );
        let cwd = format!("/project/{}", relative.to_string_lossy())
            .trim_end_matches('/')
            .to_owned();
        let label = argv.join(" ");
        (
            argv,
            Some(root),
            cwd,
            Limits {
                cpu_seconds: 60,
                address_space: if program.ends_with("node") {
                    8 << 30
                } else {
                    2 << 30
                },
                file_size: 16 << 20,
            },
            label,
        )
    };
    let finished = run(
        &work,
        project.as_deref(),
        &cwd,
        &program,
        limits,
        execution.cancellation,
    )
    .await;
    let mut outputs = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&work) {
        for entry in entries.flatten().take(32) {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(metadata) = std::fs::symlink_metadata(entry.path()) else {
                continue;
            };
            if name == "main.py"
                || !metadata.is_file()
                || metadata.len() > MAX_OUTPUT_FILE_BYTES
                || outputs.len() >= MAX_OUTPUT_FILES
            {
                continue;
            }
            let Ok(bytes) = std::fs::read(entry.path()) else {
                continue;
            };
            if String::from_utf8(bytes.clone()).is_err() {
                continue;
            }
            if let Ok(artifact) =
                create_artifact(execution, &name, &name, &bytes, "output", Vec::new())
            {
                outputs.push(json!({ "artifactId": artifact.id, "fileName": artifact.file_name, "sizeBytes": artifact.size_bytes }));
            }
        }
    }
    let _ = std::fs::remove_dir_all(&work);
    let finished = finished?;
    if finished.cancelled {
        return Err(fail(
            "REQUEST_CANCELLED",
            "The run was cancelled and its processes were stopped.",
        ));
    }
    let status = if finished.timed_out {
        "timed out after 60 seconds (all processes stopped)".to_owned()
    } else {
        match finished.exit_code {
            Some(0) => "finished".into(),
            Some(code) => format!("exited with code {code}"),
            None => "was stopped by a limit".into(),
        }
    };
    Ok(Outcome::success(
        json!({
            "exitCode": finished.exit_code,
            "timedOut": finished.timed_out,
            "stdout": finished.stdout,
            "stderr": finished.stderr,
            "outputs": outputs,
            "sandbox": "Linux bubblewrap: no network, read-only system, private working folder, project folder read-only if mounted",
        }),
        format!("Ran {label} in the sandbox; it {status}"),
    ))
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    async fn python(code: &str, project: Option<&std::path::Path>) -> Finished {
        let work = std::env::temp_dir().join(format!("juniper-sandbox-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&work).unwrap();
        std::fs::write(work.join("main.py"), code).unwrap();
        let finished = run(
            &work,
            project,
            "/work",
            &["/usr/bin/python3".into(), "-I".into(), "main.py".into()],
            Limits {
                cpu_seconds: 10,
                address_space: 2 << 30,
                file_size: 1 << 20,
            },
            &crate::commands::Cancellation::default(),
        )
        .await
        .expect("sandbox runs");
        std::fs::remove_dir_all(work).ok();
        finished
    }

    #[tokio::test]
    async fn the_sandbox_runs_code_and_hides_the_users_files_secrets_and_network() {
        if availability().is_err() {
            eprintln!("sandbox unavailable here; skipping");
            return;
        }
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
        let ran = python(
            &format!(
                r#"
import os, socket
print("sum", 2 + 3)
print("home-visible", os.path.exists({home:?}))
print("env-keys", sorted(k for k in os.environ if k not in ("PATH","HOME","LANG","PYTHONDONTWRITEBYTECODE","PYTHONNOUSERSITE","PWD")))
try:
    open("/usr/escape.txt", "w")
    print("usr-writable", True)
except OSError:
    print("usr-writable", False)
try:
    socket.create_connection(("1.1.1.1", 53), timeout=3)
    print("network", True)
except OSError:
    print("network", False)
open("result.txt", "w").write("ok")
"#
            ),
            None,
        )
        .await;
        assert_eq!(ran.exit_code, Some(0), "{}", ran.stderr);
        assert!(ran.stdout.contains("sum 5"));
        assert!(ran.stdout.contains("home-visible False"), "{}", ran.stdout);
        assert!(ran.stdout.contains("env-keys []"), "{}", ran.stdout);
        assert!(ran.stdout.contains("usr-writable False"));
        assert!(ran.stdout.contains("network False"));
    }

    #[tokio::test]
    async fn a_mounted_project_is_read_only() {
        if availability().is_err() {
            return;
        }
        let project =
            std::env::temp_dir().join(format!("juniper-sandbox-project-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("data.txt"), "visible").unwrap();
        let ran = python(
            "print(open('/project/data.txt').read())\ntry:\n    open('/project/data.txt','w').write('x')\n    print('wrote')\nexcept OSError:\n    print('read-only')\n",
            Some(&project),
        )
        .await;
        assert!(
            ran.stdout.contains("visible") && ran.stdout.contains("read-only"),
            "{}",
            ran.stdout
        );
        assert_eq!(
            std::fs::read_to_string(project.join("data.txt")).unwrap(),
            "visible"
        );
        std::fs::remove_dir_all(project).ok();
    }

    #[tokio::test]
    async fn runaway_processes_are_stopped_with_their_children() {
        if availability().is_err() {
            return;
        }
        let ran = python("while True:\n    pass\n", None).await;
        // The CPU limit (10 s here) stops a busy loop well before the wall limit.
        assert!(ran.exit_code != Some(0));
        let cancellation = crate::commands::Cancellation::default();
        let work =
            std::env::temp_dir().join(format!("juniper-sandbox-cancel-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&work).unwrap();
        std::fs::write(work.join("main.py"), "import subprocess, time\nsubprocess.Popen(['/usr/bin/sleep','300'])\ntime.sleep(300)\n").unwrap();
        let cancel = cancellation.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            cancel.cancel();
        });
        let started = std::time::Instant::now();
        let finished = run(
            &work,
            None,
            "/work",
            &["/usr/bin/python3".into(), "-I".into(), "main.py".into()],
            Limits {
                cpu_seconds: 10,
                address_space: 2 << 30,
                file_size: 1 << 20,
            },
            &cancellation,
        )
        .await
        .unwrap();
        assert!(finished.cancelled);
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        std::fs::remove_dir_all(work).ok();
    }
}
