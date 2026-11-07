//! User-owned application shortcuts, shared by settings and runtime dispatch.
use super::*;

pub(super) type Overrides = BTreeMap<String, Vec<String>>;
struct ActiveShortcuts(Overrides);
impl Global for ActiveShortcuts {}
pub(super) fn hint(id: &str, cx: &App) -> String {
    display(
        id,
        &cx.try_global::<ActiveShortcuts>()
            .map(|state| state.0.clone())
            .unwrap_or_default(),
    )
}
pub(super) const OWNED_BINDING: KeyBindingMetaIndex = KeyBindingMetaIndex(0x434f4e44);
pub(super) const DOUBLE_SHIFT: &str = "shift shift";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Scope {
    Global,
    Settings,
    Composer,
    File,
    Diff,
    Terminal,
}
impl Scope {
    pub fn label(self) -> &'static str {
        match self {
            Self::Global => "全局",
            Self::Settings => "设置页",
            Self::Composer => "输入区",
            Self::File => "文件预览",
            Self::Diff => "Diff",
            Self::Terminal => "终端",
        }
    }
    fn overlaps(self, other: Self) -> bool {
        self == Self::Global || other == Self::Global || self == other
    }
}

pub(super) struct Shortcut {
    pub id: &'static str,
    pub label: &'static str,
    pub scope: Scope,
    pub defaults: &'static [&'static str],
    contexts: &'static [Option<&'static str>],
}
macro_rules! shortcut {
    ($id:literal, $label:literal, $scope:ident, [$($key:literal),*], $contexts:expr) => {
        Shortcut { id: $id, label: $label, scope: Scope::$scope, defaults: &[$($key),*], contexts: $contexts }
    };
}
const GLOBAL: &[Option<&str>] = &[None];
const SETTINGS: &[Option<&str>] = &[Some("ConduitSettings"), Some("ConduitSettings > Input")];
const COMPOSER: &[Option<&str>] = &[Some("ConduitComposer"), Some("ConduitComposer > Input")];
const FILE: &[Option<&str>] = &[
    Some("ConduitFilePreview"),
    Some("ConduitFilePreview > Input"),
    Some("ConduitFilePreview > TextView"),
];
const DIFF: &[Option<&str>] = &[Some("GitDiffDialog"), Some("Diff")];
pub(super) static SHORTCUTS: &[Shortcut] = &[
    shortcut!("recent_sessions", "最近会话", Global, ["ctrl-e"], GLOBAL),
    shortcut!("search_sessions", "搜索会话", Global, ["ctrl-p"], GLOBAL),
    shortcut!(
        "open_settings",
        "打开设置",
        Global,
        ["ctrl-,", "ctrl-m"],
        GLOBAL
    ),
    shortcut!(
        "search_files",
        "文件名搜索",
        Global,
        ["shift shift"],
        GLOBAL
    ),
    shortcut!("toggle_git", "切换 Git 侧栏", Global, ["ctrl-k"], GLOBAL),
    shortcut!(
        "toggle_terminal",
        "显示 / 隐藏终端",
        Global,
        ["ctrl-`"],
        GLOBAL
    ),
    shortcut!("new_terminal", "新建终端", Global, ["ctrl-shift-`"], GLOBAL),
    shortcut!(
        "settings_search",
        "搜索设置",
        Settings,
        ["ctrl-f"],
        SETTINGS
    ),
    shortcut!(
        "settings_back",
        "清除搜索 / 返回工作台",
        Settings,
        ["escape"],
        SETTINGS
    ),
    shortcut!(
        "send_prompt",
        "发送消息 / 加入队列",
        Composer,
        ["enter"],
        COMPOSER
    ),
    shortcut!(
        "newline",
        "输入区换行",
        Composer,
        ["shift-enter", "ctrl-enter"],
        COMPOSER
    ),
    shortcut!(
        "stop_reply",
        "停止当前会话回复",
        Composer,
        ["ctrl-shift-x"],
        COMPOSER
    ),
    shortcut!("file_find", "文件内查找", File, ["ctrl-f"], FILE),
    shortcut!("next_diff", "下一处修改", Diff, ["f7"], DIFF),
    shortcut!("previous_diff", "上一处修改", Diff, ["shift-f7"], DIFF),
    shortcut!(
        "close_diff",
        "关闭 Diff 标签",
        Diff,
        ["escape", "ctrl-w"],
        &[Some("GitDiffDocument")]
    ),
    shortcut!(
        "leave_terminal",
        "从终端返回工作台",
        Terminal,
        ["ctrl-shift-f6"],
        &[Some("ConduitTerminal")]
    ),
    shortcut!(
        "previous_terminal",
        "上一个终端",
        Terminal,
        ["ctrl-shift-pageup"],
        &[Some("ConduitTerminal")]
    ),
    shortcut!(
        "next_terminal",
        "下一个终端",
        Terminal,
        ["ctrl-shift-pagedown"],
        &[Some("ConduitTerminal")]
    ),
];

