use crate::app::{ConduitApp, DocumentTab, workspace_color_rgba};
use crate::{domain::Workspace, store::Store};
use gpui_kit::component::tab::folder_tab_border_geometry;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{Colorize as _, Theme, ThemeMode};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, Bounds, Context, Entity, InteractiveElement as _, IntoElement, Modifiers,
    MouseButton, Pixels, Point, Render, TestAppContext, VisualTestContext, Window, point, px, size,
};

fn fixture(cx: &mut TestAppContext) -> (Entity<ConduitApp>, &mut VisualTestContext) {
    cx.update(gpui_kit::init);
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let output = slot.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let first = crate::app::tests::session();
            let mut second = first.clone();
            second.id = 2;
            let store = Store {
                workspaces: vec![
                    Workspace {
                        path: std::env::temp_dir().join("conduit-color-first"),
                        sessions: vec![first],
                    },
                    Workspace {
                        path: std::env::temp_dir().join("conduit-color-second"),
                        sessions: vec![second],
                    },
                ],
                ..Default::default()
            };
            let mut app = ConduitApp::with_store(window, cx, Ok(store), false, false);
            app.startup = None;
            app.persistence = crate::persistence::Persistence::with_writer(|_| Ok(()));
            app.open_tabs = vec![DocumentTab::Chat(1), DocumentTab::Chat(2)];
            app
        });
        *output.borrow_mut() = Some(app.clone());
        gpui_kit::component::Root::new(app, window, cx)
    });
    let app = slot.borrow().as_ref().unwrap().clone();
    cx.simulate_resize(size(px(1100.), px(700.)));
    (app, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
}

fn click(cx: &mut VisualTestContext, position: Point<Pixels>) {
    cx.simulate_mouse_move(position, None, Modifiers::default());
    draw(cx);
    cx.simulate_mouse_down(position, MouseButton::Left, Modifiers::default());
    // Native pointer events can have a rendered frame between press and release.
    draw(cx);
    cx.simulate_mouse_up(position, MouseButton::Left, Modifiers::default());
    draw(cx);
}

#[gpui_kit::test]
fn workspace_tab_border_spans_curves_and_follows_tab_selection(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx);
    draw(cx);
    let original = [
        cx.debug_bounds("session-tab-1").unwrap(),
        cx.debug_bounds("session-tab-2").unwrap(),
    ];
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            for (index, color) in [Theme::global(cx).blue, Theme::global(cx).red]
                .into_iter()
                .enumerate()
            {
                let root = app.store.workspaces[index].path.clone();
                app.set_workspace_color(&root, Some(workspace_color_rgba(color)), cx);
            }
        });
    });
    draw(cx);
    for (index, (tab, border)) in [
        ("session-tab-1", "folder-tab-border-0"),
        ("session-tab-2", "folder-tab-border-1"),
    ]
    .into_iter()
    .enumerate()
    {
        let tab = cx.debug_bounds(tab).unwrap();
        assert_eq!(tab, original[index], "color must not change tab layout");
        let border = cx.debug_bounds(border).expect("colored curved border");
        assert_eq!(border.top(), tab.top());
        assert_eq!(border.bottom(), tab.bottom());
        assert!(border.left() < tab.left());
        assert!(border.right() > tab.right());
        assert!(
            border.size.height > px(20.),
            "not an inset horizontal stripe"
        );
    }
    for id in [2, 1] {
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.select_document(DocumentTab::Chat(id), window, cx)
            });
        });
        draw(cx);
        cx.update(|_, cx| assert_eq!(app.read(cx).selected_session, Some(id)));
        assert!(cx.debug_bounds("folder-tab-border-0").is_some());
        assert!(cx.debug_bounds("folder-tab-border-1").is_some());
    }
}

