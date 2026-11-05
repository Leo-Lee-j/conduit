//! Optional manifest-driven development window; keeps the G3 baseline fixture intact.
use anyhow::Result;
use conduit_shell_prototype::{
    adapter::{
        ApprovedPackage, DevelopmentHost, HostData, PreparedActivity, PreparedCommand,
        WorkspaceMetadata,
    },
    plugins::{
        context::{ContextInput, WorkspaceKey},
        contributions::Location,
        ids::{ItemId, ViewId},
        state::ViewInstanceKey,
    },
};
use gpui_kit::{
    component::{button::Button, *},
    *,
};
use gpui_shell::{ShellRoot, view::ScriptView};
use std::{path::PathBuf, time::Duration};

pub(crate) fn context(workspace: usize, writable: bool) -> (ContextInput, HostData) {
    let label = if workspace == 0 { "A" } else { "B" };
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("synthetic-workspace-{label}"));
    (
        ContextInput {
            workspace: Some(WorkspaceKey::from_host_path(path).unwrap()),
            writable,
            ..Default::default()
        },
        HostData {
            workspace: Some(WorkspaceMetadata {
                id: format!("workspace-{label}"),
                name: format!("Workspace {label}"),
                session_count: if workspace == 0 { 3 } else { 7 },
            }),
        },
    )
}

