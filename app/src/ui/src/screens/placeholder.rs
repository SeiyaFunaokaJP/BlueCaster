//! A screen that is not ported yet: its name alone

use gpui_kit::{Context, IntoElement, ParentElement as _, Render, Window};

use crate::app::RouteName;
use crate::ui;

pub struct Placeholder(RouteName);

impl Placeholder {
    pub fn new(name: RouteName) -> Placeholder {
        Placeholder(name)
    }
}

impl Render for Placeholder {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        ui::page("placeholder", self.0.as_str()).child(ui::hint("Not ported yet", cx))
    }
}