#[gpui_kit::test]
fn workspace_tab_border_keeps_custom_colors_across_themes(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx);
    // Keep both the hue and alpha of opaque red and translucent pink.
    let colors = [0xff0000ff, 0xff008080];
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            for (index, color) in colors.into_iter().enumerate() {
                let root = app.store.workspaces[index].path.clone();
                app.set_workspace_color(&root, Some(color), cx);
            }
        });
    });
    for (light, dark) in [
        ("Slate & Teal", "Deep Slate & Teal"),
        ("Graphite & Indigo", "Graphite & Indigo Dark"),
        ("Warm Paper & Terracotta", "Warm Charcoal & Terracotta"),
        ("Conduit Light", "Conduit Dark"),
    ] {
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            let name = if mode.is_dark() { dark } else { light };
            cx.update(|_, cx| {
                crate::app::apply_theme(mode, light, dark, cx);
                app.update(cx, |_, cx| cx.notify());
                assert_eq!(Theme::global(cx).theme_name().as_ref(), name);
            });
            draw(cx);
            cx.update(|_, cx| {
                let app = app.read(cx);
                for (workspace, color) in app.store.workspaces.iter().zip(colors) {
                    let stored = app.store.ui.workspace_colors.get(&workspace.path).copied();
                    assert_eq!(stored, Some(color), "{name}: saved color is unchanged");
                    assert_eq!(
                        crate::app::theme::project_marker_color(&workspace.path, stored),
                        Some(color),
                        "{name}: tab border must use the explicit workspace color"
                    );
                }
            });
            assert!(cx.debug_bounds("folder-tab-border-0").is_some());
            assert!(cx.debug_bounds("folder-tab-border-1").is_some());
        }
    }
}

#[gpui_kit::test]
fn workspace_tab_border_restores_theme_fallback_after_clearing_color(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx);
    cx.update(|_, cx| {
        crate::app::apply_theme(ThemeMode::Light, "Slate & Teal", "Deep Slate & Teal", cx);
        app.update(cx, |_, cx| cx.notify());
    });
    draw(cx);
    let (root, fallback) = cx.update(|_, cx| {
        assert_eq!(Theme::global(cx).theme_name().as_ref(), "Slate & Teal");
        let app = app.read(cx);
        assert!(app.store.ui.workspace_colors.is_empty());
        let root = app.store.workspaces[0].path.clone();
        let fallback = crate::app::theme::project_marker_color(&root, None)
            .expect("paired themes provide an automatic project color");
        assert_eq!(
            crate::app::theme::project_marker_color(&root, None),
            Some(fallback),
            "the automatic project color is stable"
        );
        (root, fallback)
    });
    assert!(cx.debug_bounds("folder-tab-border-0").is_some());
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.set_workspace_color(&root, Some(0xff0000ff), cx);
            app.set_workspace_color(&root, None, cx);
            assert!(!app.store.ui.workspace_colors.contains_key(&root));
            assert_eq!(
                crate::app::theme::project_marker_color(&root, None),
                Some(fallback)
            );
        });
    });
    draw(cx);
    assert!(cx.debug_bounds("folder-tab-border-0").is_some());
    cx.update(|_, cx| {
        crate::app::apply_theme(ThemeMode::Light, "Conduit Light", "Conduit Dark", cx);
        app.update(cx, |_, cx| cx.notify());
        assert_eq!(crate::app::theme::project_marker_color(&root, None), None);
    });
    draw(cx);
    assert!(cx.debug_bounds("folder-tab-border-0").is_none());
}

struct ColorBorderTabs {
    selected: usize,
    colored: bool,
}

impl Render for ColorBorderTabs {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        TabBar::new("color-border-tabs")
            .folder()
            .selected_index(self.selected)
            .children(
                [
                    ("First", "border-first-tab"),
                    ("Second", "border-second-tab"),
                ]
                .into_iter()
                .enumerate()
                .map(|(index, (label, selector))| {
                    Tab::new()
                        .label(label)
                        .debug_selector(move || selector.into())
                        .when(self.colored, |tab| {
                            tab.folder_border_color(Theme::global(cx).blue)
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.selected = index;
                            cx.notify();
                        }))
                }),
            )
    }
}

#[gpui_kit::test]
fn workspace_tab_border_does_not_intercept_pointer_clicks(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let (tabs, cx) = cx.add_window_view(|_, _| ColorBorderTabs {
        selected: 0,
        colored: false,
    });
    for colored in [false, true] {
        cx.update(|_, cx| {
            tabs.update(cx, |tabs, cx| {
                tabs.selected = 0;
                tabs.colored = colored;
                cx.notify();
            })
        });
        draw(cx);
        let tab = cx.debug_bounds("border-second-tab").unwrap();
        cx.simulate_click(tab.center(), Modifiers::default());
        draw(cx);
        cx.update(|_, cx| assert_eq!(tabs.read(cx).selected, 1, "colored: {colored}"));
    }
}

