//! Compact form controls shared by every settings page and editor.
//!
//! Native small controls supply the same 14px type and 8px horizontal inset.
//! Only the frame height is adjusted to 28px; explicit UI font scaling still
//! applies, while window size never changes control density. Keep native
//! theme/state handling so focus, disabled and open states remain functional.

use super::*;

pub(in crate::app) trait SettingsControl: Sizable + Styled {
    fn settings_control(self) -> Self {
        self.small().h(ui_text_size(28.)).rounded(px(4.))
    }
}

impl<T: Sizable + Styled> SettingsControl for T {}

pub(in crate::app) fn settings_input(state: &Entity<InputState>) -> Input {
    // A minimum, rather than a fixed height, keeps textareas and code editors
    // free to grow or use their explicit editing height.
    Input::new(state)
        .small()
        .min_h(ui_text_size(28.))
        .rounded(px(4.))
}

pub(in crate::app) fn settings_textarea(state: &Entity<TextareaState>) -> Textarea {
    Textarea::new(state)
        .small()
        .min_h(ui_text_size(28.))
        .rounded(px(4.))
}

pub(in crate::app) fn settings_dropdown(id: impl Into<ElementId>) -> Button {
    Button::new(id)
        .settings_control()
        .outline()
        .dropdown_caret(true)
}

pub(super) fn settings_stepper_frame(cx: &App) -> Div {
    h_flex()
        .h(ui_text_size(28.))
        .items_center()
        .rounded(px(4.))
        .border_1()
        .border_color(cx.theme().input)
        .bg(cx.theme().input_background())
}
