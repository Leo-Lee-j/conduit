use conduit::i18n::Message;

/// Stable failure categories keep diagnostics separate from the current UI language.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Failure {
    Query,
    SolidTagConflict,
    Color,
    Response,
    Format,
    RateLimited,
    Cancelled,
    DownloadLimit,
    Network,
    Decode,
    ImageLimit,
    CacheMissing,
    CacheRead,
    CacheWrite,
    CacheLimit,
    Unknown,
}

impl Failure {
    pub(super) fn key(self) -> &'static str {
        match self {
            Self::Query => "wallpaper.error.query",
            Self::SolidTagConflict => "wallpaper.error.solid_tag_conflict",
            Self::Color => "wallpaper.error.color",
            Self::Response => "wallpaper.error.response",
            Self::Format => "wallpaper.error.format",
            Self::RateLimited => "wallpaper.error.rate_limited",
            Self::Cancelled => "wallpaper.error.cancelled",
            Self::DownloadLimit => "wallpaper.error.download_limit",
            Self::Network => "wallpaper.error.network",
            Self::Decode => "wallpaper.error.decode",
            Self::ImageLimit => "wallpaper.error.image_limit",
            Self::CacheMissing => "wallpaper.error.cache_missing",
            Self::CacheRead => "wallpaper.error.cache_read",
            Self::CacheWrite => "wallpaper.error.cache_write",
            Self::CacheLimit => "wallpaper.error.cache_limit",
            Self::Unknown => "wallpaper.error.unknown",
        }
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.key())
    }
}

impl std::error::Error for Failure {}

/// Translate at render time, including errors received before a language change.
/// The innermost category wins: e.g. a bounded network read can fail on size or cancellation.
pub fn error_message(error: &anyhow::Error) -> Message {
    let failure = error
        .chain()
        .filter_map(|cause| cause.downcast_ref::<Failure>())
        .last()
        // anyhow contexts are not standalone sources; use its typed context lookup as well.
        .or_else(|| error.downcast_ref::<Failure>())
        .copied()
        .unwrap_or(Failure::Unknown);
    Message::new(failure.key(), &[])
}
