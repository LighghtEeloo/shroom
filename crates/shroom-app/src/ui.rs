use std::sync::Arc;

use freya::prelude::*;
use shroom_core::{SandboxStatus, Workspace, WorkspaceName};

use crate::model::{
    Actions, Backend, CheckResult, Command, ConnectionExport, CreateForm, ExportFormat,
    ImageStatus, Model, SetupForm, Timestamp,
};

mod icons;
mod style;
use icons::{Icon, IconButton};
use style::Palette;
mod agents;
use agents::Agent;

#[derive(Default)]
pub struct Shroom {
    pub backend: Arc<Backend>,
    pub initial: Model,
}

#[derive(Clone, Copy, Default, Eq, PartialEq)]
enum Page {
    #[default]
    Workspace,
    Create,
    Environment,
}

#[derive(Clone)]
struct Navigation {
    page: Page,
    details: bool,
    sidebar: bool,
    actions: bool,
    format: ExportFormat,
    agent: Agent,
    show_agents: bool,
    inspect: bool,
    diagnostics: bool,
}

impl Default for Navigation {
    fn default() -> Self {
        Self {
            page: Page::Workspace,
            details: true,
            sidebar: false,
            actions: false,
            format: ExportFormat::Command,
            agent: Agent::default(),
            show_agents: false,
            inspect: false,
            diagnostics: false,
        }
    }
}

impl Navigation {
    fn reconcile(&mut self, command: &Command, view: &Model, succeeded: bool) {
        let show_workspace = match command {
            Command::Open { .. } => succeeded,
            Command::Create(name, _) => {
                succeeded
                    || view
                        .workspaces
                        .iter()
                        .any(|workspace| &workspace.name == name)
                    || view.incomplete.contains(name)
            }
            _ => false,
        };
        if show_workspace {
            self.page = Page::Workspace;
        }
    }
}

impl App for Shroom {
    fn render(&self) -> impl IntoElement {
        let platform = Platform::get();
        let preference = platform.preferred_theme;
        let mut theme = use_init_theme(|| preference.peek().to_theme());
        use_side_effect(move || theme.set(preference.read().to_theme()));
        let colors = Palette::with_theme(*preference.read());
        let width = platform.root_size.read().width;
        let model = use_state(|| self.initial.clone());
        let navigation = use_state(Navigation::default);
        let setup = use_state(SetupForm::default);
        let create = use_state(CreateForm::default);
        let archive = use_state(|| std::env::var("SHROOM_IMAGE_ARCHIVE").unwrap_or_default());
        let updates = self.backend.agents.subscribe();
        let mut web_sessions = use_state(|| updates.borrow().clone());
        use_future(move || {
            let mut updates = updates.clone();
            async move {
                web_sessions.set(updates.borrow_and_update().clone());
                while updates.changed().await.is_ok() {
                    web_sessions.set(updates.borrow_and_update().clone());
                }
            }
        });
        let ui = Ui {
            model,
            navigation,
            backend: self.backend.clone(),
            colors,
            web_sessions,
        };
        let view = model.read().clone();
        let nav = navigation.read().clone();
        let compact = width < 960.;
        let docked_details = width >= 1100. && nav.details && view.connected();

        rect().expanded().horizontal().content(Content::Flex).background(colors.background)
            .color(colors.text).font_size(14.)
            .maybe_child((!compact || nav.sidebar).then(|| ui.sidebar(&view, &nav)))
            .child(
                rect().key("workspace-column").width(Size::flex(1.)).height(Size::fill()).content(Content::Flex)
                    .child(ui.toolbar(&view, &nav, compact))
                    .child(colors.divider())
                    .maybe_child(view.pending.as_ref().map(|command| {
                        rect().width(Size::fill()).padding((10., 24.)).background(colors.surface)
                            .child(label().text(command.progress()).font_size(13.).color(colors.green))
                    }))
                    .child(
                        rect().key("workspace-area").horizontal().width(Size::fill()).height(Size::flex(1.)).content(Content::Flex)
                            .child(
                                rect().width(Size::flex(1.)).height(Size::fill()).content(Content::Flex)
                                    .child(
                                        ScrollView::new().width(Size::fill()).height(Size::flex(1.))
                                            .child(
                                                rect().width(Size::fill()).padding((28., 24.)).spacing(24.)
                                                    .maybe_child((!view.errors.is_empty()).then(|| ui.errors(&view)))
                                                    .maybe_child((view.connected() && !view.catalog_current).then(|| {
                                                        colors.panel().background(colors.surface)
                                                            .child(label().text("Workspace information is out of date"))
                                                            .child(colors.caption("Refresh before changing a workspace."))
                                                            .child(colors.primary(ui.command_icon("Refresh now", Icon::Refresh, Command::Refresh, view.pending.is_none())))
                                                    }))
                                                    .child(rect().key("active-page").width(Size::fill()).child(if !view.connected() {
                                                        ui.setup(setup, view.pending.is_none()).into_element()
                                                    } else {
                                                        match nav.page {
                                                            Page::Create => ui.create(&view, create).into_element(),
                                                            Page::Environment => ui.environment(&view, archive).into_element(),
                                                            Page::Workspace => ui.workspace(&view, &nav).into_element(),
                                                        }
                                                    }))
                                                    .maybe_child(view.notice.as_ref().map(|notice| {
                                                        label().text(notice.clone()).font_size(13.).color(colors.green)
                                                    }))
                                                    .maybe_child(view.connected().then(|| ui.activity(&view, &nav)))
                                                    .maybe_child((nav.details && view.connected() && !docked_details)
                                                        .then(|| ui.details(&view, &nav))),
                                            ),
                                    )
                                    .child(rect().width(Size::fill()).padding((14., 24.))
                                        .child(colors.caption(if web_sessions.read().is_empty() {
                                            "Your workspaces keep running when you close Shroom."
                                        } else {
                                            "Web connections close with Shroom; workspaces keep running."
                                        }))),
                            )
                            .maybe_child(docked_details.then(|| {
                                rect().width(Size::px(236.)).height(Size::fill())
                                    .border(Border::new().width(1.).fill(colors.line))
                                    .child(ScrollView::new().width(Size::fill()).height(Size::fill())
                                        .child(rect().width(Size::fill()).padding(18.).child(ui.details(&view, &nav))))
                            })),
                    ),
            )
    }
}

