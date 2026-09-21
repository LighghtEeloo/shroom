use super::*;
use crate::preferences::WorkingDirectoryPreference;

impl Ui {
    pub(super) fn working_directory(
        &self,
        view: &Model,
        nav: &Navigation,
        workspace: &WorkspaceView,
    ) -> Rect {
        let p = self.colors;
        let mut navigation = self.navigation;
        let name = workspace.name.clone();
        let editing = nav.directory_editor.as_ref() == Some(&name);
        rect()
            .width(Size::fill())
            .spacing(8.)
            .child(p.caption("Default working directory"))
            .child(if editing {
                DirectoryEditor {
                    ui: self.clone(),
                    name: name.clone(),
                    initial: workspace
                        .directory()
                        .map(|directory| directory.to_string())
                        .unwrap_or_else(|_| workspace.options.user.default_working_directory()),
                    suggestions: workspace
                        .options
                        .folders
                        .iter()
                        .map(|folder| folder.guest().to_owned())
                        .collect(),
                    key: DiffKey::from(&format!(
                        "{:?}/{name}",
                        view.session.as_ref().map(|session| &session.state_dir)
                    )),
                }
                .into_element()
            } else {
                rect()
                    .width(Size::fill())
                    .spacing(6.)
                    .child(
                        p.code(
                            workspace
                                .directory()
                                .map(|directory| directory.to_string())
                                .unwrap_or_else(|error| error.to_string()),
                        ),
                    )
                    .maybe_child(
                        matches!(
                            workspace.preference,
                            Ok(WorkingDirectoryPreference::Default)
                        )
                        .then(|| p.caption("Default")),
                    )
                    .child(
                        p.button(
                            "Edit working directory",
                            view.available() && workspace.directory_editable,
                        )
                        .flat()
                        .on_press(move |_| {
                            navigation.write().directory_editor = Some(name.clone())
                        }),
                    )
                    .into_element()
            })
    }
}

#[derive(PartialEq)]
struct DirectoryEditor {
    ui: Ui,
    name: WorkspaceName,
    initial: String,
    suggestions: Vec<String>,
    key: DiffKey,
}

impl Component for DirectoryEditor {
    fn render_key(&self) -> DiffKey {
        self.key.clone()
    }

    fn render(&self) -> impl IntoElement {
        let p = self.ui.colors;
        let mut path = use_state(|| self.initial.clone());
        let available = self.ui.model.read().available();
        let ui = self.ui.clone();
        let name = self.name.clone();
        let mut navigation = self.ui.navigation;
        rect().width(Size::fill()).spacing(8.)
            .child(p.field("Guest folder", "/mnt/project", path.into(), available))
            .child(p.caption("Folder inside this workspace. Used for new terminals and agent projects. Checked when opening a project; existing sessions keep their directory."))
            .children(self.suggestions.iter().map(|suggestion| {
                let suggestion = suggestion.clone();
                p.button(format!("Use {suggestion}"), available).flat()
                    .on_press(move |_| path.set(suggestion.clone())).into_element()
            }))
            .child(p.primary(p.button("Save working directory", available)
                .on_press(move |_| ui.clone().submit(path.peek().parse()
                    .map(|directory| Command::SetWorkingDirectory(name.clone(), directory))
                    .map_err(crate::model::Error::from)))))
            .child(p.button("Cancel directory edit", available).flat()
                .on_press(move |_| navigation.write().directory_editor = None))
            .child(self.ui.command("Use default", Command::ResetWorkingDirectory(self.name.clone()), available).flat())
    }
}
