//! Fixed content groups. Tab order remains owned by ConduitApp::open_tabs.
use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::app) enum Pane {
    #[default]
    Main,
    Reference,
}
impl Pane {
    pub(super) fn key(self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::Reference => "reference",
        }
    }
}

#[derive(Default)]
pub(super) struct State {
    pub(super) files: HashSet<u64>,
    pub(super) active: Option<u64>,
    pub(super) preview: Option<u64>,
    pub(super) tab_scroll: ScrollHandle,
    pub(super) tab_layout: TabLayout,
    pub(super) split: bool,
    pub(super) last: Pane,
    pub(super) width: f32,
    visible: HashSet<u64>,
    pub(super) return_main: Option<DocumentTab>,
    pub(super) overlay: Option<(Option<DocumentTab>, bool, Pane)>,
    // Container width is part of the state identity: minimum-width clamping
    // during a narrow layout must not replace the user's preferred ratio.
    resize: Option<(f32, Entity<ResizableState>)>,
}

impl ConduitApp {
    pub(in crate::app) fn content_overlay_open(&self) -> bool {
        self.settings_ui.open || self.usage_ui.active.is_some() || self.github_ui.active.is_some()
    }

    pub(in crate::app) fn remember_content_overlay(&mut self) {
        if self.files.panes.overlay.is_none() {
            self.files.panes.overlay = Some((
                self.active_document(),
                self.files.panes.split,
                self.files.panes.last,
            ));
        }
    }

    pub(in crate::app) fn leave_content_overlay(&mut self) {
        if let Some((_, split, last)) = self.files.panes.overlay.take() {
            self.files.panes.split = split && self.files.panes.active.is_some();
            self.files.panes.last = last;
        }
    }

    pub(in crate::app) fn content_overlay_return(&self) -> Option<DocumentTab> {
        self.files
            .panes
            .overlay
            .and_then(|(tab, _, _)| tab)
            .filter(|tab| self.open_tabs.contains(tab) && self.document_pane(*tab) == Pane::Main)
    }

    pub(in crate::app) fn document_pane(&self, tab: DocumentTab) -> Pane {
        if matches!(tab, DocumentTab::File(id) if self.files.panes.files.contains(&id)) {
            Pane::Reference
        } else {
            Pane::Main
        }
    }

    pub(in crate::app) fn pane_document(&self, pane: Pane) -> Option<DocumentTab> {
        match pane {
            Pane::Main => self.active_document(),
            Pane::Reference => self.files.panes.active.map(DocumentTab::File),
        }
    }

    pub(in crate::app) fn pane_file(&self, pane: Pane) -> Option<u64> {
        match pane {
            Pane::Main => self.active_file,
            Pane::Reference => self.files.panes.active,
        }
    }

    pub(super) fn pane_preview(&self, pane: Pane) -> Option<u64> {
        match pane {
            Pane::Main => self.files.preview,
            Pane::Reference => self.files.panes.preview,
        }
    }

    pub(super) fn set_pane_preview(&mut self, pane: Pane, id: Option<u64>) {
        match pane {
            Pane::Main => self.files.preview = id,
            Pane::Reference => self.files.panes.preview = id,
        }
    }

    pub(super) fn pin_file(&mut self, id: u64) {
        let pane = self.document_pane(DocumentTab::File(id));
        if self.pane_preview(pane) == Some(id) {
            self.set_pane_preview(pane, None);
        }
    }

    pub(super) fn pane_tab_scroll(&self, pane: Pane) -> &ScrollHandle {
        match pane {
            Pane::Main => &self.files.tab_scroll,
            Pane::Reference => &self.files.panes.tab_scroll,
        }
    }

    pub(super) fn pane_tab_layout(&self, pane: Pane) -> &TabLayout {
        match pane {
            Pane::Main => &self.files.tab_layout,
            Pane::Reference => &self.files.panes.tab_layout,
        }
    }

    pub(in crate::app) fn main_pane_visible(&self) -> bool {
        !self.files.panes.split
            || self.files.panes.active.is_none()
            || self.files.panes.width >= 641.
            || self.files.panes.last == Pane::Main
    }

    pub(super) fn reference_pane_visible(&self) -> bool {
        !self.content_overlay_open()
            && self.files.panes.split
            && self.files.panes.active.is_some()
            && (self.files.panes.width >= 641. || self.files.panes.last == Pane::Reference)
    }

    pub(in crate::app) fn visible_file_documents(&self) -> Vec<u64> {
        if self.content_overlay_open() {
            return vec![];
        }
        let mut ids = Vec::with_capacity(2);
        if self.main_pane_visible() {
            ids.extend(self.active_file);
        }
        if self.reference_pane_visible() {
            ids.extend(self.files.panes.active);
        }
        ids
    }

    pub(in crate::app) fn focus_main_pane(&mut self) {
        self.files.panes.last = Pane::Main;
    }

