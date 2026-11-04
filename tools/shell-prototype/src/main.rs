mod model;

use anyhow::{Context as _, Result};
use gpui_kit::{
    component::{button::Button, *},
    *,
};
use gpui_shell::{HostError, HostModule, HostValue, ShellRoot, ShellRuntime, view::ScriptView};
use model::{Model, OWNER, SHOW_SUMMARY, Stamp};
use std::{cell::RefCell, path::PathBuf, rc::Rc};

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/workspace-overview")
}

fn host_value(value: serde_json::Value) -> HostValue {
    match value {
        serde_json::Value::Null => HostValue::Null,
        serde_json::Value::Bool(v) => HostValue::Bool(v),
        serde_json::Value::Number(v) => HostValue::Number(v.as_f64().unwrap_or_default()),
        serde_json::Value::String(v) => HostValue::Str(v),
        serde_json::Value::Array(v) => HostValue::Array(v.into_iter().map(host_value).collect()),
        serde_json::Value::Object(v) => {
            HostValue::Object(v.into_iter().map(|(k, v)| (k, host_value(v))).collect())
        }
    }
}

fn stamp(arguments: &gpui_shell::HostArguments) -> Result<Stamp, HostError> {
    let generation =
        u64::try_from(arguments.integer(0)?).map_err(|_| HostError::new("invalid generation"))?;
    let context_version = u64::try_from(arguments.integer(1)?)
        .map_err(|_| HostError::new("invalid context version"))?;
    Ok(Stamp {
        generation,
        context_version,
    })
}

fn schedule_host_refresh(host: WeakEntity<Probe>) {
    gpui_shell::with_current_app(|cx| {
        cx.defer(move |cx| {
            let _ = host.update(cx, |host, cx| {
                host.refresh_views(cx);
                cx.notify();
            });
        });
    });
}

struct Probe {
    model: Rc<RefCell<Model>>,
    runtime: Option<Rc<ShellRuntime>>,
    navigation: Option<Entity<ScriptView>>,
    summary: Option<Entity<ScriptView>>,
    error: Option<String>,
}

impl Probe {
    fn new(cx: &mut Context<Self>) -> Result<Self> {
        let model = Rc::new(RefCell::new(Model::default()));
        let read = Rc::downgrade(&model);
        let open = Rc::downgrade(&model);
        let status = Rc::downgrade(&model);
        let lifecycle = Rc::downgrade(&model);
        let open_host = cx.weak_entity();
        let status_host = cx.weak_entity();
        let module = HostModule::new("conduit-probe")
            .function("lifecycle_ping", move |_| {
                let model = lifecycle
                    .upgrade()
                    .ok_or_else(|| HostError::new("host closed"))?;
                model.borrow_mut().lifecycle_callbacks += 1;
                Ok(HostValue::Null)
            })
            .function("snapshot", move |arguments| {
                let model = read
                    .upgrade()
                    .ok_or_else(|| HostError::new("host closed"))?;
                let value = model
                    .borrow()
                    .snapshot(OWNER, arguments.string(0)?)
                    .map_err(|e| HostError::new(e.to_string()))?;
                Ok(host_value(
                    serde_json::to_value(value).map_err(|e| HostError::new(e.to_string()))?,
                ))
            })
            .function("request_summary", move |arguments| {
                let model = open
                    .upgrade()
                    .ok_or_else(|| HostError::new("host closed"))?;
                model
                    .borrow_mut()
                    .dispatch(OWNER, SHOW_SUMMARY, stamp(arguments)?)
                    .map_err(|e| HostError::new(e.to_string()))?;
                schedule_host_refresh(open_host.clone());
                Ok(HostValue::Null)
            })
            .function("publish_status", move |arguments| {
                let model = status
                    .upgrade()
                    .ok_or_else(|| HostError::new("host closed"))?;
                model
                    .borrow_mut()
                    .publish_status(OWNER, stamp(arguments)?, arguments.string(2)?)
                    .map_err(|e| HostError::new(e.to_string()))?;
                schedule_host_refresh(status_host.clone());
                Ok(HostValue::Null)
            });
        // A single trusted fixture for the lifetime of this experimental window.
        // This is NOT a per-plugin Policy injection solution; G2 remains blocked.
        gpui_shell::policy::set_default(
            gpui_shell::policy::Policy::new()
                .with_application(OWNER)
                .with_host_module(module)?,
        );
        Ok(Self {
            model,
            runtime: None,
            navigation: None,
            summary: None,
            error: None,
        })
    }

