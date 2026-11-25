use super::*;
use chrono::Datelike as _;

fn title_preview(title: &str) -> String {
    let title = single_line_title(title);
    let title = title.trim();
    let title = if title.is_empty() {
        conduit::i18n::text_static("notifications.untitled_archived_session")
    } else {
        title
    };
    let mut preview: String = title.chars().take(120).collect();
    if title.chars().count() > 120 {
        preview.push('…');
    }
    preview
}

fn archive_message(
    workspace: &str,
    count: usize,
    days: u64,
    titles: &[String],
) -> conduit::i18n::Message {
    let mut preview = titles
        .iter()
        .take(5)
        .map(|title| title_preview(title))
        .collect::<Vec<_>>();
    if count > 5 {
        preview.push(conduit::i18n::format(
            "notifications.archived_sessions_more",
            &[("count", (count - 5).to_string())],
        ));
    }
    conduit::i18n::Message::new(
        "notifications.sessions_auto_archived_summary",
        &[
            (
                "workspace",
                single_line_title(workspace).chars().take(200).collect(),
            ),
            ("count", count.to_string()),
            ("days", days.to_string()),
            ("titles", preview.join("\n")),
        ],
    )
}

impl ConduitApp {
    pub(in crate::app) fn set_auto_archive_enabled(
        &mut self,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.writable || self.closing || self.store.ui.auto_archive.enabled == enabled {
            return;
        }
        self.store.ui.auto_archive.enabled = enabled;
        if enabled {
            self.auto_archive_sessions(true, cx);
        } else {
            self.persist();
        }
        cx.notify();
    }

    pub(in crate::app) fn set_auto_archive_days(
        &mut self,
        after: crate::store::AutoArchiveDays,
        cx: &mut Context<Self>,
    ) {
        if !self.writable || self.closing || self.store.ui.auto_archive.after == after {
            return;
        }
        self.store.ui.auto_archive.after = after;
        if self.store.ui.auto_archive.enabled {
            self.auto_archive_sessions(true, cx);
        } else {
            self.persist();
        }
        cx.notify();
    }

    pub(in crate::app) fn auto_archive_sessions(&mut self, force: bool, cx: &mut Context<Self>) {
        if !self.writable || self.closing || !self.store.ui.auto_archive.enabled {
            return;
        }
        let now = Local::now();
        let day = now.date_naive().num_days_from_ce();
        if !force && !self.store.ui.auto_archive.due(day) {
            return;
        }
        let now = now.timestamp().max(0) as u64;
        self.store.ui.auto_archive.last_check_day = Some(day);
        let days = self.store.ui.auto_archive.after.days();
        let batches: Vec<_> = self
            .store
            .workspaces
            .iter()
            .filter_map(|workspace| {
                let mut candidates: Vec<_> = workspace
                    .visible_sessions()
                    .filter(|session| self.store.ui.auto_archive.eligible(session, now))
                    .filter(|session| !self.auto_archive_protected(session.id, cx))
                    .map(|session| {
                        (
                            session.last_activity_at(),
                            session.id,
                            session.title.clone(),
                        )
                    })
                    .collect();
                candidates.sort_by_key(|(activity, id, _)| (*activity, *id));
                (!candidates.is_empty())
                    .then(|| (workspace.path.clone(), workspace.name(), candidates))
            })
            .collect();
        let mut notices = Vec::new();
        for (path, name, candidates) in batches {
            let count = candidates.len();
            let titles: Vec<_> = candidates
                .iter()
                .take(5)
                .map(|(_, _, title)| title.clone())
                .collect();
            for (_, id, _) in candidates {
                if let Some(session) = self.session_mut(id) {
                    session.archived = true;
                }
                self.store.ui.auto_archive.restored_at.remove(&id);
            }
            notices.push(
                Notice::new(
                    NoticeSource::App,
                    NoticeLevel::Success,
                    conduit::i18n::text_static("notifications.sessions_auto_archived"),
                    "",
                )
                .localized_message(archive_message(&name, count, days, &titles))
                .target(NoticeTarget::ArchivedSessions(path)),
            );
        }
        if notices.is_empty() {
            // Even an empty sweep counts as today's check.
            self.persist();
        } else {
            self.finish_archiving_sessions(cx);
            for notice in notices {
                self.notify_user(notice, cx);
            }
        }
    }

    fn auto_archive_protected(&self, id: u64, cx: &Context<Self>) -> bool {
        self.session_busy(id)
            || self.session_has_unsent_content(id, cx)
            || self.child_draft_pending(id, cx)
            || self.queued_messages.contains_key(&id)
            || self.session_control.queue_saves.contains_key(&id)
            || self.session_draft_save_pending(id)
            || self.session_fork_pending(id)
            || self
                .store
                .child_commands
                .iter()
                .any(|command| command.root == id)
            || self.store.child_bindings.iter().any(|(child, binding)| {
                binding.root == id
                    && (self.session_busy(*child)
                        || self.session_has_unsent_content(*child, cx)
                        || self.queued_messages.contains_key(child)
                        || self.session_control.queue_saves.contains_key(child)
                        || self.session_draft_save_pending(*child))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{archive_message, title_preview};

    #[test]
    fn auto_archive_preview_shows_five_titles_and_only_then_an_ellipsis() {
        let _locale = crate::test_support::chinese();
        for count in [1, 5, 6, 20] {
            let titles: Vec<_> = (1..=count).map(|id| format!("会话{id}")).collect();
            let message = archive_message("项目A", count, 7, &titles).render();
            assert!(message.contains("项目A"));
            assert!(message.contains(&format!("{count} 个")));
            for title in titles.iter().take(5) {
                assert!(message.lines().any(|line| line == title));
            }
            assert_eq!(message.contains('…'), count > 5);
            if count > 5 {
                assert!(!message.lines().any(|line| line == "会话6"));
            }
        }
        assert_eq!(title_preview("a\r\nb"), "a  b");
        assert!(!title_preview(" ").is_empty());
        assert_eq!(title_preview(&"长".repeat(200)).chars().count(), 121);
    }
}
