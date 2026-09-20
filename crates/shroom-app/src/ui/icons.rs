use freya::{icons::lucide, prelude::*};

use super::style::Palette;

/// The approved preview’s Lucide symbols, provided by Freya’s official icon library.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Icon {
    SquarePen,
    Folder,
    Settings,
    PanelLeft,
    Box,
    Refresh,
    Ellipsis,
    PanelRight,
    Stop,
    Trash,
    History,
    ShieldCheck,
    ShieldQuestion,
    Terminal,
    Copy,
    Plus,
    Play,
    Sliders,
    Laptop,
    Cpu,
    CircleCheck,
    CircleAlert,
}

impl Icon {
    fn source(self) -> ImageSource {
        match self {
            Self::SquarePen => lucide::square_pen(),
            Self::Folder => lucide::folder(),
            Self::Settings => lucide::settings_2(),
            Self::PanelLeft => lucide::panel_left(),
            Self::Box => lucide::icon_box(),
            Self::Refresh => lucide::refresh_cw(),
            Self::Ellipsis => lucide::ellipsis(),
            Self::PanelRight => lucide::panel_right(),
            Self::Stop => lucide::square(),
            Self::Trash => lucide::trash_2(),
            Self::History => lucide::history(),
            Self::ShieldCheck => lucide::shield_check(),
            Self::ShieldQuestion => lucide::shield_question_mark(),
            Self::Terminal => lucide::terminal(),
            Self::Copy => lucide::copy(),
            Self::Plus => lucide::plus(),
            Self::Play => lucide::play(),
            Self::Sliders => lucide::sliders_horizontal(),
            Self::Laptop => lucide::laptop(),
            Self::Cpu => lucide::cpu(),
            Self::CircleCheck => lucide::circle_check(),
            Self::CircleAlert => lucide::circle_alert(),
        }
        .into()
    }

    pub fn view(self) -> SvgViewer {
        SvgViewer::new(self.source())
            .width(Size::px(16.))
            .height(Size::px(16.))
            .show_loader(false)
            .a11y_builder(|node| node.set_hidden())
    }

    pub fn beside(self, content: impl IntoElement) -> Rect {
        rect()
            .horizontal()
            .cross_align(Alignment::Center)
            .spacing(7.)
            .child(self.view())
            .child(content)
    }
}

/// Freya's stock Button does not expose accessibility labels or expanded state.
/// Keep those on the focusable control itself, with the SVG hidden from screen readers.
#[derive(Clone, PartialEq)]
pub(super) struct IconButton {
    pub icon: Icon,
    pub label: &'static str,
    pub colors: Palette,
    pub enabled: bool,
    pub expanded: Option<bool>,
    pub compact: bool,
    pub on_press: EventHandler<Event<PressEventData>>,
}

impl Component for IconButton {
    fn render(&self) -> impl IntoElement {
        let p = self.colors;
        let mut hovering = use_state(|| false);
        let a11y_id = use_a11y();
        let focus = use_focus(a11y_id);
        let active = self.expanded == Some(true);
        let on_press = self.on_press.clone();
        let tooltip = Tooltip::new(self.label)
            .background(p.sidebar)
            .color(p.text)
            .border_fill(p.line)
            .font_size(12.);
        TooltipContainer::new(tooltip).child(
            rect()
                .width(Size::px(if self.compact { 44. } else { 30. }))
                .height(Size::px(30.))
                .center()
                .corner_radius(6.)
                .a11y_id(a11y_id)
                .a11y_role(AccessibilityRole::Button)
                .a11y_alt(self.label)
                .a11y_focusable(self.enabled)
                .a11y_builder(|node| {
                    if !self.enabled {
                        node.set_disabled();
                    }
                    if let Some(expanded) = self.expanded {
                        node.set_expanded(expanded);
                    }
                })
                .background(if self.enabled && (active || hovering()) {
                    p.hover
                } else {
                    p.background
                })
                .color(if self.enabled && (active || hovering()) {
                    p.text
                } else {
                    p.muted
                })
                .opacity(if self.enabled { 1. } else { 0.45 })
                .maybe(focus() == Focus::Keyboard, |node| {
                    node.border(
                        Border::new()
                            .width(2.)
                            .fill(p.green)
                            .alignment(BorderAlignment::Inner),
                    )
                })
                .maybe(self.enabled, |node| {
                    node.on_press(move |event| {
                        a11y_id.request_focus();
                        on_press.call(event);
                    })
                    .on_pointer_over(move |_| hovering.set(true))
                    .on_pointer_out(move |_| hovering.set(false))
                })
                .child(self.icon.view()),
        )
    }
}
