//! Confinement d'une action et de tous les processus qu'elle lance.

use std::io;
use std::process::{Child, Command};

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

        pub fn terminate(&self) {
            unsafe { TerminateJobObject(self.0, 1) };
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
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
}
