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
        // Buttons size to their label; they never stretch across a column. Rows that need a
        // vertically centred button say `.self_center()`.
        .self_start()
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
/// A bordered surface that groups related controls.
pub fn panel() -> Div {
    div()
        .p_4()
        .rounded_lg()
        .bg(rgb(PANEL))
        .border_1()
        .border_color(rgb(BORDER))
        .flex()
        .flex_col()
        .gap_3()
}
/// A titled block inside a screen: a clear title with one line saying what it is for.
pub fn group(title: impl Into<SharedString>, hint: impl Into<SharedString>) -> Div {
    div()
        .mt_4()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_size(px(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title.into()),
        )
        .child(caption(hint))
}
fn radio(selected: bool) -> Div {
    div()
        .size(px(16.))
        .flex_shrink_0()
        .rounded_full()
        .border_1()
        .border_color(rgb(if selected { ACCENT } else { MUTED }))
        .flex()
        .items_center()
        .justify_center()
        .when(selected, |d| {
            d.child(div().size(px(8.)).rounded_full().bg(rgb(ACCENT)))
        })
}
/// One choice among several, drawn as a bordered tile with a radio mark so it reads as a control.
pub fn tile(
    id: impl Into<ElementId>,
    title: impl Into<SharedString>,
    subtitle: Option<String>,
    selected: bool,
) -> Button {
    let title = title.into();
    button_base(id, "")
        .accessibility_label(title.clone())
        .w_full()
        .min_h(px(48.))
        .px_3()
        .py_2()
        .rounded_lg()
        .justify_start()
        .gap_3()
        .border_color(rgb(if selected { ACCENT } else { BORDER }))
        .bg(rgb(if selected { SELECTED } else { PANEL }))
        .aria_selected(selected)
        .hover(move |s| s.bg(rgb(if selected { SELECTED } else { ELEVATED })))
        .child(radio(selected))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .items_start()
                .child(div().text_size(px(13.)).child(title))
                .when_some(subtitle, |d, text| d.child(caption(text))),
        )
}
fn switch(on: bool) -> Div {
    div()
        .w(px(34.))
        .h(px(20.))
        .flex_shrink_0()
        .rounded_full()
        .p(px(2.))
        .flex()
        .items_center()
        .when(on, |d| d.justify_end())
        .bg(rgb(if on { ACCENT } else { BORDER }))
        .child(
            div()
                .size(px(16.))
                .rounded_full()
                .bg(rgb(if on { 0x15223a } else { MUTED })),
        )
}
/// A whole-row on/off setting: what it does on the left, the switch on the right.
pub fn toggle_row(
    id: impl Into<ElementId>,
    title: impl Into<SharedString>,
    hint: impl Into<SharedString>,
    on: bool,
) -> Button {
    let title = title.into();
    button_base(id, "")
        .accessibility_label(title.clone())
        .w_full()
        .px_3()
        .py_3()
        .rounded_lg()
        .justify_between()
        .gap_4()
        .bg(rgb(PANEL))
        .aria_selected(on)
        .hover(|s| s.bg(rgb(ELEVATED)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .items_start()
                .gap_1()
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(FontWeight::MEDIUM)
                        .child(title),
                )
                .child(caption(hint)),
        )
        .child(switch(on))
}
/// Lays `items` out in rows of `columns` equal-width cells.
pub fn grid(columns: usize, items: Vec<AnyElement>) -> Div {
    let columns = columns.max(1);
    let mut rows = div().flex().flex_col().gap_3();
    let mut items = items.into_iter().peekable();
    while items.peek().is_some() {
        let mut row = div().flex().gap_3();
        for _ in 0..columns {
            row = row.child(match items.next() {
                Some(item) => div().flex_1().min_w_0().flex().flex_col().child(item),
                None => div().flex_1().min_w_0(),
            });
        }
        rows = rows.child(row);
    }
    rows
}
/// A one-line fact: the value large, its label small.
pub fn stat(value: impl Into<SharedString>, label: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .flex_col()
        .child(
            div()
                .text_size(px(26.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(value.into()),
        )
        .child(caption(label))
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
        "parakeet-tdt-0.6b-v2" => "Parakeet TDT v2 · English",
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