#[derive(Clone)]
struct Ui {
    model: State<Model>,
    navigation: State<Navigation>,
    backend: Arc<Backend>,
    colors: Palette,
    web_sessions: State<Vec<crate::agents::WebSession>>,
}

impl PartialEq for Ui {
    fn eq(&self, other: &Self) -> bool {
        self.model == other.model
            && self.navigation == other.navigation
            && self.colors == other.colors
            && self.web_sessions == other.web_sessions
            && Arc::ptr_eq(&self.backend, &other.backend)
    }
}

impl Ui {
    fn submit(mut self, command: crate::model::Result<Command>) {
        let command = match command {
            Ok(command) => command,
            Err(error) => {
                self.model.write().errors = vec![error.to_string()];
                self.model.write().notice = None;
                return;
            }
        };
        if !self.model.write().begin(&command) {
            return;
        }
        self.navigation.write().actions = false;
        self.navigation.write().diagnostics = false;
        let completed = command.clone();
        // Confirmation and menu controls disappear during dispatch. The root owns result delivery.
        spawn_forever(async move {
            match tokio::spawn(async move { self.backend.execute(command).await }).await {
                Ok(reply) => {
                    let succeeded = reply.outcome.is_ok();
                    self.model.write().apply(reply);
                    self.navigation
                        .write()
                        .reconcile(&completed, &self.model.peek(), succeeded);
                    let copied = self.model.peek().copied.clone();
                    if let Some(export) = copied {
                        match Clipboard::set(export.text) {
                            Ok(()) => {
                                self.model.write().notice = Some(export.format.copied().into())
                            }
                            Err(_) => self.model.write().errors.push(
                                "Clipboard unavailable. Select and copy the connection text below."
                                    .into(),
                            ),
                        }
                    }
                }
                Err(error) => self.model.write().worker_failed(format!(
                    "Workspace operation failed: {error}. Refresh to read the current state."
                )),
            }
        });
    }

    fn command(&self, text: &str, command: Command, enabled: bool) -> Button {
        let ui = self.clone();
        self.colors
            .button(text, enabled)
            .on_press(move |_| ui.clone().submit(Ok(command.clone())))
    }

    fn command_icon(&self, text: &str, icon: Icon, command: Command, enabled: bool) -> Button {
        let ui = self.clone();
        self.colors
            .button_icon(text, icon, enabled)
            .on_press(move |_| ui.clone().submit(Ok(command.clone())))
    }

    fn page_button(&self, text: &str, page: Page, enabled: bool) -> Button {
        self.navigate(self.colors.button(text, enabled), page)
    }

    fn page_icon(&self, text: &str, icon: Icon, page: Page, enabled: bool) -> Button {
        self.navigate(self.colors.button_icon(text, icon, enabled), page)
    }

    fn navigate(&self, button: Button, page: Page) -> Button {
        let mut nav = self.navigation;
        let mut model = self.model;
        button.flat().on_press(move |_| {
            nav.write().page = page;
            nav.write().sidebar = false;
            nav.write().actions = false;
            model.write().confirm_remove = None;
            model.write().notice = None;
        })
    }