impl Shortcut {
    fn binding(&self, key: &str, context: Option<&str>) -> KeyBinding {
        match self.id {
            "recent_sessions" => KeyBinding::new(key, RecentSessions, context),
            "search_sessions" => KeyBinding::new(key, OpenSessionSearch, context),
            "open_settings" => KeyBinding::new(key, OpenSettingsShortcut, context),
            "search_files" => KeyBinding::new(key, OpenFileSearchShortcut, context),
            "toggle_git" => KeyBinding::new(key, ToggleGitNavigation, context),
            "toggle_terminal" => KeyBinding::new(key, ToggleTerminal, context),
            "new_terminal" => KeyBinding::new(key, NewTerminal, context),
            "settings_search" => KeyBinding::new(key, FocusSettingsSearch, context),
            "settings_back" => KeyBinding::new(key, LeaveSettings, context),
            "send_prompt" => KeyBinding::new(key, SendPrompt, context),
            "newline" => KeyBinding::new(key, Newline, context),
            "stop_reply" => KeyBinding::new(key, StopReply, context),
            "file_find" => KeyBinding::new(key, file_preview::ToggleFind, context),
            "next_diff" => KeyBinding::new(key, NextDiff, context),
            "previous_diff" => KeyBinding::new(key, PreviousDiff, context),
            "close_diff" => KeyBinding::new(key, CloseDocument, context),
            "leave_terminal" => KeyBinding::new(key, LeaveTerminal, context),
            "previous_terminal" => KeyBinding::new(key, PreviousTerminal, context),
            "next_terminal" => KeyBinding::new(key, NextTerminal, context),
            _ => unreachable!("shortcut catalogue owns the action IDs"),
        }
        .with_meta(OWNED_BINDING)
    }
    pub fn keys(&self, overrides: &Overrides) -> Vec<String> {
        overrides
            .get(self.id)
            .cloned()
            .unwrap_or_else(|| self.defaults.iter().map(|key| key.to_string()).collect())
    }
}
pub(super) fn get(id: &str) -> &'static Shortcut {
    SHORTCUTS
        .iter()
        .find(|shortcut| shortcut.id == id)
        .expect("known shortcut")
}

pub(super) fn normalize(raw: &str) -> Result<String, String> {
    let raw = raw.trim().to_lowercase().replace(" + ", "-");
    if raw == DOUBLE_SHIFT {
        return Ok(raw);
    }
    if raw.is_empty() || raw.split_whitespace().count() != 1 {
        return Err("请录制一个组合键，或点击取消绑定。".into());
    }
    let key = Keystroke::parse(&raw).map_err(|_| "无法识别此按键，请重新录制组合键。")?;
    let supported = key.key.chars().count() == 1
        || matches!(
            key.key.as_str(),
            "enter"
                | "escape"
                | "tab"
                | "space"
                | "backspace"
                | "delete"
                | "insert"
                | "home"
                | "end"
                | "pageup"
                | "pagedown"
                | "up"
                | "down"
                | "left"
                | "right"
        )
        || key
            .key
            .strip_prefix('f')
            .and_then(|n| n.parse::<u8>().ok())
            .is_some_and(|n| (1..=24).contains(&n));
    if !supported {
        return Err("请按下字母、数字、功能键或导航键，不能只使用修饰键。".into());
    }
    if key.key.chars().count() == 1
        && !key.modifiers.control
        && !key.modifiers.alt
        && !key.modifiers.platform
    {
        return Err("字母和数字需要搭配 Ctrl、Alt 或 Win，避免影响正常输入。".into());
    }
    Ok(key.unparse())
}