    pub(super) fn repair_reference_pane(&mut self) {
        self.files
            .panes
            .files
            .retain(|id| self.files.documents.contains_key(id));
        if !self
            .files
            .panes
            .active
            .is_some_and(|id| self.files.panes.files.contains(&id))
        {
            self.files.panes.active = self.open_tabs.iter().rev().find_map(|tab| match tab {
                DocumentTab::File(id) if self.files.panes.files.contains(id) => Some(*id),
                _ => None,
            });
        }
        if !self
            .files
            .panes
            .preview
            .is_some_and(|id| self.files.panes.files.contains(&id))
        {
            self.files.panes.preview = None;
        }
        if self.files.panes.active.is_none() {
            self.files.panes.split = false;
            self.files.panes.last = Pane::Main;
        }
        if self
            .files
            .panes
            .return_main
            .is_some_and(|tab| !self.open_tabs.contains(&tab))
        {
            self.files.panes.return_main = None;
        }
    }

    pub(super) fn select_reference_file(
        &mut self,
        id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.files.panes.files.contains(&id) {
            return;
        }
        self.files.panes.active = Some(id);
        self.files.panes.split = true;
        self.files.panes.last = Pane::Reference;
        self.files.documents[&id]
            .preview
            .read(cx)
            .focus_handle(cx)
            .focus(window, cx);
        if !self.files.documents[&id].loading {
            self.load_file(id, cx);
        }
        cx.notify();
    }

    pub(in crate::app) fn move_file_to_pane(
        &mut self,
        id: u64,
        target: Pane,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.files.documents.contains_key(&id) {
            return;
        }
        let tab = DocumentTab::File(id);
        let origin = self.document_pane(tab);
        if target == Pane::Main {
            if self.active_document() != Some(tab) && !self.content_overlay_open() {
                self.files.panes.return_main = self.active_document();
            }
            self.pin_file(id);
            self.files.panes.files.remove(&id);
            self.repair_reference_pane();
            self.select_document(tab, window, cx);
            self.files.panes.split = false;
            self.files.panes.last = Pane::Main;
        } else {
            let restore = (origin == Pane::Main && self.active_file == Some(id))
                .then(|| {
                    self.files
                        .panes
                        .return_main
                        .filter(|previous| {
                            *previous != tab
                                && self.open_tabs.contains(previous)
                                && self.document_pane(*previous) == Pane::Main
                        })
                        .or_else(|| {
                            self.open_tabs.iter().rev().copied().find(|previous| {
                                *previous != tab
                                    && self.document_pane(*previous) == Pane::Main
                                    && !matches!(
                                        previous,
                                        DocumentTab::Settings
                                            | DocumentTab::Usage(_)
                                            | DocumentTab::PullRequest(_)
                                    )
                            })
                        })
                        .or_else(|| self.selected_session.map(DocumentTab::Chat))
                })
                .flatten();
            if origin == Pane::Main && self.active_file == Some(id) && restore.is_none() {
                return;
            }
            self.pin_file(id);
            self.files.panes.files.insert(id);
            if let Some(previous) = restore {
                self.active_file = None;
                self.select_document(previous, window, cx);
                self.files.panes.return_main = None;
            }
            self.select_document(tab, window, cx);
        }
        cx.notify();
    }

    pub(in crate::app) fn refresh_visible_files(&mut self, cx: &mut Context<Self>) {
        for id in self.visible_file_documents() {
            self.load_file(id, cx);
        }
    }

    pub(in crate::app) fn update_content_pane_layout(
        &mut self,
        width: f32,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        self.files.panes.width = width;
        if !self.content_overlay_open() {
            for pane in [Pane::Main, Pane::Reference] {
                if let Some(id) = self.pane_file(pane)
                    && let Some(document) = self.files.documents.get(&id)
                    && document
                        .preview
                        .read(cx)
                        .focus_handle(cx)
                        .contains_focused(window, cx)
                {
                    self.files.panes.last = pane;
                }
            }
            if self
                .input
                .read(cx)
                .focus_handle(cx)
                .contains_focused(window, cx)
                || self.chat_focus.contains_focused(window, cx)
            {
                self.files.panes.last = Pane::Main;
            }
        }
        let visible: HashSet<_> = self.visible_file_documents().into_iter().collect();
        for id in visible
            .difference(&self.files.panes.visible)
            .copied()
            .collect::<Vec<_>>()
        {
            if self
                .files
                .documents
                .get(&id)
                .is_some_and(|document| !document.loading)
            {
                self.load_file(id, cx);
            }
        }
        for (id, document) in &mut self.files.documents {
            if !visible.contains(id) {
                super::proposals::invalidate(document, cx);
            }
            document.preview.update(cx, |preview, cx| {
                preview.set_review_visibility(visible.contains(id), cx)
            });
        }
        self.files.panes.visible = visible;
        self.sync_file_watch();
    }