    fn activate(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Result<()> {
        self.model.borrow_mut().activate()?;
        let result = (|| {
            let runtime = gpui_shell::components::new_isolated_runtime()?;
            runtime.set_editor_files(false);
            let root = fixture_root();
            let navigation = runtime.load_application(&root, "navigation.js")?;
            let navigation = runtime.mount_application(&navigation, window, cx)?;
            let summary = runtime.load_application(&root, "summary.js")?;
            let summary = runtime.mount_application(&summary, window, cx)?;
            self.navigation = Some(navigation);
            self.summary = Some(summary);
            self.runtime = Some(runtime);
            Ok(())
        })();
        if result.is_err() {
            self.stop(cx);
        }
        cx.notify();
        result
    }

    fn refresh_views(&mut self, cx: &mut Context<Self>) {
        for view in [&self.navigation, &self.summary].into_iter().flatten() {
            view.update(cx, |view, cx| view.refresh(cx));
        }
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        self.model.borrow_mut().stop();
        self.navigation = None;
        self.summary = None;
        self.runtime = None;
        cx.notify();
    }

    fn select_workspace(&mut self, workspace: usize, window: &mut Window, cx: &mut Context<Self>) {
        let active = self.model.borrow().active;
        let right_open = self.model.borrow().right_open;
        if active {
            self.stop(cx);
        }
        let result = self.model.borrow_mut().select_workspace(workspace);
        if result.is_ok() && active {
            let result = self.activate(window, cx);
            if result.is_ok() {
                self.model.borrow_mut().right_open = right_open;
            }
            self.record(result, cx);
        } else {
            self.record(result, cx);
        }
    }

    fn show_summary(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Result<()> {
        if !self.model.borrow().active {
            self.activate(window, cx)?;
        }
        let stamp = self.model.borrow().stamp();
        self.model
            .borrow_mut()
            .dispatch(OWNER, SHOW_SUMMARY, stamp)?;
        self.refresh_views(cx);
        cx.notify();
        Ok(())
    }

    fn record(&mut self, result: Result<()>, cx: &mut Context<Self>) {
        self.error = result.err().map(|e| format!("{e:#}"));
        cx.notify();
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        self.model.borrow_mut().stop();
        gpui_shell::clear_exported_modules();
        gpui_shell::policy::set_default(gpui_shell::policy::Policy::new());
    }
}

struct Overlay;
impl Render for Overlay {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().p_6().child("Shell overlay probe — Esc to close")
    }
}

impl Render for Probe {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let model = self.model.borrow();
        let active = model.active;
        let right_open = model.right_open;
        let writable = model.writable;
        let status = model.status.clone();
        drop(model);
        let muted = cx.theme().muted_foreground;
        let mut navigation = div()
            .w(px(260.))
            .h_full()
            .flex_shrink_0()
            .border_r_1()
            .border_color(cx.theme().border)
            .debug_selector(|| "probe-navigation".into());
        if let Some(view) = &self.navigation {
            navigation = navigation.child(view.clone());
        } else {
            navigation = navigation.child(
                div()
                    .p_4()
                    .child("Open the icon or footer to activate the fixture."),
            );
        }
        let mut right = div()
            .w(px(300.))
            .h_full()
            .flex_shrink_0()
            .border_l_1()
            .border_color(cx.theme().border)
            .debug_selector(|| "probe-right".into());
        if right_open && let Some(view) = &self.summary {
            right = right.child(view.clone());
        }
        let narrow = window.viewport_size().width < px(1100.);
        let mut body = h_flex().flex_1().min_h_0().relative()
            .child(v_flex().w(px(52.)).h_full().p_1().gap_2()
                .child(Button::new("probe-activity").label("W").tooltip("Activate workspace overview")
                    .on_click(cx.listener(|this, _, window, cx| {
                        let result = if this.model.borrow().active { Ok(()) } else { this.activate(window, cx) };
                        this.record(result, cx);
                    }))))
            .child(navigation)
            .child(v_flex().flex_1().min_w_0().h_full()
                .child(v_flex().flex_1().p_4().gap_3()
                    .child("Conduit Shell integration fixture")
                    .child(div().text_color(muted).child("Synthetic workspaces; no chat history or credentials are read."))
                    .child(div().text_color(muted).child("Single trusted Policy. Multi-plugin isolation and resource limits remain unproven."))
                    .child(Button::new("probe-host-dialog").label("Host dialog").on_click(|_, window, cx| {
                        window.open_dialog(cx, |dialog, _, _| dialog.title("Host dialog").child("Component Root remains available"));
                    }))
                    .child(Button::new("probe-shell-dialog").label("Shell dialog").on_click(|_, window, cx| {
                        let overlay = cx.new(|_| Overlay);
                        ShellRoot::update(window, cx, |root, window, cx| root.open_dialog(overlay.into(), window, cx));
                    }))
                    .children(self.error.clone().map(|error| div().child(error))))
                .child(h_flex().h(px(34.)).px_2().border_t_1().border_color(cx.theme().border)
                    .debug_selector(|| "probe-footer".into()).child(
                        Button::new("probe-status").label(status).on_click(cx.listener(|this, _, window, cx| {
                            let result = this.show_summary(window, cx);
                            this.record(result, cx);
                        })))))
            ;
        if right_open {
            if narrow {
                right = right
                    .absolute()
                    .right_0()
                    .top_0()
                    .bottom_0()
                    .bg(cx.theme().background);
            }
            body = body.child(right);
        }
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                h_flex()
                    .h(px(48.))
                    .px_3()
                    .gap_2()
                    .child(
                        Button::new("probe-workspace-a")
                            .label("Workspace A")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.select_workspace(0, window, cx)
                            })),
                    )
                    .child(
                        Button::new("probe-workspace-b")
                            .label("Workspace B")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.select_workspace(1, window, cx)
                            })),
                    )
                    .child(
                        Button::new("probe-stop")
                            .label("Stop")
                            .disabled(!active)
                            .on_click(cx.listener(|this, _, _, cx| this.stop(cx))),
                    )
                    .child(
                        Button::new("probe-error")
                            .label("Render error / recover")
                            .disabled(!active)
                            .on_click(cx.listener(|this, _, _, cx| {
                                let failing = this.model.borrow().fail_render;
                                this.model.borrow_mut().fail_render = !failing;
                                this.refresh_views(cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("probe-readonly")
                            .label(if writable {
                                "Set read-only"
                            } else {
                                "Set writable"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                let writable = this.model.borrow().writable;
                                if writable {
                                    this.stop(cx);
                                }
                                this.model.borrow_mut().set_writable(!writable);
                                cx.notify();
                            })),
                    ),
            )
            .child(body)
    }
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--help") {
        println!(
            "Conduit Shell prototype: run without arguments for the synthetic UI fixture.\nUse cargo test --features ui-tests for automated integration probes."
        );
        return Ok(());
    }
    anyhow::ensure!(args.is_empty(), "unknown arguments; use --help");
    let manifest = gpui_shell::plugin::PluginManifest::read(&fixture_root())
        .context("read trusted fixture manifest without executing code")?;
    anyhow::ensure!(
        manifest.id() == OWNER && manifest.dependencies().is_empty(),
        "unexpected fixture identity or dependencies"
    );
    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(|cx| {
            gpui_kit::init(cx);
            gpui_shell::init(cx);
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::centered(size(px(1200.), px(780.)), cx)),
                    window_min_size: Some(size(px(850.), px(500.))),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    window.set_window_title("Conduit — Shell prototype");
                    let host = cx.new(|cx| Probe::new(cx).expect("prototype host module"));
                    cx.new(|cx| ShellRoot::new(host.into(), window, cx))
                },
            )
            .expect("open prototype window");
            cx.activate(true);
        });
    Ok(())
}

#[cfg(all(test, feature = "ui-tests"))]
mod ui_tests;