pub(crate) struct DevelopmentProbe {
    pub(crate) host: DevelopmentHost,
    pub(crate) navigation: Option<(ViewInstanceKey, Entity<ScriptView>)>,
    pub(crate) summary: Option<(ViewInstanceKey, Entity<ScriptView>)>,
    workspace: usize,
    writable: bool,
    pub(crate) focus: FocusHandle,
    pub(crate) result: String,
    pub(crate) error: Option<String>,
    _poll: Task<()>,
}
impl DevelopmentProbe {
    pub(crate) fn new(packages: Vec<ApprovedPackage>, cx: &mut Context<Self>) -> Result<Self> {
        let (input, data) = context(0, true);
        let host = DevelopmentHost::new(packages, input, data)?;
        let poll = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                if this
                    .update(cx, |this, cx| {
                        let faults = this.host.reap_faults();
                        if !faults.is_empty() {
                            this.error = Some(
                                faults
                                    .iter()
                                    .map(|(owner, error)| format!("{owner}: {error}"))
                                    .collect::<Vec<_>>()
                                    .join("; "),
                            );
                            this.prune_views();
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Ok(Self {
            host,
            navigation: None,
            summary: None,
            workspace: 0,
            writable: true,
            focus: cx.focus_handle(),
            result: "Open a declared view or execute a status command to activate.".into(),
            error: None,
            _poll: poll,
        })
    }
    fn record(&mut self, result: Result<()>, cx: &mut Context<Self>) {
        self.error = result.err().map(|e| format!("{e:#}"));
        self.prune_views();
        cx.notify();
    }
    fn prune_views(&mut self) {
        for slot in [&mut self.navigation, &mut self.summary] {
            if slot
                .as_ref()
                .is_some_and(|(key, _)| self.host.view(key).is_none())
            {
                *slot = None;
            }
        }
    }
    fn mount(&mut self, key: ViewInstanceKey, view: Entity<ScriptView>) -> Result<()> {
        match self.host.location(&key.view)? {
            Location::Navigation => self.navigation = Some((key, view)),
            Location::Right => self.summary = Some((key, view)),
        }
        self.host.set_visible(
            &[&self.navigation, &self.summary]
                .into_iter()
                .filter_map(|slot| slot.as_ref().map(|(key, _)| key.clone()))
                .collect::<Vec<_>>(),
        );
        Ok(())
    }
    fn open(&mut self, id: &ViewId, window: &mut Window, cx: &mut Context<Self>) -> Result<()> {
        let (key, view) = self.host.open_view(id, window, cx)?;
        self.mount(key, view)
    }
    fn execute(
        &mut self,
        invocation: PreparedCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let pending = self.host.dispatch(invocation, window, cx)?;
        let key = pending.target().clone();
        let view = self.host.view(&key).expect("dispatch mounted target");
        self.mount(key, view)?;
        cx.spawn(async move |this, cx| {
            let reply = pending.wait().await;
            let _ = this.update(cx, |this, cx| {
                match this.host.accept(reply, cx) {
                    Ok(Some(value)) => this.result = format!("Command returned: {value:?}"),
                    Ok(None) => {}
                    Err(error) => this.error = Some(format!("{error:#}")),
                }
                this.prune_views();
                cx.notify();
            });
        })
        .detach();
        Ok(())
    }
    fn activity(&mut self, id: &ItemId, window: &mut Window, cx: &mut Context<Self>) -> Result<()> {
        match self.host.prepare_activity(id)? {
            PreparedActivity::OpenView(request) => self.open(&request.key().view, window, cx),
            PreparedActivity::Command(invocation) => self.execute(invocation, window, cx),
        }
    }
    pub(crate) fn switch(&mut self, workspace: usize, cx: &mut Context<Self>) -> Result<()> {
        self.workspace = workspace;
        let (input, data) = context(self.workspace, self.writable);
        self.host.set_context(input, data, cx)?;
        self.prune_views();
        self.result = "Context changed. Open a view to activate its new instance.".into();
        Ok(())
    }
    pub(crate) fn stop(&mut self) {
        self.host.stop_all();
        self.navigation = None;
        self.summary = None;
        self.result = "Development plugins stopped.".into();
    }
}

impl Render for DevelopmentProbe {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let activities = self.host.activities();
        let views = self.host.views();
        let statuses = self.host.statuses();
        let border = cx.theme().border;
        v_flex().size_full().bg(cx.theme().background).text_color(cx.theme().foreground)
            .child(h_flex().p_2().gap_2()
                .child(Button::new("dev-workspace-a").label("Workspace A").on_click(cx.listener(|this, _, _, cx| { let result = this.switch(0,cx); this.record(result,cx); })))
                .child(Button::new("dev-workspace-b").label("Workspace B").on_click(cx.listener(|this, _, _, cx| { let result = this.switch(1,cx); this.record(result,cx); })))
                .child(Button::new("dev-stop").label("Stop").on_click(cx.listener(|this,_,_,cx| { this.stop(); cx.notify(); })))
                .child(Button::new("dev-readonly").label(if self.writable {"Set read-only"} else {"Set writable"}).on_click(cx.listener(|this,_,_,cx| {
                    this.writable = !this.writable; let result = this.switch(this.workspace,cx); this.record(result,cx);
                }))))
            .child(h_flex().flex_1().min_h_0()
                .child(v_flex().w(px(290.)).h_full().border_r_1().border_color(border)
                    .children(activities.into_iter().map(|item| {
                        let id=item.id.clone();
                        Button::new(SharedString::from(format!("dev-activity-{}",item.id))).label(item.title)
                            .on_click(cx.listener(move |this,_,window,cx| { let result=this.activity(&id,window,cx); this.record(result,cx); }))
                    }))
                    .children(self.navigation.as_ref().map(|(_,view)|view.clone())))
                .child(v_flex().flex_1().min_w_0().h_full()
                    .child(v_flex().flex_1().p_4().gap_3().track_focus(&self.focus)
                        .child("Conduit development plugin")
                        .child("Synthetic context; package declarations supply the entries below.")
                        .child(Button::new("dev-host-focus").label("Focus host content").on_click(cx.listener(|this,_,window,cx|window.focus(&this.focus,cx))))
                        .children(views.into_iter().map(|view| {
                            let id=view.id.clone();
                            Button::new(SharedString::from(format!("dev-view-{}",view.id))).label(format!("Open {}",view.title))
                                .on_click(cx.listener(move |this,_,window,cx|{let result=this.open(&id,window,cx);this.record(result,cx);} ))
                        }))
                        .child(div().child(self.result.clone()))
                        .children(self.error.clone().map(|e|div().child(e))))
                    .child(h_flex().h(px(34.)).border_t_1().border_color(border).children(statuses.into_iter().map(|item| {
                        let id=item.id.clone();
                        Button::new(SharedString::from(format!("dev-status-{}",item.id))).label(item.initial.text).disabled(item.command.is_none())
                            .on_click(cx.listener(move |this,_,window,cx| {
                                let result=this.host.prepare_status(&id).and_then(|invocation|this.execute(invocation,window,cx));
                                this.record(result,cx);
                            }))
                    }))))
                .child(v_flex().w(px(300.)).h_full().border_l_1().border_color(border).children(self.summary.as_ref().map(|(_,view)|view.clone()))))
    }
}

pub(crate) fn run(packages: Vec<ApprovedPackage>) {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            gpui_shell::init(cx);
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::centered(size(px(1200.), px(780.)), cx)),
                    ..Default::default()
                },
                cx,
                move |window, cx| {
                    window.set_window_title("Conduit — Development plugin");
                    let host = cx.new(|cx| {
                        DevelopmentProbe::new(packages, cx).expect("validated development packages")
                    });
                    cx.new(|cx| ShellRoot::new(host.into(), window, cx))
                },
            )
            .expect("open development window");
            cx.activate(true);
        });
}
