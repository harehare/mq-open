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
            .with_title("mq"),
        persist_window: true,
        ..Default::default()
    };

    eframe::run_native(
        "mq-open",
        options,
        Box::new(|cc| Ok(Box::new(MqOpenApp::new(cc, args.file)))),
    )
}
