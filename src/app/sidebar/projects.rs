//! Display-only grouping. Workspace paths and session ownership remain independent.
use super::*;

#[derive(Default)]
pub(super) struct Groups {
    parents: HashMap<PathBuf, PathBuf>,
    projects: HashMap<PathBuf, Project>,
}

pub(super) struct Project {
    pub(super) name: String,
    pub(super) members: Vec<PathBuf>,
}

fn name_key(name: &str) -> String {
    if cfg!(windows) {
        name.to_lowercase()
    } else {
        name.to_owned()
    }
}

impl Groups {
    pub(super) fn new(paths: impl Iterator<Item = PathBuf>) -> Self {
        let paths: Vec<_> = paths.collect();
        let siblings: HashMap<_, _> = paths
            .iter()
            .filter_map(|path| {
                Some((
                    (
                        path.parent()?.to_owned(),
                        name_key(path.file_name()?.to_str()?),
                    ),
                    path,
                ))
            })
            .collect();
        let mut names: HashMap<String, (String, Vec<PathBuf>)> = HashMap::new();
        for path in &paths {
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let base = name
                .rsplit_once('-')
                .and_then(|(base, suffix)| {
                    if base.is_empty()
                        || !suffix.bytes().all(|byte| byte.is_ascii_digit())
                        || suffix.parse::<u32>().ok()? < 2
                    {
                        return None;
                    }
                    siblings
                        .get(&(path.parent()?.to_owned(), name_key(base)))
                        .and_then(|path| path.file_name()?.to_str())
                })
                .unwrap_or(name);
            let entry = names
                .entry(name_key(base))
                .or_insert_with(|| (base.to_owned(), Vec::new()));
            entry.1.push(path.clone());
        }
        let mut groups = Self::default();
        for (_, (name, mut members)) in names {
            if members.len() < 2 {
                continue;
            }
            // Stable across manual reordering and activity sorting.
            members.sort();
            let key = members[0].clone();
            for path in &members {
                groups.parents.insert(path.clone(), key.clone());
            }
            groups.projects.insert(key, Project { name, members });
        }
        groups
    }

    pub(super) fn parent(&self, path: &Path) -> Option<&PathBuf> {
        self.parents.get(path)
    }

    pub(super) fn contains(&self, key: &Path) -> bool {
        self.projects.contains_key(key)
    }

    pub(super) fn is_empty(&self) -> bool {
        self.projects.is_empty()
    }

    /// Group complete workspace blocks without changing stored workspace indices.
    pub(super) fn project(
        &self,
        rows: Vec<cache::Row>,
        expanded: &HashSet<PathBuf>,
        attention: &HashMap<PathBuf, (u64, bool)>,
        searching: bool,
    ) -> Vec<cache::Row> {
        use cache::Row;
        let mut blocks: Vec<Vec<Row>> = Vec::new();
        for row in rows {
            if matches!(row, Row::Empty) {
                return vec![row];
            }
            if matches!(row, Row::Workspace { .. }) {
                blocks.push(Vec::new());
            }
            blocks
                .last_mut()
                .expect("workspace precedes children")
                .push(row);
        }
        let mut positions = HashMap::new();
        let mut buckets: Vec<(PathBuf, Vec<Vec<Row>>)> = Vec::new();
        for block in blocks {
            let path = block[0].path().expect("workspace path");
            let key = self.parent(path).map_or(path, PathBuf::as_path).to_owned();
            let index = *positions.entry(key.clone()).or_insert_with(|| {
                let index = buckets.len();
                buckets.push((key, Vec::new()));
                index
            });
            buckets[index].1.push(block);
        }
        let mut result = Vec::new();
        for (key, blocks) in buckets {
            if let Some(project) = self.projects.get(&key) {
                let count = blocks
                    .iter()
                    .map(|block| match block[0] {
                        Row::Workspace { count, .. } => count,
                        _ => 0,
                    })
                    .sum();
                let activity = project
                    .members
                    .iter()
                    .filter_map(|path| attention.get(path).copied())
                    .min_by_key(|(id, running)| (!running, *id));
                result.push(Row::Project {
                    key: key.clone(),
                    name: project.name.clone(),
                    copies: project.members.len(),
                    count,
                    activity,
                });
                if !searching && !expanded.contains(&key) {
                    continue;
                }
            }
            result.extend(blocks.into_iter().flatten());
        }
        result
    }
}

