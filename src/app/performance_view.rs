//! Performance monitor overlay.

use super::*;

impl Render for PerformanceMonitor {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let memory = |bytes: Option<usize>| {
            bytes
                .map(|value| format!("{:.1} MiB", value as f64 / 1_048_576.0))
                .unwrap_or_else(|| {
                    conduit::i18n::text_static("performance_view.unavailable").into()
                })
        };
        let percent = |value: Option<f64>| {
            value
                .map(|value| format!("{value:.1}%"))
                .unwrap_or_else(|| {
                    if self.sample_duration.is_zero() {
                        conduit::i18n::text_static("performance_view.sampling").into()
                    } else {
                        conduit::i18n::text_static("performance_view.unavailable").into()
                    }
                })
        };
        let timing = |value: crate::performance::Timings| {
            value
                .average()
                .map(|average| {
                    format!(
                        "{:.2} / {:.2} ms",
                        average.as_secs_f64() * 1000.0,
                        value.maximum.as_secs_f64() * 1000.0
                    )
                })
                .unwrap_or_else(|| {
                    conduit::i18n::text_static("performance_view.idle_this_interval").into()
                })
        };
        let rows = [
            (
                conduit::i18n::text_static("performance_view.cpu_total_system_capacity"),
                percent(self.cpu_percent),
            ),
            (
                conduit::i18n::text_static("performance_view.cpu_peak_in_last_60_seconds"),
                percent(self.recent_cpu_peak()),
            ),
            (
                conduit::i18n::text_static("performance_view.resident_memory_working_set"),
                memory(self.memory_bytes),
            ),
            (
                conduit::i18n::text_static("performance_view.private_committed_memory"),
                memory(self.private_bytes),
            ),
            (
                conduit::i18n::text_static("performance_view.working_set_process_lifetime_peak"),
                memory(self.peak_memory_bytes),
            ),
            (
                conduit::i18n::text_static(
                    "performance_view.private_committed_change_in_sample_window",
                ),
                self.private_growth()
                    .map(|bytes| format!("{:+.1} MiB", bytes as f64 / 1_048_576.0))
                    .unwrap_or_else(|| {
                        conduit::i18n::text_static("performance_view.waiting_for_more_samples")
                            .into()
                    }),
            ),
            (
                conduit::i18n::text_static("performance_view.main_view_build_mean_max"),
                timing(self.render),
            ),
            (
                conduit::i18n::text_static("performance_view.message_build_mean_max"),
                timing(self.message_render),
            ),
            (
                conduit::i18n::text_static("performance_view.latest_frame_build_session_messages"),
                conduit::i18n::format(
                    "performance_view.messages",
                    &[
                        ("arg0", format!("{}", self.mounted_messages())),
                        ("arg1", format!("{}", self.message_count)),
                    ],
                ),
            ),
            (
                conduit::i18n::text_static("performance_view.main_view_builds_this_interval"),
                conduit::i18n::format(
                    "performance_view.times",
                    &[("arg0", format!("{}", self.render.count))],
                ),
            ),
            (
                conduit::i18n::text_static("performance_view.engine_events_this_interval"),
                conduit::i18n::format(
                    "performance_view.events",
                    &[("arg0", format!("{}", self.event_count))],
                ),
            ),
            (
                conduit::i18n::text_static("performance_view.running_tasks_loaded_sessions"),
                format!("{} / {}", self.active_runs, self.session_count),
            ),
            (
                conduit::i18n::text_static("performance_view.uptime"),
                self.uptime(),
            ),
        ];
        v_flex()
            .id("performance-details")
            .w(px(350.))
            .max_h((window.viewport_size().height - px(72.)).min(px(460.)))
            .overflow_y_scroll()
            .p_3()
            .gap_2()
            .rounded(px(10.))
            .border_1()
            .border_color(rgb(palette().border))
            .bg(rgb(palette().surface))
            .shadow_md()
            .text_color(rgb(palette().text))
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(Icon::new(gpui_kit::assets::IconName::Activity).small())
                    .child(div().text_size(ui_text_size(13.)).font_semibold().child(
                        conduit::i18n::text_static("performance_view.performance_monitor"),
                    ))
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_size(ui_text_size(11.))
                            .text_color(rgb(palette().muted))
                            .child(if cfg!(debug_assertions) {
                                conduit::i18n::text_static("performance_view.development_build")
                            } else {
                                conduit::i18n::text_static("performance_view.release_build")
                            }),
                    ),
            )
            .child(
                div()
                    .text_size(ui_text_size(11.))
                    .text_color(rgb(palette().muted))
                    .child(conduit::i18n::format(
                        "performance_view.conduit_pid_logical_processors",
                        &[
                            ("arg0", format!("{}", std::process::id())),
                            ("arg1", format!("{}", self.logical_cpus)),
                        ],
                    )),
            )
            .child(
                v_flex()
                    .gap_2()
                    .py_2()
                    .border_t_1()
                    .border_b_1()
                    .border_color(rgb(palette().border))
                    .children(rows.into_iter().map(|(label, value)| {
                        h_flex()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .text_size(ui_text_size(12.))
                            .child(div().text_color(rgb(palette().muted)).child(label))
                            .child(div().flex_shrink_0().child(value))
                    })),
            )
            .child(
                v_flex()
                    .gap_1()
                    .text_size(ui_text_size(11.))
                    .text_color(rgb(palette().muted))
                    .child(conduit::i18n::text_static(
                        "performance_view.conduit_main_process_only_excludes_cli_and",
                    ))
                    .child(conduit::i18n::text_static(
                        "performance_view.working_set_includes_shared_pages_private_commit",
                    ))
                    .child(conduit::i18n::text_static(
                        "performance_view.build_time_excludes_layout_and_gpu_rendering",
                    ))
                    .child(conduit::i18n::text_static(
                        "performance_view.samples_about_every_2_seconds_keeps_up",
                    ))
                    .child(conduit::i18n::text_static(
                        "performance_view.click_the_icon_to_pin_this_panel",
                    )),
            )
    }
}
