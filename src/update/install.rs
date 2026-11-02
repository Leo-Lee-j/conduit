//! Per-user NSIS handoff. The helper runs from the update cache, outside the
//! installation it replaces. No shell command or elevation is involved.
use super::{
    PUBLIC_KEY, Release,
    manifest::{verify_manifest, verify_package},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::os::windows::{fs::OpenOptionsExt, process::CommandExt};
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    process::{Child, Command},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
        QueryFullProcessImageNameW, WaitForSingleObject,
    },
};
use winreg::{
    RegKey,
    enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE},
};

const CREATE_NO_WINDOW: u32 = 0x08000000;
pub const INSTALL_MARKER: &str = "conduit.install";

/// Every installed app holds a shared lock. An updater requires exclusive
/// access, including against instances using a different CONDUIT_DATA_DIR.
pub fn running_lock(executable: &Path) -> Result<Option<File>> {
    let dir = executable.parent().context("无法确定安装目录")?;
    if !dir.join(INSTALL_MARKER).is_file() {
        return Ok(None);
    }
    Ok(Some(
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(3)
            .open(dir.join(".conduit-running.lock"))
            .context("Conduit 正在更新，请稍后再启动")?,
    ))
}

fn exclusive(path: &Path) -> Result<File> {
    Ok(OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .open(path)?)
}

fn registered_install() -> Result<PathBuf> {
    let key =
        RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags("Software\\Conduit", KEY_READ)?;
    let value: String = key.get_value("InstallDir")?;
    Ok(PathBuf::from(value).canonicalize()?)
}

pub fn installation(exe: &Path) -> Result<PathBuf> {
    ensure!(
        exe.file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case("conduit.exe")),
        "开发或便携构建不能自动安装更新"
    );
    let dir = exe.parent().context("缺少安装目录")?.canonicalize()?;
    ensure!(
        dir.join(INSTALL_MARKER).is_file() && dir.join("uninstall.exe").is_file(),
        "请先使用正式安装包安装 Conduit"
    );
    ensure!(
        dir == registered_install()?,
        "当前路径不是注册的 Conduit 安装目录，请重新安装"
    );
    ensure!(
        dir.join("conduit-updater.exe").is_file(),
        "当前安装缺少更新助手，请重新安装"
    );
    Ok(dir)
}

#[derive(Serialize, Deserialize)]
pub struct Handoff {
    pub parent_pid: u32,
    pub executable: PathBuf,
    pub data_dir: PathBuf,
    pub package: PathBuf,
    pub manifest: Vec<u8>,
    pub signature: String,
}

pub struct Pending {
    pub child: Child,
    pub dir: PathBuf,
    pub started: Instant,
}

impl Pending {
    pub fn ready(&mut self) -> Result<bool> {
        if let Ok(error) = fs::read_to_string(self.dir.join("error.txt")) {
            anyhow::bail!("{error}");
        }
        if let Some(status) = self.child.try_wait()? {
            let error = fs::read_to_string(self.dir.join("error.txt"))
                .unwrap_or_else(|_| format!("更新助手提前退出：{status}"));
            anyhow::bail!("{error}");
        }
        ensure!(
            self.started.elapsed() < Duration::from_secs(30),
            "更新助手启动超时"
        );
        Ok(self.dir.join("ready").exists())
    }

