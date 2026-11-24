//! Reconcile persistent Codex Windows sandbox denies with explicitly writable
//! local metadata directories. Older sandbox profiles leave deny ACEs behind;
//! granting a directory again does not override those ACEs.
use anyhow::{Context, Result};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{OsString, c_void},
    fs::{self, File, OpenOptions},
    os::windows::{
        ffi::OsStringExt,
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    path::{Path, PathBuf},
    ptr::null_mut,
};
use windows_sys::Win32::{
    Foundation::{ERROR_SUCCESS, GENERIC_WRITE, LocalFree},
    Security::{
        ACCESS_DENIED_ACE, ACE_HEADER, ACL,
        Authorization::{
            ConvertStringSidToSidW, DENY_ACCESS, EXPLICIT_ACCESS_W, GetSecurityInfo,
            SE_FILE_OBJECT, SetEntriesInAclW, SetSecurityInfo, TRUSTEE_IS_SID, TRUSTEE_IS_UNKNOWN,
            TRUSTEE_W,
        },
        DACL_SECURITY_INFORMATION, DeleteAce, EqualSid, GetAce,
    },
    Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, DELETE, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
        FILE_DELETE_CHILD, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_GENERIC_WRITE, GetFileInformationByHandle, GetFinalPathNameByHandleW,
    },
};

// Win32 access/ACE constants not exposed by windows-sys's Win32 modules.
const MAXIMUM_ALLOWED: u32 = 0x0200_0000;
const ACCESS_DENIED_ACE_TYPE: u8 = 1;
// Codex's add_deny_write_ace also appears with GENERIC_WRITE mapped to file rights.
const CODEX_WRITE_DENY: u32 = FILE_GENERIC_WRITE | GENERIC_WRITE | DELETE | FILE_DELETE_CHILD;
const MAPPED_WRITE_DENY: u32 = CODEX_WRITE_DENY & !GENERIC_WRITE;

#[derive(Default, Deserialize)]
struct CapSids {
    #[serde(default)]
    workspace: String,
    #[serde(default)]
    readonly: String,
    #[serde(default)]
    workspace_by_cwd: BTreeMap<String, String>,
    #[serde(default)]
    writable_root_by_path: BTreeMap<String, String>,
}

fn path_key(path: &Path) -> String {
    let key = path.to_string_lossy().replace('\\', "/");
    let key = if let Some(unc) = key.strip_prefix("//?/UNC/") {
        format!("//{unc}")
    } else {
        key.strip_prefix("//?/").unwrap_or(&key).to_owned()
    };
    key.to_ascii_lowercase()
}

fn environment(key: &str, route: &BTreeMap<String, String>) -> Option<OsString> {
    route
        .iter()
        .rev()
        .find(|(name, _)| name.eq_ignore_ascii_case(key))
        .map(|(_, value)| OsString::from(value))
        .or_else(|| std::env::var_os(key))
}

fn codex_home(workspace: &Path, route: &BTreeMap<String, String>) -> Option<PathBuf> {
    let home = environment("CODEX_HOME", route)
        .filter(|value| !value.to_string_lossy().trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            environment("USERPROFILE", route)
                .or_else(|| environment("HOME", route))
                .map(|home| PathBuf::from(home).join(".codex"))
        })?;
    Some(if home.is_absolute() {
        home
    } else {
        workspace.join(home)
    })
}

struct LocalAllocation(*mut c_void);
impl Drop for LocalAllocation {
    fn drop(&mut self) {
        unsafe { LocalFree(self.0) };
    }
}

