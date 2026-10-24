//! Structured CLI transport: anonymous pipes and a kill-on-close Windows Job.
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
    path::{Path, PathBuf},
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::*,
    Security::SECURITY_ATTRIBUTES,
    System::{
        JobObjects::*,
        Pipes::{CreatePipe, PeekNamedPipe},
        Threading::*,
    },
};

fn checked(ok: i32) -> io::Result<()> {
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
fn wide(s: &OsStr) -> Result<Vec<u16>> {
    let mut value: Vec<_> = s.encode_wide().collect();
    if value.contains(&0) {
        bail!("路径或参数包含 NUL");
    }
    value.push(0);
    Ok(value)
}
fn pipe() -> Result<(File, File)> {
    let (mut read, mut write) = (null_mut(), null_mut());
    let security = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: null_mut(),
        bInheritHandle: 1,
    };
    unsafe {
        checked(CreatePipe(&mut read, &mut write, &security, 0))?;
        Ok((File::from_raw_handle(read), File::from_raw_handle(write)))
    }
}
pub fn available(file: &File) -> io::Result<u32> {
    let mut bytes = 0;
    unsafe {
        checked(PeekNamedPipe(
            file.as_raw_handle(),
            null_mut(),
            0,
            null_mut(),
            &mut bytes,
            null_mut(),
        ))?;
    }
    Ok(bytes)
}
pub(crate) fn quote_arg(arg: &str) -> String {
    let mut out = String::from("\"");
    let mut slashes = 0;
    for c in arg.chars() {
        if c == '\\' {
            slashes += 1;
            continue;
        }
        out.extend(std::iter::repeat_n(
            '\\',
            if c == '"' { slashes * 2 + 1 } else { slashes },
        ));
        slashes = 0;
        out.push(c);
    }
    out.extend(std::iter::repeat_n('\\', slashes * 2));
    out.push('"');
    out
}
struct Attributes {
    memory: Vec<usize>,
    initialized: bool,
}
impl Attributes {
    fn new(handles: &mut [HANDLE]) -> Result<Self> {
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
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                handles.as_ptr().cast(),
                std::mem::size_of_val(handles),
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

/// Strip the `\\?\` extended-length prefix: cmd.exe rejects it in current
/// directories and batch-launcher paths. Canonicalization on Windows adds it.
pub(crate) fn normalized_dir(path: &Path) -> PathBuf {
    let s = path.as_os_str();
    if let Some(rest) = s.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        // Keep genuine UNC (\\?\UNC\server\share) -> \\server\share
        if let Some(unc) = rest.strip_prefix(r"UNC\") {
            return PathBuf::from(format!(r"\\{}", unc));
        }
        return PathBuf::from(rest);
    }
    path.to_path_buf()
}
pub struct Child {
    job: OwnedHandle,
    process: OwnedHandle,
}
pub struct Pipes {
    pub stdin: File,
    pub stdout: File,
    pub stderr: File,
}
impl Child {
    pub fn spawn(executable: &Path, args: &[String], cwd: &Path) -> Result<(Self, Pipes)> {
        let (stdin_child, stdin) = pipe()?;
        let (stdout, stdout_child) = pipe()?;
        let (stderr, stderr_child) = pipe()?;
        unsafe {
            for file in [&stdin, &stdout, &stderr] {
                checked(SetHandleInformation(
                    file.as_raw_handle(),
                    HANDLE_FLAG_INHERIT,
                    0,
                ))?;
            }
            let raw_job = CreateJobObjectW(null(), null());
            if raw_job.is_null() {
                return Err(io::Error::last_os_error().into());
            }
            let job = OwnedHandle::from_raw_handle(raw_job);
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            checked(SetInformationJobObject(
                job.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ))?;
            let mut handles = [
                stdin_child.as_raw_handle(),
                stdout_child.as_raw_handle(),
                stderr_child.as_raw_handle(),
            ];
            let mut attributes = Attributes::new(&mut handles)?;
            let mut startup: STARTUPINFOEXW = zeroed();
            startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
            startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
            startup.StartupInfo.hStdInput = handles[0];
            startup.StartupInfo.hStdOutput = handles[1];
            startup.StartupInfo.hStdError = handles[2];
            startup.lpAttributeList = attributes.ptr();
            let exe = wide(executable.as_os_str())?;
            let cwd = wide(normalized_dir(cwd).as_os_str())?;
            let mut command = quote_arg(&executable.to_string_lossy());
            for arg in args {
                command.push(' ');
                command.push_str(&quote_arg(arg));
            }
            let mut command = wide(OsStr::new(&command))?;
            let mut info: PROCESS_INFORMATION = zeroed();
            checked(CreateProcessW(
                exe.as_ptr(),
                command.as_mut_ptr(),
                null(),
                null(),
                1,
                EXTENDED_STARTUPINFO_PRESENT | CREATE_SUSPENDED | CREATE_NO_WINDOW,
                null(),
                cwd.as_ptr(),
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
                return Err(error).context("绑定进程 Job 失败");
            }
            if ResumeThread(thread.as_raw_handle()) == u32::MAX {
                let error = io::Error::last_os_error();
                TerminateJobObject(job.as_raw_handle(), 1);
                return Err(error.into());
            }
            Ok((
                Self { job, process },
                Pipes {
                    stdin,
                    stdout,
                    stderr,
                },
            ))
        }
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
    pub fn terminate(&self) -> Result<()> {
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
impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.terminate();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Read,
        path::PathBuf,
        thread,
        time::{Duration, Instant},
    };
    fn powershell() -> PathBuf {
        PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe")
    }
    fn output_line(file: &mut File) -> String {
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut bytes = vec![];
        while Instant::now() < deadline {
            if available(file).unwrap_or(0) > 0 {
                let mut byte = [0];
                file.read_exact(&mut byte).unwrap();
                if byte[0] == b'\n' {
                    return String::from_utf8(bytes).unwrap().trim().into();
                }
                bytes.push(byte[0]);
            } else {
                thread::sleep(Duration::from_millis(10));
            }
        }
        panic!("fixture output timed out");
    }
    #[test]
    fn structured_stdio_preserves_unicode_paths_and_argument_boundaries() {
        let directory = std::env::temp_dir().join(format!("conduit 参数 {}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let script = directory.join("echo args.ps1");
        std::fs::write(&script, "[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding; ConvertTo-Json -Compress -InputObject @($args)").unwrap();
        let expected = vec![
            String::new(),
            "中文 空格".into(),
            "quote\"inside".into(),
            "trailing\\".into(),
        ];
        let mut args: Vec<String> = [
            "-NoProfile",
            "-NonInteractive",
            "-File",
            script.to_str().unwrap(),
        ]
        .map(String::from)
        .into();
        args.extend(expected.clone());
        let (child, mut pipes) = Child::spawn(&powershell(), &args, &directory).unwrap();
        let actual: Vec<String> = serde_json::from_str(&output_line(&mut pipes.stdout)).unwrap();
        assert_eq!(actual, expected);
        child.terminate().unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while child.active_processes().unwrap() > 0 || child.exit_code().unwrap().is_none() {
            assert!(Instant::now() < deadline, "fixture cleanup timed out");
            thread::sleep(Duration::from_millis(10));
        }
        drop(child);
        std::fs::remove_file(script).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
    #[test]
    fn explicit_stop_and_drop_kill_descendants() {
        for explicit in [true, false] {
            let args = ["-NoProfile", "-NonInteractive", "-Command", "$nested = Start-Process -FilePath (Join-Path $PSHOME 'powershell.exe') -ArgumentList '-NoProfile -NonInteractive -Command Start-Sleep -Seconds 60' -WindowStyle Hidden -PassThru; [Console]::WriteLine($nested.Id); Start-Sleep -Seconds 60"].map(String::from);
            let (child, mut pipes) =
                Child::spawn(&powershell(), &args, &std::env::temp_dir()).unwrap();
            let pid: u32 = output_line(&mut pipes.stdout).parse().unwrap();
            let nested = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
            assert!(!nested.is_null());
            let nested = unsafe { OwnedHandle::from_raw_handle(nested) };
            assert!(child.active_processes().unwrap() >= 2);
            if explicit {
                child.terminate().unwrap();
                let deadline = Instant::now() + Duration::from_secs(10);
                while child.active_processes().unwrap() > 0 || child.exit_code().unwrap().is_none()
                {
                    assert!(Instant::now() < deadline, "Job cleanup timed out");
                    thread::sleep(Duration::from_millis(10));
                }
            }
            drop(child);
            assert_eq!(
                unsafe { WaitForSingleObject(nested.as_raw_handle(), 10000) },
                WAIT_OBJECT_0
            );
        }
    }
}