    fn sidebar(&self, view: &Model, nav: &Navigation) -> Rect {
        let p = self.colors;
        rect()
            .width(Size::px(216.))
            .height(Size::fill())
            .content(Content::Flex)
            .background(p.sidebar)
            .padding(12.)
            .spacing(8.)
            .child(
                rect()
                    .width(Size::fill())
                    .height(Size::px(44.))
                    .main_align(Alignment::Center)
                    .padding((0., 8.))
                    .child(
                        label()
                            .text("Shroom")
                            .font_size(20.)
                            .font_weight(FontWeight::MEDIUM),
                    ),
            )
            .child(
                self.page_icon(
                    "New workspace",
                    Icon::SquarePen,
                    Page::Create,
                    view.connected() && view.pending.is_none(),
                )
                .width(Size::fill())
                .background(p.sidebar)
                .hover_background(p.hover),
            )
            .child(
                ScrollView::new()
                    .width(Size::fill())
                    .height(Size::flex(1.))
                    .child(
                        rect()
                            .width(Size::fill())
                            .spacing(5.)
                            .padding((18., 0.))
                            .child(rect().padding((8., 8.)).child(
                                p.caption(format!("Workspaces · {}", view.workspaces.len())),
                            ))
                            .children(view.workspaces.iter().map(|workspace| {
                                self.sidebar_row(
                                    &workspace.name,
                                    &format!("{:?}", workspace.state),
                                    workspace.state == SandboxStatus::Running,
                                    view,
                                    nav,
                                )
                                .into_element()
                            }))
                            .maybe_child(
                                (view.workspaces.is_empty()
                                    && view.connected()
                                    && view.catalog_current)
                                    .then(|| {
                                        rect().padding(8.).child(p.caption("No workspaces yet"))
                                    }),
                            )
                            .maybe_child((!view.incomplete.is_empty()).then(|| {
                                rect()
                                    .width(Size::fill())
                                    .padding((20., 0.))
                                    .spacing(5.)
                                    .child(rect().padding(8.).child(p.caption("Needs attention")))
                                    .children(view.incomplete.iter().map(|name| {
                                        self.sidebar_row(name, "Incomplete", false, view, nav)
                                            .into_element()
                                    }))
                            })),
                    ),
            )
            .child(p.divider())
            .child(
                self.navigate(
                    Button::new()
                        .enabled(view.pending.is_none())
                        .width(Size::fill())
                        .padding((10., 8.))
                        .corner_radius(7.)
                        .background(p.sidebar)
                        .hover_background(p.hover)
                        .color(p.text)
                        .child(
                            rect()
                                .horizontal()
                                .width(Size::fill())
                                .content(Content::Flex)
                                .cross_align(Alignment::Center)
                                .spacing(9.)
                                .child(Icon::Folder.view().color(p.muted))
                                .child(
                                    rect()
                                        .width(Size::flex(1.))
                                        .spacing(4.)
                                        .child(label().text("Local workspaces").font_size(13.))
                                        .child(
                                            p.caption(
                                                view.session
                                                    .as_ref()
                                                    .map(|session| {
                                                        session.state_dir.display().to_string()
                                                    })
                                                    .unwrap_or_else(|| {
                                                        "Choose a workspace directory".into()
                                                    }),
                                            ),
                                        ),
                                )
                                .child(Icon::Settings.view().color(p.muted)),
                        ),
                    Page::Environment,
                ),
            )
    }

    fn sidebar_row(
        &self,
        name: &WorkspaceName,
        state: &str,
        running: bool,
        view: &Model,
        nav: &Navigation,
    ) -> Button {
        let p = self.colors;
        let active = nav.page == Page::Workspace && view.selected.as_ref() == Some(name);
        let name = name.clone();
        let mut model = self.model;
        let mut navigation = self.navigation;
        Button::new()
            .key(name.as_str())
            .flat()
            .width(Size::fill())
            .padding((10., 8.))
            .corner_radius(7.)
            .enabled(view.pending.is_none())
            .background(if active { p.selected } else { p.sidebar })
            .hover_background(p.hover)
            .color(p.text)
            .on_press({
                let name = name.clone();
                move |_| {
                    model.write().select(name.clone());
                    let mut nav = navigation.write();
                    nav.page = Page::Workspace;
                    nav.sidebar = false;
                    nav.actions = false;
                    nav.diagnostics = false;
                }
            })
            .child(
                rect()
                    .horizontal()
                    .width(Size::fill())
                    .content(Content::Flex)
                    .spacing(8.)
                    .cross_align(Alignment::Center)
                    .child(
                        rect()
                            .width(Size::px(6.))
                            .height(Size::px(6.))
                            .corner_radius(if state == "Incomplete" { 2. } else { 3. })
                            .background(if running {
                                p.green
                            } else if state == "Incomplete" {
                                p.warning
                            } else {
                                p.muted
                            }),
                    )
                    .child(
                        rect()
                            .width(Size::flex(1.))
                            .child(label().text(name.to_string()).font_size(13.)),
                    )
                    .child(p.caption(state)),
            )
    }