fn capability_sids(home: &Path, roots: &BTreeSet<String>) -> Result<Vec<LocalAllocation>> {
    let file = home.join("cap_sid");
    let contents = match fs::read_to_string(&file) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(error) => return Err(error).with_context(|| format!("read {}", file.display())),
    };
    let caps = if contents.trim().starts_with('{') {
        // Codex replaces malformed caches itself; do not guess their ownership.
        let Ok(caps) = serde_json::from_str::<CapSids>(&contents) else {
            return Ok(vec![]);
        };
        caps
    } else {
        CapSids {
            workspace: contents.trim().to_owned(),
            ..Default::default()
        }
    };
    let mut sids = BTreeSet::from([caps.workspace]);
    for (path, sid) in caps
        .workspace_by_cwd
        .into_iter()
        .chain(caps.writable_root_by_path)
    {
        if roots.contains(&path_key(Path::new(&path))) {
            sids.insert(sid);
        }
    }
    sids.into_iter()
        .filter(|sid| sid != &caps.readonly && sid.starts_with("S-1-5-21-"))
        .map(|sid| {
            let wide: Vec<u16> = sid.encode_utf16().chain(Some(0)).collect();
            let mut allocated = null_mut();
            win32(unsafe { ConvertStringSidToSidW(wide.as_ptr(), &mut allocated) })?;
            Ok(LocalAllocation(allocated))
        })
        .collect()
}

fn win32(success: i32) -> std::io::Result<()> {
    if success == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn status(code: u32) -> std::io::Result<()> {
    if code == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(std::io::Error::from_raw_os_error(code as i32))
    }
}

fn open_local(path: &Path) -> Result<Option<File>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Ok(None);
    }
    // MAXIMUM_ALLOWED prevents SetSecurityInfo from propagating ACEs to children.
    // Walk them ourselves so reparse points and externally shared files are skipped.
    let file = OpenOptions::new()
        .access_mode(MAXIMUM_ALLOWED)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let handle = file.as_raw_handle();
    let mut information: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    win32(unsafe { GetFileInformationByHandle(handle, &mut information) })?;
    if information.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || (information.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0
            && information.nNumberOfLinks > 1)
    {
        return Ok(None);
    }
    let length = unsafe { GetFinalPathNameByHandleW(handle, null_mut(), 0, 0) };
    win32(i32::from(length != 0))?;
    let mut name = vec![0; length as usize + 1];
    let written =
        unsafe { GetFinalPathNameByHandleW(handle, name.as_mut_ptr(), name.len() as u32, 0) };
    win32(i32::from(written != 0 && (written as usize) < name.len()))?;
    let resolved = PathBuf::from(OsString::from_wide(&name[..written as usize]));
    // Check the opened object, not just the path before opening. An ancestor
    // replaced by a junction must not redirect ACL changes outside the root.
    Ok((path_key(&resolved) == path_key(path)).then_some(file))
}

fn remove_stale_write_denies(
    file: &File,
    sids: &[LocalAllocation],
) -> Result<Vec<EXPLICIT_ACCESS_W>> {
    let mut descriptor = null_mut();
    let mut dacl: *mut ACL = null_mut();
    let code = unsafe {
        GetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            &mut dacl,
            null_mut(),
            &mut descriptor,
        )
    };
    let _allocation = LocalAllocation(descriptor);
    status(code)?;
    // A null DACL must stay null; replacing it with an empty ACL denies everyone.
    if dacl.is_null() {
        return Ok(vec![]);
    }
    let mut removed = vec![];
    for index in (0..unsafe { (*dacl).AceCount } as u32).rev() {
        let mut entry = null_mut();
        win32(unsafe { GetAce(dacl, index, &mut entry) })?;
        if unsafe { (*entry.cast::<ACE_HEADER>()).AceType } != ACCESS_DENIED_ACE_TYPE {
            continue;
        }
        let denied = entry.cast::<ACCESS_DENIED_ACE>();
        let mask = unsafe { (*denied).Mask };
        if mask != CODEX_WRITE_DENY && mask != MAPPED_WRITE_DENY {
            continue;
        }
        let sid = unsafe { std::ptr::addr_of_mut!((*denied).SidStart).cast() };
        let Some(known) = sids
            .iter()
            .find(|known| unsafe { EqualSid(sid, known.0) } != 0)
        else {
            continue;
        };
        removed.push(EXPLICIT_ACCESS_W {
            grfAccessPermissions: mask,
            grfAccessMode: DENY_ACCESS,
            grfInheritance: unsafe { (*denied).Header.AceFlags } as u32,
            Trustee: TRUSTEE_W {
                pMultipleTrustee: null_mut(),
                MultipleTrusteeOperation: 0,
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_UNKNOWN,
                ptstrName: known.0.cast(),
            },
        });
        win32(unsafe { DeleteAce(dacl, index) })?;
    }
    if !removed.is_empty() {
        status(unsafe {
            SetSecurityInfo(
                file.as_raw_handle(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                dacl,
                null_mut(),
            )
        })?;
    }
    Ok(removed)
}

