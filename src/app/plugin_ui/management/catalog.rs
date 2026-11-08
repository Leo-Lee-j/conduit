use super::*;
use conduit::plugin_packages::catalog::{self as service, Entry};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(in crate::app) enum Filter {
    #[default]
    All,
    Official,
    Community,
}
pub(in crate::app) struct CatalogUi {
    pub discover: bool,
    pub input: Option<Entity<InputState>>,
    pub entries: Vec<Entry>,
    pub filter: Filter,
    pub loading: bool,
    pub queried: bool,
    pub query: String,
    pub page: u32,
    pub has_more: bool,
    pub cached: bool,
    pub fetched_at: u64,
    pub errors: Vec<String>,
    generation: u64,
}
impl Default for CatalogUi {
    fn default() -> Self {
        Self {
            discover: false,
            input: None,
            entries: vec![service::bundled_entry()],
            filter: Filter::All,
            loading: false,
            queried: false,
            query: String::new(),
            page: 1,
            has_more: false,
            cached: false,
            fetched_at: 0,
            errors: vec![],
            generation: 0,
        }
    }
}
impl ConduitApp {
    pub(in crate::app) fn show_plugin_discovery(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.plugins.packages.catalog.discover = true;
        if self.plugins.packages.catalog.input.is_none() {
            let input = cx.new(|cx| {
                InputState::new(window, cx).placeholder(conduit::i18n::text_static(
                    "settings.plugins.catalog_search",
                ))
            });
            self.plugins.packages.catalog.input = Some(input);
        }
        if !self.plugins.packages.catalog.queried {
            self.query_plugin_catalog(false, cx);
        }
        cx.notify();
    }
    pub(in crate::app) fn query_plugin_catalog(&mut self, more: bool, cx: &mut Context<Self>) {
        let state = &mut self.plugins.packages.catalog;
        if self.closing || state.loading {
            return;
        }
        let query = if more {
            state.query.clone()
        } else {
            state
                .input
                .as_ref()
                .map(|input| input.read(cx).value().trim().to_owned())
                .unwrap_or_default()
        };
        if query.len() > 200 {
            state.errors = vec![conduit::i18n::text("settings.plugins.query_too_long")];
            cx.notify();
            return;
        }
        let page = if more { state.page + 1 } else { 1 };
        state.loading = true;
        state.queried = true;
        state.cached = false;
        state.errors.clear();
        state.generation = state.generation.wrapping_add(1);
        let generation = state.generation;
        if !more {
            state.query = query.clone();
            state.entries = vec![service::bundled_entry()];
            state.has_more = false;
            state.fetched_at = 0;
        }
        let root = installation_root();
        let can_cache = self.writable;
        cx.spawn(async move |this, cx| {
            let cache_root = root.clone();
            let cache_query = query.clone();
            let cached = cx
                .background_executor()
                .spawn(async move { service::cached(&cache_root, &cache_query, page) })
                .await;
            if let Some(cached) = cached {
                let _ = this.update(cx, |app, cx| {
                    let state = &mut app.plugins.packages.catalog;
                    if state.generation != generation || app.closing {
                        return;
                    }
                    service::merge(&mut state.entries, cached.entries);
                    state.cached = true;
                    state.fetched_at = cached.fetched_at;
                    state.has_more = cached.has_more;
                    state.page = page;
                    cx.notify();
                });
            }
            let result = cx
                .background_executor()
                .spawn(async move {
                    let result = service::fetch(&query, page);
                    if let Ok(value) = &result
                        && can_cache
                    {
                        let _ = service::save_cache(&root, &query, page, value);
                    }
                    result
                })
                .await;
            let _ = this.update(cx, |app, cx| {
                let state = &mut app.plugins.packages.catalog;
                if state.generation != generation {
                    return;
                }
                state.loading = false;
                match result {
                    Ok(result) => {
                        // A successful page replaces that page's cached results, avoiding stale removed plugins.
                        if !more {
                            state.entries = vec![service::bundled_entry()];
                        }
                        service::merge(&mut state.entries, result.entries);
                        state.errors = result.errors;
                        state.page = page;
                        state.has_more = result.has_more;
                        state.fetched_at = result.fetched_at;
                        state.cached = false;
                    }
                    Err(error) => state.errors.push(format!("{error:#}")),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
