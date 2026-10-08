//! Confinement des actions et des outils audio, avec leurs descendants.

use std::io;
use std::process::{Child, Command, ExitStatus, Output, Stdio};

/// Runs a tool in the same containment as actions and playback decoders.
pub(super) fn status(command: &mut Command) -> io::Result<ExitStatus> {
    let mut process = ActionProcess::spawn(command)?;
    process.child.wait()
}

/// Drains both pipes concurrently, then closes the process group/job before
/// joining the readers: a descendant must not keep a finished tool's pipes open.
pub(super) fn output(command: &mut Command) -> io::Result<Output> {
    use std::io::Read;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut process = ActionProcess::spawn(command)?;
    let mut stdout = process.child.stdout.take().expect("piped stdout");
    let mut stderr = process.child.stderr.take().expect("piped stderr");
    let stdout = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).map(|_| bytes)
    });
    let stderr = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).map(|_| bytes)
    });
    let status = process.child.wait();
    process.terminate();
    let stdout = stdout
        .join()
        .map_err(|_| io::Error::other("stdout reader panicked"));
    let stderr = stderr
        .join()
        .map_err(|_| io::Error::other("stderr reader panicked"));
    Ok(Output {
        status: status?,
        stdout: stdout??,
        stderr: stderr??,
    })
}

pub(super) struct ActionProcess {
    pub child: Child,
    terminated: bool,
    #[cfg(windows)]
    job: windows::Job,
}

impl ActionProcess {
    pub fn spawn(command: &mut Command) -> io::Result<Self> {
        #[cfg(windows)]
        {
            windows::spawn(command)
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
            Ok(Self {
                child: command.spawn()?,
                terminated: false,
            })
        }
    }

