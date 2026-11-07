//! Opt-in counters for the G5 chat probe, compiled only into UI tests.
//! Timings include descendants; layout requests exclude Taffy's later solve.
use gpui_kit::component::text::TextView;
use gpui_kit::{
    App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement, LayoutId,
    Pixels, Window,
};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    time::{Duration, Instant},
};

type Counters = BTreeMap<&'static str, (u64, f64)>;

thread_local! {
    static COUNTERS: RefCell<Option<Counters>> = const { RefCell::new(None) };
}

pub(super) fn enabled() -> bool {
    COUNTERS.with_borrow(|counters| counters.is_some())
}

pub(in crate::app) fn record(name: &'static str, elapsed: Duration) {
    COUNTERS.with_borrow_mut(|counters| {
        if let Some(counters) = counters {
            let entry = counters.entry(name).or_default();
            entry.0 += 1;
            entry.1 += elapsed.as_secs_f64() * 1000.;
        }
    });
}

pub(in crate::app) fn begin() {
    COUNTERS.with_borrow_mut(|counters| *counters = Some(BTreeMap::new()));
}

pub(in crate::app) fn finish() -> Value {
    let counters = COUNTERS.with_borrow_mut(|counters| counters.take().unwrap());
    let mut result = json!({});
    for name in [
        "row_build",
        "markdown_request_layout",
        "markdown_prepaint",
        "markdown_paint",
    ] {
        let (count, elapsed_ms) = counters.get(name).copied().unwrap_or_default();
        result[name] = json!({"count": count, "elapsed_ms": elapsed_ms});
    }
    result
}

pub(super) struct MeasuredMarkdown(pub TextView);
impl IntoElement for MeasuredMarkdown {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for MeasuredMarkdown {
    type RequestLayoutState = <TextView as Element>::RequestLayoutState;
    type PrepaintState = <TextView as Element>::PrepaintState;
    fn id(&self) -> Option<ElementId> {
        self.0.id()
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        self.0.source_location()
    }
    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let started = Instant::now();
        let result = self.0.request_layout(id, inspector, window, cx);
        record("markdown_request_layout", started.elapsed());
        result
    }
    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let started = Instant::now();
        let result = self.0.prepaint(id, inspector, bounds, layout, window, cx);
        record("markdown_prepaint", started.elapsed());
        result
    }
    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let started = Instant::now();
        self.0
            .paint(id, inspector, bounds, layout, prepaint, window, cx);
        record("markdown_paint", started.elapsed());
    }
}

// Host attribution is separate from the historical four-counter chat schema.
thread_local! {
    static HOST: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub(in crate::app) fn begin_host() {
    begin();
    HOST.set(true);
}

pub(in crate::app) fn finish_host() -> Value {
    HOST.set(false);
    let counters = COUNTERS.with_borrow_mut(|counters| counters.take().unwrap());
    let mut result = json!({});
    for (name, (count, elapsed_ms)) in counters {
        result[name] = json!({"count": count, "elapsed_ms": elapsed_ms});
    }
    result
}

pub(in crate::app) struct HostSpan(&'static str, Option<Instant>);
impl Drop for HostSpan {
    fn drop(&mut self) {
        if let Some(started) = self.1 {
            record(self.0, started.elapsed());
        }
    }
}

pub(in crate::app) fn host_span(name: &'static str) -> HostSpan {
    HostSpan(name, HOST.get().then(Instant::now))
}

pub(in crate::app) fn host_element<E: IntoElement>(
    element: E,
    names: [&'static str; 3],
) -> gpui_kit::AnyElement {
    if HOST.get() {
        MeasuredHost(element.into_element(), names).into_any_element()
    } else {
        element.into_any_element()
    }
}

pub(super) struct MeasuredHost<E: Element>(pub E, pub [&'static str; 3]);
impl<E: Element> IntoElement for MeasuredHost<E> {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl<E: Element> Element for MeasuredHost<E> {
    type RequestLayoutState = <E as Element>::RequestLayoutState;
    type PrepaintState = <E as Element>::PrepaintState;
    fn id(&self) -> Option<ElementId> {
        self.0.id()
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        self.0.source_location()
    }
    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let started = Instant::now();
        let result = self.0.request_layout(id, inspector, window, cx);
        record(self.1[0], started.elapsed());
        result
    }
    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let started = Instant::now();
        let result = self.0.prepaint(id, inspector, bounds, layout, window, cx);
        record(self.1[1], started.elapsed());
        result
    }
    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let started = Instant::now();
        self.0
            .paint(id, inspector, bounds, layout, prepaint, window, cx);
        record(self.1[2], started.elapsed());
    }
}
