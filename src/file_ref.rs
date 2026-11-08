//! 文件引用：校验待发送文件，发送时再生成路径引用。
//!
//! 引用永远是「路径」而不是文件内容：引擎 CLI 自己读文件，所以 prompt 保持很轻，
//! 支持文本与图片。工作区内的路径用相对形式（工作区搬家后同一条消息仍然
//! 有效）；工作区外的文件同样可以引用，只能给绝对形式，引擎按绝对路径读取。

use gpui_kit::assets::IconName;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

/// 引擎自己读取被引用的文件，这里只防止误引用大产物（打包结果、数据库文件）。
const MAX_REFERENCED_BYTES: u64 = 8 * 1024 * 1024;
/// 判定文本/二进制时嗅探的字节数。
const BINARY_SNIFF_BYTES: u64 = 1024;

/// 一个路径为什么不能成为引用。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Rejection {
    Missing,
    Directory,
    TooLarge,
    Binary,
}

impl Rejection {
    pub fn label(self) -> &'static str {
        match self {
            Self::Missing => {
                conduit::i18n::text_static("file_ref.file_does_not_exist_or_is_unreadable")
            }
            Self::Directory => {
                conduit::i18n::text_static("file_ref.directory_references_are_not_supported_yet")
            }
            Self::TooLarge => conduit::i18n::text_static("file_ref.exceeds_8_mib"),
            Self::Binary => {
                conduit::i18n::text_static("file_ref.binary_files_other_than_images_are_not")
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RejectedFile {
    pub path: PathBuf,
    pub reason: Rejection,
}

/// 一条可以进入 prompt 的文件引用。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FileReference {
    pub path: PathBuf,
    /// 实际发送的文本：工作区内是相对正斜杠路径，工作区外是绝对正斜杠路径。
    pub mention: String,
}

/// 把原始路径分成「可用引用」与「拒绝原因」，同一文件重复出现只保留一次。
pub fn resolve(paths: &[PathBuf], workspace: &Path) -> (Vec<FileReference>, Vec<RejectedFile>) {
    let root = workspace.canonicalize().ok();
    let mut accepted: Vec<FileReference> = Vec::new();
    let mut rejected = Vec::new();
    for path in paths {
        match classify(path, root.as_ref()) {
            // 重复引用同一个文件没有额外信息量，静默丢弃。
            Ok(reference) if !accepted.contains(&reference) => accepted.push(reference),
            Ok(_) => {}
            Err(reason) => rejected.push(RejectedFile {
                path: path.clone(),
                reason,
            }),
        }
    }
    (accepted, rejected)
}

fn classify(path: &Path, root: Option<&PathBuf>) -> Result<FileReference, Rejection> {
    let absolute = path.canonicalize().map_err(|_| Rejection::Missing)?;
    let metadata = absolute.metadata().map_err(|_| Rejection::Missing)?;
    if metadata.is_dir() {
        return Err(Rejection::Directory);
    }
    if metadata.len() > MAX_REFERENCED_BYTES {
        return Err(Rejection::TooLarge);
    }
    if looks_binary(&absolute)? {
        return Err(Rejection::Binary);
    }
    // 引擎以工作区为 cwd，相对形式最短也最稳；工作区外没有相对形式可写，只能给绝对路径。
    let mention = match root.and_then(|root| absolute.strip_prefix(root).ok()) {
        Some(relative) => relative.to_string_lossy().replace('\\', "/"),
        None => display_path(&absolute).replace('\\', "/"),
    };
    Ok(FileReference {
        path: absolute,
        mention,
    })
}

/// 去掉 Windows `canonicalize()` 附带的 `\\?\` 前缀，得到能展示、也能发给引擎的路径。
pub fn display_path(path: &Path) -> String {
    let text = path.display().to_string();
    if let Some(path) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{path}")
    } else {
        text.strip_prefix(r"\\?\").unwrap_or(&text).to_string()
    }
}

/// Read errors should be reported as unreadable files, rather than binary files.
fn looks_binary(path: &Path) -> Result<bool, Rejection> {
    let file = File::open(path).map_err(|_| Rejection::Missing)?;
    let mut buffer = Vec::new();
    file.take(BINARY_SNIFF_BYTES)
        .read_to_end(&mut buffer)
        .map_err(|_| Rejection::Missing)?;
    Ok(buffer.contains(&0) && image::guess_format(&buffer).is_err())
}

/// Persist clipboard pixels alongside app state, so history and retries keep a
/// readable path after a restart. Repeated pastes resolve to the same attachment.
pub fn save_clipboard_image(image: &gpui_kit::Image, directory: &Path) -> anyhow::Result<PathBuf> {
    use anyhow::{Context, bail};
    use std::io::Write;

    if image.bytes.is_empty() {
        bail!(conduit::i18n::text("file_ref.clipboard_image_is_empty"));
    }
    if image.bytes.len() as u64 > MAX_REFERENCED_BYTES {
        bail!(conduit::i18n::text("file_ref.image_exceeds_8_mib"));
    }
    std::fs::create_dir_all(directory).context(conduit::i18n::text_static(
        "file_ref.unable_to_create_image_attachment_directory",
    ))?;
    let path = directory.join(format!(
        "clipboard-{:016x}.{}",
        image.id(),
        image.format.extension()
    ));
    match File::options().write(true).create_new(true).open(&path) {
        Ok(mut file) => {
            if let Err(error) = file.write_all(&image.bytes).and_then(|()| file.sync_all()) {
                drop(file);
                let _ = std::fs::remove_file(&path);
                return Err(error).context(conduit::i18n::text_static(
                    "file_ref.unable_to_save_clipboard_image",
                ));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if std::fs::read(&path).context(conduit::i18n::text_static(
                "file_ref.unable_to_read_saved_image",
            ))? != image.bytes
            {
                bail!(conduit::i18n::text(
                    "file_ref.image_attachment_name_collision_please_retry"
                ));
            }
        }
        Err(error) => {
            return Err(error).context(conduit::i18n::text_static(
                "file_ref.unable_to_save_clipboard_image",
            ));
        }
    }
    Ok(path)
}

/// Only flat, app-generated clipboard files belong to this collector. Never
/// follow directory links or remove user files, subdirectories, or symlinks.
pub fn prune_clipboard_images(directory: &Path, referenced: &[PathBuf]) -> anyhow::Result<usize> {
    use anyhow::{Context, bail};

    let metadata = match directory.symlink_metadata() {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => {
            return Err(error).context(conduit::i18n::text_static(
                "file_ref.unable_to_access_image_attachment_directory",
            ));
        }
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        bail!(conduit::i18n::text(
            "file_ref.image_attachment_directory_is_not_a_regular"
        ));
    }
    let root = directory.canonicalize()?;
    let referenced: HashSet<_> = referenced
        .iter()
        .filter_map(|path| path.canonicalize().ok())
        .collect();
    let mut removed = 0;
    for entry in std::fs::read_dir(&root)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name();
        let Some((id, extension)) = name
            .to_str()
            .and_then(|name| name.strip_prefix("clipboard-"))
            .and_then(|name| name.split_once('.'))
        else {
            continue;
        };
        if id.len() != 16
            || !id.bytes().all(|byte| byte.is_ascii_hexdigit())
            || !matches!(
                extension,
                "png" | "jpg" | "webp" | "gif" | "svg" | "bmp" | "tiff" | "ico" | "pnm"
            )
        {
            continue;
        }
        let path = entry.path().canonicalize()?;
        if path.parent() != Some(root.as_path()) || referenced.contains(&path) {
            continue;
        }
        match std::fs::remove_file(&path) {
            Ok(()) => removed += 1,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).context(conduit::i18n::text_static(
                    "file_ref.unable_to_remove_unreferenced_image_attachment",
                ));
            }
        }
    }
    Ok(removed)
}

