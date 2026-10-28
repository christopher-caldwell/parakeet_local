use gpui::{prelude::*, *};
use gpui_base::Button;

pub const BACKGROUND: u32 = 0x1b1d22;
pub const SIDEBAR: u32 = 0x15171c;
pub const PANEL: u32 = 0x22252c;
pub const ELEVATED: u32 = 0x2a2e37;
pub const BORDER: u32 = 0x343944;
pub const TEXT: u32 = 0xeeeef2;
pub const MUTED: u32 = 0xa5abb7;
pub const ACCENT: u32 = 0x91b6ff;
pub const SELECTED: u32 = 0x293b59;
pub const SUCCESS: u32 = 0x90d0b1;
pub const WARNING: u32 = 0xe5bc7c;
pub const DANGER: u32 = 0xf1a4ab;

fn button_base(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
    let label = label.into();
    Button::new(id)
        .flex_shrink_0()
        .accessibility_label(label.clone())
        .px_3()
        .py_1p5()
        .min_h(px(28.))
        .rounded_md()
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .bg(rgb(ELEVATED))
        .border_1()
        .border_color(rgb(BORDER))
        .text_color(rgb(TEXT))
        .cursor_pointer()
        .focus(|s| s.border_color(rgb(ACCENT)))
        .styles(|s| s.disabled(|s| s.opacity(0.4)))
        .child(label)
}
pub fn button(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
    button_base(id, label).hover(|s| s.bg(rgb(0x353b46)))
}
pub fn primary(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
    button_base(id, label)
        .bg(rgb(ACCENT))
        .border_color(rgb(ACCENT))
        .text_color(rgb(0x15223a))
        .hover(|s| s.bg(rgb(0xafcbff)))
}
pub fn quiet(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
    button_base(id, label)
        .bg(gpui::transparent_black())
        .border_color(gpui::transparent_black())
        .text_color(rgb(MUTED))
        .hover(|s| s.bg(rgb(ELEVATED)).text_color(rgb(TEXT)))
}
/// A selected control retains its active background while hovered.
pub fn choice(id: impl Into<ElementId>, label: impl Into<SharedString>, selected: bool) -> Button {
    button_base(id, label)
        .bg(if selected {
            rgb(SELECTED).into()
        } else {
            gpui::transparent_black()
        })
        .border_color(gpui::transparent_black())
        .text_color(rgb(if selected { TEXT } else { MUTED }))
        .aria_selected(selected)
        .hover(move |s| {
            s.bg(rgb(if selected { SELECTED } else { ELEVATED }))
                .text_color(rgb(TEXT))
        })
}
pub fn danger(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
    button(id, label)
        .text_color(rgb(DANGER))
        .border_color(rgb(0x68434d))
}
pub fn pill(text: impl Into<SharedString>, color: u32) -> Div {
    div()
        .flex_shrink_0()
        .px_2()
        .py_0p5()
        .rounded_sm()
        .text_size(px(10.))
        .font_weight(FontWeight::MEDIUM)
        .bg(rgba((color << 8) | 0x16))
        .text_color(rgb(color))
        .child(text.into())
}
pub fn caption(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(11.))
        .text_color(rgb(MUTED))
        .child(text.into())
}
pub fn section(title: impl Into<SharedString>, description: impl Into<SharedString>) -> Div {
    div()
        .flex_shrink_0()
        .flex()
        .flex_col()
        .gap_1()
        .mb_3()
        .child(
            div()
                .text_size(px(22.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title.into()),
        )
        .child(caption(description))
}
pub fn icon(name: &'static str) -> Svg {
    svg().path(name).size(px(16.)).flex_shrink_0()
}
pub struct Icons;
impl AssetSource for Icons {
    fn load(&self, path: &str) -> anyhow::Result<Option<std::borrow::Cow<'static, [u8]>>> {
        let bytes: &'static [u8] = match path {
            "wave" => include_bytes!("../assets/icons/wave.svg"),
            "library" => include_bytes!("../assets/icons/library.svg"),
            "mic" => include_bytes!("../assets/icons/mic.svg"),
            "people" => include_bytes!("../assets/icons/people.svg"),
            "keyboard" => include_bytes!("../assets/icons/keyboard.svg"),
            "models" => include_bytes!("../assets/icons/models.svg"),
            "jobs" => include_bytes!("../assets/icons/jobs.svg"),
            "shield" => include_bytes!("../assets/icons/shield.svg"),
            "search" => include_bytes!("../assets/icons/search.svg"),
            "plus" => include_bytes!("../assets/icons/plus.svg"),
            "play" => include_bytes!("../assets/icons/play.svg"),
            "file" => include_bytes!("../assets/icons/file.svg"),
            "link" => include_bytes!("../assets/icons/link.svg"),
            "chevron" => include_bytes!("../assets/icons/chevron.svg"),
            "copy" => include_bytes!("../assets/icons/copy.svg"),
            "edit" => include_bytes!("../assets/icons/edit.svg"),
            "check" => include_bytes!("../assets/icons/check.svg"),
            "close" => include_bytes!("../assets/icons/close.svg"),
            _ => return Ok(None),
        };
        Ok(Some(std::borrow::Cow::Borrowed(bytes)))
    }
    fn list(&self, _: &str) -> anyhow::Result<Vec<SharedString>> {
        Ok(vec![])
    }
}

pub fn model_name(id: &str) -> &str {
    match id {
        "whisper-base.en" => "Whisper Base · English",
        "whisper-small.en" => "Whisper Small · English",
        "whisper-medium.en" => "Whisper Medium · English",
        "whisper-large-v3" => "Whisper Large v3",
        "whisper-large-v3-turbo" => "Whisper Large v3 Turbo",
        "speakrs-community-1-coreml" => "Community-1 · Core ML",
        "speakrs-community-1-cpu" => "Community-1 · CPU",
        "nemotron3-fast128" => "Nemotron · Fast",
        "nemotron3-offline" => "Nemotron · Offline",
        _ => id,
    }
}