fn restore_write_denies(file: &File, entries: &[EXPLICIT_ACCESS_W]) -> Result<()> {
    let mut descriptor = null_mut();
    let mut dacl = null_mut();
    let code = unsafe {
        GetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            &mut dacl,
            null_mut(),
            &mut descriptor,
        )
    };
    let _allocation = LocalAllocation(descriptor);
    status(code)?;
    anyhow::ensure!(
        !dacl.is_null(),
        "Codex workspace ACL changed to a null DACL during reconciliation"
    );
    let mut restored = null_mut();
    let code =
        unsafe { SetEntriesInAclW(entries.len() as u32, entries.as_ptr(), dacl, &mut restored) };
    let _restored = LocalAllocation(restored.cast());
    status(code)?;
    status(unsafe {
        SetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            restored,
            null_mut(),
        )
    })?;
    Ok(())
}

fn reconcile_descendants(root: &Path, sids: &[LocalAllocation]) -> Result<()> {
    let mut pending = fs::read_dir(root)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    while let Some(path) = pending.pop() {
        let Some(file) = open_local(&path)
            .with_context(|| format!("open Codex workspace ACL {}", path.display()))?
        else {
            continue;
        };
        // Parent denies must be removed first; Windows otherwise re-inherits
        // them when a descendant's security descriptor is written.
        remove_stale_write_denies(&file, sids)
            .with_context(|| format!("reconcile Codex workspace ACL {}", path.display()))?;
        if file.metadata()?.is_dir() {
            for entry in fs::read_dir(&path)? {
                pending.push(entry?.path());
            }
        }
    }
    Ok(())
}

pub(super) fn prepare(
    workspace: &Path,
    writable_roots: &[String],
    route: &BTreeMap<String, String>,
) -> Result<()> {
    if !workspace.is_absolute() {
        return Ok(());
    }
    let Ok(canonical_workspace) = fs::canonicalize(workspace) else {
        return Ok(());
    };
    let Some(home) = codex_home(workspace, route) else {
        return Ok(());
    };
    let roots = writable_roots
        .iter()
        .map(|root| {
            let path = Path::new(root);
            path_key(&fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()))
        })
        .collect();
    let sids = capability_sids(&home, &roots)?;
    if sids.is_empty() {
        return Ok(());
    }
    let workspace = canonical_workspace;
    for root in writable_roots.iter().skip(1) {
        let path = Path::new(root);
        let Ok(metadata) = fs::symlink_metadata(path) else {
            continue;
        };
        if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            continue;
        }
        let path = fs::canonicalize(path)?;
        if path.parent() != Some(workspace.as_path()) {
            continue;
        }
        let Some(file) = open_local(&path)? else {
            continue;
        };
        let original = remove_stale_write_denies(&file, &sids)?;
        if original.is_empty() {
            continue;
        }
        if let Err(error) = reconcile_descendants(&path, &sids) {
            // Keep the original Codex root denies as a retry marker after a
            // partial failure. Merge only those entries into the current ACL.
            // Successful later launches need only inspect the root directory.
            restore_write_denies(&file, &original).with_context(|| {
                format!(
                    "restore Codex workspace ACL {} after {error:#}",
                    path.display()
                )
            })?;
            return Err(error);
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "codex_windows/tests.rs"]
pub(super) mod tests;