/// Deduplicate selections across picker, drop and paste operations by canonical path.
pub fn merge_references(existing: &mut Vec<FileReference>, incoming: Vec<FileReference>) -> usize {
    let before = existing.len();
    for reference in incoming {
        if !existing.iter().any(|file| file.path == reference.path) {
            existing.push(reference);
        }
    }
    existing.len() - before
}

/// Keep the user's prose separate from references until the send boundary.
pub fn compose_prompt(text: &str, references: &[FileReference]) -> String {
    if references.is_empty() {
        return text.to_string();
    }
    if text.trim().is_empty() {
        reference_block(references)
    } else {
        format!("{}\n\n{}", text.trim_end(), reference_block(references))
    }
}

/// Separate the exact trailing reference block generated by compose_prompt for display.
/// Keep the stored prompt intact so retries and engine history retain every file path.
pub fn split_prompt(prompt: &str) -> (&str, Vec<&str>) {
    const HEADER: &str = "引用文件：\n";
    let (text, block) = if let Some(parts) = prompt.rsplit_once("\n\n引用文件：\n") {
        parts
    } else if let Some(block) = prompt.strip_prefix(HEADER) {
        ("", block)
    } else {
        return (prompt, Vec::new());
    };
    let Some(block) = block.strip_suffix('\n') else {
        return (prompt, Vec::new());
    };
    let references = block
        .lines()
        .map(|line| line.strip_prefix("- ").filter(|path| !path.is_empty()))
        .collect::<Option<Vec<_>>>();
    match references {
        Some(references) if !references.is_empty() => (text, references),
        _ => (prompt, Vec::new()),
    }
}

