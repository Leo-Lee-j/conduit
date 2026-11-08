use super::*;

#[derive(Clone, Debug, Default)]
pub(super) struct GraphRow {
    pub node: usize,
    pub incoming: bool,
    pub through: Vec<(usize, usize)>,
    pub parents: Vec<usize>,
    pub lanes: usize,
}

pub(super) fn layout(commits: &[git::LogCommit]) -> Vec<GraphRow> {
    let mut lanes: Vec<String> = Vec::new();
    commits
        .iter()
        .map(|commit| {
            let incoming = lanes.iter().position(|oid| oid == &commit.oid);
            let node = incoming.unwrap_or_else(|| {
                lanes.push(commit.oid.clone());
                lanes.len() - 1
            });
            let before = lanes.clone();
            lanes.remove(node);
            for (index, parent) in commit.parents.iter().enumerate() {
                if !lanes.contains(parent) {
                    if index == 0 {
                        lanes.insert(node.min(lanes.len()), parent.clone());
                    } else {
                        lanes.push(parent.clone());
                    }
                }
            }
            GraphRow {
                node,
                incoming: incoming.is_some(),
                through: before
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| *i != node)
                    .filter_map(|(from, oid)| {
                        lanes.iter().position(|p| p == oid).map(|to| (from, to))
                    })
                    .collect(),
                parents: commit
                    .parents
                    .iter()
                    .filter_map(|p| lanes.iter().position(|oid| oid == p))
                    .collect(),
                lanes: before.len().max(lanes.len()),
            }
        })
        .collect()
}

fn color(lane: usize) -> u32 {
    [
        palette().git_modified_text,
        palette().git_added_text,
        0xb897d9,
        0xd1a351,
        0x62b7b2,
    ][lane % 5]
}

pub(super) fn render(row: GraphRow, width: f32) -> AnyElement {
    let node = row.node;
    div()
        .relative()
        .w(px(width))
        .h(px(30.))
        .flex_shrink_0()
        .overflow_hidden()
        .child(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    let x = |lane| bounds.left() + px(10. + lane as f32 * 12.);
                    let top = bounds.top();
                    let mid = top + px(15.);
                    let bottom = bounds.bottom();
                    let mut line = |from, to, stroke| {
                        let mut path = gpui_kit::PathBuilder::stroke(px(1.5));
                        path.move_to(from);
                        path.line_to(to);
                        if let Ok(path) = path.build() {
                            window.paint_path(path, rgb(stroke));
                        }
                    };
                    for (from, to) in row.through {
                        line(point(x(from), top), point(x(to), bottom), color(from));
                    }
                    if row.incoming {
                        line(point(x(node), top), point(x(node), mid), color(node));
                    }
                    for parent in row.parents {
                        line(point(x(node), mid), point(x(parent), bottom), color(node));
                    }
                },
            )
            .absolute()
            .size_full(),
        )
        .child(
            div()
                .absolute()
                .left(px(6.5 + node as f32 * 12.))
                .top(px(11.5))
                .size(px(7.))
                .rounded_full()
                .bg(rgb(color(node))),
        )
        .into_any_element()
}
