//! One logical focus for the virtual tree; rows do not become thousands of Tab stops.
use super::*;
use gpui_kit::component::menu::PopupMenu;

impl ConduitApp {
    pub(super) fn sidebar_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sidebar.menu.is_some() {
            return;
        }
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;
        if modifiers.control || modifiers.alt || modifiers.platform {
            return;
        }
        self.refresh_sidebar();
        let rows: Vec<_> = self
            .sidebar
            .rows
            .iter()
            .filter(|row| {
                matches!(
                    row,
                    cache::Row::Workspace { .. }
                        | cache::Row::Session { .. }
                        | cache::Row::More { .. }
                )
            })
            .cloned()
            .collect();
        if rows.is_empty() {
            return;
        }
        let current = self.sidebar.keyboard_row.as_ref().and_then(|row| rows.iter().position(|candidate| candidate.same_identity(row)))
            .or_else(|| rows.iter().position(|row| matches!(row, cache::Row::Session { id, .. } if Some(*id) == self.selected_session))).unwrap_or(0);
        let next = match key {
            "up" => current.saturating_sub(1),
            "down" => (current + 1).min(rows.len() - 1),
            "home" => 0,
            "end" => rows.len() - 1,
            "enter" | "left" | "right" | "f10" | "menu" => current,
            _ => return,
        };
        if key == "f10" && !modifiers.shift {
            return;
        }
        cx.stop_propagation();
        let row = rows[next].clone();
        self.sidebar.keyboard_row = Some(row.clone());
        match &row {
            cache::Row::Session { id, path } => {
                if key == "enter" {
                    self.switch_sidebar_session(*id, window, cx);
                }
                if key == "left" {
                    self.sidebar.keyboard_row = self
                        .sidebar
                        .rows
                        .iter()
                        .find(
                            |row| matches!(row, cache::Row::Workspace { path: p, .. } if p == path),
                        )
                        .cloned();
                }
                if key == "menu" || key == "f10" {
                    let app = cx.entity();
                    let id = *id;
                    let focus = self.sidebar.focus.as_ref().unwrap().clone();
                    let menu = PopupMenu::build(window, cx, |menu, _, _| {
                        session_menu(menu, app, id, self).action_context(focus)
                    });
                    self.sidebar.menu_subscription = Some(cx.subscribe_in(
                        &menu,
                        window,
                        |this, _, _: &gpui_kit::DismissEvent, window, cx| {
                            this.sidebar.menu = None;
                            if let Some(focus) = &this.sidebar.focus {
                                focus.focus(window, cx);
                            }
                            this.sidebar_view.update(cx, |_, cx| cx.notify());
                            cx.notify();
                        },
                    ));
                    menu.focus_handle(cx).focus(window, cx);
                    self.sidebar.menu = Some(menu);
                }
            }
            cache::Row::Workspace { path, last, .. } => {
                if key == "left" {
                    self.collapsed_workspaces.insert(path.clone());
                }
                if key == "right" {
                    self.collapsed_workspaces.remove(path);
                }
                if key == "enter"
                    && let Some(index) = self.sidebar.workspace_index(path)
                {
                    self.switch(index, *last, window, cx);
                }
            }
            cache::Row::More { path, .. } if key == "enter" || key == "right" => {
                if let Some(index) = self.sidebar.workspace_index(path)
                    && !self.expanded_workspaces.insert(index)
                {
                    self.expanded_workspaces.remove(&index);
                }
            }
            _ => {}
        }
        if self.sidebar.menu.is_none() {
            if let Some(focus) = &self.sidebar.focus {
                focus.focus(window, cx);
            }
            self.refresh_sidebar();
            if let Some(index) = self.sidebar.keyboard_row.as_ref().and_then(|row| {
                self.sidebar
                    .rows
                    .iter()
                    .position(|candidate| candidate.same_identity(row))
            }) {
                self.sidebar
                    .scroll
                    .scroll_to_item(index, gpui_kit::ScrollStrategy::Top);
            }
        }
        self.sidebar_view.update(cx, |_, cx| cx.notify());
        cx.notify();
    }
}