/// 发送给引擎并保存在会话历史中的引用块。两个 CLI 都以工作区为 cwd 启动，相对路径可直接被其文件工具
/// 解析，工作区外的绝对路径本身已足够定位；这比 `--print` 模式下并不保证生效的
/// `@` mention 展开要稳。
pub fn reference_block(references: &[FileReference]) -> String {
    let mut block = String::from("引用文件：");
    for reference in references {
        block.push_str("\n- ");
        block.push_str(&reference.mention);
    }
    block.push('\n');
    block
}

/// 按扩展名给出图标；未知扩展名回落到通用文件图标。
pub fn icon_for(path: &Path) -> IconName {
    let lower = |value: Option<&std::ffi::OsStr>| {
        value
            .map(|value| value.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default()
    };
    let name = lower(path.file_name());
    let extension = lower(path.extension());
    // `.env`、`.gitignore` 这类点在开头的文件在 Path 模型里没有扩展名。
    if extension.is_empty() {
        return if name.starts_with('.') {
            IconName::FileKey
        } else {
            IconName::File
        };
    }
    match extension.as_str() {
        "rs" | "go" | "java" | "kt" | "c" | "h" | "cpp" | "cc" | "hpp" | "cs" | "swift" | "py"
        | "pyi" | "rb" | "php" | "js" | "mjs" | "cjs" | "jsx" | "ts" | "tsx" | "vue" | "svelte"
        | "sql" | "sh" | "bash" | "zsh" | "fish" | "ps1" | "psm1" | "bat" | "cmd" | "pl"
        | "lua" | "dart" | "scala" | "groovy" | "tf" | "hcl" => IconName::FileCode,
        "json" | "jsonc" | "yaml" | "yml" | "toml" | "ini" | "conf" | "properties" => {
            IconName::FileBraces
        }
        "md" | "markdown" | "mdx" | "rst" | "adoc" | "txt" | "log" => IconName::FileText,
        "csv" | "tsv" | "xls" | "xlsx" | "ods" => IconName::FileSpreadsheet,
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "avif" | "bmp" | "tif" | "tiff" | "ico"
        | "heic" | "svg" => IconName::FileImage,
        "mp4" | "mov" | "avi" | "mkv" | "webm" | "wmv" | "flv" => IconName::FileVideoCamera,
        "mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac" | "opus" => IconName::FileMusic,
        "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "7z" | "rar" | "zst" => IconName::FileArchive,
        "diff" | "patch" => IconName::FileDiff,
        "lock" => IconName::FileLock,
        "exe" | "dll" | "so" | "dylib" | "bin" | "class" | "pdb" => IconName::Binary,
        _ => IconName::File,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace_root() -> PathBuf {
        std::env::temp_dir().join(format!("conduit-file-ref-{}", std::process::id()))
    }

    /// 每个用例写独立文件，避免测试之间互相依赖执行顺序。
    fn write(relative: &str, contents: &[u8]) -> PathBuf {
        let root = workspace_root();
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn image_files_are_references_and_clipboard_images_are_persistent_and_deduplicated() {
        let _locale = crate::test_support::chinese();
        let bytes = include_bytes!("../assets/app-icons/Square44x44Logo.png");
        let directory = workspace_root().join("clipboard-images");
        let image = gpui_kit::Image::from_bytes(gpui_kit::ImageFormat::Png, bytes.to_vec());
        let path = save_clipboard_image(&image, &directory).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(save_clipboard_image(&image, &directory).unwrap(), path);
        let (references, rejected) = resolve(std::slice::from_ref(&path), &workspace_root());
        assert!(rejected.is_empty());
        assert_eq!(references.len(), 1);
        // Snipping Tool can expose BMP rather than PNG bytes.
        let bmp = write("clipboard-images/screenshot.bmp", b"BM\0\0\0\0");
        assert!(resolve(&[bmp], &workspace_root()).1.is_empty());
        let disguised_binary = write("clipboard-images/binary.png", b"\0not an image");
        assert_eq!(
            resolve(&[disguised_binary], &workspace_root()).1[0].reason,
            Rejection::Binary
        );
        let empty = gpui_kit::Image::empty();
        assert!(save_clipboard_image(&empty, &directory).is_err());
        let large = gpui_kit::Image::from_bytes(
            gpui_kit::ImageFormat::Png,
            vec![0; MAX_REFERENCED_BYTES as usize + 1],
        );
        assert!(save_clipboard_image(&large, &directory).is_err());
        // A failed or colliding write must never replace an existing attachment.
        std::fs::write(&path, b"existing").unwrap();
        assert!(save_clipboard_image(&image, &directory).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"existing");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn clipboard_cleanup_removes_only_unreferenced_managed_files() {
        let _locale = crate::test_support::chinese();
        let directory = workspace_root().join("clipboard-gc");
        let image = gpui_kit::Image::from_bytes(
            gpui_kit::ImageFormat::Png,
            include_bytes!("../assets/app-icons/Square44x44Logo.png").to_vec(),
        );
        let path = save_clipboard_image(&image, &directory).unwrap();
        let original = directory.join("original.png");
        let non_image = directory.join("clipboard-0000000000000000.txt");
        let malformed = directory.join("clipboard-not-a-hash.png");
        for file in [&original, &non_image, &malformed] {
            std::fs::write(file, b"user owned").unwrap();
        }
        let nested = directory.join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        let nested_image = save_clipboard_image(&image, &nested).unwrap();
        let outside = workspace_root().join(path.file_name().unwrap());
        std::fs::write(&outside, &image.bytes).unwrap();
        let alias = directory.join("nested/..").join(path.file_name().unwrap());
        assert_eq!(prune_clipboard_images(&directory, &[alias]).unwrap(), 0);
        assert!(path.exists());
        assert_eq!(prune_clipboard_images(&directory, &[]).unwrap(), 1);
        assert!(!path.exists());
        assert!(original.exists());
        assert!(non_image.exists());
        assert!(malformed.exists());
        assert!(nested_image.exists());
        assert!(outside.exists());
        assert_eq!(prune_clipboard_images(&directory, &[]).unwrap(), 0);
    }

    #[test]
    fn resolves_paths_inside_the_workspace_to_forward_slash_mentions() {
        let _locale = crate::test_support::chinese();
        let text = write("src/main.rs", b"fn main() {}");
        let (references, rejected) = resolve(std::slice::from_ref(&text), &workspace_root());
        assert_eq!(rejected, vec![]);
        assert_eq!(references.len(), 1);
        assert_eq!(references[0].mention, "src/main.rs");
    }

    #[test]
    fn rejects_directory_missing_and_binary_paths() {
        let _locale = crate::test_support::chinese();
        let binary = write("assets/logo.bin", &[0x89, b'P', 0, 0, 0]);
        let (references, rejected) = resolve(
            &[
                binary,
                workspace_root().join("nowhere.rs"),
                workspace_root(),
            ],
            &workspace_root(),
        );
        assert!(references.is_empty());
        assert!(rejected.iter().any(|file| file.reason == Rejection::Binary));
        assert!(
            rejected
                .iter()
                .any(|file| file.reason == Rejection::Missing)
        );
        assert!(
            rejected
                .iter()
                .any(|file| file.reason == Rejection::Directory)
        );
    }

    /// 工作区外的文件也必须能引用：相对形式无从谈起，就用绝对路径。
    #[test]
    fn resolves_paths_outside_the_workspace_to_absolute_mentions() {
        let _locale = crate::test_support::chinese();
        let outside = std::env::temp_dir().join(format!(
            "conduit-file-ref-outside-{}.rs",
            std::process::id()
        ));
        std::fs::write(&outside, b"fn outside() {}").unwrap();
        let inside = write("src/main.rs", b"fn main() {}");
        let (references, rejected) = resolve(&[outside.clone(), inside], &workspace_root());
        assert_eq!(rejected, vec![]);
        let name = outside.file_name().unwrap().to_string_lossy().to_string();
        let absolute = references
            .iter()
            .find(|reference| reference.mention.ends_with(&name))
            .expect("the outside file must become a reference");
        assert!(
            Path::new(&absolute.mention).is_absolute(),
            "an outside mention has to carry its root: {}",
            absolute.mention
        );
        assert!(
            !absolute.mention.contains('\\'),
            "a verbatim prefix would leak into the history: {}",
            absolute.mention
        );
        assert!(
            references
                .iter()
                .any(|reference| reference.mention == "src/main.rs"),
            "inside paths stay relative"
        );
        let _ = std::fs::remove_file(&outside);
    }

    #[test]
    fn dedupes_the_same_file_referenced_twice() {
        let _locale = crate::test_support::chinese();
        let text = write("README.md", b"# conduit");
        let (references, rejected) = resolve(&[text.clone(), text], &workspace_root());
        assert_eq!(rejected, vec![]);
        assert_eq!(references.len(), 1);
    }

    #[test]
    fn accumulated_selections_dedupe_by_path_and_preserve_order() {
        let _locale = crate::test_support::chinese();
        let a = FileReference {
            path: "a.rs".into(),
            mention: "a.rs".into(),
        };
        let b = FileReference {
            path: "b.rs".into(),
            mention: "b.rs".into(),
        };
        let mut references = vec![a.clone()];
        let alias = FileReference {
            mention: "./a.rs".into(),
            ..a.clone()
        };
        assert_eq!(merge_references(&mut references, vec![alias, b.clone()]), 1);
        assert_eq!(references, vec![a.clone(), b.clone()]);
        assert_eq!(merge_references(&mut references, vec![a, b]), 0);
    }

    #[test]
    fn composes_text_and_file_only_prompts_without_modifying_prose() {
        let _locale = crate::test_support::chinese();
        let file = FileReference {
            path: "folder/a b.rs".into(),
            mention: "folder/a b.rs".into(),
        };
        let references = vec![file];
        let text = "请检查这个文件\n";
        assert_eq!(compose_prompt(text, &[]), text);
        assert_eq!(
            compose_prompt(text, &references),
            "请检查这个文件\n\n引用文件：\n- folder/a b.rs\n"
        );
        assert_eq!(
            compose_prompt("  \n", &references),
            reference_block(&references)
        );
    }

    #[test]
    fn message_display_splits_only_a_complete_trailing_reference_block() {
        let _locale = crate::test_support::chinese();
        let references = vec![
            FileReference {
                path: "docs/a b.md".into(),
                mention: "docs/a b.md".into(),
            },
            FileReference {
                path: "C:/outside/资料.txt".into(),
                mention: "C:/outside/资料.txt".into(),
            },
        ];
        let prompt = compose_prompt("这个文档里有什么内容", &references);
        let (text, paths) = split_prompt(&prompt);
        assert_eq!(text, "这个文档里有什么内容");
        assert_eq!(paths, vec!["docs/a b.md", "C:/outside/资料.txt"]);
        assert_eq!(split_prompt(&compose_prompt("", &references)), ("", paths));
        let prose = "引用文件：\n这里只是正文里的说明";
        assert_eq!(split_prompt(&compose_prompt(prose, &references)).0, prose);
        for literal in [
            "普通消息",
            "引用文件：",
            "引用文件：\n",
            "正文\n\n引用文件：\n- a.md\n这段还是正文\n",
            "示例：\n```\n引用文件：\n- a.md\n```\n",
            "正文\n\n引用文件：\n- \n",
            "正文\n\n引用文件：\n- a.md",
        ] {
            assert_eq!(split_prompt(literal), (literal, vec![]));
        }
    }

    #[test]
    fn detects_deleted_and_oversized_files_on_revalidation() {
        let _locale = crate::test_support::chinese();
        let path = write("send-validation/removed.rs", b"fn main() {}");
        let (references, rejected) = resolve(std::slice::from_ref(&path), &workspace_root());
        assert!(rejected.is_empty());
        std::fs::remove_file(&path).unwrap();
        let (_, rejected) = resolve(&[references[0].path.clone()], &workspace_root());
        assert_eq!(rejected[0].reason, Rejection::Missing);
        let path = write("send-validation/large.txt", b"");
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(MAX_REFERENCED_BYTES + 1)
            .unwrap();
        let (_, rejected) = resolve(std::slice::from_ref(&path), &workspace_root());
        assert_eq!(rejected[0].reason, Rejection::TooLarge);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn reference_block_lists_every_mention() {
        let _locale = crate::test_support::chinese();
        let block = reference_block(&[
            FileReference {
                path: PathBuf::from("a"),
                mention: "src/app.rs".into(),
            },
            FileReference {
                path: PathBuf::from("b"),
                mention: "docs/design.md".into(),
            },
        ]);
        assert_eq!(block, "引用文件：\n- src/app.rs\n- docs/design.md\n");
    }

    #[test]
    fn icon_for_maps_extensions_and_dotfiles() {
        let _locale = crate::test_support::chinese();
        assert_eq!(icon_for(Path::new("src/app.rs")), IconName::FileCode);
        assert_eq!(icon_for(Path::new("Cargo.lock")), IconName::FileLock);
        assert_eq!(icon_for(Path::new("docs/a.md")), IconName::FileText);
        assert_eq!(icon_for(Path::new("table.csv")), IconName::FileSpreadsheet);
        assert_eq!(icon_for(Path::new("a.DNG")), IconName::File);
        assert_eq!(icon_for(Path::new("a.png")), IconName::FileImage);
        assert_eq!(icon_for(Path::new(".env")), IconName::FileKey);
        assert_eq!(icon_for(Path::new("Dockerfile")), IconName::File);
        // 每个分支至少一个代表值，防止 match 静默吞掉新扩展名。
        assert_eq!(icon_for(Path::new("a.yaml")), IconName::FileBraces);
        assert_eq!(icon_for(Path::new("a.mp4")), IconName::FileVideoCamera);
        assert_eq!(icon_for(Path::new("a.mp3")), IconName::FileMusic);
        assert_eq!(icon_for(Path::new("a.zip")), IconName::FileArchive);
        assert_eq!(icon_for(Path::new("a.patch")), IconName::FileDiff);
        assert_eq!(icon_for(Path::new("a.dll")), IconName::Binary);
    }
}
