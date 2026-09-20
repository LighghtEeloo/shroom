use freya::prelude::*;

use super::icons::Icon;

type Rgb = (u8, u8, u8);

#[derive(Clone, Copy, PartialEq)]
pub(super) struct Palette {
    pub background: Rgb,
    pub sidebar: Rgb,
    pub surface: Rgb,
    pub text: Rgb,
    pub muted: Rgb,
    pub line: Rgb,
    pub selected: Rgb,
    pub hover: Rgb,
    pub green: Rgb,
    pub warning: Rgb,
    pub error: Rgb,
    pub error_surface: Rgb,
    pub primary: Rgb,
    pub primary_text: Rgb,
}

impl Palette {
    pub fn with_theme(theme: PreferredTheme) -> Self {
        match theme {
            PreferredTheme::Light => Self {
                background: (255, 255, 255),
                sidebar: (241, 242, 243),
                surface: (249, 250, 249),
                text: (36, 38, 37),
                muted: (102, 108, 105),
                line: (227, 230, 228),
                selected: (222, 227, 223),
                hover: (232, 235, 232),
                green: (51, 111, 67),
                warning: (141, 94, 24),
                error: (160, 53, 46),
                error_surface: (251, 241, 239),
                primary: (37, 43, 39),
                primary_text: (255, 255, 255),
            },
            PreferredTheme::Dark => Self {
                background: (31, 33, 32),
                sidebar: (39, 42, 40),
                surface: (37, 40, 38),
                text: (233, 237, 233),
                muted: (165, 174, 167),
                line: (58, 64, 59),
                selected: (56, 65, 57),
                hover: (49, 55, 50),
                green: (167, 207, 150),
                warning: (226, 177, 108),
                error: (241, 168, 157),
                error_surface: (60, 39, 36),
                primary: (223, 232, 219),
                primary_text: (31, 38, 30),
            },
        }
    }

    pub fn button(self, text: impl Into<String>, enabled: bool) -> Button {
        self.button_base(enabled)
            .child(label().text(text.into()).font_size(13.))
    }

    pub fn button_icon(self, text: impl Into<String>, icon: Icon, enabled: bool) -> Button {
        self.button_base(enabled)
            .child(icon.beside(label().text(text.into()).font_size(13.)))
    }

    fn button_base(self, enabled: bool) -> Button {
        Button::new()
            .enabled(enabled)
            .corner_radius(7.)
            .padding((8., 12.))
            .background(self.background)
            .hover_background(self.hover)
            .border_fill(self.line)
            .color(if enabled { self.text } else { self.muted })
    }

    pub fn primary(self, button: Button) -> Button {
        button
            .background(self.primary)
            .hover_background(self.primary)
            .border_fill(self.primary)
            .color(self.primary_text)
    }

    pub fn heading(self, text: impl Into<String>) -> Label {
        label()
            .text(text.into())
            .font_size(26.)
            .font_weight(FontWeight::MEDIUM)
            .color(self.text)
    }

    pub fn caption(self, text: impl Into<String>) -> Label {
        label().text(text.into()).font_size(12.).color(self.muted)
    }

    pub fn divider(self) -> Rect {
        rect()
            .width(Size::fill())
            .height(Size::px(1.))
            .background(self.line)
    }

    pub fn panel(self) -> Rect {
        rect()
            .width(Size::fill())
            .padding(18.)
            .spacing(14.)
            .corner_radius(10.)
            .border(Border::new().width(1.).fill(self.line))
            .background(self.background)
    }

    pub fn code(self, text: impl Into<String>) -> SelectableText {
        SelectableText::new()
            .span(text.into())
            .font_family("Menlo")
            .font_family("DejaVu Sans Mono")
            .font_family("monospace")
            .font_size(12.)
            .color(self.text)
            .width(Size::fill())
    }

    pub fn value(self, title: &str, value: impl Into<String>) -> Rect {
        rect()
            .width(Size::fill())
            .spacing(6.)
            .child(self.caption(title))
            .child(self.code(value))
    }

    pub fn field(
        self,
        title: &str,
        placeholder: &str,
        value: Writable<String>,
        enabled: bool,
    ) -> Rect {
        rect()
            .width(Size::fill())
            .spacing(7.)
            .child(label().text(title.to_owned()).font_size(13.))
            .child(
                Input::new(value)
                    .placeholder(placeholder.to_owned())
                    .width(Size::fill())
                    .enabled(enabled),
            )
    }
}