    pub(in crate::app) fn focus_main_content(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.active_file
            && let Some(document) = self.files.documents.get(&id)
        {
            document.preview.read(cx).focus_handle(cx).focus(window, cx);
        } else if let Some(id) = self.active_diff
            && let Some(document) = self.files.diffs.get(&id)
        {
            document
                .view
                .update(cx, |view, cx| view.focus_diff(window, cx));
        } else if self.viewing_subagent() {
            self.focus_subagent_page(window, cx);
        } else {
            self.input.read(cx).focus_handle(cx).focus(window, cx);
        }
    }

    pub(super) fn show_content_pane(
        &mut self,
        pane: Pane,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.files.panes.split = self.files.panes.active.is_some();
        self.files.panes.last = pane;
        if let Some(id) = self.pane_file(pane)
            && let Some(document) = self.files.documents.get(&id)
        {
            document.preview.read(cx).focus_handle(cx).focus(window, cx);
            self.load_file(id, cx);
        } else {
            self.focus_main_content(window, cx);
        }
        cx.notify();
    }

    fn remember_content_split_ratio(
        &mut self,
        state: &Entity<ResizableState>,
        cx: &mut Context<Self>,
    ) {
        if self.content_overlay_open()
            || !self.files.panes.split
            || self.files.panes.active.is_none()
            || self.files.panes.width < 641.
            || self
                .files
                .panes
                .resize
                .as_ref()
                .is_none_or(|(width, current)| *width != self.files.panes.width || current != state)
        {
            return;
        }
        let [main, reference] = state.read(cx).sizes().as_slice() else {
            return;
        };
        let (main, reference) = (f32::from(*main), f32::from(*reference));
        let total = main + reference;
        if !main.is_finite()
            || !reference.is_finite()
            || !total.is_finite()
            || main < 320.
            || reference < 320.
        {
            return;
        }
        let ratio = main / total;
        if self.store.ui.content_split_ratio != Some(ratio) {
            self.store.ui.content_split_ratio = Some(ratio);
            self.dirty = true;
        }
    }

    pub(in crate::app) fn render_content_panes(
        &mut self,
        main: AnyElement,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.content_overlay_open() {
            return main;
        }
        let has_reference = self.files.panes.active.is_some();
        let main_visible = self.main_pane_visible();
        let reference_visible = self.reference_pane_visible();
        let main = div()
            .id("content-main")
            .debug_selector(|| "content-main".into())
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.files.panes.last = Pane::Main),
            )
            .child(main)
            .into_any_element();
        let reference = reference_visible.then(|| {
            v_flex()
                .id("content-reference")
                .debug_selector(|| "content-reference".into())
                .size_full()
                .min_w_0()
                .min_h_0()
                .overflow_hidden()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, _| this.files.panes.last = Pane::Reference),
                )
                .child(self.document_tab_bar_for(Pane::Reference, cx))
                .child(
                    self.reference_file_view
                        .clone()
                        .cached(StyleRefinement::default().w_full().flex_1().min_h_0()),
                )
                .into_any_element()
        });
        let body = if let Some(reference) = reference {
            if main_visible {
                let width = self.files.panes.width;
                let state = match &self.files.panes.resize {
                    Some((previous_width, state)) if *previous_width == width => state.clone(),
                    _ => {
                        let state = cx.new(|_| ResizableState::default());
                        self.files.panes.resize = Some((width, state.clone()));
                        state
                    }
                };
                let main_width =
                    (width * self.store.ui.content_split_ratio()).clamp(320., width - 320.);
                let app = cx.entity();
                h_resizable("content-split")
                    .with_state(&state)
                    .child(
                        resizable_panel()
                            .size(px(main_width))
                            .size_range(px(320.)..px(10000.))
                            .child(main),
                    )
                    .child(
                        resizable_panel()
                            .size(px(width - main_width))
                            .size_range(px(320.)..px(10000.))
                            .child(reference),
                    )
                    .on_resize(move |state, _, cx| {
                        app.update(cx, |app, cx| app.remember_content_split_ratio(state, cx));
                    })
                    .into_any_element()
            } else {
                reference
            }
        } else {
            main
        };
        let mut area = v_flex().size_full().min_w_0().min_h_0();
        if has_reference && (!self.files.panes.split || self.files.panes.width < 641.) {
            area = area.child(
                h_flex()
                    .gap_1()
                    .px_2()
                    .h(px(30.))
                    .flex_shrink_0()
                    .items_center()
                    .child(
                        Button::new("show-main-pane")
                            .debug_selector(|| "show-main-pane".into())
                            .ghost()
                            .small()
                            .compact()
                            .label(conduit::i18n::text_static("files.main_pane"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.show_content_pane(Pane::Main, window, cx)
                            })),
                    )
                    .child(
                        Button::new("show-reference-pane")
                            .debug_selector(|| "show-reference-pane".into())
                            .ghost()
                            .small()
                            .compact()
                            .label(conduit::i18n::text_static("files.reference_pane"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.show_content_pane(Pane::Reference, window, cx)
                            })),
                    ),
            );
        }
        area.child(div().flex_1().min_h_0().min_w_0().child(body))
            .into_any_element()
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests;
