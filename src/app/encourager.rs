use super::*;
use crate::{providers::claude::Role, store::EncouragerPersona};
use std::io::Read;

mod native_window;
mod pet;
use pet::{PetOutput, PetView, pet_window_bounds, position_from_bounds};

const MAX_RESPONSE_BYTES: u64 = 64 * 1024;
const MAX_CONTEXT_CHARS: usize = 2_000;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(25);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Scene {
    #[default]
    Stuck,
    Finished,
    Motivation,
    WrapUp,
}
impl Scene {
    const ALL: [Self; 4] = [Self::Stuck, Self::Finished, Self::Motivation, Self::WrapUp];
    fn label(self) -> &'static str {
        match self {
            Self::Stuck => "encourager.scene.stuck",
            Self::Finished => "encourager.scene.finished",
            Self::Motivation => "encourager.scene.motivation",
            Self::WrapUp => "encourager.scene.wrap_up",
        }
    }
}

#[derive(Default)]
pub(super) struct State {
    scene: Scene,
    reply: Option<String>,
    error: Option<String>,
    loading: bool,
    generation: u64,
    cancel: Option<Arc<AtomicBool>>,
    session: Option<u64>,
}

impl ConduitApp {
    pub(crate) fn restore_encourager_pet(&mut self, cx: &mut Context<Self>) {
        if self.store.ui.encourager.enabled && self.store.ui.encourager.pet_enabled {
            self.open_encourager_pet_window(cx);
            if !self.store.ui.encourager.pet_enabled {
                self.persist();
            }
        }
    }

