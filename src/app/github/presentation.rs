use super::*;
use crate::github::Check;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum CheckState {
    Attention,
    Running,
    Unknown,
    Cancelled,
    Skipped,
    Passed,
}

fn state(check: &Check) -> CheckState {
    if check.needs_attention() {
        return CheckState::Attention;
    }
    match check.state.as_str() {
        "PENDING" | "QUEUED" | "IN_PROGRESS" | "WAITING" | "REQUESTED" | "EXPECTED" => {
            CheckState::Running
        }
        "SUCCESS" => CheckState::Passed,
        "NEUTRAL" | "SKIPPED" => CheckState::Skipped,
        "CANCELLED" | "STALE" => CheckState::Cancelled,
        _ => CheckState::Unknown,
    }
}

pub(super) fn check_order(checks: &[Check]) -> Vec<usize> {
    let mut indices: Vec<_> = (0..checks.len()).collect();
    // Keep source indices for attaching evidence and finding existing repair tasks.
    indices.sort_by_key(|index| state(&checks[*index]));
    indices
}

pub(super) fn check_status(check: &Check) -> Div {
    let (key, color) = match state(check) {
        CheckState::Attention => ("github.status_attention", palette().danger_text),
        CheckState::Running => ("github.status_running", palette().activity_running),
        CheckState::Passed => ("github.status_passed", palette().git_added_text),
        CheckState::Skipped => ("github.status_skipped", palette().muted),
        CheckState::Cancelled => ("github.status_cancelled", palette().warning_text),
        CheckState::Unknown => ("github.status_unknown", palette().muted),
    };
    div()
        .text_size(ui_text_size(12.))
        .text_color(rgb(color))
        .child(format!("{} · {}", label(key), check.state))
}

fn summary(checks: &[Check]) -> String {
    if checks.is_empty() {
        return label("github.no_checks").into();
    }
    let groups = [
        (CheckState::Attention, "github.status_attention"),
        (CheckState::Running, "github.status_running"),
        (CheckState::Unknown, "github.status_unknown"),
        (CheckState::Cancelled, "github.status_cancelled"),
        (CheckState::Skipped, "github.status_skipped"),
        (CheckState::Passed, "github.status_passed"),
    ];
    groups
        .into_iter()
        .filter_map(|(group, key)| {
            let count = checks.iter().filter(|check| state(check) == group).count();
            (count > 0).then(|| format!("{count} {}", label(key)))
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

fn pr_state(value: &str) -> String {
    match value {
        "OPEN" => label("github.pr_open").into(),
        "MERGED" => label("github.pr_merged").into(),
        "CLOSED" => label("github.pr_closed").into(),
        "APPROVED" => label("github.review_approved").into(),
        "CHANGES_REQUESTED" => label("github.review_changes").into(),
        "REVIEW_REQUIRED" => label("github.review_required").into(),
        "" => label("github.review_unknown").into(),
        _ => value.into(),
    }
}

impl PullRequestPanel {
    pub(super) fn overview(&self, pr: &crate::github::PullRequest) -> Div {
        let review = pr.threads.as_ref().map_or_else(
            || label("github.review_unavailable").to_owned(),
            |snapshot| {
                conduit::i18n::format(
                    "github.unresolved_count",
                    &[(
                        "count",
                        snapshot
                            .threads
                            .iter()
                            .filter(|thread| !thread.is_resolved)
                            .count()
                            .to_string(),
                    )],
                )
            },
        );
        let section = |title, text: String, attention| {
            v_flex()
                .flex_1()
                .min_w(px(180.))
                .gap_1()
                .p_3()
                .border_1()
                .border_color(rgb(palette().border))
                .rounded_md()
                .child(
                    div()
                        .text_size(ui_text_size(12.))
                        .text_color(rgb(palette().muted))
                        .child(title),
                )
                .child(
                    div()
                        .text_size(ui_text_size(13.))
                        .font_semibold()
                        .text_color(rgb(if attention {
                            palette().danger_text
                        } else {
                            palette().text
                        }))
                        .child(text),
                )
        };
        v_flex()
            .gap_2()
            .child(
                div()
                    .text_size(ui_text_size(20.))
                    .font_semibold()
                    .child(format!("#{}  {}", pr.number, pr.title)),
            )
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        div()
                            .text_size(ui_text_size(12.))
                            .font_semibold()
                            .child(pr_state(&pr.state)),
                    )
                    .child(
                        div()
                            .text_size(ui_text_size(12.))
                            .child(format!("{} → {}", pr.branch, pr.base)),
                    )
                    .child(
                        div()
                            .text_size(ui_text_size(12.))
                            .text_color(rgb(palette().muted))
                            .child(pr_state(&pr.decision)),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(section(
                        label("github.ci_summary"),
                        summary(&pr.checks),
                        pr.checks.iter().any(Check::needs_attention),
                    ))
                    .child(section(label("github.review_summary"), review, false)),
            )
            .child(
                div()
                    .text_size(ui_text_size(11.))
                    .text_color(rgb(palette().muted))
                    .child(conduit::i18n::format(
                        "github.snapshot_time",
                        &[
                            ("time", message_timestamp(pr.fetched_at)),
                            ("head", pr.head.chars().take(12).collect::<String>()),
                        ],
                    )),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn check(value: &str) -> Check {
        Check {
            name: value.into(),
            state: value.into(),
            url: None,
        }
    }

    #[::core::prelude::v1::test]
    fn attention_precedes_pending_and_preserves_original_evidence_indices() {
        let checks = [
            check("SUCCESS"),
            check("IN_PROGRESS"),
            check("FAILURE"),
            check("ERROR"),
            check("NEW_STATE"),
        ];
        assert_eq!(check_order(&checks), vec![2, 3, 1, 4, 0]);
        for value in ["", "COMPLETED", "NEW_STATE"] {
            assert_eq!(state(&check(value)), CheckState::Unknown);
        }
        assert_eq!(state(&check("SKIPPED")), CheckState::Skipped);
        assert_eq!(state(&check("CANCELLED")), CheckState::Cancelled);
        assert_eq!(state(&check("ACTION_REQUIRED")), CheckState::Attention);
    }
}
