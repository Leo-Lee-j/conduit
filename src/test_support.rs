//! Existing fixtures describe the original Chinese interface. Scope their locale
//! explicitly so production defaults and other tests remain independent.
use std::sync::{Mutex, MutexGuard};
static LOCALE: Mutex<()> = Mutex::new(());
pub(crate) struct LocaleGuard {
    previous: conduit::i18n::Locale,
    _lock: MutexGuard<'static, ()>,
}
pub(crate) fn chinese() -> LocaleGuard {
    scoped("zh-CN")
}
pub(crate) fn scoped(language: &str) -> LocaleGuard {
    let lock = LOCALE.lock().unwrap_or_else(|error| error.into_inner());
    let previous = conduit::i18n::locale();
    conduit::i18n::set_locale(&language.to_string().into());
    LocaleGuard {
        previous,
        _lock: lock,
    }
}
impl Drop for LocaleGuard {
    fn drop(&mut self) {
        conduit::i18n::set_locale(&self.previous);
    }
}
