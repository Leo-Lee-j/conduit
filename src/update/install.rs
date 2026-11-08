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
    let dir = executable.parent().context(conduit::i18n::text_static(
        "update.install.unable_to_determine_install_directory",
    ))?;
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
            .context(conduit::i18n::text_static(
                "update.install.conduit_is_updating_try_starting_it_again",
            ))?,
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
        conduit::i18n::text(
            "update.install.development_and_portable_builds_cannot_install_updates"
        )
    );
    let dir = exe
        .parent()
        .context(conduit::i18n::text_static(
            "update.install.install_directory_missing",
        ))?
        .canonicalize()?;
    ensure!(
        dir.join(INSTALL_MARKER).is_file() && dir.join("uninstall.exe").is_file(),
        conduit::i18n::text("update.install.install_conduit_using_the_official_installer_first")
    );
    ensure!(
        dir == registered_install()?,
        conduit::i18n::text("update.install.current_path_is_not_the_registered_conduit")
    );
    ensure!(
        dir.join("conduit-updater.exe").is_file(),
        conduit::i18n::text(
            "update.install.update_helper_missing_from_this_installation_reinstall"
        )
    );
    Ok(dir)
}

#[derive(Serialize, Deserialize)]
pub struct Handoff {
    #[serde(default = "crate::i18n::Locale::legacy")]
    pub language: crate::i18n::Locale,
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
            let error = fs::read_to_string(self.dir.join("error.txt")).unwrap_or_else(|_| {
                conduit::i18n::format(
                    "update.install.update_helper_exited_early",
                    &[("status", format!("{}", status))],
                )
            });
            anyhow::bail!("{error}");
        }
        ensure!(
            self.started.elapsed() < Duration::from_secs(30),
            conduit::i18n::text("update.install.update_helper_startup_timed_out")
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
        language: crate::i18n::locale(),
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
        .context(conduit::i18n::text_static(
            "update.install.unable_to_start_update_helper",
        ))?;
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
        conduit::i18n::format(
            "update.install.failed_to_save_update_information",
            &[("arg0", format!("{}", std::io::Error::last_os_error()))]
        )
    );
    Ok(())
}

pub fn run(job_path: &Path) -> Result<()> {
    let dir = job_path
        .parent()
        .context(conduit::i18n::text_static(
            "update.install.handoff_directory_missing",
        ))?
        .canonicalize()?;
    let result = apply(job_path, &dir);
    if let Err(error) = &result {
        let _ = fs::write(dir.join("error.txt"), format!("{error:#}"));
    }
    result
}

fn apply(job_path: &Path, cache: &Path) -> Result<()> {
    let job: Handoff = serde_json::from_slice(&fs::read(job_path)?)?;
    crate::i18n::set_locale(&job.language);
    let install = installation(&job.executable)?;
    ensure!(
        cache.parent() == Some(job.data_dir.join("updates").as_path()),
        conduit::i18n::text("update.install.invalid_update_handoff_directory")
    );
    let manifest = verify_manifest(&job.manifest, &job.signature, PUBLIC_KEY)?;
    verify_package(&job.package, &manifest)?;
    let previous =
        semver::Version::parse(fs::read_to_string(install.join(INSTALL_MARKER))?.trim())?;
    ensure!(
        semver::Version::parse(&manifest.version)? > previous,
        conduit::i18n::text("update.install.cannot_downgrade_or_reinstall_the_same_version")
    );
    let _update_lock = exclusive(&install.join(".conduit-update.lock")).context(
        conduit::i18n::text_static("update.install.another_update_is_being_installed"),
    )?;

    // Open the exact parent process before acknowledging the handoff, avoiding
    // PID reuse and waiting until Windows has released its executable/mutexes.
    let process = unsafe {
        OpenProcess(
            PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
            0,
            job.parent_pid,
        )
    };
    ensure!(
        !process.is_null(),
        conduit::i18n::text("update.install.unable_to_wait_for_the_previous_version")
    );
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
        conduit::i18n::text("update.install.unable_to_verify_parent_process")
    );
    let actual = PathBuf::from(String::from_utf16(&image[..length as usize])?).canonicalize()?;
    ensure!(
        actual == job.executable.canonicalize()?,
        conduit::i18n::text("update.install.update_parent_process_does_not_match_install")
    );
    fs::write(cache.join("ready"), b"ready")?;
    let waited = unsafe { WaitForSingleObject(process.0, 90_000) };
    ensure!(
        waited != WAIT_TIMEOUT,
        conduit::i18n::text("update.install.previous_version_did_not_exit_update_cancelled")
    );
    ensure!(
        waited == WAIT_OBJECT_0,
        conduit::i18n::text("update.install.failed_while_waiting_for_the_previous_version")
    );
    let _running_lock = match exclusive(&install.join(".conduit-running.lock")) {
        Ok(lock) => lock,
        Err(error) => {
            let _ = restart(&job);
            return Err(error.context(conduit::i18n::text_static(
                "update.install.other_conduit_instances_are_using_this_installation",
            )));
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
        ensure!(
            status.success(),
            conduit::i18n::format(
                "update.install.installation_failed_with_exit_status",
                &[("status", format!("{}", status))]
            )
        );
        ensure!(
            fs::read_to_string(install.join(INSTALL_MARKER))?.trim() == manifest.version,
            conduit::i18n::text("update.install.installer_did_not_update_the_version")
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
            restore(&install, &backup).context(conduit::i18n::format(
                "update.install.installation_failed_restoring_the_previous_version_failed",
                &[
                    ("error", format!("{:#}", error)),
                    ("arg0", format!("{}", backup.display())),
                ],
            ))?;
            restore_registry_version(&previous.to_string())?;
        }
        drop(_running_lock);
        let _ = restart(&job);
        return Err(error.context(conduit::i18n::text_static(
            "update.install.update_failed_previous_version_preserved_or_restored",
        )));
    }
    drop(_running_lock);
    restart(&job).context(conduit::i18n::text_static(
        "update.install.installation_succeeded_but_automatic_restart_failed_open",
    ))?;
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
        .context(conduit::i18n::text_static(
            "update.install.failed_to_start_installer",
        ))?;
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if start.elapsed() > Duration::from_secs(10 * 60) {
            child.kill()?;
            child.wait()?;
            anyhow::bail!(conduit::i18n::text(
                "update.install.installer_timed_out_installation_stopped_preparing_to"
            ));
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
            ensure!(
                entry.file_type()?.is_file(),
                conduit::i18n::text("update.install.invalid_installation_file_type")
            );
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
        .current_dir(job.executable.parent().context(conduit::i18n::text_static(
            "update.install.install_directory_missing",
        ))?)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nsis_directory_preserves_spaces_and_unicode_without_extended_prefix() {
        let _locale = crate::test_support::chinese();
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
        let _locale = crate::test_support::chinese();
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
        let _locale = crate::test_support::chinese();
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
        let _locale = crate::test_support::chinese();
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
