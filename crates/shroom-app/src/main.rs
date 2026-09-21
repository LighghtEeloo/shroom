mod agents;
mod codex;
mod model;
mod preferences;
mod project;
mod ui;

#[cfg(test)]
mod test_support;

use freya::prelude::*;

fn main() -> std::io::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let _guard = runtime.enter();
    let backend = std::sync::Arc::new(model::Backend::default());
    launch(
        LaunchConfig::new().with_window(
            WindowConfig::new_app(ui::Shroom {
                backend: backend.clone(),
                ..ui::Shroom::default()
            })
            .with_title("Shroom")
            .with_size(1120., 800.)
            .with_min_size(820., 640.),
        ),
    );
    runtime.block_on(backend.agents.close(None));
    Ok(())
}