fn path_contains(path: &gpui_kit::Path<Pixels>, p: Point<Pixels>) -> bool {
    let cross = |a: Point<Pixels>, b: Point<Pixels>, p: Point<Pixels>| {
        f32::from(b.x - a.x) * f32::from(p.y - a.y) - f32::from(b.y - a.y) * f32::from(p.x - a.x)
    };
    path.vertices.as_chunks::<3>().0.iter().any(|vertices| {
        let [a, b, c] = [
            vertices[0].xy_position,
            vertices[1].xy_position,
            vertices[2].xy_position,
        ];
        if cross(a, b, c).abs() < f32::EPSILON {
            return false;
        }
        let signs = [cross(a, b, p), cross(b, c, p), cross(c, a, p)];
        signs.iter().all(|sign| *sign >= -0.001) || signs.iter().all(|sign| *sign <= 0.001)
    })
}

#[test]
fn workspace_tab_border_tracks_curves_without_filling_or_a_bottom_seam() {
    let bounds = Bounds::new(point(px(0.), px(0.)), size(px(216.), px(32.)));
    let (path, _) = folder_tab_border_geometry(bounds, px(8.), gpui_kit::rgb(0x4a82db).into(), 1.)
        .expect("folder border tessellates");
    for (x, y) in [
        (108., 0.),
        (10.343, 2.343),
        (205.657, 2.343),
        (8., 16.),
        (208., 16.),
        (5.657, 29.657),
        (210.343, 29.657),
    ] {
        assert!(
            path_contains(&path, point(px(x), px(y))),
            "missing curve at {x}, {y}"
        );
    }
    for (x, y) in [(108., 2.), (108., 16.), (108., 32.)] {
        assert!(
            !path_contains(&path, point(px(x), px(y))),
            "unexpected fill/seam at {x}, {y}"
        );
    }
    assert!(path.bounds.top() < bounds.top());
    assert!(path.bounds.bottom() > bounds.bottom());
}

#[test]
fn workspace_tab_border_fades_down_to_transparent_without_changing_hue() {
    let color = gpui_kit::hsla(0.6, 0.8, 0.5, 0.6);
    // GPUI serializes HSLA as 8-bit RGB(A), so round-trip hue and alpha can
    // differ slightly from the original float components.
    let roundtrip: gpui_kit::Hsla =
        serde_json::from_value(serde_json::to_value(color).unwrap()).unwrap();
    let bounds = Bounds::new(point(px(0.), px(0.)), size(px(216.), px(32.)));
    for strength in [1., 0.55, 0.3] {
        let (_, border) = folder_tab_border_geometry(bounds, px(8.), color, strength).unwrap();
        let background = serde_json::to_value(border).unwrap();
        assert_eq!(background["gradient_angle_or_pattern_height"], 180.);
        let [top, bottom]: [gpui_kit::LinearColorStop; 2] =
            serde_json::from_value(background["colors"].clone()).unwrap();
        assert_eq!(top.percentage, 0.);
        assert!(bottom.percentage > 0.9 && bottom.percentage < 1.);
        assert!((top.color.a - color.a * strength).abs() <= 1. / 255. + f32::EPSILON);
        assert_eq!(bottom.color.a, 0.);
        for stop in [top, bottom] {
            assert_eq!(
                (stop.color.h, stop.color.s, stop.color.l),
                (roundtrip.h, roundtrip.s, roundtrip.l)
            );
        }
    }
}

