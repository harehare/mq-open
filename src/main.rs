use clap::Parser;
use mq_open::MqOpenApp;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(author, version, about = "Graphical previewer for mq", long_about = None)]
struct Args {
    /// Path to the markdown file to preview
    file: Option<PathBuf>,
}

fn main() -> eframe::Result {
    let args = Args::parse();

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 900.0])
            .with_title(format!(
                "mq{}",
                args.file
                    .as_ref()
                    .map_or(String::new(), |f| format!(" - {}", f.display()))
            )),
        persist_window: true,
        ..Default::default()
    };

    eframe::run_native(
        "mq-open",
        options,
        Box::new(|cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            Ok(Box::new(MqOpenApp::new(cc, args.file)))
        }),
    )
}
