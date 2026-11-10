use super::{Arc, AtomicBool, ConduitApp, Ordering, Store, rendered};
use gpui_kit::{AppContext as _, TestAppContext};

#[gpui_kit::test]
fn readonly_background_changes_preserve_preferences(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    cx.add_window_view(|window, cx| {
        let app =
            cx.new(|cx| ConduitApp::with_store(window, cx, Ok(Store::default()), false, false));
        app.update(cx, |app, cx| {
            app.writable = false;
            let before = app.store.ui.background.clone();
            app.adjust_background_strength(5, cx);
            app.set_background_enabled(true, window, cx);
            assert_eq!(app.store.ui.background, before);
            assert!(app.background.operation.is_none());
        });
        gpui_kit::component::Root::new(app, window, cx)
    });
}

#[gpui_kit::test]
fn disabling_background_cancels_pending_apply_and_releases_pixels(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    cx.add_window_view(|window, cx| {
        let app =
            cx.new(|cx| ConduitApp::with_store(window, cx, Ok(Store::default()), false, false));
        app.update(cx, |app, cx| {
            let cancel = Arc::new(AtomicBool::new(false));
            app.background.operation = Some(cancel.clone());
            app.store.ui.background.enabled = true;
            app.store.ui.background.id = Some("aaaaaa".into());
            app.background.layer.update(cx, |layer, cx| {
                layer.replace(Some(rendered(image::RgbaImage::new(4, 4))), 15, cx)
            });
            app.set_background_enabled(false, window, cx);
            assert!(cancel.load(Ordering::Acquire));
            assert!(!app.store.ui.background.enabled);
            assert_eq!(app.store.ui.background.id.as_deref(), Some("aaaaaa"));
            assert!(app.background.layer.read(cx).image.is_none());
            // Cannot start another restore until the old operation has really returned.
            app.set_background_enabled(true, window, cx);
            assert!(!app.store.ui.background.enabled);
            assert!(Arc::ptr_eq(
                app.background.operation.as_ref().unwrap(),
                &cancel
            ));
            app.background.operation = None;
        });
        gpui_kit::component::Root::new(app, window, cx)
    });
}

#[gpui_kit::test]
fn failed_cancelled_and_stale_downloads_keep_the_previous_background(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    cx.add_window_view(|window, cx| {
        let app =
            cx.new(|cx| ConduitApp::with_store(window, cx, Ok(Store::default()), false, false));
        app.update(cx, |app, cx| {
            let previous = rendered(image::RgbaImage::new(4, 4));
            app.background.layer.update(cx, |layer, cx| {
                layer.replace(Some(previous.clone()), 15, cx)
            });
            app.store.ui.background.id = Some("aaaaaa".into());
            app.store.ui.background.enabled = true;
            let active = Arc::new(AtomicBool::new(false));
            let stale = Arc::new(AtomicBool::new(false));
            app.background.operation = Some(active.clone());
            app.finish_background(
                "bbbbbb".into(),
                true,
                &stale,
                Ok(image::RgbaImage::new(8, 8)),
                cx,
            );
            assert!(Arc::ptr_eq(
                app.background.operation.as_ref().unwrap(),
                &active
            ));
            app.finish_background(
                "bbbbbb".into(),
                true,
                &active,
                Err(anyhow::anyhow!("download failed")),
                cx,
            );
            assert!(app.background.error.is_some());
            assert!(app.background.operation.is_none());
            active.store(true, Ordering::Release);
            app.background.operation = Some(active.clone());
            app.finish_background(
                "bbbbbb".into(),
                true,
                &active,
                Ok(image::RgbaImage::new(8, 8)),
                cx,
            );
            assert_eq!(app.store.ui.background.id.as_deref(), Some("aaaaaa"));
            assert!(Arc::ptr_eq(
                app.background.layer.read(cx).image.as_ref().unwrap(),
                &previous
            ));
            assert!(app.background.operation.is_none());
            app.writable = false;
        });
        gpui_kit::component::Root::new(app, window, cx)
    });
}