pub(super) fn validate(
    shortcut: &Shortcut,
    keys: &[String],
    overrides: &Overrides,
) -> Result<(), String> {
    let mut seen = HashSet::new();
    for key in keys {
        let normalized = normalize(key)?;
        if normalized == DOUBLE_SHIFT && shortcut.id != "search_files" {
            return Err("双击 Shift 仅用于文件名搜索，请选择普通组合键。".into());
        }
        if !seen.insert(normalized.clone()) {
            return Err("同一操作不能重复绑定相同按键。".into());
        }
        if matches!(
            normalized.as_str(),
            "ctrl-c"
                | "ctrl-v"
                | "ctrl-x"
                | "ctrl-a"
                | "ctrl-z"
                | "ctrl-y"
                | "ctrl-shift-z"
                | "ctrl-shift-v"
                | "shift-insert"
        ) {
            return Err("此按键用于复制、粘贴或文本编辑，请选择其他组合键。".into());
        }
        if matches!(
            normalized.as_str(),
            "alt-f4" | "ctrl-alt-delete" | "alt-tab" | "alt-shift-tab"
        ) {
            return Err("此按键由系统使用，请选择其他组合键。".into());
        }
        for other in SHORTCUTS {
            if other.id != shortcut.id
                && shortcut.scope.overlaps(other.scope)
                && other
                    .keys(overrides)
                    .iter()
                    .any(|key| normalize(key).is_ok_and(|key| key == normalized))
            {
                return Err(format!(
                    "与「{} · {}」的 {} 冲突，请选择其他按键或先修改该操作。",
                    other.scope.label(),
                    other.label,
                    display_key(&normalized)
                ));
            }
        }
        if shortcut.scope == Scope::Settings
            && matches!(normalized.as_str(), "enter" | "tab" | "shift-tab")
        {
            return Err("与设置搜索结果和表单导航按键冲突，请选择其他按键。".into());
        }
        if shortcut.scope == Scope::Composer
            && matches!(normalized.as_str(), "up" | "down" | "tab" | "escape")
        {
            return Err("与提示词历史或文件引用菜单按键冲突，请选择其他按键。".into());
        }
        // Keep the file preview's navigation controls and form navigation usable.
        if (shortcut.scope == Scope::Global || shortcut.scope == Scope::File)
            && matches!(
                normalized.as_str(),
                "f3" | "shift-f3" | "escape" | "enter" | "shift-enter"
            )
        {
            return Err("与文件查找的跳转 / 关闭按键冲突，请选择其他按键。".into());
        }
        if shortcut.scope == Scope::Global
            && matches!(
                normalized.as_str(),
                "enter"
                    | "escape"
                    | "tab"
                    | "up"
                    | "down"
                    | "left"
                    | "right"
                    | "home"
                    | "end"
                    | "pageup"
                    | "pagedown"
                    | "backspace"
                    | "delete"
                    | "space"
            )
        {
            return Err("全局绑定会影响输入和界面导航，请为此按键添加修饰键。".into());
        }
    }
    Ok(())
}

pub(super) fn sanitize(overrides: &mut Overrides) {
    overrides.retain(|id, keys| {
        SHORTCUTS.iter().any(|shortcut| shortcut.id == id)
            && keys.iter_mut().all(|key| match normalize(key) {
                Ok(value) => {
                    *key = value;
                    true
                }
                Err(_) => false,
            })
    });
    loop {
        let invalid = SHORTCUTS
            .iter()
            .filter(|shortcut| {
                overrides.contains_key(shortcut.id)
                    && validate(shortcut, &shortcut.keys(overrides), overrides).is_err()
            })
            .map(|shortcut| shortcut.id)
            .collect::<Vec<_>>();
        if invalid.is_empty() {
            break;
        }
        for id in invalid {
            overrides.remove(id);
        }
    }
}

pub(super) fn install(overrides: &Overrides, cx: &mut App) {
    cx.set_global(ActiveShortcuts(overrides.clone()));
    let retained = cx
        .key_bindings()
        .borrow()
        .bindings()
        .filter(|binding| binding.meta() != Some(OWNED_BINDING))
        .cloned()
        .collect::<Vec<_>>();
    cx.clear_key_bindings();
    cx.bind_keys(retained);
    let mut bindings = Vec::new();
    for shortcut in SHORTCUTS {
        for key in shortcut.keys(overrides) {
            if key == DOUBLE_SHIFT {
                continue;
            }
            for context in shortcut.contexts {
                bindings.push(shortcut.binding(&key, *context));
            }
            if shortcut.scope == Scope::Global
                && !matches!(shortcut.id, "toggle_terminal" | "new_terminal")
            {
                bindings.push(
                    KeyBinding::new(&key, NoAction, Some("ConduitTerminal"))
                        .with_meta(OWNED_BINDING),
                );
            }
        }
    }
    cx.bind_keys(bindings);
}

