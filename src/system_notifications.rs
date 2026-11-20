//! Windows toast permission detection for the notification-channel preference.
#[cfg(not(test))]
use windows::UI::Notifications::{NotificationSetting, ToastNotificationManager};
#[cfg(not(test))]
use winreg::{RegKey, enums::HKEY_CURRENT_USER};

pub(crate) const APP_USER_MODEL_ID: &str = "Conduit.Desktop";
pub(crate) const APP_NAME: &str = "Conduit";

/// Match GPUI's unpackaged-app registration before checking the OS setting.
#[cfg(not(test))]
fn register_app() -> std::io::Result<()> {
    let current_user = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = current_user.create_subkey(format!(
        r"Software\Classes\AppUserModelId\{APP_USER_MODEL_ID}"
    ))?;
    key.set_value("DisplayName", &APP_NAME)
}

/// Treat a disabled, unregistered, or inaccessible Windows notifier as unavailable,
/// so the notification dispatcher can keep the existing in-app delivery path.
#[cfg(not(test))]
pub(crate) fn enabled() -> bool {
    if register_app().is_err() {
        return false;
    }
    let Ok(notifier) = ToastNotificationManager::CreateToastNotifierWithId(
        &windows::core::HSTRING::from(APP_USER_MODEL_ID),
    ) else {
        return false;
    };
    matches!(notifier.Setting(), Ok(NotificationSetting::Enabled))
}
