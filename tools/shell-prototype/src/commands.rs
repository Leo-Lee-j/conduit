//! Prototype routing: registry identity and workspace tickets stay in the host.
use crate::{Model, Stamp, model};
use anyhow::{Result, anyhow, ensure};
use gpui_kit::{App, Entity, Window};
use gpui_shell::{
    CommandCall, CommandReply, CommandTarget, HostValue, ShellRuntime, view::ScriptView,
};
use std::{
    cell::RefCell,
    rc::{Rc, Weak},
};

#[derive(Clone)]
struct Ticket {
    model: Weak<RefCell<Model>>,
    owner: &'static str,
    view: &'static str,
    stamp: Stamp,
}

impl Ticket {
    fn check(&self) -> Result<()> {
        let model = self.model.upgrade().ok_or_else(|| anyhow!("host closed"))?;
        model.borrow().check_view(self.owner, self.view, self.stamp)
    }
}

#[derive(Clone)]
pub(crate) struct ViewCommandTarget {
    ticket: Ticket,
    target: CommandTarget,
}

pub(crate) struct ViewCommandCall {
    ticket: Ticket,
    call: CommandCall,
}

pub(crate) struct ViewCommandReply {
    ticket: Ticket,
    reply: CommandReply,
}

impl ViewCommandTarget {
    pub(crate) fn bind(
        model: &Rc<RefCell<Model>>,
        view_id: &'static str,
        runtime: &Rc<ShellRuntime>,
        view: &Entity<ScriptView>,
        cx: &App,
    ) -> Result<Self> {
        ensure!(
            view.read(cx).policy().application() == model::OWNER,
            "view owner mismatch"
        );
        let ticket = Ticket {
            model: Rc::downgrade(model),
            owner: model::OWNER,
            view: view_id,
            stamp: model.borrow().stamp(),
        };
        ticket.check()?;
        Ok(Self {
            ticket,
            target: runtime.command_target(view, cx)?,
        })
    }

    pub(crate) fn dispatch(
        &self,
        name: &str,
        payload: HostValue,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<ViewCommandCall> {
        self.ticket.check()?;
        ensure!(
            matches!(name, "increment" | "read-after-delay"),
            "unregistered view command"
        );
        let call = self.target.dispatch(name, payload, window, cx)?;
        Ok(ViewCommandCall {
            ticket: self.ticket.clone(),
            call,
        })
    }
}

impl ViewCommandCall {
    pub(crate) async fn wait(self) -> ViewCommandReply {
        ViewCommandReply {
            ticket: self.ticket,
            reply: self.call.wait().await,
        }
    }
}

impl ViewCommandReply {
    pub(crate) fn accept(self, cx: &App) -> Result<HostValue> {
        self.ticket.check()?;
        self.reply.accept(cx)
    }
}