pub(super) fn display_key(key: &str) -> String {
    if key == DOUBLE_SHIFT {
        return "Shift × 2".into();
    }
    key.split('-')
        .map(|part| match part {
            "ctrl" => "Ctrl".into(),
            "shift" => "Shift".into(),
            "alt" => "Alt".into(),
            "super" | "win" | "cmd" => "Win".into(),
            "enter" => "Enter".into(),
            "escape" => "Esc".into(),
            "pageup" => "PageUp".into(),
            "pagedown" => "PageDown".into(),
            key => key.to_uppercase(),
        })
        .collect::<Vec<_>>()
        .join(" + ")
}
pub(super) fn display(id: &str, overrides: &Overrides) -> String {
    let keys = get(id).keys(overrides);
    if keys.is_empty() {
        "未绑定".into()
    } else {
        keys.iter()
            .map(|key| display_key(key))
            .collect::<Vec<_>>()
            .join(" / ")
    }
}

#[cfg(test)]
mod tests {
    use super::{DOUBLE_SHIFT, Overrides, SHORTCUTS, get, normalize, sanitize, validate};
    use crate::store::Store;

    #[test]
    fn shortcut_conflicts_respect_scope_defaults_and_disabled_actions() {
        let mut overrides = Overrides::new();
        for shortcut in SHORTCUTS {
            validate(shortcut, &shortcut.keys(&overrides), &overrides).unwrap();
        }
        assert!(
            validate(get("open_settings"), &["ctrl-p".into()], &overrides)
                .unwrap_err()
                .contains("搜索会话")
        );
        assert!(validate(get("settings_search"), &["ctrl-f".into()], &overrides).is_ok());
        assert!(validate(get("recent_sessions"), &["ctrl-f".into()], &overrides).is_err());
        assert!(validate(get("recent_sessions"), &["ctrl-v".into()], &overrides).is_err());
        assert!(validate(get("recent_sessions"), &["alt-f4".into()], &overrides).is_err());
        assert!(validate(get("file_find"), &["f3".into()], &overrides).is_err());
        overrides.insert("search_sessions".into(), vec![]);
        assert!(validate(get("recent_sessions"), &["ctrl-p".into()], &overrides).is_ok());
        overrides.insert("recent_sessions".into(), vec!["ctrl-alt-e".into()]);
        assert!(
            validate(get("settings_search"), &["ctrl-alt-e".into()], &overrides)
                .unwrap_err()
                .contains("最近会话")
        );
    }

    #[test]
    fn shortcut_configuration_roundtrips_and_rejects_invalid_saved_bindings() {
        let mut store = Store::default();
        store
            .ui
            .shortcuts
            .insert("open_settings".into(), vec!["ctrl-alt-s".into()]);
        store.ui.shortcuts.insert("search_sessions".into(), vec![]);
        let serialized = serde_json::to_string(&store).unwrap();
        let mut restored: Store = serde_json::from_str(&serialized).unwrap();
        assert_eq!(restored.ui.shortcuts, store.ui.shortcuts);
        restored
            .ui
            .shortcuts
            .insert("recent_sessions".into(), vec!["ctrl-alt-s".into()]);
        restored
            .ui
            .shortcuts
            .insert("unknown_action".into(), vec!["bad".into()]);
        restored
            .ui
            .shortcuts
            .insert("file_find".into(), vec!["ctrl-v".into()]);
        sanitize(&mut restored.ui.shortcuts);
        assert!(!restored.ui.shortcuts.contains_key("recent_sessions"));
        assert!(!restored.ui.shortcuts.contains_key("unknown_action"));
        assert!(!restored.ui.shortcuts.contains_key("file_find"));
        for shortcut in SHORTCUTS {
            validate(
                shortcut,
                &shortcut.keys(&restored.ui.shortcuts),
                &restored.ui.shortcuts,
            )
            .unwrap();
        }
        assert_eq!(normalize("Ctrl + Shift + P").unwrap(), "ctrl-shift-p");
        assert!(normalize("ctrl-invalid-key").is_err());
        assert!(normalize("x").is_err());
        assert!(normalize("shift").is_err());
        assert_eq!(normalize(DOUBLE_SHIFT).unwrap(), DOUBLE_SHIFT);
        let old: Store = serde_json::from_str(r#"{"version":2,"workspaces":[]}"#).unwrap();
        assert!(old.ui.shortcuts.is_empty());
    }
}