    pub fn abort(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Run on a worker thread: copies binaries and validates the complete package.
pub fn prepare(release: &Release, package: &Path, data_dir: &Path) -> Result<Pending> {
    let executable = std::env::current_exe()?.canonicalize()?;
    let install = installation(&executable)?;
    let manifest = verify_manifest(&release.raw, &release.signature, PUBLIC_KEY)?;
    verify_package(package, &manifest)?;
    fs::create_dir_all(data_dir)?;
    let data_dir = data_dir.canonicalize()?;
    let dir = data_dir.join("updates").join(format!(
        "handoff-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    fs::create_dir(&dir)?;
    fs::copy(
        install.join("conduit-updater.exe"),
        dir.join("conduit-updater.exe"),
    )?;
    for entry in fs::read_dir(&install)? {
        let entry = entry?;
        if entry
            .path()
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("dll"))
        {
            fs::copy(entry.path(), dir.join(entry.file_name()))?;
        }
    }
    let job = Handoff {
        parent_pid: std::process::id(),
        executable,
        data_dir,
        package: package.canonicalize()?,
        manifest: release.raw.clone(),
        signature: release.signature.clone(),
    };
    write_json(&dir.join("handoff.json"), &job)?;
    let child = Command::new(dir.join("conduit-updater.exe"))
        .arg(dir.join("handoff.json"))
        .current_dir(&dir)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .context("无法启动更新助手")?;
    Ok(Pending {
        child,
        dir,
        started: Instant::now(),
    })
}

pub fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    use std::io::Write;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    let temporary = path.with_extension("json.tmp");
    let mut file = File::create(&temporary)?;
    file.write_all(&serde_json::to_vec_pretty(value)?)?;
    file.sync_all()?;
    drop(file);
    let from: Vec<_> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    ensure!(
        unsafe {
            MoveFileExW(
                from.as_ptr(),
                to.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        } != 0,
        "保存更新信息失败：{}",
        std::io::Error::last_os_error()
    );
    Ok(())
}

pub fn run(job_path: &Path) -> Result<()> {
    let dir = job_path.parent().context("缺少交接目录")?.canonicalize()?;
    let result = apply(job_path, &dir);
    if let Err(error) = &result {
        let _ = fs::write(dir.join("error.txt"), format!("{error:#}"));
    }
    result
}

fn apply(job_path: &Path, cache: &Path) -> Result<()> {
    let job: Handoff = serde_json::from_slice(&fs::read(job_path)?)?;
    let install = installation(&job.executable)?;
    ensure!(
        cache.parent() == Some(job.data_dir.join("updates").as_path()),
        "更新交接目录无效"
    );
    let manifest = verify_manifest(&job.manifest, &job.signature, PUBLIC_KEY)?;
    verify_package(&job.package, &manifest)?;
    let previous =
        semver::Version::parse(fs::read_to_string(install.join(INSTALL_MARKER))?.trim())?;
    ensure!(
        semver::Version::parse(&manifest.version)? > previous,
        "不能降级或重复安装同一版本"
    );
    let _update_lock =
        exclusive(&install.join(".conduit-update.lock")).context("另一个更新正在安装")?;

    // Open the exact parent process before acknowledging the handoff, avoiding
    // PID reuse and waiting until Windows has released its executable/mutexes.
    let process = unsafe {
        OpenProcess(
            PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
            0,
            job.parent_pid,
        )
    };
    ensure!(!process.is_null(), "无法等待旧版本退出");
    struct Handle(windows_sys::Win32::Foundation::HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    let process = Handle(process);
    let mut image = vec![0_u16; 32768];
    let mut length = image.len() as u32;
    ensure!(
        unsafe { QueryFullProcessImageNameW(process.0, 0, image.as_mut_ptr(), &mut length) } != 0,
        "无法核实父进程"
    );
    let actual = PathBuf::from(String::from_utf16(&image[..length as usize])?).canonicalize()?;
    ensure!(
        actual == job.executable.canonicalize()?,
        "更新父进程与安装目录不一致"
    );
    fs::write(cache.join("ready"), b"ready")?;
    let waited = unsafe { WaitForSingleObject(process.0, 90_000) };
    ensure!(waited != WAIT_TIMEOUT, "旧版本未退出，本次更新已取消");
    ensure!(waited == WAIT_OBJECT_0, "等待旧版本失败");
    let _running_lock = match exclusive(&install.join(".conduit-running.lock")) {
        Ok(lock) => lock,
        Err(error) => {
            let _ = restart(&job);
            return Err(error.context("其他 Conduit 实例仍在使用此安装，请关闭后重试"));
        }
    };
    let backup = cache.join("backup");
    let result = (|| {
        snapshot(&install, &backup)?;
        super::ensure_disk_space(&install, manifest.size.saturating_mul(3))?;
        fs::copy(
            job.data_dir.join("state.json"),
            cache.join("state.before-update.json"),
        )?;
        fs::write(cache.join("installing"), &manifest.version)?;
        // NSIS requires the final /D= value to be unquoted, even with spaces.
        let status = run_installer(&job.package, &install, cache)?;
        ensure!(status.success(), "安装失败，退出状态 {status}");
        ensure!(
            fs::read_to_string(install.join(INSTALL_MARKER))?.trim() == manifest.version,
            "安装器未完成版本更新"
        );
        fs::write(cache.join("completed"), &manifest.version)?;
        write_json(
            &job.data_dir.join("installed-release.json"),
            &Release {
                manifest: manifest.clone(),
                raw: job.manifest.clone(),
                signature: job.signature.clone(),
            },
        )?;
        Ok(())
    })();
    if let Err(error) = result {
        if backup.join("complete").exists() {
            restore(&install, &backup).context(format!(
                "安装失败：{error:#}；恢复旧版本失败，备份位于 {}",
                backup.display()
            ))?;
            restore_registry_version(&previous.to_string())?;
        }
        drop(_running_lock);
        let _ = restart(&job);
        return Err(error.context("更新失败，已保留或恢复旧版本"));
    }
    drop(_running_lock);
    restart(&job).context("安装成功，但自动重启失败，请从开始菜单打开 Conduit")?;
    Ok(())
}

fn managed_file(path: &Path) -> bool {
    path.file_name().is_some_and(|n| n == INSTALL_MARKER)
        || path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("exe") || e.eq_ignore_ascii_case("dll"))
}

fn nsis_directory(directory: &Path) -> String {
    let path = directory.to_string_lossy();
    if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else {
        path.strip_prefix(r"\\?\").unwrap_or(&path).to_string()
    }
}

fn run_installer(package: &Path, install: &Path, cache: &Path) -> Result<std::process::ExitStatus> {
    let mut child = Command::new(package)
        .arg("/S")
        .raw_arg(format!("/D={}", nsis_directory(install)))
        .current_dir(cache)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .context("启动安装器失败")?;
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if start.elapsed() > Duration::from_secs(10 * 60) {
            child.kill()?;
            child.wait()?;
            anyhow::bail!("安装器超时，已停止安装并准备恢复旧版本");
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn snapshot(install: &Path, backup: &Path) -> Result<()> {
    fs::create_dir(backup)?;
    let bytes: u64 = fs::read_dir(install)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| managed_file(&entry.path()))
        .map(|entry| entry.metadata().map(|m| m.len()).unwrap_or(0))
        .sum();
    super::ensure_disk_space(backup, bytes + 32 * 1024 * 1024)?;
    for entry in fs::read_dir(install)? {
        let entry = entry?;
        if managed_file(&entry.path()) {
            ensure!(entry.file_type()?.is_file(), "安装文件类型无效");
            fs::copy(entry.path(), backup.join(entry.file_name()))?;
        }
    }
    fs::write(backup.join("complete"), b"complete")?;
    Ok(())
}

fn restore(install: &Path, backup: &Path) -> Result<()> {
    for entry in fs::read_dir(install)? {
        let entry = entry?;
        if managed_file(&entry.path()) && !backup.join(entry.file_name()).exists() {
            fs::remove_file(entry.path())?;
        }
    }
    for entry in fs::read_dir(backup)? {
        let entry = entry?;
        if managed_file(&entry.path()) {
            fs::copy(entry.path(), install.join(entry.file_name()))?;
        }
    }
    Ok(())
}

fn restore_registry_version(version: &str) -> Result<()> {
    let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(
        "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Conduit",
        KEY_WRITE,
    )?;
    key.set_value("DisplayVersion", &version)?;
    Ok(())
}

fn restart(job: &Handoff) -> Result<()> {
    Command::new(&job.executable)
        .env("CONDUIT_DATA_DIR", &job.data_dir)
        .arg("--updated")
        .current_dir(job.executable.parent().context("缺少安装目录")?)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nsis_directory_preserves_spaces_and_unicode_without_extended_prefix() {
        assert_eq!(
            nsis_directory(Path::new(r"\\?\C:\用户\Conduit App")),
            r"C:\用户\Conduit App"
        );
        assert_eq!(
            nsis_directory(Path::new(r"\\?\UNC\server\共享\App")),
            r"\\server\共享\App"
        );
    }
    #[test]
    fn installation_lock_blocks_updates_until_all_instances_exit() {
        let root = std::env::temp_dir().join(format!("conduit-lock-test-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join(INSTALL_MARKER), "0.1.0").unwrap();
        let first = running_lock(&root.join("conduit.exe")).unwrap();
        let second = running_lock(&root.join("conduit.exe")).unwrap();
        assert!(exclusive(&root.join(".conduit-running.lock")).is_err());
        drop(first);
        assert!(exclusive(&root.join(".conduit-running.lock")).is_err());
        drop(second);
        let updater = exclusive(&root.join(".conduit-running.lock")).unwrap();
        assert!(running_lock(&root.join("conduit.exe")).is_err());
        drop(updater);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recovery_json_replaces_existing_file_atomically() {
        let path =
            std::env::temp_dir().join(format!("conduit-json-test-{}.json", std::process::id()));
        write_json(&path, &vec!["old"]).unwrap();
        write_json(&path, &vec!["new"]).unwrap();
        assert_eq!(
            serde_json::from_slice::<Vec<String>>(&fs::read(&path).unwrap()).unwrap(),
            vec!["new"]
        );
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn rollback_restores_original_files_and_removes_partial_new_files() {
        let root = std::env::temp_dir().join(format!("conduit-rollback-{}", std::process::id()));
        let install = root.join("安装目录 with spaces");
        let backup = root.join("backup");
        fs::create_dir_all(&install).unwrap();
        fs::write(install.join("conduit.exe"), b"old").unwrap();
        fs::write(install.join("keep.txt"), b"data").unwrap();
        snapshot(&install, &backup).unwrap();
        fs::write(install.join("conduit.exe"), b"new").unwrap();
        fs::write(install.join("new.dll"), b"new").unwrap();
        restore(&install, &backup).unwrap();
        assert_eq!(fs::read(install.join("conduit.exe")).unwrap(), b"old");
        assert_eq!(fs::read(install.join("keep.txt")).unwrap(), b"data");
        assert!(!install.join("new.dll").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
