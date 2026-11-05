//! Bounded status data and width allocation in the space left by host controls.
use super::*;
use conduit::plugins::{
    commands::CommandInvocation,
    context::ScopeKey,
    contributions::{Alignment, StatusItem, StatusValue},
    ids::ItemId,
    registry::Registration,
};

const GAP: f32 = 4.;
const MORE_WIDTH: f32 = 28.;
const ITEM_MAX_WIDTH: f32 = 180.;

#[cfg(all(test, feature = "ui-tests"))]
mod integration_tests;

pub(super) struct StatusProvider {
    pub registration: Registration,
    value: fn(&ConduitApp, &ScopeKey) -> StatusValue,
    element_id: Option<&'static str>,
}

struct StatusEntry {
    item: StatusItem,
    value: StatusValue,
    tooltip: String,
    request: Option<CommandInvocation>,
    element_id: String,
    width: f32,
}

pub(super) fn bind_core(ui: &mut PluginUi) {
    ui.bind_status(
        builtins::WORKSPACE_STATUS,
        Some("toolbar-workspace"),
        |app, scope| {
            let ScopeKey::Workspace(key) = scope else {
                unreachable!("workspace status scope")
            };
            let workspace = app.store.workspaces.iter().find(|w| w.path == key.path());
            StatusValue {
                text: workspace
                    .map(Workspace::name)
                    .unwrap_or_else(|| "工作区".into()),
                tooltip: Some(display_path(key.path())),
            }
        },
    );
}

// Native data can contain arbitrary paths/titles. Apply the same byte bounds as
// declarations without splitting a UTF-8 character or admitting multiline text.
pub(super) fn bounded_text(text: &str, limit: usize) -> String {
    let mut result = String::new();
    for ch in text.chars().map(|c| if c.is_control() { ' ' } else { c }) {
        if result.len() + ch.len_utf8() > limit {
            break;
        }
        result.push(ch);
    }
    result
}

impl PluginUi {
    pub(super) fn bind_status(
        &mut self,
        id: &str,
        element_id: Option<&'static str>,
        value: fn(&ConduitApp, &ScopeKey) -> StatusValue,
    ) {
        let id = ItemId::new(id).expect("native status id");
        if self.registry.status(&id).is_none() {
            return;
        }
        let owner = self.registry.owner_for(id.as_str()).unwrap();
        let registration = self.registry.registration(owner).unwrap().clone();
        self.status_providers.insert(
            id,
            StatusProvider {
                registration,
                value,
                element_id,
            },
        );
    }
}

// Keep a priority prefix across BOTH alignments. Once overflow is necessary its
// button gets space first; smaller low-priority items cannot displace earlier ones.
fn visible_count(widths: impl Iterator<Item = f32> + Clone, available: f32) -> usize {
    let total: f32 = widths.clone().map(|w| w + GAP).sum();
    let budget = if total <= available {
        available
    } else {
        (available - MORE_WIDTH - GAP).max(0.)
    };
    let mut used = 0.;
    widths
        .take_while(|width| {
            used += width + GAP;
            used <= budget
        })
        .count()
}

impl ConduitApp {
    fn status_entries(&self) -> Vec<StatusEntry> {
        let mut items = [Alignment::Left, Alignment::Right]
            .into_iter()
            .flat_map(|alignment| {
                self.plugins
                    .registry
                    .statuses(&self.plugins.snapshot, alignment)
            })
            .collect::<Vec<_>>();
        items.sort_by(|a, b| b.priority.cmp(&a.priority).then(a.id.cmp(&b.id)));
        items
            .into_iter()
            .map(|item| {
                let binding = self
                    .plugins
                    .status_providers
                    .get(&item.id)
                    .filter(|binding| self.plugins.registry.is_registered(&binding.registration));
                let value = binding
                    .map(|binding| {
                        (binding.value)(self, &self.plugins.snapshot.scope(item.scope).unwrap())
                    })
                    .unwrap_or_else(|| item.initial.clone());
                let value = StatusValue {
                    text: bounded_text(&value.text, 256),
                    tooltip: value.tooltip.map(|text| bounded_text(&text, 1024)),
                };
                let tooltip = match &value.tooltip {
                    Some(tooltip) if tooltip != &value.text => {
                        format!("{} · {tooltip}", value.text)
                    }
                    _ => value.text.clone(),
                };
                // A conservative natural width with a hard cap; the text child always
                // ellipsizes, including fonts wider than this estimate.
                let text_width: f32 = value
                    .text
                    .chars()
                    .map(|c| if c.is_ascii() { 6. } else { 11. })
                    .sum();
                let width = (text_width + 16. + if item.icon.is_some() { 20. } else { 0. })
                    .clamp(40., ITEM_MAX_WIDTH);
                let request = self
                    .plugins
                    .registry
                    .prepare_status_command(&item.id, &self.plugins.snapshot)
                    .ok()
                    .filter(|request| self.plugins.command_bound(request.command()));
                StatusEntry {
                    item: item.clone(),
                    value,
                    tooltip,
                    request,
                    width,
                    element_id: binding
                        .and_then(|binding| binding.element_id)
                        .map(str::to_owned)
                        .unwrap_or_else(|| item.id.to_string()),
                }
            })
            .collect()
    }