    fn toolbar(&self, view: &Model, nav: &Navigation, compact: bool) -> Rect {
        let p = self.colors;
        let mut navigation = self.navigation;
        let title = if !view.connected() {
            "Open workspaces".into()
        } else {
            match nav.page {
                Page::Create => "New workspace".into(),
                Page::Environment => "Environment".into(),
                Page::Workspace => view
                    .selected
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "Workspaces".into()),
            }
        };
        rect()
            .horizontal()
            .width(Size::fill())
            .height(Size::px(58.))
            .padding((10., 16.))
            .content(Content::Flex)
            .spacing(8.)
            .cross_align(Alignment::Center)
            .maybe_child(compact.then(|| {
                IconButton {
                    icon: Icon::PanelLeft,
                    label: if nav.sidebar {
                        "Hide workspaces"
                    } else {
                        "Show workspaces"
                    },
                    colors: p,
                    enabled: true,
                    expanded: Some(nav.sidebar),
                    compact,
                    on_press: (move |_| {
                        let shown = navigation.peek().sidebar;
                        navigation.write().sidebar = !shown;
                    })
                    .into(),
                }
            }))
            .child(
                rect().width(Size::flex(1.)).child(
                    (if nav.page == Page::Environment {
                        Icon::Settings
                    } else {
                        Icon::Box
                    })
                    .beside(label().text(title).font_weight(FontWeight::MEDIUM)),
                ),
            )
            .maybe_child(view.connected().then(|| {
                let ui = self.clone();
                IconButton {
                    icon: Icon::Refresh,
                    label: "Refresh workspaces",
                    colors: p,
                    enabled: view.pending.is_none(),
                    expanded: None,
                    compact,
                    on_press: (move |_| ui.clone().submit(Ok(Command::Refresh))).into(),
                }
            }))
            .maybe_child(
                (view.connected()
                    && nav.page == Page::Workspace
                    && (view.selected_incomplete().is_some()
                        || view.workspace().is_some_and(|workspace| {
                            Actions::stop(workspace.state) || Actions::remove(workspace.state)
                        })))
                .then(|| {
                    rect()
                        .child(IconButton {
                            icon: Icon::Ellipsis,
                            label: "Workspace actions",
                            colors: p,
                            enabled: view.available(),
                            expanded: Some(nav.actions),
                            compact,
                            on_press: (move |_| {
                                let shown = navigation.peek().actions;
                                navigation.write().actions = !shown;
                            })
                            .into(),
                        })
                        .maybe_child(nav.actions.then(|| self.actions_menu(view)))
                }),
            )
            .maybe_child(view.connected().then(|| {
                IconButton {
                    icon: Icon::PanelRight,
                    label: "Toggle workspace details",
                    colors: p,
                    enabled: true,
                    expanded: Some(nav.details),
                    compact,
                    on_press: (move |_| {
                        let shown = navigation.peek().details;
                        navigation.write().details = !shown;
                    })
                    .into(),
                }
            }))
    }

    fn actions_menu(&self, view: &Model) -> Menu {
        let mut navigation = self.navigation;
        let mut menu = Menu::new().on_close(move |_| navigation.write().actions = false);
        if let Some(workspace) = view.workspace() {
            if Actions::stop(workspace.state) {
                let ui = self.clone();
                let name = workspace.name.clone();
                menu = menu.child(
                    MenuButton::new()
                        .on_press(move |_| ui.clone().submit(Ok(Command::Stop(name.clone()))))
                        .child(Icon::Stop.beside(label().text("Stop workspace"))),
                );
            }
            if Actions::remove(workspace.state) {
                menu = menu.child(self.remove_menu_item(&workspace.name, "Delete workspace"));
            }
        } else if let Some(name) = view.selected_incomplete() {
            menu = menu.child(self.remove_menu_item(name, "Remove incomplete setup"));
        }
        menu
    }

    fn remove_menu_item(&self, name: &WorkspaceName, text: &str) -> MenuButton {
        let mut model = self.model;
        let mut navigation = self.navigation;
        let name = name.clone();
        MenuButton::new()
            .on_press(move |_| {
                model.write().confirm_remove = Some(name.clone());
                navigation.write().actions = false;
            })
            .child(Icon::Trash.beside(label().text(text.to_owned())))
    }

    fn errors(&self, view: &Model) -> Rect {
        let p = self.colors;
        p.panel()
            .background(p.error_surface)
            .children(view.errors.iter().map(|error| {
                SelectableText::new()
                    .span(error.clone())
                    .color(p.error)
                    .width(Size::fill())
                    .font_size(13.)
                    .into_element()
            }))
    }

    fn setup(&self, setup: State<SetupForm>, enabled: bool) -> Rect {
        let p = self.colors;
        let form = setup.into_writable();
        let ui = self.clone();
        rect()
            .width(Size::fill())
            .spacing(20.)
            .child(p.heading("Open your workspace directory"))
            .child(
                label()
                    .text("Choose where your workspaces live and the installed runtime.")
                    .color(p.muted),
            )
            .child(p.field(
                "State directory",
                "/absolute/path/to/.shroom",
                form.map(|f| &f.state_dir, |f| &mut f.state_dir),
                enabled,
            ))
            .child(p.field(
                "Runtime executable",
                "/absolute/path/to/msb",
                form.map(|f| &f.runtime, |f| &mut f.runtime),
                enabled,
            ))
            .child(p.field(
                "Firmware library",
                "/absolute/path/to/libkrunfw",
                form.map(|f| &f.firmware, |f| &mut f.firmware),
                enabled,
            ))
            .child(
                p.caption("Use microsandbox 0.7.2 and its actual executable, not a shell wrapper."),
            )
            .child(
                p.primary(
                    p.button("Open workspaces", enabled)
                        .on_press(move |_| ui.clone().submit(setup.peek().parse())),
                ),
            )
    }

    fn create(&self, view: &Model, create: State<CreateForm>) -> Rect {
        let p = self.colors;
        let form = create.into_writable();
        let ui = self.clone();
        let enabled = view.available() && view.image == ImageStatus::Ready;
        rect().width(Size::fill()).spacing(22.)
            .child(p.heading("A little room to build."))
            .child(label().text("Create a persistent workspace. Bring the terminal or editor you already use.").color(p.muted))
            .maybe_child((view.image != ImageStatus::Ready).then(|| {
                p.panel().background(p.surface).child(label().text("Prepare your workspace image"))
                    .child(p.caption("Import the image once before creating a workspace here."))
                    .child(self.page_icon("Open environment", Icon::Settings, Page::Environment, view.pending.is_none()))
            }))
            .child(
                rect().horizontal().width(Size::fill()).content(Content::Flex).spacing(16.)
                    .child(rect().width(Size::flex(1.)).child(p.field(
                        "Workspace name", "my-project", form.map(|f| &f.name, |f| &mut f.name), view.pending.is_none(),
                    )))
                    .child(rect().width(Size::px(120.)).child(p.field(
                        "Host SSH port", "2222", form.map(|f| &f.port, |f| &mut f.port), view.pending.is_none(),
                    ))),
            )
            .child(p.caption("Use a lowercase name and an unused local port from 1024 to 65535."))
            .child(rect().horizontal().spacing(10.)
                .child(p.primary(p.button_icon("Create workspace", Icon::Plus, enabled).on_press(move |_| ui.clone().submit(create.peek().parse()))))
                .child(self.page_button("Cancel", Page::Workspace, view.pending.is_none())))
    }

    fn environment(&self, view: &Model, archive: State<String>) -> Rect {
        let p = self.colors;
        let session = view.session.as_ref().expect("open directory");
        let ui = self.clone();
        rect().width(Size::fill()).spacing(22.)
            .child(p.heading("Local environment"))
            .child(label().text("Workspaces in this directory share a runtime and prepared image.").color(p.muted))
            .child(p.value("Workspace directory", session.state_dir.display().to_string()))
            .child(p.value("Runtime executable", session.config.runtime_executable.display().to_string()))
            .child(p.value("Firmware library", session.config.firmware.display().to_string()))
            .child(self.command("Change directory", Command::Disconnect, view.pending.is_none()).flat())
            .child(p.divider())
            .child(label().text("Workspace image").font_weight(FontWeight::MEDIUM))
            .child((if view.image == ImageStatus::Ready { Icon::CircleCheck } else { Icon::CircleAlert }).beside(p.caption(match view.image {
                ImageStatus::Ready => "Image available. Ready for new workspaces.",
                ImageStatus::Missing => "Import a prepared archive to create workspaces.",
                ImageStatus::Unknown => "Image status unavailable. Refresh or import an archive.",
            })).color(if view.image == ImageStatus::Ready { p.green } else { p.muted }))
            .child(p.code(shroom_core::WORKSPACE_IMAGE))
            .maybe_child((view.image != ImageStatus::Ready).then(|| {
                rect().width(Size::fill()).spacing(16.)
                    .child(p.field("Image archive", "/absolute/path/to/workspace.tar", archive.into_writable(), view.pending.is_none()))
                    .child(p.caption("Build images/workspace for this host, then export with docker save or podman save. See README.md."))
                    .child(p.primary(p.button("Import image", view.pending.is_none()).on_press(move |_| {
                        ui.clone().submit(SetupForm::path(&archive.peek(), "Image archive").map(Command::ImportImage));
                    })))
            }))
            .maybe_child((view.image == ImageStatus::Ready).then(|| self.page_icon("New workspace", Icon::Plus, Page::Create, view.available())))
    }

    fn workspace(&self, view: &Model, nav: &Navigation) -> Rect {
        let p = self.colors;
        if let Some(name) = view.selected_incomplete() {
            let mut model = self.model;
            let target = name.clone();
            return rect().width(Size::fill()).spacing(20.)
                .child(rect().horizontal().cross_align(Alignment::Center).spacing(6.)
                    .child(rect().width(Size::px(7.)).height(Size::px(7.)).corner_radius(2.).background(p.warning))
                    .child(label().text("Incomplete setup").color(p.warning).font_size(12.)))
                .child(p.heading(format!("Clean up {name}")))
                .child(label().text("An earlier attempt left SSH access files. No workspace is recorded for this name.").color(p.muted))
                .maybe_child((view.confirm_remove.as_ref() != Some(name)).then(|| {
                    p.primary(p.button_icon("Remove incomplete setup", Icon::Trash, view.available()).on_press(move |_| model.write().confirm_remove = Some(target.clone())))
                }))
                .maybe_child((view.confirm_remove.as_ref() == Some(name)).then(|| self.confirmation(name, true, view.available())));
        }
        let Some(workspace) = view.workspace() else {
            return rect().width(Size::fill()).spacing(20.)
                .child(p.heading("Your next workspace starts here."))
                .child(label().text("Create a workspace, connect your tools, and pick up where you left off.").color(p.muted))
                .child(p.primary(self.page_icon("New workspace", Icon::Plus, Page::Create, view.available())));
        };
        let check = view.check(&workspace.name);
        let verified = check.is_some_and(|check| matches!(check.result, CheckResult::Verified(_)));
        let heading = match workspace.state {
            SandboxStatus::Running if verified => "Ready to connect",
            SandboxStatus::Running => "Check your connection",
            SandboxStatus::Stopped => "Pick up where you left off",
            SandboxStatus::Crashed => "Workspace exited unexpectedly",
            SandboxStatus::Created => "Ready for its first start",
            SandboxStatus::Paused => "Workspace is paused",
            SandboxStatus::Starting => "Workspace is starting",
            SandboxStatus::Draining => "Workspace is stopping",
        };
        let description = match workspace.state {
            SandboxStatus::Running => "Use your terminal or SSH-compatible editor to work here.",
            SandboxStatus::Stopped | SandboxStatus::Created => {
                "Your files are saved. Start this workspace to connect your tools."
            }
            SandboxStatus::Crashed => {
                "Your files remain available. Start again or inspect the failure details."
            }
            SandboxStatus::Paused => "Stop this workspace before starting it again.",
            _ => "Refresh to read the current runtime state.",
        };
        rect()
            .width(Size::fill())
            .spacing(18.)
            .child(
                rect()
                    .horizontal()
                    .spacing(16.)
                    .child(
                        rect()
                            .horizontal()
                            .cross_align(Alignment::Center)
                            .spacing(6.)
                            .child(
                                rect()
                                    .width(Size::px(7.))
                                    .height(Size::px(7.))
                                    .corner_radius(3.)
                                    .background(if workspace.state == SandboxStatus::Running {
                                        p.green
                                    } else {
                                        p.muted
                                    }),
                            )
                            .child(
                                label()
                                    .text(format!("{:?}", workspace.state))
                                    .font_size(12.)
                                    .color(if workspace.state == SandboxStatus::Running {
                                        p.green
                                    } else {
                                        p.muted
                                    }),
                            ),
                    )
                    .maybe_child((workspace.state == SandboxStatus::Running).then(|| {
                        (if verified {
                            Icon::ShieldCheck
                        } else {
                            Icon::ShieldQuestion
                        })
                        .beside(
                            label()
                                .text(match check {
                                    Some(check) => format!(
                                        "SSH {} · {}",
                                        if verified { "verified" } else { "check failed" },
                                        Timestamp::label(check.at)
                                    ),
                                    None => "SSH not checked".into(),
                                })
                                .font_size(12.),
                        )
                        .color(if verified { p.green } else { p.muted })
                    })),
            )
            .child(p.heading(heading))
            .child(label().text(description).color(p.muted))
            .maybe_child(
                (workspace.state == SandboxStatus::Running)
                    .then(|| self.connection(view, nav, workspace, verified)),
            )
            .maybe_child(
                (workspace.state != SandboxStatus::Running && Actions::start(workspace.state))
                    .then(|| {
                        p.primary(self.command_icon(
                            "Start workspace",
                            Icon::Play,
                            Command::Start(workspace.name.clone()),
                            view.available(),
                        ))
                    }),
            )
            .maybe_child((workspace.state == SandboxStatus::Paused).then(|| {
                p.primary(self.command_icon(
                    "Stop workspace",
                    Icon::Stop,
                    Command::Stop(workspace.name.clone()),
                    view.available(),
                ))
            }))
            .maybe_child(
                (view.confirm_remove.as_ref() == Some(&workspace.name))
                    .then(|| self.confirmation(&workspace.name, false, view.available())),
            )
    }

    fn connection(
        &self,
        view: &Model,
        nav: &Navigation,
        workspace: &Workspace,
        verified: bool,
    ) -> Rect {
        let p = self.colors;
        let mut navigation = self.navigation;
        let export = workspace
            .ssh
            .clone()
            .ok_or(crate::model::Error::SshUnavailable)
            .and_then(|connection| {
                ConnectionExport::with_connection(&workspace.name, connection, nav.format)
            });
        let enabled = view.available() && export.is_ok();
        let copy = self.command_icon(
            nav.format.copy_label(),
            Icon::Copy,
            Command::Export(workspace.name.clone(), nav.format),
            enabled,
        );
        let verify = self.command_icon(
            "Verify SSH",
            Icon::ShieldCheck,
            Command::Verify(workspace.name.clone()),
            view.available(),
        );
        rect().width(Size::fill()).spacing(14.)
            .child(
                p.panel()
                    .child(rect().horizontal().width(Size::fill()).content(Content::Flex).cross_align(Alignment::Center).spacing(6.)
                        .child(rect().width(Size::flex(1.)).child(Icon::Terminal.beside(label().text("Connect").font_weight(FontWeight::MEDIUM))))
                        .children([ExportFormat::Command, ExportFormat::Config].into_iter().map(|format| {
                            p.button(format.label(), view.pending.is_none()).flat()
                                .background(if !nav.show_agents && nav.format == format { p.selected } else { p.background })
                                .on_press(move |_| {
                                    let mut nav = navigation.write();
                                    nav.format = format;
                                    nav.show_agents = false;
                                }).into_element()
                        }))
                        .child(p.button("Agents", true).flat()
                            .background(if nav.show_agents { p.selected } else { p.background })
                            .on_press(move |_| navigation.write().show_agents = true)))
                    .child(if nav.show_agents {
                        self.agent_panel(view, nav, workspace).into_element()
                    } else {
                        rect().width(Size::fill()).spacing(14.)
                    .child(
                        ScrollView::new().width(Size::fill()).height(Size::px(170.))
                            .child(rect().width(Size::fill()).padding(10.).background(p.surface)
                                .child(p.code(match export {
                                    Ok(export) if view.catalog_current && view.pending.is_none() => export.text,
                                    Ok(_) => "Refresh or wait for the current operation to finish to read connection details.".into(),
                                    Err(error) => error.to_string(),
                                }))),
                    )
                    .child(p.caption(match nav.format {
                        ExportFormat::Command => "Identity and pinned host verification included.",
                        ExportFormat::Config => "Place this stanza before matching Host defaults.",
                        ExportFormat::ConnectionDetails => "Verify the host key in the client before connecting.",
                    }))
                    .child(if verified { p.primary(copy) } else { copy }).into_element()
                    }),
            )
            .child(if verified { verify.flat() } else { p.primary(verify) })
    }

    fn confirmation(&self, name: &WorkspaceName, incomplete: bool, enabled: bool) -> Rect {
        let p = self.colors;
        let mut model = self.model;
        p.panel()
            .background(p.error_surface)
            .child(label().text(if incomplete {
                format!("Remove the incomplete setup for {name}?")
            } else {
                format!("Permanently remove {name} and all of its files?")
            }))
            .child(p.caption(if incomplete {
                "Remove the leftover access files to reuse this name."
            } else {
                "This cannot be undone."
            }))
            .child(
                rect()
                    .horizontal()
                    .spacing(10.)
                    .child(
                        self.command(
                            if incomplete {
                                "Confirm cleanup"
                            } else {
                                "Delete workspace"
                            },
                            if incomplete {
                                Command::Cleanup(name.clone())
                            } else {
                                Command::Remove(name.clone())
                            },
                            enabled,
                        )
                        .color(p.error),
                    )
                    .child(
                        p.button("Cancel", enabled)
                            .flat()
                            .on_press(move |_| model.write().confirm_remove = None),
                    ),
            )
    }

    fn activity(&self, view: &Model, nav: &Navigation) -> Rect {
        let p = self.colors;
        let mut navigation = self.navigation;
        let items = view
            .activity
            .iter()
            .filter(|item| {
                if nav.page == Page::Workspace && view.selected.is_some() {
                    item.command.target() == view.selected.as_ref()
                } else {
                    true
                }
            })
            .take(4)
            .collect::<Vec<_>>();
        rect()
            .width(Size::fill())
            .spacing(10.)
            .maybe_child((!items.is_empty()).then(|| {
                Icon::History
                    .beside(p.caption("This session"))
                    .color(p.muted)
            }))
            .children(items.into_iter().map(|item| {
                rect()
                    .width(Size::fill())
                    .spacing(8.)
                    .child(p.divider())
                    .child(
                        rect()
                            .horizontal()
                            .width(Size::fill())
                            .content(Content::Flex)
                            .spacing(12.)
                            .child(
                                rect().width(Size::flex(1.)).child(
                                    label()
                                        .text(if item.error.is_some() {
                                            format!(
                                                "{} Failed",
                                                item.command.progress().trim_end_matches('…')
                                            )
                                        } else {
                                            item.command.success().into()
                                        })
                                        .font_size(12.),
                                ),
                            )
                            .child(p.caption(Timestamp::label(item.at))),
                    )
                    .maybe_child(item.error.as_ref().map(|error| {
                        rect()
                            .width(Size::fill())
                            .spacing(8.)
                            .child(
                                p.button(
                                    if nav.diagnostics {
                                        "Hide diagnostics"
                                    } else {
                                        "Show diagnostics"
                                    },
                                    true,
                                )
                                .flat()
                                .on_press(move |_| {
                                    let shown = navigation.peek().diagnostics;
                                    navigation.write().diagnostics = !shown;
                                }),
                            )
                            .maybe_child(nav.diagnostics.then(|| p.code(error.clone())))
                    }))
                    .into_element()
            }))
    }

    fn details(&self, view: &Model, nav: &Navigation) -> Rect {
        let p = self.colors;
        let session = view.session.as_ref().expect("open directory");
        let mut navigation = self.navigation;
        rect()
            .width(Size::fill())
            .spacing(18.)
            .child(Icon::Sliders.beside(label().text("Details").font_weight(FontWeight::MEDIUM)))
            .child(p.divider())
            .child(p.caption("ENVIRONMENT"))
            .child(
                Icon::Laptop
                    .beside(
                        label()
                            .text("Local · this computer")
                            .font_size(13.)
                            .color(p.text),
                    )
                    .color(p.muted),
            )
            .child(
                Icon::Cpu
                    .beside(p.caption(format!("microsandbox {}", shroom_core::RUNTIME_VERSION)))
                    .color(p.muted),
            )
            .child(p.value(
                "Workspace directory",
                session.state_dir.display().to_string(),
            ))
            .child(p.value(
                "Runtime storage",
                session.runtime_home().display().to_string(),
            ))
            .child(
                p.button(
                    if nav.inspect {
                        "Hide msb command"
                    } else {
                        "Inspect with msb"
                    },
                    true,
                )
                .flat()
                .on_press(move |_| {
                    let shown = navigation.peek().inspect;
                    navigation.write().inspect = !shown;
                }),
            )
            .maybe_child(nav.inspect.then(|| p.code(session.inspection_command())))
            .maybe_child(
                (nav.page == Page::Workspace && view.selected.is_some()).then(|| {
                    rect()
                        .width(Size::fill())
                        .spacing(16.)
                        .child(p.divider())
                        .child(p.caption("SSH CONNECTION"))
                        .maybe_child(view.selected.as_ref().map(|name| {
                            p.value(
                                "Identity file",
                                session
                                    .state_dir
                                    .join("access")
                                    .join(name.as_str())
                                    .join("client_ed25519")
                                    .display()
                                    .to_string(),
                            )
                        }))
                        .maybe_child(
                            view.workspace()
                                .and_then(|w| w.ssh.as_ref())
                                .filter(|_| view.catalog_current && view.pending.is_none())
                                .map(|ssh| {
                                    rect()
                                        .width(Size::fill())
                                        .spacing(16.)
                                        .child(p.value("Address", ssh.endpoint.to_string()))
                                        .child(p.value("User", ssh.user.clone()))
                                        .child(p.value("Guest directory", ssh.directory.clone()))
                                        .child(
                                            p.value(
                                                "Host identity",
                                                ssh.host_key_alias.to_string(),
                                            ),
                                        )
                                }),
                        )
                }),
            )
            .maybe_child(
                (nav.page != Page::Workspace || view.selected.is_none()).then(|| {
                    rect()
                        .width(Size::fill())
                        .spacing(16.)
                        .child(p.divider())
                        .child(p.caption("IMAGE SETUP"))
                        .child(
                            (if view.image == ImageStatus::Ready {
                                Icon::CircleCheck
                            } else {
                                Icon::CircleAlert
                            })
                            .beside(p.caption(match view.image {
                                ImageStatus::Ready => "Image available",
                                ImageStatus::Missing => "Image required",
                                ImageStatus::Unknown => "Image status unavailable",
                            }))
                            .color(p.muted),
                        )
                }),
            )
    }
}

#[cfg(test)]
mod tests;