#[gpui_kit::test]
fn workspace_color_mouse_selection_updates_its_workspace_and_tab(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx);
    draw(cx);

    let trigger = cx.debug_bounds("workspace-color-trigger-1").unwrap();
    let original_name = cx.debug_bounds("workspace-name-1").unwrap();
    cx.simulate_mouse_move(original_name.center(), None, Modifiers::default());
    draw(cx);
    cx.update(|_, cx| assert!(!app.read(cx).workspace_color_picker.read(cx).is_open()));
    assert_eq!(cx.debug_bounds("workspace-name-1").unwrap(), original_name);
    let session_visible = cx.debug_bounds("session-row-2").is_some();
    click(cx, trigger.center());
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert_eq!(app.selected_workspace, Some(0));
        assert_eq!(
            app.workspace_color_target.as_ref(),
            Some(&app.store.workspaces[1].path),
            "clicking the folder slot must bind the picker before selection"
        );
        assert_eq!(app.selected_session, Some(1));
        assert!(app.workspace_color_picker.read(cx).is_open());
    });
    assert_eq!(cx.debug_bounds("session-row-2").is_some(), session_visible);

    assert!(cx.debug_bounds("color-picker").is_some());
    assert_eq!(cx.debug_bounds("workspace-name-1").unwrap(), original_name);
    let color = cx.update(|_, cx| Theme::global(cx).red);
    let swatch_selector =
        Box::leak(format!("color-picker-swatch-{}", color.to_hex()).into_boxed_str());
    let swatch = cx.debug_bounds(swatch_selector).expect("palette swatch");
    cx.simulate_mouse_move(swatch.center(), None, Modifiers::default());
    draw(cx);
    cx.simulate_mouse_down(swatch.center(), MouseButton::Left, Modifiers::default());
    draw(cx);
    assert!(
        cx.debug_bounds(swatch_selector).is_some(),
        "the palette must remain mounted before mouse release"
    );
    cx.simulate_mouse_up(swatch.center(), MouseButton::Left, Modifiers::default());
    draw(cx);

    let rgba = workspace_color_rgba(color);
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert_eq!(
            app.store
                .ui
                .workspace_colors
                .get(&app.store.workspaces[1].path),
            Some(&rgba)
        );
        assert!(
            !app.store
                .ui
                .workspace_colors
                .contains_key(&app.store.workspaces[0].path)
        );
        assert!(!app.workspace_color_picker.read(cx).is_open());
        assert!(app.dirty);
    });
    assert!(cx.debug_bounds("folder-tab-border-1").is_some());
    assert!(cx.debug_bounds("folder-tab-border-0").is_none());

    let first_trigger = cx.debug_bounds("workspace-color-trigger-0").unwrap();
    click(cx, first_trigger.center());
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert_eq!(
            app.workspace_color_target.as_ref(),
            Some(&app.store.workspaces[0].path)
        );
        assert!(app.workspace_color_picker.read(cx).value().is_none());
    });
    let blue = cx.update(|_, cx| Theme::global(cx).blue);
    let blue_selector =
        Box::leak(format!("color-picker-swatch-{}", blue.to_hex()).into_boxed_str());
    let blue_swatch = cx.debug_bounds(blue_selector).unwrap();
    click(cx, blue_swatch.center());
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert_eq!(
            app.store
                .ui
                .workspace_colors
                .get(&app.store.workspaces[0].path),
            Some(&workspace_color_rgba(blue))
        );
        assert_eq!(
            app.store
                .ui
                .workspace_colors
                .get(&app.store.workspaces[1].path),
            Some(&rgba)
        );
    });
    assert!(cx.debug_bounds("folder-tab-border-0").is_some());
    assert!(cx.debug_bounds("folder-tab-border-1").is_some());

    let menu = cx.debug_bounds("workspace-menu-trigger-1").unwrap();
    click(cx, menu.center());
    assert!(
        cx.debug_bounds("color-picker").is_none(),
        "the menu has no picker"
    );
    let clear = cx
        .debug_bounds("workspace-clear-color-1")
        .expect("clear color action");
    click(cx, clear.center());
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert!(
            !app.store
                .ui
                .workspace_colors
                .contains_key(&app.store.workspaces[1].path)
        );
        assert_eq!(
            app.store
                .ui
                .workspace_colors
                .get(&app.store.workspaces[0].path),
            Some(&workspace_color_rgba(blue))
        );
        assert_eq!(app.workspace_color_picker.read(cx).value(), Some(blue));
    });
    assert!(cx.debug_bounds("folder-tab-border-0").is_some());
    assert!(cx.debug_bounds("folder-tab-border-1").is_none());
}

#[gpui_kit::test]
fn workspace_color_picker_dismisses_and_reopens_without_navigating(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx);
    draw(cx);
    let trigger = cx.debug_bounds("workspace-color-trigger-1").unwrap();
    for escape in [true, false] {
        click(cx, trigger.center());
        cx.update(|_, cx| assert!(app.read(cx).workspace_color_picker.read(cx).is_open()));
        if escape {
            cx.simulate_keystrokes("escape");
            draw(cx);
        } else {
            click(cx, point(px(900.), px(400.)));
        }
        cx.update(|_, cx| {
            let app = app.read(cx);
            assert!(!app.workspace_color_picker.read(cx).is_open());
            assert_eq!(app.selected_workspace, Some(0));
            assert_eq!(app.selected_session, Some(1));
            assert!(app.store.ui.workspace_colors.is_empty());
        });
        assert!(cx.debug_bounds("color-picker").is_none());
        assert_eq!(
            cx.debug_bounds("workspace-color-trigger-1").unwrap(),
            trigger
        );
    }
}