    fn status_element(&self, entry: &StatusEntry, cx: &mut Context<Self>) -> AnyElement {
        let text = h_flex()
            .w_full()
            .min_w_0()
            .gap_1()
            .items_center()
            .when_some(
                entry.item.icon.as_deref().and_then(activity_icon),
                |row, icon| row.child(Icon::new(icon).small().flex_shrink_0()),
            )
            .child(div().min_w_0().truncate().child(entry.value.text.clone()));
        let id = entry.element_id.clone();
        if entry.item.command.is_some() {
            let request = entry.request.clone();
            Button::new(SharedString::from(id.clone()))
                .debug_selector(move || id.clone())
                .ghost()
                .small()
                .h(px(26.))
                .w(px(entry.width))
                .flex_shrink_0()
                .px_2()
                .text_size(px(11.))
                .text_color(rgb(palette().muted))
                .accessibility_label(entry.value.text.clone())
                .tooltip(entry.tooltip.clone())
                .disabled(request.is_none())
                .child(text)
                .on_click(cx.listener(move |app, _, window, cx| {
                    if let Some(request) = &request {
                        app.dispatch_plugin_command(request, window, cx);
                    }
                }))
                .into_any_element()
        } else {
            let tooltip = SharedString::from(entry.tooltip.clone());
            div()
                .id(SharedString::from(id.clone()))
                .debug_selector(move || id.clone())
                .w(px(entry.width))
                .flex_shrink_0()
                .px_2()
                .text_size(px(11.))
                .text_color(rgb(palette().muted))
                .tooltip(move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(tooltip.clone()).build(window, cx)
                })
                .child(text)
                .into_any_element()
        }
    }

    pub(in crate::app) fn render_status_contributions(&self, cx: &mut Context<Self>) -> AnyElement {
        let entries = self.status_entries();
        let count = visible_count(
            entries.iter().map(|entry| entry.width),
            self.plugins.status_width,
        );
        let weak = cx.weak_entity();
        let mut row = h_flex()
            .id("toolbar-contributions")
            .debug_selector(|| "toolbar-contributions".into())
            .flex_1()
            .min_w(px(MORE_WIDTH + GAP))
            .h(px(28.))
            .items_center()
            .overflow_hidden()
            .on_prepaint(move |bounds, window, cx| {
                let _ = weak.update(cx, |app, cx| {
                    let width = f32::from(bounds.size.width).floor();
                    if app.plugins.status_width != width {
                        app.plugins.status_width = width;
                        let weak = cx.weak_entity();
                        window.on_next_frame(move |_, cx| {
                            let _ = weak.update(cx, |_, cx| cx.notify());
                        });
                    }
                });
            });
        for entry in entries[..count]
            .iter()
            .filter(|entry| entry.item.alignment == Alignment::Left)
        {
            row = row.child(
                div()
                    .mr(px(GAP))
                    .flex_shrink_0()
                    .child(self.status_element(entry, cx)),
            );
        }
        row = row.child(div().flex_1());
        for entry in entries[..count]
            .iter()
            .filter(|entry| entry.item.alignment == Alignment::Right)
        {
            row = row.child(
                div()
                    .ml(px(GAP))
                    .flex_shrink_0()
                    .child(self.status_element(entry, cx)),
            );
        }
        if count < entries.len() {
            let hidden: Vec<_> = entries.into_iter().skip(count).collect();
            let app = cx.entity();
            row = row.child(
                Button::new("toolbar-more")
                    .debug_selector(|| "toolbar-more".into())
                    .ghost()
                    .small()
                    .compact()
                    .w(px(MORE_WIDTH))
                    .h(px(26.))
                    .ml(px(GAP))
                    .flex_shrink_0()
                    .icon(IconName::Ellipsis)
                    .tooltip("更多状态")
                    .accessibility_label("更多状态")
                    .dropdown_menu(move |mut menu, _, _| {
                        menu = menu.scrollable(true).max_w(px(360.));
                        for entry in &hidden {
                            let label = entry.value.text.clone();
                            let tooltip = SharedString::from(entry.tooltip.clone());
                            let debug = format!("toolbar-more-{}", entry.item.id);
                            let request = entry.request.clone();
                            let app = app.clone();
                            menu = menu.item(
                                PopupMenuItem::element(move |_, _| {
                                    div()
                                        .id(SharedString::from(debug.clone()))
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .debug_selector({
                                            let debug = debug.clone();
                                            move || debug.clone()
                                        })
                                        .tooltip({
                                            let tooltip = tooltip.clone();
                                            move |window, cx| {
                                                gpui_kit::component::tooltip::Tooltip::new(
                                                    tooltip.clone(),
                                                )
                                                .build(window, cx)
                                            }
                                        })
                                        .child(label.clone())
                                })
                                .disabled(request.is_none())
                                .on_click(move |_, window, cx| {
                                    if let Some(request) = &request {
                                        app.update(cx, |app, cx| {
                                            app.dispatch_plugin_command(request, window, cx);
                                        });
                                    }
                                }),
                            );
                        }
                        menu
                    }),
            );
        }
        row.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{bounded_text, visible_count};

    #[test]
    fn priority_prefix_reserves_overflow_and_does_not_backfill() {
        let widths = [180., 80., 40.];
        for (width, expected) in [
            (0., 0),
            (28., 0),
            (211., 0),
            (216., 1),
            (300., 2),
            (312., 3),
        ] {
            assert_eq!(
                visible_count(widths.into_iter(), width),
                expected,
                "width {width}"
            );
        }
    }

    #[test]
    fn native_status_values_are_bounded_utf8_single_lines() {
        assert_eq!(bounded_text("ab\n中文", 6), "ab 中");
        assert_eq!(bounded_text(&"界".repeat(300), 256).len(), 255);
        assert_eq!(bounded_text(&"x".repeat(2000), 1024).len(), 1024);
    }
}