impl ConduitApp {
    fn set_sidebar_project_workspaces_expanded(
        &mut self,
        key: &Path,
        expanded: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.sidebar.projects.projects.get(key) else {
            return;
        };
        let members = project.members.clone();
        for path in members {
            self.set_workspace_collapsed(&path, !expanded);
        }
        if expanded {
            self.sidebar.expanded_projects.insert(key.to_owned());
        }
        self.refresh_sidebar();
        self.sidebar.reveal_project(key);
        self.focus_sidebar(window, cx);
    }

    pub(super) fn set_sidebar_project_expanded(
        &mut self,
        key: &Path,
        expanded: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if expanded {
            self.sidebar.expanded_projects.insert(key.to_owned());
        } else {
            self.sidebar.expanded_projects.remove(key);
        }
        self.refresh_sidebar();
        self.sidebar.reveal_project(key);
        self.focus_sidebar(window, cx);
    }

    pub(super) fn sidebar_project(&self, row: &cache::Row, cx: &mut Context<Self>) -> AnyElement {
        let cache::Row::Project {
            key,
            name,
            copies,
            count,
            activity,
        } = row
        else {
            unreachable!();
        };
        let expanded =
            self.sidebar.expanded_projects.contains(key) || !self.search_query.trim().is_empty();
        let workspaces_expanded = expanded
            && self.sidebar.projects.projects[key]
                .members
                .iter()
                .all(|path| !self.workspace_collapsed(path));
        let workspaces_key = key.clone();
        let workspaces_label = conduit::i18n::text_static(if workspaces_expanded {
            "sidebar.collapse_project_workspaces"
        } else {
            "sidebar.expand_project_workspaces"
        });
        let toggle_key = key.clone();
        let selector = self.sidebar.workspace_index(key).expect("project anchor");
        h_flex()
            .id(SharedString::from(format!(
                "workspace-project-{}",
                key.display()
            )))
            .debug_selector(move || format!("workspace-project-{selector}"))
            .role(Role::TreeItem)
            .aria_label(conduit::i18n::format(
                "sidebar.project_summary",
                &[
                    ("name", name.clone()),
                    ("copies", copies.to_string()),
                    ("count", count.to_string()),
                ],
            ))
            .aria_expanded(expanded)
            .relative()
            .h(px(ROW_HEIGHT))
            .pl(px(TREE_AXIS - TREE_TOGGLE_SIZE / 2.))
            .pr_2()
            .gap(px(4.))
            .items_center()
            .rounded(px(8.))
            .text_size(ui_text_size(13.))
            .text_color(rgb(palette().text))
            .cursor_pointer()
            .hover(|style| style.bg(rgb(palette().surface_hover)))
            .when(
                self.sidebar
                    .keyboard_row
                    .as_ref()
                    .is_some_and(|focused| focused.same_identity(row)),
                |row| {
                    row.aria_active_descendant()
                        .child(self.sidebar_focus_indicator())
                },
            )
            .on_click(cx.listener(move |app, _, window, cx| {
                app.set_sidebar_project_expanded(&toggle_key, !expanded, window, cx);
            }))
            .child(
                h_flex()
                    .size(px(TREE_TOGGLE_SIZE))
                    .flex_shrink_0()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .debug_selector(move || format!("project-folder-{selector}"))
                            .size(px(TREE_MARK_SIZE))
                            .child(
                                Icon::new(if expanded {
                                    IconName::FolderOpen
                                } else {
                                    IconName::Folder
                                })
                                .size(px(TREE_MARK_SIZE))
                                .text_color(rgb(palette().muted)),
                            ),
                    ),
            )
            .child(
                div()
                    .debug_selector(move || format!("project-name-{selector}"))
                    .flex_1()
                    .min_w_0()
                    .font_semibold()
                    .truncate()
                    .child(name.clone()),
            )
            .when_some(*activity, |row, (id, running)| {
                row.child(session_activity_dot("project-activity", id, running))
            })
            .child(
                Button::new(("project-toggle-workspaces", selector))
                    .debug_selector(move || {
                        format!(
                            "project-{}-workspaces-{selector}",
                            if workspaces_expanded {
                                "collapse"
                            } else {
                                "expand"
                            }
                        )
                    })
                    .ghost()
                    .small()
                    .compact()
                    .w(px(24.))
                    .h(px(24.))
                    .p_0()
                    .flex_shrink_0()
                    .cursor_pointer()
                    .icon(if workspaces_expanded {
                        FullIconName::ListChevronsDownUp
                    } else {
                        FullIconName::ListChevronsUpDown
                    })
                    .tooltip(workspaces_label)
                    .accessibility_label(workspaces_label)
                    .on_click(cx.listener(move |app, _, window, cx| {
                        cx.stop_propagation();
                        app.set_sidebar_project_workspaces_expanded(
                            &workspaces_key,
                            !workspaces_expanded,
                            window,
                            cx,
                        );
                    })),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::Groups;
    use std::path::{Path, PathBuf};

    #[test]
    fn names_group_exact_copies_and_only_numbered_siblings_with_a_base() {
        let paths: Vec<PathBuf> = [
            "a/jupiter-back-end",
            "a/jupiter-back-end-2",
            "a/jupiter-back-end-3",
            "b/jupiter-back-end",
            "b/jupiter-back-end-1",
            "c/jupiter-back-end-4",
            "a/orphan-2",
            "b/orphan-3",
            "a/version-2026",
        ]
        .into_iter()
        .map(PathBuf::from)
        .collect();
        let groups = Groups::new(paths.iter().cloned());
        let key = groups.parent(&paths[0]).unwrap();
        assert_eq!(groups.projects[key].name, "jupiter-back-end");
        assert_eq!(groups.projects[key].members.len(), 4);
        for path in &paths[..4] {
            assert_eq!(groups.parent(path), Some(key));
        }
        for path in &paths[4..] {
            assert!(groups.parent(path).is_none());
        }
        let reordered = Groups::new(paths.iter().rev().cloned());
        assert_eq!(reordered.parent(&paths[3]), Some(key));
        if cfg!(windows) {
            let groups = Groups::new(["a/Project", "b/project"].map(PathBuf::from).into_iter());
            assert_eq!(
                groups.parent(Path::new("a/Project")),
                groups.parent(Path::new("b/project"))
            );
            assert!(!groups.is_empty());
        }
    }

    #[test]
    fn projection_preserves_membership_counts_and_search_visibility() {
        use super::cache::Row;
        use std::collections::{HashMap, HashSet};
        let paths = ["a/project", "a/other", "a/project-2"].map(PathBuf::from);
        let groups = Groups::new(paths.iter().cloned());
        let rows: Vec<_> = paths
            .iter()
            .enumerate()
            .flat_map(|(index, path)| {
                [
                    Row::Workspace {
                        path: path.clone(),
                        count: index + 1,
                        last: Some(index as u64),
                    },
                    Row::Session {
                        path: path.clone(),
                        id: index as u64,
                    },
                ]
            })
            .collect();
        let attention = HashMap::from([
            (paths[0].clone(), (0, false)),
            (paths[2].clone(), (2, true)),
        ]);
        let collapsed = groups.project(rows.clone(), &HashSet::new(), &attention, false);
        assert_eq!(collapsed.len(), 3);
        assert!(matches!(
            &collapsed[0],
            Row::Project {
                copies: 2,
                count: 4,
                activity: Some((2, true)),
                ..
            }
        ));
        let expanded = groups.project(
            rows.clone(),
            &HashSet::from([paths[0].clone()]),
            &attention,
            false,
        );
        let search = groups.project(rows, &HashSet::new(), &attention, true);
        assert_eq!(expanded, search);
        let ids: Vec<_> = expanded
            .iter()
            .filter_map(|row| match row {
                Row::Session { id, .. } => Some(*id),
                _ => None,
            })
            .collect();
        assert_eq!(ids, [0, 2, 1]);
        assert!(Groups::new([paths[0].clone()].into_iter()).is_empty());
    }
}
