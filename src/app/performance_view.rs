//! Performance monitor overlay.

use super::*;

impl Render for PerformanceMonitor {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let memory = |bytes: Option<usize>| {
            bytes
                .map(|value| format!("{:.1} MiB", value as f64 / 1_048_576.0))
                .unwrap_or_else(|| "不可用".into())
        };
        let percent = |value: Option<f64>| {
            value
                .map(|value| format!("{value:.1}%"))
                .unwrap_or_else(|| {
                    if self.sample_duration.is_zero() {
                        "采样中…".into()
                    } else {
                        "不可用".into()
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
                .unwrap_or_else(|| "本周期空闲".into())
        };
        let rows = [
            ("CPU · 整机容量", percent(self.cpu_percent)),
            ("CPU · 近 60 秒峰值", percent(self.recent_cpu_peak())),
            ("驻留内存 · 工作集", memory(self.memory_bytes)),
            ("私有提交内存", memory(self.private_bytes)),
            ("工作集 · 进程生命周期峰值", memory(self.peak_memory_bytes)),
            (
                "私有提交 · 窗口内变化",
                self.private_growth()
                    .map(|bytes| format!("{:+.1} MiB", bytes as f64 / 1_048_576.0))
                    .unwrap_or_else(|| "等待后续样本".into()),
            ),
            ("主视图构建 · 均值 / 最大", timing(self.render)),
            ("单条消息构建 · 均值 / 最大", timing(self.message_render)),
            (
                "最近一帧构建 / 会话消息",
                format!("{} / {} 条", self.mounted_messages(), self.message_count),
            ),
            ("本周期主视图构建", format!("{} 次", self.render.count)),
            ("本周期引擎事件", format!("{} 条", self.event_count)),
            (
                "运行任务 / 已加载会话",
                format!("{} / {}", self.active_runs, self.session_count),
            ),
            ("运行时长", self.uptime()),
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
                    .child(
                        div()
                            .text_size(ui_text_size(13.))
                            .font_semibold()
                            .child("性能检测"),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_size(ui_text_size(11.))
                            .text_color(rgb(palette().muted))
                            .child(if cfg!(debug_assertions) {
                                "开发构建"
                            } else {
                                "发布构建"
                            }),
                    ),
            )
            .child(
                div()
                    .text_size(ui_text_size(11.))
                    .text_color(rgb(palette().muted))
                    .child(format!(
                        "Conduit · PID {} · {} 逻辑处理器",
                        std::process::id(),
                        self.logical_cpus
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
                    .child("仅统计 Conduit 主进程，CLI / Git 子进程不计入")
                    .child("工作集含共享页；私有提交不等于实际驻留内存")
                    .child("构建耗时不含布局、GPU 绘制，不代表帧率")
                    .child("约每 2 秒采样，保留最近 60 秒内最多 30 条")
                    .child("点击图标可固定展开；窗口变化按已有样本计算"),
            )
    }
}
