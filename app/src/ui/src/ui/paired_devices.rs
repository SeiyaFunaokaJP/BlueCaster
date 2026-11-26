//! What Windows paired through an adapter about to go to WinUSB (issue #86:
//! a built-in adapter takes the keyboard and mouse with it). The setup asks
//! first when there are any; the Adapters screen's dialog names them. A list
//! that cannot be read asks nothing. Every device is named (issue #86: not
//! "and N more").

use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::{
    App, AsyncApp, Div, FontWeight, InteractiveElement as _, ParentElement as _, Styled as _, Task, div, px,
};
use ipc::{AdapterInfo, DriverKind, Method, WindowsPairedDevice};
use ui_model::i18n::t;

use super::{IconName, icon};
use crate::app;
use crate::theme::{self, ActivePalette as _};

/// The devices Windows paired through the adapter; none when it is on
/// WinUSB already or the list cannot be read
pub fn windows_devices(a: &AdapterInfo, cx: &App) -> Task<Vec<WindowsPairedDevice>> {
    if a.driver == DriverKind::Winusb {
        return Task::ready(Vec::new());
    }
    let task = app::request::<Option<Vec<WindowsPairedDevice>>>(
        Method::AdapterWindowsDevices { instance_id: a.instance_id.clone() },
        cx,
    );
    cx.spawn(async move |_: &mut AsyncApp| task.await.ok().flatten().unwrap_or_default())
}

/// The warning (issue #86): the devices first, one per line with what kind
/// they are (headphones or another Bluetooth device; Windows' name, else
/// the address), in a box in the warning colour with what happens to them;
/// then what keeps them. Nothing when there are none.
pub fn paired_devices_note(devices: &[WindowsPairedDevice], cx: &App) -> Div {
    if devices.is_empty() {
        return div();
    }
    let p = *cx.palette();
    let list =
        div().id("paired-list").flex().flex_col().gap(theme::S1).max_h(px(192.)).overflow_y_scrollbar().children(
            devices.iter().map(|d| {
                let name = if d.name.is_empty() { d.address.clone() } else { d.name.clone() };
                div()
                    .flex()
                    .items_center()
                    .gap(theme::S2)
                    .min_w_0()
                    .child(div().flex_none().text_color(p.warn_text).child(icon(if d.audio {
                        IconName::Headphones
                    } else {
                        IconName::Bluetooth
                    })))
                    // Up to two lines (`.clamp-2`)
                    .child(div().min_w_0().font_weight(FontWeight::SEMIBOLD).line_clamp(2).child(name))
            }),
        );
    div()
        .flex()
        .flex_col()
        .gap(theme::S2)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(theme::S2)
                .p(theme::S3)
                .border_1()
                .border_color(p.warn_line)
                .rounded(theme::RADIUS)
                .child(list)
                .child(div().text_color(p.warn_text).child(t("ui.adapter.paired_text"))),
        )
        .child(
            div().flex().flex_col().pl(theme::S5).text_size(theme::FS_SMALL).text_color(p.text_2).children(
                ["ui.adapter.paired_receiver", "ui.adapter.paired_keep"]
                    .map(|key| div().flex().gap(theme::S2).child("•").child(div().flex_1().child(t(key)))),
            ),
        )
}
