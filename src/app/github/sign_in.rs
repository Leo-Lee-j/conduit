//! Inline forge sign-in shown when no credential works. Saves a token, then asks the owner to reload.
use super::*;
use crate::github::auth::{self, AuthRequired};
use crate::github::forge::Forge;

pub(in crate::app) struct SignedIn;

/// Split a forge error into display text and any sign-in requirement it carries.
pub(in crate::app) fn describe(error: anyhow::Error) -> (String, Option<AuthRequired>) {
    (format!("{error:#}"), auth::required(&error))
}

fn token_url(required: &AuthRequired) -> String {
    match required.forge {
        Forge::Github => format!(
            "https://{}/settings/tokens/new?scopes=repo&description=Conduit",
            required.host
        ),
        Forge::Gitlab => format!(
            "https://{}/-/user_settings/personal_access_tokens?name=Conduit&scopes=api",
            required.host
        ),
    }
}

pub(in crate::app) struct ForgeSignIn {
    required: AuthRequired,
    input: Entity<InputState>,
    saved: bool,
    error: Option<String>,
    _input: Subscription,
}

impl gpui_kit::EventEmitter<SignedIn> for ForgeSignIn {}

impl ForgeSignIn {
    fn new(required: AuthRequired, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder(label("forge.token_placeholder"))
        });
        Self {
            saved: auth::has_saved(required.forge, &required.host),
            required,
            _input: cx.observe(&input, |_, _, cx| cx.notify()),
            input,
            error: None,
        }
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let token = self.input.read(cx).value().to_string();
        match auth::save(self.required.forge, &self.required.host, &token) {
            Ok(()) => cx.emit(SignedIn),
            Err(error) => self.error = Some(format!("{error:#}")),
        }
        cx.notify();
    }

    fn remove(&mut self, cx: &mut Context<Self>) {
        match auth::remove(self.required.forge, &self.required.host) {
            Ok(()) => cx.emit(SignedIn),
            Err(error) => self.error = Some(format!("{error:#}")),
        }
        cx.notify();
    }
}

impl Render for ForgeSignIn {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let scope = match self.required.forge {
            Forge::Github => "repo",
            Forge::Gitlab => "api",
        };
        let url = token_url(&self.required);
        v_flex()
            .debug_selector(|| "forge-sign-in".into())
            .p_2()
            .gap_2()
            .rounded_md()
            .border_1()
            .border_color(rgb(palette().border))
            .text_size(ui_text_size(12.))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(conduit::i18n::format(
                        "forge.sign_in_title",
                        &[
                            ("forge", self.required.forge.name().into()),
                            ("host", self.required.host.clone()),
                        ],
                    )),
            )
            .child(
                div()
                    .text_color(rgb(palette().muted))
                    .child(conduit::i18n::format(
                        "forge.sign_in_hint",
                        &[("scope", scope.into())],
                    )),
            )
            .child(
                Input::new(&self.input)
                    .mask_toggle()
                    .aria_label(label("forge.token_placeholder")),
            )
            .when_some(self.error.as_ref(), |body, error| {
                body.child(
                    div()
                        .text_color(rgb(palette().danger_text))
                        .child(error.clone()),
                )
            })
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        Button::new("forge-token-save")
                            .small()
                            .primary()
                            .label(label("forge.token_save"))
                            .disabled(self.input.read(cx).value().trim().is_empty())
                            .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
                    )
                    .child(
                        Button::new("forge-token-create")
                            .small()
                            .ghost()
                            .label(label("forge.token_create"))
                            .on_click(move |_, _, cx| cx.open_url(&url)),
                    )
                    .when(self.saved, |row| {
                        row.child(
                            Button::new("forge-token-remove")
                                .small()
                                .ghost()
                                .label(label("forge.token_remove"))
                                .on_click(cx.listener(|this, _, _, cx| this.remove(cx))),
                        )
                    }),
            )
    }
}

/// Owner-side state: which host needs sign-in and the lazily created form for it.
#[derive(Default)]
pub(in crate::app) struct SignInSlot {
    required: Option<AuthRequired>,
    view: Option<Entity<ForgeSignIn>>,
    _subscription: Option<Subscription>,
}

impl SignInSlot {
    pub(in crate::app) fn set(&mut self, required: Option<AuthRequired>) {
        if self.required != required {
            self.view = None;
            self._subscription = None;
        }
        self.required = required;
    }

    /// The form to show, created on first render; `reload` runs after a token is saved or removed.
    pub(in crate::app) fn view<T: 'static>(
        &mut self,
        window: &mut Window,
        cx: &mut Context<T>,
        reload: fn(&mut T, &mut Context<T>),
    ) -> Option<Entity<ForgeSignIn>> {
        let required = self.required.clone()?;
        if self.view.is_none() {
            let view = cx.new(|cx| ForgeSignIn::new(required, window, cx));
            self._subscription =
                Some(cx.subscribe(&view, move |this, _, _: &SignedIn, cx| reload(this, cx)));
            self.view = Some(view);
        }
        self.view.clone()
    }
}

#[cfg(test)]
mod tests {
    // Not a glob import: the app prelude's `test` attribute would shadow the std one.
    use super::{AuthRequired, Forge, token_url};

    #[test]
    fn token_pages_request_the_minimum_api_scope() {
        let github = AuthRequired {
            forge: Forge::Github,
            host: "github.com".into(),
            rejected: false,
        };
        assert_eq!(
            token_url(&github),
            "https://github.com/settings/tokens/new?scopes=repo&description=Conduit"
        );
        let gitlab = AuthRequired {
            forge: Forge::Gitlab,
            host: "gitlab.example.test:8443".into(),
            rejected: true,
        };
        assert!(token_url(&gitlab).starts_with(
            "https://gitlab.example.test:8443/-/user_settings/personal_access_tokens?"
        ));
    }
}