    pub fn terminate(&mut self) {
        if self.terminated {
            return;
        }
        self.terminated = true;
        #[cfg(windows)]
        self.job.terminate();
        #[cfg(unix)]
        // Le groupe survit au shell : ses descendants sont aussi arrêtés
        // s'ils conservent les pipes ouverts après la sortie du shell.
        unsafe {
            libc::kill(-(self.child.id() as libc::pid_t), libc::SIGKILL);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for ActionProcess {
    fn drop(&mut self) {
        self.terminate();
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn output_drains_both_pipes_and_preserves_status() {
        let result = output(Command::new("sh").args([
            "-c",
            "head -c 131072 /dev/zero; head -c 131072 /dev/zero >&2; exit 7",
        ]))
        .unwrap();
        assert_eq!(result.status.code(), Some(7));
        assert_eq!(result.stdout.len(), 131072);
        assert_eq!(result.stderr.len(), 131072);
    }

    #[test]
    fn output_closes_pipes_held_by_a_finished_tools_descendant() {
        let start = Instant::now();
        let result = output(
            Command::new("sh").args(["-c", "sleep 30 & printf ready; printf diagnostic >&2"]),
        )
        .unwrap();
        assert!(result.status.success());
        assert_eq!(result.stdout, b"ready");
        assert_eq!(result.stderr, b"diagnostic");
        assert!(start.elapsed() < Duration::from_secs(5));
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::os::windows::io::AsRawHandle;
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
    };
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject,
    };
    use windows_sys::Win32::System::Threading::{
        CREATE_NO_WINDOW, CREATE_SUSPENDED, OpenThread, ResumeThread, THREAD_SUSPEND_RESUME,
    };

    pub(super) struct Job(HANDLE);

    // SAFETY: a job handle belongs to the process, not to the creating thread.
    // Moving the sole owner transfers responsibility for closing it; methods
    // that terminate or close the handle still require exclusive access.
    unsafe impl Send for Job {}

    impl Job {
        fn new() -> io::Result<Self> {
            unsafe {
                let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if handle.is_null() {
                    return Err(io::Error::last_os_error());
                }
                let job = Self(handle);
                let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                if SetInformationJobObject(
                    handle,
                    JobObjectExtendedLimitInformation,
                    &limits as *const _ as *const _,
                    std::mem::size_of_val(&limits) as u32,
                ) == 0
                {
                    return Err(io::Error::last_os_error());
                }
                Ok(job)
            }
        }

        pub fn terminate(&mut self) {
            if self.0.is_null() {
                return;
            }
            // Fermer le dernier handle avant les attentes et les join, même
            // si TerminateJobObject échoue : KILL_ON_JOB_CLOSE arrête l'arbre.
            let handle = std::mem::replace(&mut self.0, std::ptr::null_mut());
            unsafe {
                TerminateJobObject(handle, 1);
                CloseHandle(handle);
            }
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CloseHandle(self.0) };
            }
        }
    }

    fn resume(pid: u32) -> io::Result<()> {
        // std::process ferme le handle du thread initial. Le retrouver avant
        // la reprise du processus, avant tout code applicatif ou descendant.
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
            if snapshot == INVALID_HANDLE_VALUE {
                return Err(io::Error::last_os_error());
            }
            let result = (|| {
                let mut entry: THREADENTRY32 = std::mem::zeroed();
                entry.dwSize = std::mem::size_of_val(&entry) as u32;
                let mut found = Thread32First(snapshot, &mut entry);
                while found != 0 {
                    if entry.th32OwnerProcessID == pid {
                        let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                        if thread.is_null() {
                            return Err(io::Error::last_os_error());
                        }
                        let resumed = ResumeThread(thread);
                        let error = io::Error::last_os_error();
                        CloseHandle(thread);
                        return if resumed == u32::MAX {
                            Err(error)
                        } else {
                            Ok(())
                        };
                    }
                    found = Thread32Next(snapshot, &mut entry);
                }
                Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "action thread not found",
                ))
            })();
            CloseHandle(snapshot);
            result
        }
    }

    pub(super) fn spawn(command: &mut Command) -> io::Result<ActionProcess> {
        let job = Job::new()?;
        command.creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED);
        let child = command.spawn()?;
        let process = ActionProcess {
            child,
            job,
            terminated: false,
        };
        // Assigner avant la reprise : tout descendant hérite du job dès sa
        // création. Sur échec, la garde arrête et récupère le shell suspendu.
        unsafe {
            if AssignProcessToJobObject(process.job.0, process.child.as_raw_handle() as HANDLE) == 0
            {
                return Err(io::Error::last_os_error());
            }
        }
        resume(process.child.id())?;
        Ok(process)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::io::{BufRead, BufReader, Read};
        use std::process::Stdio;
        use std::time::{Duration, Instant};
        use windows_sys::Win32::Foundation::{DuplicateHandle, ERROR_ACCESS_DENIED};
        use windows_sys::Win32::System::Threading::GetCurrentProcess;

        #[test]
        fn job_close_reaps_descendants_when_terminate_is_denied() {
            let mut command = crate::platform::silent_command("cmd");
            command
                .args(["/C", "ping -n 30 127.0.0.1"])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            let mut process = ActionProcess::spawn(&mut command).unwrap();
            let mut stdout = BufReader::new(process.child.stdout.take().unwrap());
            let stderr = process.child.stderr.take().unwrap();
            // La première ligne atteste que ping, descendant du shell, a
            // démarré et conserve les handles des deux flux.
            assert!(stdout.read_line(&mut String::new()).unwrap() > 0);
            let stdout_reader = std::thread::spawn(move || {
                let mut bytes = Vec::new();
                stdout.read_to_end(&mut bytes).unwrap();
            });
            let stderr_reader = std::thread::spawn(move || {
                let mut stderr = stderr;
                let mut bytes = Vec::new();
                stderr.read_to_end(&mut bytes).unwrap();
            });
            unsafe {
                let current = GetCurrentProcess();
                let mut restricted = std::ptr::null_mut();
                // Dupliquer sans droits, puis fermer l'ancien handle : le
                // vrai appel Win32 échoue désormais avec ACCESS_DENIED.
                assert_ne!(
                    DuplicateHandle(current, process.job.0, current, &mut restricted, 0, 0, 0),
                    0
                );
                let previous = std::mem::replace(&mut process.job.0, restricted);
                assert_ne!(CloseHandle(previous), 0);
                assert_eq!(TerminateJobObject(process.job.0, 1), 0);
                assert_eq!(
                    io::Error::last_os_error().raw_os_error(),
                    Some(ERROR_ACCESS_DENIED as i32)
                );
            }
            let started = Instant::now();
            process.terminate();
            assert!(process.job.0.is_null());
            stdout_reader.join().unwrap();
            stderr_reader.join().unwrap();
            assert!(started.elapsed() < Duration::from_secs(5));
        }
    }
}
