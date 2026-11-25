use super::*;
use std::os::windows::process::CommandExt;
use windows_sys::Win32::{
    Foundation::GENERIC_READ,
    Security::{
        Authorization::{
            ConvertSidToStringSidW, DENY_ACCESS, EXPLICIT_ACCESS_W, SetEntriesInAclW,
            TRUSTEE_IS_SID, TRUSTEE_IS_UNKNOWN, TRUSTEE_W,
        },
        CONTAINER_INHERIT_ACE, OBJECT_INHERIT_ACE,
    },
    Storage::FileSystem::{FILE_GENERIC_READ, READ_CONTROL, WRITE_DAC},
};

pub(crate) const WORKSPACE_SID: &str = "S-1-5-21-1111-2222-3333-4444";
pub(crate) const ROOT_SID: &str = "S-1-5-21-1111-2222-3333-5555";
const LEGACY_SID: &str = "S-1-5-21-1111-2222-3333-6666";
const READONLY_SID: &str = "S-1-5-21-1111-2222-3333-7777";
const FOREIGN_SID: &str = "S-1-5-21-1111-2222-3333-8888";

pub(crate) struct Fixture {
    pub root: PathBuf,
    pub workspace: PathBuf,
    pub home: PathBuf,
}
impl Fixture {
    pub fn new() -> Self {
        Self::in_directory(&std::env::temp_dir())
    }
    pub fn in_directory(directory: &Path) -> Self {
        let root = directory.join(format!(
            "conduit codex 权限 {}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let workspace = root.join("repo");
        let home = root.join("codex-home");
        fs::create_dir_all(workspace.join(".git/info")).unwrap();
        fs::write(workspace.join(".git/info/local.txt"), "fixture").unwrap();
        fs::create_dir(&home).unwrap();
        Self {
            root,
            workspace,
            home,
        }
    }
    pub fn cache(&self) {
        let key = path_key(&fs::canonicalize(&self.workspace).unwrap());
        fs::write(
            self.home.join("cap_sid"),
            serde_json::json!({
                "workspace": LEGACY_SID,
                "readonly": READONLY_SID,
                "workspace_by_cwd": {key.clone(): WORKSPACE_SID},
                "writable_root_by_path": {
                    key: ROOT_SID,
                    path_key(&self.root.join("outside")): FOREIGN_SID,
                },
            })
            .to_string(),
        )
        .unwrap();
    }
    pub fn environment(&self) -> BTreeMap<String, String> {
        BTreeMap::from([(
            "CODEX_HOME".into(),
            self.home.to_string_lossy().into_owned(),
        )])
    }
    pub fn roots(&self) -> Vec<String> {
        ["", ".git", ".agents", ".codex", ".aws"]
            .iter()
            .map(|name| self.workspace.join(name).to_string_lossy().into_owned())
            .collect()
    }
    pub fn protect_git(&self) {
        deny(
            &self.workspace.join(".git"),
            WORKSPACE_SID,
            CODEX_WRITE_DENY,
        );
    }
    fn prepare(&self) {
        prepare(&self.workspace, &self.roots(), &self.environment()).unwrap();
    }
    fn junction(&self, link: &Path, target: &Path) {
        let script = self.root.join("junction.ps1");
        fs::write(&script, "param($Link, $Target)\n$ErrorActionPreference = 'Stop'\nNew-Item -ItemType Junction -Path $Link -Target $Target | Out-Null\n").unwrap();
        let shell =
            PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:/Windows".into()))
                .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let output = std::process::Command::new(shell)
            .args(["-NoProfile", "-NonInteractive", "-File"])
            .arg(script)
            .arg(link)
            .arg(target)
            .creation_flags(0x0800_0000)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn sid(value: &str) -> LocalAllocation {
    let wide: Vec<u16> = value.encode_utf16().chain(Some(0)).collect();
    let mut allocated = null_mut();
    win32(unsafe { ConvertStringSidToSidW(wide.as_ptr(), &mut allocated) }).unwrap();
    LocalAllocation(allocated)
}

pub(crate) fn deny(path: &Path, trustee: &str, mask: u32) {
    let file = OpenOptions::new()
        .access_mode(READ_CONTROL | WRITE_DAC)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .unwrap();
    let mut descriptor = null_mut();
    let mut dacl = null_mut();
    status(unsafe {
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
    })
    .unwrap();
    let _descriptor = LocalAllocation(descriptor);
    let trustee = sid(trustee);
    let explicit = EXPLICIT_ACCESS_W {
        grfAccessPermissions: mask,
        grfAccessMode: DENY_ACCESS,
        grfInheritance: CONTAINER_INHERIT_ACE | OBJECT_INHERIT_ACE,
        Trustee: TRUSTEE_W {
            pMultipleTrustee: null_mut(),
            MultipleTrusteeOperation: 0,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_UNKNOWN,
            ptstrName: trustee.0.cast(),
        },
    };
    let mut changed = null_mut();
    let result = unsafe { SetEntriesInAclW(1, &explicit, dacl, &mut changed) };
    let _changed = LocalAllocation(changed.cast());
    status(result).unwrap();
    // Use the native sandbox's inheritable behavior when creating the fixture.
    status(unsafe {
        SetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            changed,
            null_mut(),
        )
    })
    .unwrap();
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Ace {
    kind: u8,
    mask: u32,
    sid: String,
    bytes: Vec<u8>,
}
impl Ace {
    pub fn owned_write_deny(&self) -> bool {
        self.kind == ACCESS_DENIED_ACE_TYPE
            && matches!(self.mask, CODEX_WRITE_DENY | MAPPED_WRITE_DENY)
            && [WORKSPACE_SID, ROOT_SID, LEGACY_SID].contains(&self.sid.as_str())
    }
}

pub(crate) fn acl(path: &Path) -> Vec<Ace> {
    let file = OpenOptions::new()
        .access_mode(READ_CONTROL)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .unwrap();
    let mut descriptor = null_mut();
    let mut dacl: *mut ACL = null_mut();
    status(unsafe {
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
    })
    .unwrap();
    let _descriptor = LocalAllocation(descriptor);
    assert!(!dacl.is_null());
    (0..unsafe { (*dacl).AceCount } as u32)
        .map(|index| {
            let mut entry = null_mut();
            win32(unsafe { GetAce(dacl, index, &mut entry) }).unwrap();
            let header = unsafe { &*entry.cast::<ACE_HEADER>() };
            assert!(header.AceType <= 1);
            let denied = unsafe { &*entry.cast::<ACCESS_DENIED_ACE>() };
            let mut name = null_mut();
            win32(unsafe {
                ConvertSidToStringSidW(
                    std::ptr::addr_of!(denied.SidStart).cast_mut().cast(),
                    &mut name,
                )
            })
            .unwrap();
            let _name = LocalAllocation(name.cast());
            let mut length = 0;
            while unsafe { *name.add(length) } != 0 {
                length += 1;
            }
            Ace {
                kind: header.AceType,
                mask: denied.Mask,
                sid: String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(name, length) }),
                bytes: unsafe {
                    std::slice::from_raw_parts(entry.cast::<u8>(), header.AceSize as usize)
                }
                .to_vec(),
            }
        })
        .collect()
}

#[test]
fn removes_scoped_codex_write_denies_and_preserves_other_aces() {
    let fixture = Fixture::new();
    fixture.cache();
    for name in [".git", ".agents", ".codex"] {
        let directory = fixture.workspace.join(name);
        fs::create_dir_all(directory.join("nested")).unwrap();
        fs::write(directory.join("nested/state.txt"), "fixture").unwrap();
        for trustee in [
            WORKSPACE_SID,
            ROOT_SID,
            LEGACY_SID,
            FOREIGN_SID,
            READONLY_SID,
        ] {
            deny(&directory, trustee, CODEX_WRITE_DENY);
        }
        deny(&directory, WORKSPACE_SID, FILE_GENERIC_READ | GENERIC_READ);
    }
    let paths: Vec<_> = [".git", ".agents", ".codex"]
        .into_iter()
        .flat_map(|name| {
            [
                fixture.workspace.join(name),
                fixture.workspace.join(name).join("nested/state.txt"),
            ]
        })
        .collect();
    let expected: Vec<_> = paths
        .iter()
        .map(|path| {
            let entries = acl(path);
            assert!(entries.iter().any(Ace::owned_write_deny));
            entries
                .into_iter()
                .filter(|entry| !entry.owned_write_deny())
                .collect::<Vec<_>>()
        })
        .collect();
    // A case/separator variant must use the same Codex capability cache key.
    let alternate = fixture
        .workspace
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_uppercase();
    let environment = BTreeMap::from([
        (
            "CODEX_HOME".into(),
            fixture
                .root
                .join("unused-home")
                .to_string_lossy()
                .into_owned(),
        ),
        (
            "codex_home".into(),
            fixture.home.to_string_lossy().into_owned(),
        ),
    ]);
    prepare(Path::new(&alternate), &fixture.roots(), &environment).unwrap();
    for (path, expected) in paths.iter().zip(expected) {
        assert_eq!(acl(path), expected, "{}", path.display());
    }
    let repaired = acl(&fixture.workspace.join(".git"));
    fixture.prepare();
    assert_eq!(acl(&fixture.workspace.join(".git")), repaired);
    assert!(!fixture.workspace.join(".aws").exists());
}

#[test]
fn leaves_metadata_junctions_and_shared_files_untouched() {
    let fixture = Fixture::new();
    fixture.cache();
    let outside = fixture.root.join("outside");
    fs::create_dir(&outside).unwrap();
    let shared = outside.join("shared.txt");
    fs::write(&shared, "outside").unwrap();
    let git = fixture.workspace.join(".git");
    fs::hard_link(&shared, git.join("shared.txt")).unwrap();
    fixture.junction(&git.join("external"), &outside);
    fixture.junction(&fixture.workspace.join(".codex"), &outside);
    deny(&git, WORKSPACE_SID, CODEX_WRITE_DENY);
    deny(&outside, WORKSPACE_SID, CODEX_WRITE_DENY);
    let directory_acl = acl(&outside);
    let shared_acl = acl(&shared);
    fixture.prepare();
    assert!(!acl(&git).iter().any(Ace::owned_write_deny));
    assert_eq!(acl(&outside), directory_acl);
    assert_eq!(acl(&shared), shared_acl);
    fs::remove_dir(git.join("external")).unwrap();
    fs::remove_dir(fixture.workspace.join(".codex")).unwrap();
}

#[test]
fn leaves_linked_worktree_metadata_untouched() {
    let fixture = Fixture::new();
    fixture.cache();
    let git = fixture.workspace.join(".git");
    fs::remove_dir_all(&git).unwrap();
    let outside = fixture.root.join("shared-git");
    fs::create_dir(&outside).unwrap();
    fs::write(&git, format!("gitdir: {}\n", outside.display())).unwrap();
    deny(&outside, WORKSPACE_SID, CODEX_WRITE_DENY);
    deny(&git, WORKSPACE_SID, CODEX_WRITE_DENY);
    let before = (acl(&git), acl(&outside));
    fixture.prepare();
    assert_eq!((acl(&git), acl(&outside)), before);
}

#[test]
fn does_not_guess_ownership_for_missing_or_malformed_capability_cache() {
    let fixture = Fixture::new();
    let git = fixture.workspace.join(".git");
    deny(&git, WORKSPACE_SID, CODEX_WRITE_DENY);
    let before = acl(&git);
    fixture.prepare();
    fs::write(fixture.home.join("cap_sid"), "{broken cache").unwrap();
    fixture.prepare();
    assert_eq!(acl(&git), before);
    assert_eq!(
        fs::read_to_string(fixture.home.join("cap_sid")).unwrap(),
        "{broken cache"
    );
}

#[test]
fn retries_descendant_reconciliation_after_a_locked_file() {
    let fixture = Fixture::new();
    fixture.cache();
    fixture.protect_git();
    let git = fixture.workspace.join(".git");
    let before = acl(&git);
    let locked = OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(git.join("info/local.txt"))
        .unwrap();
    assert!(prepare(&fixture.workspace, &fixture.roots(), &fixture.environment()).is_err());
    let (mut restored_denies, restored_other): (Vec<_>, Vec<_>) =
        acl(&git).into_iter().partition(Ace::owned_write_deny);
    let (mut original_denies, original_other): (Vec<_>, Vec<_>) =
        before.into_iter().partition(Ace::owned_write_deny);
    restored_denies.sort_by(|left, right| left.bytes.cmp(&right.bytes));
    original_denies.sort_by(|left, right| left.bytes.cmp(&right.bytes));
    assert_eq!(restored_denies, original_denies);
    assert_eq!(restored_other, original_other);
    drop(locked);
    fixture.prepare();
    assert!(!acl(&git).iter().any(Ace::owned_write_deny));
    assert!(
        !acl(&git.join("info/local.txt"))
            .iter()
            .any(Ace::owned_write_deny)
    );
}