    pub(super) fn set_encourager_pet_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if !self.writable || self.closing || self.store.ui.encourager.pet_enabled == enabled {
            return;
        }
        self.store.ui.encourager.pet_enabled = enabled;
        if enabled {
            if self.store.ui.encourager.enabled {
                self.open_encourager_pet_window(cx);
            }
        } else {
            self.close_encourager_pet_window(cx);
        }
        self.persist();
        cx.notify();
    }

    pub(super) fn close_encourager_pet_window(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(handle) = self.encourager_pet_window.take() else {
            return false;
        };
        let position = handle
            .update(cx, |_, window, _| position_from_bounds(window.bounds()))
            .ok();
        let changed = position.is_some_and(|position| {
            if self.store.ui.encourager.pet_position == Some(position) {
                false
            } else {
                self.store.ui.encourager.pet_position = Some(position);
                true
            }
        });
        let _ = handle.update(cx, |_, window, _| window.remove_window());
        changed
    }

    fn open_encourager_pet_window(&mut self, cx: &mut Context<Self>) {
        if self.encourager_pet_window.is_some()
            || !self.store.ui.encourager.enabled
            || !self.store.ui.encourager.pet_enabled
            || self.closing
        {
            return;
        }
        let position = self.store.ui.encourager.pet_position;
        let bounds = pet_window_bounds(position, cx);
        let parent = cx.entity();
        // Opening a GPUI window draws its first frame synchronously. Capture the
        // current app state before that happens so PetView never reads the host
        // entity while its update lease is still active.
        let output = PetOutput::from_app(self);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: None,
            kind: WindowKind::PopUp,
            focus: false,
            show: !cfg!(target_os = "windows"),
            is_movable: true,
            app_owns_titlebar_drag: true,
            is_resizable: false,
            is_minimizable: false,
            window_decorations: None,
            window_background: WindowBackgroundAppearance::Transparent,
            ..Default::default()
        };
        // The component Root paints an opaque theme background and window border.
        // A desktop character needs a bare GPUI root, including on its first frame.
        let mut native_result = Ok(());
        let result = cx
            .open_window(options, |window, cx| {
                native_result = native_window::prepare(window, bounds, cx);
                window.set_window_title(conduit::i18n::text_static("encourager.pet.title"));
                cx.new(|cx| PetView::new(parent, output, window, cx))
            })
            .and_then(|handle| {
                if let Err(error) = native_result {
                    let _ = handle.update(cx, |_, window, _| window.remove_window());
                    return Err(error);
                }
                Ok(handle)
            });
        match result {
            Ok(handle) => {
                self.encourager_pet_window = Some(handle.into());
                // Let native resize callbacks settle and release the host/view
                // leases before drawing at the corrected client dimensions.
                cx.defer(move |cx| {
                    let _ = handle.update(cx, |_, window, cx| {
                        window.defer(cx, |window, cx| {
                            window.draw(cx).clear(cx);
                            native_window::show(window);
                        });
                    });
                });
            }
            Err(error) => {
                self.store.ui.encourager.pet_enabled = false;
                self.message = Some(conduit::i18n::format(
                    "encourager.pet.open_failed",
                    &[("error", format!("{error:#}"))],
                ));
            }
        }
    }

    fn encourager_pet_window_closed(&mut self, position: crate::store::EncouragerPetPosition) {
        self.encourager_pet_window = None;
        self.store.ui.encourager.pet_enabled = false;
        self.store.ui.encourager.pet_position = Some(position);
        self.persist();
    }

    pub(super) fn render_encourager_view(
        &mut self,
        _key: &conduit::plugins::state::ViewInstanceKey,
        _window: &Window,
        cx: &mut Context<Self>,
    ) -> plugin_ui::right::RightView {
        let settings = &self.store.ui.encourager;
        let scene = self.encourager.scene;
        let persona = settings.persona;
        let busy = self.encourager.loading;
        let session = self.selected_session;
        let session_changed = self.encourager.session != session;
        if session_changed {
            self.encourager.cancel_request();
            self.encourager.reply = None;
            self.encourager.error = None;
            self.encourager.session = session;
        }
        let reply = self.encourager.reply.clone();
        let error = self.encourager.error.clone();
        let selected_service = settings.provider_id.as_ref();
        let service_name = selected_service
            .and_then(|id| {
                self.store.providers.iter().find(|provider| {
                    &provider.id == id && !encourager_model_ids(provider).is_empty()
                })
            })
            .map(|provider| provider.name.clone());

        let mut scene_buttons = h_flex().w_full().flex_wrap().gap_1();
        for option in Scene::ALL {
            let selected = option == scene;
            scene_buttons = scene_buttons.child(
                Button::new(format!("encourager-scene-{option:?}"))
                    .small()
                    .ghost()
                    .selected(selected)
                    .label(conduit::i18n::text_static(option.label()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.reset_encourager_output();
                        this.encourager.scene = option;
                        cx.notify();
                    })),
            );
        }

        let mut persona_buttons = h_flex().w_full().flex_wrap().gap_1();
        for option in EncouragerPersona::ALL {
            let selected = option == persona;
            persona_buttons = persona_buttons.child(
                Button::new(format!("encourager-persona-{option:?}"))
                    .small()
                    .ghost()
                    .selected(selected)
                    .label(conduit::i18n::text_static(option.label()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.writable || this.closing {
                            return;
                        }
                        this.reset_encourager_output();
                        this.store.ui.encourager.persona = option;
                        this.persist();
                        cx.notify();
                    })),
            );
        }

        let generate_view = cx.entity();
        let generate_label = if busy {
            conduit::i18n::text_static("encourager.generating")
        } else {
            conduit::i18n::text_static("encourager.generate")
        };
        let copy_view = cx.entity();
        let body = v_flex()
            .id("programmer-encourager-panel")
            .debug_selector(|| "programmer-encourager-panel".into())
            .w_full()
            .h_full()
            .min_h_0()
            .gap_3()
            .p_3()
            .overflow_y_scroll()
            .child(caption_line(conduit::i18n::text_static(
                "encourager.pick_scene",
            )))
            .child(scene_buttons)
            .child(caption_line(conduit::i18n::text_static(
                "encourager.pick_persona",
            )))
            .child(persona_buttons)
            .child(
                v_flex()
                    .id("encourager-reply")
                    .w_full()
                    .min_h(px(96.))
                    .gap_2()
                    .p_3()
                    .rounded_md()
                    .bg(rgb(palette().surface_hover))
                    .child(if let Some(reply) = reply {
                        div()
                            .text_size(ui_text_size(13.))
                            .line_height(relative(1.6))
                            .child(reply)
                    } else if let Some(error) = error {
                        div()
                            .text_size(ui_text_size(12.))
                            .text_color(rgb(palette().danger_text))
                            .child(error)
                    } else {
                        div()
                            .text_size(ui_text_size(12.))
                            .text_color(rgb(palette().muted))
                            .child(conduit::i18n::text_static("encourager.empty"))
                    }),
            )
            .child(
                Button::new("encourager-generate")
                    .debug_selector(|| "encourager-generate".into())
                    .w_full()
                    .label(generate_label)
                    .icon(FullIconName::Sparkles)
                    .disabled(busy || !self.writable || self.closing)
                    .on_click(move |_, _, cx| {
                        generate_view.update(cx, |this, cx| this.generate_encouragement(cx));
                    }),
            )
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .child(
                        Button::new("encourager-again")
                            .small()
                            .ghost()
                            .label(conduit::i18n::text_static("encourager.again"))
                            .disabled(busy || reply_is_empty(&self.encourager.reply))
                            .on_click(
                                cx.listener(|this, _, _, cx| this.generate_encouragement(cx)),
                            ),
                    )
                    .child(
                        Button::new("encourager-copy")
                            .small()
                            .ghost()
                            .label(conduit::i18n::text_static("encourager.copy"))
                            .disabled(reply_is_empty(&self.encourager.reply))
                            .on_click(move |_, _, cx| {
                                copy_view.update(cx, |this, cx| {
                                    if let Some(reply) = &this.encourager.reply {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            reply.clone(),
                                        ));
                                    }
                                });
                            }),
                    ),
            )
            .child(caption_line(conduit::i18n::text_static(
                if settings.include_context {
                    "encourager.context_on"
                } else {
                    "encourager.context_off"
                },
            )))
            .child(caption_line(match service_name {
                Some(name) => format!(
                    "{}: {name}",
                    conduit::i18n::text_static("encourager.service")
                ),
                None => {
                    conduit::i18n::text_static("encourager.choose_service_in_settings").to_string()
                }
            }));
        plugin_ui::right::RightView {
            body: plugin_ui::right::RightBody::Content(body.into_any_element()),
            actions: vec![],
        }
    }

    pub(super) fn deactivate_encourager(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.encourager.cancel_request();
        cx.notify();
    }

    pub(super) fn set_encourager_enabled(
        &mut self,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.writable || self.closing || self.store.ui.encourager.enabled == enabled {
            return;
        }
        if !enabled
            && self
                .plugins
                .right_is(conduit::plugins::builtins::ENCOURAGER_VIEW)
        {
            // The panel's deactivation hook cancels pending work before revocation.
            self.encourager.cancel_request();
            self.close_right_view(window, cx);
        }
        if !self.plugins.set_encourager_enabled(enabled) {
            return;
        }
        self.store.ui.encourager.enabled = enabled;
        if enabled {
            if self.store.ui.encourager.pet_enabled {
                self.open_encourager_pet_window(cx);
            }
        } else {
            self.close_encourager_pet_window(cx);
        }
        if !enabled {
            self.encourager.cancel_request();
            self.encourager.reply = None;
            self.encourager.error = None;
        }
        self.persist();
        cx.notify();
    }

    pub(super) fn reset_encourager_output(&mut self) {
        self.encourager.cancel_request();
        self.encourager.reply = None;
        self.encourager.error = None;
    }

    pub(super) fn generate_encouragement(&mut self, cx: &mut Context<Self>) {
        if !self.writable || self.closing || !self.store.ui.encourager.enabled {
            return;
        }
        self.reset_encourager_output();
        let settings = self.store.ui.encourager.clone();
        let Some(provider) = settings.provider_id.as_ref().and_then(|id| {
            self.store.providers.iter().find(|provider| {
                provider.id == *id && matches!(provider.engine, Provider::Claude | Provider::Codex)
            })
        }) else {
            self.encourager.error = Some(conduit::i18n::text("encourager.error.choose_service"));
            cx.notify();
            return;
        };
        let supported_models = encourager_model_ids(provider);
        let Some(model) = settings
            .model
            .as_ref()
            .filter(|model| supported_models.contains(model))
        else {
            self.encourager.error = Some(conduit::i18n::text("encourager.error.choose_model"));
            cx.notify();
            return;
        };
        let session_id = self.selected_session;
        let context = settings
            .include_context
            .then(|| {
                session_id.and_then(|id| self.session(id)).map(|session| {
                    let user = session
                        .messages
                        .iter()
                        .rev()
                        .find(|message| message.kind == ItemKind::User)
                        .map(|message| limit_chars(&message.text, MAX_CONTEXT_CHARS));
                    let assistant = session
                        .messages
                        .iter()
                        .rev()
                        .find(|message| message.kind == ItemKind::Assistant)
                        .map(|message| limit_chars(&message.text, MAX_CONTEXT_CHARS));
                    (user, assistant)
                })
            })
            .flatten();
        self.encourager.generation = self.encourager.generation.wrapping_add(1);
        let generation = self.encourager.generation;
        let cancel = Arc::new(AtomicBool::new(false));
        self.encourager.cancel = Some(cancel.clone());
        self.encourager.loading = true;
        self.encourager.reply = None;
        self.encourager.error = None;
        self.encourager.session = session_id;
        let scene = self.encourager.scene;
        let provider = provider.clone();
        let database = self.ccs.database_path.clone();
        let model = model.clone();
        let this_cancel = cancel.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    generate_text(
                        &provider,
                        &database,
                        &model,
                        settings,
                        scene,
                        context,
                        &this_cancel,
                    )
                })
                .await;
            let _ = this.update(cx, |app, cx| {
                if app.encourager.generation != generation
                    || !app.store.ui.encourager.enabled
                    || app.selected_session != session_id
                    || cancel.load(Ordering::Acquire)
                {
                    return;
                }
                app.encourager.loading = false;
                app.encourager.cancel = None;
                match result {
                    Ok(reply) => app.encourager.reply = Some(reply),
                    Err(error) => app.encourager.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

impl State {
    pub(super) fn cancel_request(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        if let Some(cancel) = self.cancel.take() {
            cancel.store(true, Ordering::Release);
        }
        self.loading = false;
    }
}

fn caption_line(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(ui_text_size(11.))
        .text_color(rgb(palette().muted))
        .child(text.into())
}
fn reply_is_empty(reply: &Option<String>) -> bool {
    reply.as_ref().is_none_or(String::is_empty)
}
fn limit_chars(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

pub(super) fn encourager_model_ids(provider: &ModelProvider) -> Vec<String> {
    if !matches!(provider.engine, Provider::Claude | Provider::Codex) {
        return Vec::new();
    }

    let mut models = Vec::new();
    let mut push_model = |model: &str| {
        let model = if provider.engine == Provider::Claude {
            crate::providers::claude::split_one_m(model).0
        } else {
            model
        };
        if !model.is_empty() && !models.iter().any(|existing| existing == model) {
            models.push(model.to_owned());
        }
    };
    for model in &provider.models {
        push_model(model);
    }
    if provider.engine == Provider::Claude
        && let Some(claude) = &provider.claude_models
    {
        push_model(&claude.default_model);
        for (role, mapping) in &claude.roles {
            if *role != Role::Subagent {
                push_model(&mapping.model);
            }
        }
    }
    models
}

fn generate_text(
    provider: &ModelProvider,
    database: &Path,
    model: &str,
    settings: crate::store::EncouragerSettings,
    scene: Scene,
    context: Option<(Option<String>, Option<String>)>,
    cancel: &AtomicBool,
) -> Result<String, String> {
    check_cancel(cancel)?;
    let service = api_service(provider, database)?;
    let instruction = format!(
        "{}\n{}\n{}",
        conduit::i18n::text("encourager.prompt.base"),
        conduit::i18n::text(scene.label()),
        conduit::i18n::text(settings.persona.label())
    );
    let mut prompt = format!(
        "{}\n{}",
        conduit::i18n::text("encourager.prompt.scene"),
        conduit::i18n::text(scene.label())
    );
    if let Some((user, assistant)) = context {
        if let Some(user) = user.filter(|text| !text.trim().is_empty()) {
            prompt.push_str(&format!(
                "\n\n{}\n{}",
                conduit::i18n::text("encourager.prompt.recent_user"),
                user
            ));
        }
        if let Some(assistant) = assistant.filter(|text| !text.trim().is_empty()) {
            prompt.push_str(&format!(
                "\n{}\n{}",
                conduit::i18n::text("encourager.prompt.recent_assistant"),
                assistant
            ));
        }
    }
    prompt.push_str(&format!(
        "\n\n{}",
        conduit::i18n::text("encourager.prompt.finish")
    ));
    let max_chars = settings.verbosity.max_chars();
    let max_tokens = (max_chars * 2).clamp(96, 512);
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .max_redirects(0)
        .timeout_global(Some(REQUEST_TIMEOUT))
        .build()
        .into();
    let mut request = agent.post(service.endpoint.as_str());
    for (name, value) in &service.headers {
        request = request.header(name, value);
    }
    let payload = match provider.engine {
        Provider::Claude => {
            serde_json::json!({
                "model": model,
                "max_tokens": max_tokens,
                "system": instruction,
                "messages": [{"role": "user", "content": prompt}]
            })
        }
        Provider::Codex => {
            serde_json::json!({
                "model": model,
                "instructions": instruction,
                "input": prompt,
                "max_output_tokens": max_tokens
            })
        }
        _ => return Err(conduit::i18n::text("encourager.error.unsupported_service")),
    };
    check_cancel(cancel)?;
    let mut response = request.send_json(payload).map_err(|error| match error {
        ureq::Error::StatusCode(code) => conduit::i18n::format(
            "encourager.error.http_status",
            &[("status", code.to_string())],
        ),
        _ => conduit::i18n::text("encourager.error.request_failed"),
    })?;
    if response.status().as_u16() != 200 {
        return Err(conduit::i18n::format(
            "encourager.error.http_status",
            &[("status", response.status().as_u16().to_string())],
        ));
    }
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(MAX_RESPONSE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| conduit::i18n::text("encourager.error.invalid_response"))?;
    check_cancel(cancel)?;
    if bytes.len() as u64 > MAX_RESPONSE_BYTES {
        return Err(conduit::i18n::text("encourager.error.response_too_large"));
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|_| conduit::i18n::text("encourager.error.invalid_response"))?;
    let text = match provider.engine {
        Provider::Claude => value["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|item| item["type"] == "text")
            .filter_map(|item| item["text"].as_str())
            .collect::<String>(),
        Provider::Codex => {
            if let Some(text) = value["output_text"].as_str() {
                text.to_owned()
            } else {
                value["output"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .flat_map(|item| item["content"].as_array().into_iter().flatten())
                    .filter(|item| item["type"] == "output_text" || item["type"] == "text")
                    .filter_map(|item| item["text"].as_str())
                    .collect::<String>()
            }
        }
        _ => String::new(),
    };
    let reply = text.trim();
    if reply.is_empty() {
        return Err(conduit::i18n::text("encourager.error.empty_response"));
    }
    Ok(limit_chars(reply, max_chars))
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err(conduit::i18n::text("encourager.error.cancelled"))
    } else {
        Ok(())
    }
}

// Resolve credentials at request time, including imported cc-switch services.
// The imported catalog intentionally contains no credentials.
struct ApiService {
    endpoint: url::Url,
    headers: std::collections::BTreeMap<String, String>,
}

fn api_service(provider: &ModelProvider, database: &Path) -> Result<ApiService, String> {
    let route = provider
        .runtime_route(database)
        .map_err(|_| conduit::i18n::text("encourager.error.credentials"))?;
    let mut resolved = provider.clone();
    let mut headers = std::collections::BTreeMap::new();
    let config = route
        .config
        .get("model_providers")
        .and_then(|providers| providers.get("conduit"))
        .unwrap_or(&serde_json::Value::Null);
    match provider.engine {
        Provider::Claude => {
            resolved.base_url = route
                .env
                .get("ANTHROPIC_BASE_URL")
                .cloned()
                .unwrap_or_default();
            headers.insert("anthropic-version".into(), "2023-06-01".into());
            if let Some(key) = route
                .env
                .get("ANTHROPIC_AUTH_TOKEN")
                .filter(|key| !key.is_empty())
                .or_else(|| {
                    route
                        .env
                        .get("ANTHROPIC_API_KEY")
                        .filter(|key| !key.is_empty())
                })
            {
                headers.insert("x-api-key".into(), key.clone());
            }
        }
        Provider::Codex => {
            if config["wire_api"]
                .as_str()
                .is_some_and(|api| api != "responses")
            {
                return Err(conduit::i18n::text("encourager.error.unsupported_service"));
            }
            resolved.base_url = config["base_url"].as_str().unwrap_or_default().into();
            if let Some(key) = config["env_key"]
                .as_str()
                .and_then(|name| route.env.get(name))
                .filter(|key| !key.is_empty())
            {
                headers.insert("authorization".into(), format!("Bearer {key}"));
            }
            for (name, value) in config["http_headers"].as_object().into_iter().flatten() {
                if let Some(value) = value.as_str() {
                    headers.insert(name.to_ascii_lowercase(), value.into());
                }
            }
            for (name, variable) in config["env_http_headers"].as_object().into_iter().flatten() {
                if let Some(variable) = variable.as_str() {
                    let value = std::env::var(variable)
                        .map_err(|_| conduit::i18n::text("encourager.error.credentials"))?;
                    headers.insert(name.to_ascii_lowercase(), value);
                }
            }
        }
        _ => return Err(conduit::i18n::text("encourager.error.unsupported_service")),
    }
    let mut endpoint = request_url(&resolved)?;
    if provider.engine == Provider::Codex {
        for (name, value) in config["query_params"].as_object().into_iter().flatten() {
            if let Some(value) = value.as_str() {
                endpoint.query_pairs_mut().append_pair(name, value);
            }
        }
    }
    Ok(ApiService { endpoint, headers })
}

fn request_url(provider: &ModelProvider) -> Result<url::Url, String> {
    crate::providers::discovery::endpoint(&provider.base_url)
        .map_err(|_| conduit::i18n::text("encourager.error.invalid_service"))?;
    let base = crate::providers::discovery::runtime_base_url(&provider.base_url, provider.engine);
    let mut url = url::Url::parse(&base)
        .map_err(|_| conduit::i18n::text("encourager.error.invalid_service"))?;
    let append_v1 =
        provider.engine == Provider::Claude && !url.path().trim_end_matches('/').ends_with("/v1");
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| conduit::i18n::text("encourager.error.invalid_service"))?;
        segments.pop_if_empty();
        match provider.engine {
            Provider::Claude => {
                if append_v1 {
                    segments.push("v1");
                }
                segments.push("messages");
            }
            Provider::Codex => {
                segments.push("responses");
            }
            _ => return Err(conduit::i18n::text("encourager.error.unsupported_service")),
        }
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::{api_service, encourager_model_ids, limit_chars, request_url};
    use crate::store::EncouragerVerbosity;
    use crate::{
        domain::Provider,
        providers::{
            ModelProvider, ProviderSource,
            claude::{ClaudeModels, Mapping, Role},
        },
    };

    #[test]
    fn response_length_is_bounded_by_selected_verbosity() {
        let text = "鼓励".repeat(250);
        assert_eq!(
            limit_chars(&text, EncouragerVerbosity::Short.max_chars())
                .chars()
                .count(),
            40
        );
        assert_eq!(
            limit_chars(&text, EncouragerVerbosity::Normal.max_chars())
                .chars()
                .count(),
            100
        );
        assert_eq!(
            limit_chars(&text, EncouragerVerbosity::Chatty.max_chars())
                .chars()
                .count(),
            180
        );
    }

    #[test]
    fn api_routes_are_selected_from_validated_service_urls() {
        let provider = |engine, base_url: &str| ModelProvider {
            id: "test".into(),
            name: "test".into(),
            engine,
            base_url: base_url.into(),
            api_key_env: String::new(),
            protected_api_key: None,
            models: vec!["m".into()],
            source: ProviderSource::Local,
            model_metadata: vec![],
            claude_models: None,
        };
        assert_eq!(
            request_url(&provider(Provider::Claude, "https://api.example/v1"))
                .unwrap()
                .as_str(),
            "https://api.example/v1/messages"
        );
        assert_eq!(
            request_url(&provider(Provider::Codex, "https://api.example"))
                .unwrap()
                .as_str(),
            "https://api.example/v1/responses"
        );
        assert!(request_url(&provider(Provider::Codex, "file:///tmp")).is_err());
    }

    // Imported catalog entries have no keys. Requests must use the selected
    // database record, including credentials changed after the catalog loaded.
    #[test]
    fn imported_pet_services_resolve_current_endpoints_and_credentials() {
        let path = std::env::temp_dir().join(format!(
            "conduit-pet-services-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let database = rusqlite::Connection::open(&path).unwrap();
        database
            .execute_batch("CREATE TABLE providers (id TEXT, app_type TEXT, settings_config TEXT);")
            .unwrap();
        for (engine, app, settings, expected_url, header, expected_key) in [
            (
                Provider::Claude,
                "claude",
                serde_json::json!({"env": {
                    "ANTHROPIC_BASE_URL": "https://claude.example/v1",
                    "ANTHROPIC_AUTH_TOKEN": "test-claude-key"
                }}),
                "https://claude.example/v1/messages",
                "x-api-key",
                "test-claude-key",
            ),
            (
                Provider::Codex,
                "codex",
                serde_json::json!({
                    "auth": {"OPENAI_API_KEY": "test-codex-key"},
                    "config": "model_provider = \"proxy\"\n[model_providers.proxy]\nbase_url = \"https://codex.example/v1\"\nwire_api = \"responses\"\n[model_providers.proxy.http_headers]\nx-test = \"pet\"\n[model_providers.proxy.query_params]\nversion = \"test\"\n"
                }),
                "https://codex.example/v1/responses?version=test",
                "authorization",
                "Bearer test-codex-key",
            ),
        ] {
            database
                .execute(
                    "INSERT INTO providers VALUES (?1, ?2, ?3)",
                    rusqlite::params!["pet", app, settings.to_string()],
                )
                .unwrap();
            let provider = ModelProvider {
                id: ModelProvider::ccs_id(engine, "pet"),
                name: "Imported API".into(),
                engine,
                base_url: "https://stale.example/v1".into(),
                api_key_env: String::new(),
                protected_api_key: None,
                models: vec!["existing-model".into()],
                source: ProviderSource::Ccs { id: "pet".into() },
                model_metadata: vec![],
                claude_models: None,
            };
            assert_eq!(encourager_model_ids(&provider), ["existing-model"]);
            let service = api_service(&provider, &path).unwrap();
            assert_eq!(service.endpoint.as_str(), expected_url);
            assert_eq!(service.headers[header], expected_key);
            if engine == Provider::Codex {
                assert_eq!(service.headers["x-test"], "pet");
                let mut changed = settings.clone();
                changed["auth"]["OPENAI_API_KEY"] = "updated-test-key".into();
                database
                    .execute(
                        "UPDATE providers SET settings_config = ?1 WHERE app_type = 'codex'",
                        [changed.to_string()],
                    )
                    .unwrap();
                assert_eq!(
                    api_service(&provider, &path).unwrap().headers["authorization"],
                    "Bearer updated-test-key"
                );
                changed["config"] = changed["config"]
                    .as_str()
                    .unwrap()
                    .replace("responses", "chat")
                    .into();
                database
                    .execute(
                        "UPDATE providers SET settings_config = ?1 WHERE app_type = 'codex'",
                        [changed.to_string()],
                    )
                    .unwrap();
                assert!(api_service(&provider, &path).is_err());
            }
            assert!(provider.protected_api_key.is_none());
        }
        drop(database);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn claude_role_models_remain_available_for_direct_api_calls() {
        let provider = ModelProvider {
            id: "claude-roles".into(),
            name: "Claude roles".into(),
            engine: Provider::Claude,
            base_url: "https://api.example/v1".into(),
            api_key_env: String::new(),
            protected_api_key: None,
            models: vec![],
            source: ProviderSource::Local,
            model_metadata: vec![],
            claude_models: Some(ClaudeModels {
                default_model: String::new(),
                roles: std::collections::BTreeMap::from([
                    (
                        Role::Sonnet,
                        Mapping {
                            model: "vendor/sonnet[1M]".into(),
                            display_name: "Daily".into(),
                        },
                    ),
                    (
                        Role::Subagent,
                        Mapping {
                            model: "internal/subagent".into(),
                            display_name: String::new(),
                        },
                    ),
                ]),
            }),
        };
        assert_eq!(encourager_model_ids(&provider), ["vendor/sonnet"]);
    }
}
