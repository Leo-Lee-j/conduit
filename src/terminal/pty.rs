//! A suspended child joins a non-inheritable kill-on-close Job before execution.
#![cfg(windows)]
use anyhow::{Context, Result, bail};
use std::{
    ffi::OsStr,
    fs::File,
    io,
    mem::{size_of, zeroed},
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::Path,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::*,
    System::{Console::*, JobObjects::*, Pipes::CreatePipe, Threading::*},
};

fn wide(s: &OsStr) -> Result<Vec<u16>> {
    let mut value: Vec<_> = s.encode_wide().collect();
    if value.contains(&0) {
        bail!("路径或参数包含 NUL");
    }
    value.push(0);
    Ok(value)
}

fn checked(ok: i32) -> io::Result<()> {
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
fn pipe() -> Result<(File, File)> {
    let (mut read, mut write) = (null_mut(), null_mut());
    unsafe {
        checked(CreatePipe(&mut read, &mut write, null(), 0))?;
        Ok((File::from_raw_handle(read), File::from_raw_handle(write)))
    }
}
struct Console(HPCON);
impl Drop for Console {
    fn drop(&mut self) {
        unsafe {
            ClosePseudoConsole(self.0);
        }
    }
}

struct Attributes {
    memory: Vec<usize>,
    initialized: bool,
}
impl Attributes {
    fn new(console: HPCON) -> Result<Self> {
        unsafe {
            let mut bytes = 0;
            InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut bytes);
            let mut this = Self {
                memory: vec![0; bytes.div_ceil(size_of::<usize>())],
                initialized: false,
            };
            checked(InitializeProcThreadAttributeList(
                this.ptr(),
                1,
                0,
                &mut bytes,
            ))?;
            this.initialized = true;
            checked(UpdateProcThreadAttribute(
                this.ptr(),
                0,
                PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
                console as *const _,
                size_of::<HPCON>(),
                null_mut(),
                null(),
            ))?;
            Ok(this)
        }
    }
    fn ptr(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.memory.as_mut_ptr().cast()
    }
}
impl Drop for Attributes {
    fn drop(&mut self) {
        if self.initialized {
            unsafe {
                DeleteProcThreadAttributeList(self.ptr());
            }
        }
    }
}

pub struct Pty {
    // Field drop order kills the job before closing ConPTY. Its reader drains concurrently.
    job: OwnedHandle,
    process: OwnedHandle,
    console: Option<Console>,
}
impl Pty {
    pub fn spawn(
        executable: &Path,
        args: &[String],
        cwd: &Path,
        cols: u16,
        rows: u16,
        on_reader: impl FnOnce(File),
    ) -> Result<(Self, File)> {
        let (input_read, input_write) = pipe()?;
        let (output_read, output_write) = pipe()?;
        unsafe {
            let mut raw_console = 0;
            let hr = CreatePseudoConsole(
                COORD {
                    X: cols as i16,
                    Y: rows as i16,
                },
                input_read.as_raw_handle(),
                output_write.as_raw_handle(),
                0,
                &mut raw_console,
            );
            if hr < 0 {
                bail!("CreatePseudoConsole: HRESULT {hr:#x}");
            }
            let console = Console(raw_console);
            drop(input_read);
            drop(output_write);
            on_reader(output_read);
            let job_raw = CreateJobObjectW(null(), null());
            if job_raw.is_null() {
                return Err(io::Error::last_os_error().into());
            }
            let job = OwnedHandle::from_raw_handle(job_raw);
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            checked(SetInformationJobObject(
                job.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ))?;
            let mut attributes = Attributes::new(raw_console)?;
            let mut startup: STARTUPINFOEXW = zeroed();
            startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
            // Otherwise a console-subsystem parent can leak its own console handles
            // into the child, bypassing ConPTY (notably cargo test / cargo run).
            startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
            startup.lpAttributeList = attributes.ptr();
            let exe = wide(executable.as_os_str())?;
            let dir = wide(crate::child::normalized_dir(cwd).as_os_str())?;
            let mut command = crate::child::quote_arg(&executable.to_string_lossy());
            for arg in args {
                command.push(' ');
                command.push_str(&crate::child::quote_arg(arg));
            }
            let mut command = wide(OsStr::new(&command))?;
            let mut info: PROCESS_INFORMATION = zeroed();
            checked(CreateProcessW(
                exe.as_ptr(),
                command.as_mut_ptr(),
                null(),
                null(),
                0,
                EXTENDED_STARTUPINFO_PRESENT | CREATE_SUSPENDED,
                null(),
                dir.as_ptr(),
                &startup.StartupInfo,
                &mut info,
            ))
            .with_context(|| format!("启动 {}", executable.display()))?;
            let process = OwnedHandle::from_raw_handle(info.hProcess);
            let thread = OwnedHandle::from_raw_handle(info.hThread);
            if let Err(error) = checked(AssignProcessToJobObject(
                job.as_raw_handle(),
                process.as_raw_handle(),
            )) {
                TerminateProcess(process.as_raw_handle(), 1);
                WaitForSingleObject(process.as_raw_handle(), 5000);
                return Err(error).context("绑定 Job 失败，已终止未启动的子进程");
            }
            if ResumeThread(thread.as_raw_handle()) == u32::MAX {
                let error = io::Error::last_os_error();
                TerminateJobObject(job.as_raw_handle(), 1);
                return Err(error.into());
            }
            Ok((
                Self {
                    job,
                    process,
                    console: Some(console),
                },
                input_write,
            ))
        }
    }
    pub fn process_handle(&self) -> HANDLE {
        self.process.as_raw_handle()
    }
    pub fn close_console(&mut self) -> Option<std::thread::JoinHandle<()>> {
        self.console
            .take()
            .map(|console| std::thread::spawn(move || drop(console)))
    }
    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        let hr = unsafe {
            ResizePseudoConsole(
                self.console.as_ref().expect("live console").0,
                COORD {
                    X: cols as i16,
                    Y: rows as i16,
                },
            )
        };
        if hr < 0 {
            bail!("ResizePseudoConsole: HRESULT {hr:#x}");
        }
        Ok(())
    }
    pub fn exit_code(&self) -> Result<Option<u32>> {
        unsafe {
            match WaitForSingleObject(self.process.as_raw_handle(), 0) {
                WAIT_TIMEOUT => Ok(None),
                WAIT_OBJECT_0 => {
                    let mut code = 0;
                    checked(GetExitCodeProcess(self.process.as_raw_handle(), &mut code))?;
                    Ok(Some(code))
                }
                _ => Err(io::Error::last_os_error().into()),
            }
        }
    }
    pub fn stop(&self) -> Result<()> {
        unsafe {
            checked(TerminateJobObject(self.job.as_raw_handle(), 1))?;
        }
        Ok(())
    }
    pub fn active_processes(&self) -> Result<u32> {
        let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { zeroed() };
        unsafe {
            checked(QueryInformationJobObject(
                self.job.as_raw_handle(),
                JobObjectBasicAccountingInformation,
                (&mut info as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                null_mut(),
            ))?;
        }
        Ok(info.ActiveProcesses)
    }
}
impl Drop for Pty {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
