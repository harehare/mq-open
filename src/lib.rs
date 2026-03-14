use eframe::egui;
use mq_lang::{DefaultEngine, RuntimeValue};
use mq_markdown::{Markdown, Node};
use notify::{RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Instant;

// ── Theme-aware color palette ─────────────────────────────────────────────────

struct Colors {
    bg: egui::Color32,
    surface: egui::Color32,
    surface2: egui::Color32,
    accent: egui::Color32,
    text: egui::Color32,
    text_muted: egui::Color32,
    code_bg: egui::Color32,
    border: egui::Color32,
    error: egui::Color32,
    error_bg: egui::Color32,
    html_bg: egui::Color32,
    html_border: egui::Color32,
    code_text: egui::Color32,
}

impl Colors {
    fn dark() -> Self {
        Self {
            bg: egui::Color32::from_rgb(15, 23, 42),            // #0f172a
            surface: egui::Color32::from_rgb(30, 41, 59),       // #1e293b
            surface2: egui::Color32::from_rgb(42, 52, 68),      // #2a3444
            accent: egui::Color32::from_rgb(79, 195, 247),      // #4fc3f7
            text: egui::Color32::from_rgb(226, 232, 240),       // #e2e8f0
            text_muted: egui::Color32::from_rgb(148, 163, 184), // #94a3b8
            code_bg: egui::Color32::from_rgb(10, 17, 36),
            border: egui::Color32::from_rgb(51, 65, 85), // #334155
            error: egui::Color32::from_rgb(248, 113, 113), // #f87171
            error_bg: egui::Color32::from_rgb(60, 20, 20),
            html_bg: egui::Color32::from_rgb(45, 27, 10),
            html_border: egui::Color32::from_rgb(180, 83, 9), // #b45309
            code_text: egui::Color32::from_rgb(167, 243, 208), // #a7f3d0
        }
    }

    fn light() -> Self {
        Self {
            bg: egui::Color32::from_rgb(248, 250, 252),      // #f8fafc
            surface: egui::Color32::from_rgb(241, 245, 249), // #f1f5f9
            surface2: egui::Color32::from_rgb(226, 232, 240), // #e2e8f0
            accent: egui::Color32::from_rgb(2, 132, 199),    // #0284c7
            text: egui::Color32::from_rgb(15, 23, 42),       // #0f172a
            text_muted: egui::Color32::from_rgb(100, 116, 139), // #64748b
            code_bg: egui::Color32::from_rgb(248, 250, 252), // #f8fafc
            border: egui::Color32::from_rgb(203, 213, 225),  // #cbd5e1
            error: egui::Color32::from_rgb(185, 28, 28),     // #b91c1c
            error_bg: egui::Color32::from_rgb(254, 226, 226), // #fee2e2
            html_bg: egui::Color32::from_rgb(255, 251, 235), // #fffbeb
            html_border: egui::Color32::from_rgb(180, 83, 9), // #b45309
            code_text: egui::Color32::from_rgb(5, 122, 86),  // #057a56
        }
    }

    fn for_dark_mode(dark: bool) -> Self {
        if dark { Self::dark() } else { Self::light() }
    }
}

// App state

pub struct MqOpenApp {
    file_path: Option<PathBuf>,
    source_content: String,
    query: String,
    results: Vec<Node>,
    toc: Vec<TocEntry>,
    error: Option<String>,
    engine: DefaultEngine,
    rx: Receiver<String>,
    tx: Sender<String>,
    _watcher: Option<Box<dyn Watcher>>,
    scroll_to_node: Option<usize>,
    stats: DocStats,
    last_eval_duration: std::time::Duration,
    last_dark_mode: bool,
}

#[derive(Default)]
struct DocStats {
    node_count: usize,
    word_count: usize,
}

#[derive(Clone)]
pub struct TocEntry {
    pub title: String,
    pub level: u8,
    pub node_index: usize,
    pub children: Vec<TocEntry>,
}

impl MqOpenApp {
    pub fn new(cc: &eframe::CreationContext<'_>, file_path: Option<PathBuf>) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        apply_custom_style(&cc.egui_ctx, true);

        let mut engine = DefaultEngine::default();
        engine.load_builtin_module();
        let (tx, rx) = channel();

        let mut app = Self {
            file_path: None,
            source_content: String::new(),
            query: "self".to_string(),
            results: Vec::new(),
            toc: Vec::new(),
            error: None,
            engine,
            rx,
            tx,
            _watcher: None,
            scroll_to_node: None,
            stats: DocStats::default(),
            last_eval_duration: std::time::Duration::ZERO,
            last_dark_mode: true,
        };

        if let Some(path) = file_path {
            app.open_file(path);
        }
        app
    }

    fn open_file(&mut self, path: PathBuf) {
        self.file_path = Some(path.clone());

        let tx = self.tx.clone();
        let path_for_watcher = path.clone();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(event) = res
                && event.kind.is_modify()
                && let Ok(content) = std::fs::read_to_string(&path_for_watcher)
            {
                let _ = tx.send(content);
            }
        })
        .expect("Failed to create watcher");

        let _ = watcher.watch(&path, RecursiveMode::NonRecursive);
        self._watcher = Some(Box::new(watcher));

        match std::fs::read_to_string(&path) {
            Ok(content) => {
                self.source_content = content;
                self.run_query();
                self.update_toc();
            }
            Err(e) => self.error = Some(format!("Failed to read file: {}", e)),
        }
    }

    fn run_query(&mut self) {
        let start = Instant::now();
        let input = match mq_lang::parse_markdown_input(&self.source_content) {
            Ok(input) => input,
            Err(e) => {
                self.error = Some(format!("Parse error: {}", e));
                return;
            }
        };

        match self.engine.eval(&self.query, input.into_iter()) {
            Ok(values) => {
                self.results = values
                    .values()
                    .iter()
                    .map(|v| match v {
                        RuntimeValue::Markdown(node, _) => node.clone(),
                        _ => Node::Text(mq_markdown::Text {
                            value: v.to_string(),
                            position: None,
                        }),
                    })
                    .collect();
                self.error = None;
                self.stats.node_count = self.results.len();
                self.stats.word_count = self
                    .results
                    .iter()
                    .map(|n| n.value().split_whitespace().count())
                    .sum();
            }
            Err(e) => self.error = Some(format!("Query error: {}", e)),
        }
        self.last_eval_duration = start.elapsed();
    }

    fn update_toc(&mut self) {
        let md = match self.source_content.parse::<Markdown>() {
            Ok(md) => md,
            Err(_) => return,
        };

        let headings: Vec<(u8, String, usize)> = md
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(i, node)| {
                if let Node::Heading(h) = node {
                    Some((h.depth, node.value(), i))
                } else {
                    None
                }
            })
            .collect();

        fn build_toc(headings: &[(u8, String, usize)], min_level: u8) -> (Vec<TocEntry>, usize) {
            let mut entries = Vec::new();
            let mut i = 0;
            while i < headings.len() {
                let (level, title, node_index) = &headings[i];
                if *level < min_level {
                    break;
                }
                if *level == min_level {
                    let (children, consumed) = build_toc(&headings[i + 1..], min_level + 1);
                    entries.push(TocEntry {
                        title: title.clone(),
                        level: *level,
                        node_index: *node_index,
                        children,
                    });
                    i += 1 + consumed;
                } else {
                    let (children, consumed) = build_toc(&headings[i..], min_level + 1);
                    i += consumed;
                    entries.extend(children);
                }
            }
            (entries, i)
        }

        if !headings.is_empty() {
            let min_level = headings.iter().map(|(l, _, _)| *l).min().unwrap_or(1);
            let (built_toc, _) = build_toc(&headings, min_level);
            self.toc = built_toc;
        } else {
            self.toc = Vec::new();
        }
    }
}

fn apply_custom_style(ctx: &egui::Context, dark_mode: bool) {
    let c = Colors::for_dark_mode(dark_mode);

    let mut visuals = if dark_mode {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };

    visuals.window_fill = c.bg;
    visuals.panel_fill = c.surface;
    visuals.faint_bg_color = c.surface2;
    visuals.extreme_bg_color = c.code_bg;
    visuals.override_text_color = Some(c.text);
    visuals.hyperlink_color = c.accent;
    visuals.selection.bg_fill =
        egui::Color32::from_rgba_unmultiplied(c.accent.r(), c.accent.g(), c.accent.b(), 50);
    visuals.selection.stroke = egui::Stroke::new(1.0, c.accent);
    visuals.window_stroke = egui::Stroke::new(1.0, c.border);
    visuals.window_corner_radius = egui::CornerRadius::same(6);

    visuals.widgets.noninteractive.bg_fill = c.surface;
    visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, c.text);
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, c.border);

    visuals.widgets.inactive.bg_fill = c.surface2;
    visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, c.text);
    visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, c.border);

    let hover_bg = if dark_mode {
        egui::Color32::from_rgb(55, 70, 90)
    } else {
        egui::Color32::from_rgb(210, 225, 240)
    };
    visuals.widgets.hovered.bg_fill = hover_bg;
    visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.5, c.accent);
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, c.accent);

    let active_bg = if dark_mode {
        egui::Color32::from_rgb(45, 60, 80)
    } else {
        egui::Color32::from_rgb(196, 215, 235)
    };
    visuals.widgets.active.bg_fill = active_bg;
    visuals.widgets.active.fg_stroke = egui::Stroke::new(2.0, c.accent);

    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    style.spacing.scroll.bar_width = 6.0;
    ctx.set_style(style);
}

impl eframe::App for MqOpenApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Re-apply our custom style whenever the theme preference changes.
        let dark_mode = ctx.style().visuals.dark_mode;
        if dark_mode != self.last_dark_mode {
            self.last_dark_mode = dark_mode;
            apply_custom_style(ctx, dark_mode);
        }

        let c = Colors::for_dark_mode(dark_mode);

        // Keyboard shortcuts
        let open_shortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::O);
        if ctx.input_mut(|i| i.consume_shortcut(&open_shortcut))
            && let Some(path) = rfd::FileDialog::new()
                .add_filter("Markdown", &["md", "markdown"])
                .pick_file()
        {
            self.open_file(path);
        }

        // File drop
        let dropped = ctx.input(|i| i.raw.dropped_files.first().and_then(|f| f.path.clone()));
        if let Some(path) = dropped {
            self.open_file(path);
        }

        // File-watcher updates
        let mut content_changed = false;
        while let Ok(content) = self.rx.try_recv() {
            self.source_content = content;
            content_changed = true;
        }
        if content_changed {
            self.run_query();
            self.update_toc();
            ctx.request_repaint();
        }

        // TOC Panel
        egui::SidePanel::left("toc_panel")
            .resizable(true)
            .default_width(220.0)
            .min_width(160.0)
            .frame(egui::Frame::new().fill(c.surface).inner_margin(0.0))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(c.surface2)
                    .inner_margin(egui::Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(
                            egui::RichText::new("TABLE OF CONTENTS")
                                .size(10.0)
                                .color(c.text_muted)
                                .strong(),
                        );
                    });
                ui.painter().line_segment(
                    [
                        ui.cursor().left_top(),
                        egui::pos2(
                            ui.cursor().left_top().x + ui.available_width(),
                            ui.cursor().left_top().y,
                        ),
                    ],
                    egui::Stroke::new(1.0, c.border),
                );

                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.add_space(6.0);

                    fn render_toc(
                        ui: &mut egui::Ui,
                        entries: &[TocEntry],
                        scroll_to: &mut Option<usize>,
                        text_color: egui::Color32,
                        muted_color: egui::Color32,
                    ) {
                        for entry in entries {
                            let indent = (entry.level.saturating_sub(1)) as f32 * 6.0;
                            if entry.children.is_empty() {
                                ui.horizontal(|ui| {
                                    ui.add_space(indent + 4.0);
                                    if ui
                                        .selectable_label(
                                            false,
                                            egui::RichText::new(&entry.title)
                                                .size(13.0)
                                                .color(muted_color),
                                        )
                                        .clicked()
                                    {
                                        *scroll_to = Some(entry.node_index);
                                    }
                                });
                            } else {
                                ui.horizontal(|ui| {
                                    ui.add_space(indent);
                                    egui::CollapsingHeader::new(
                                        egui::RichText::new(&entry.title)
                                            .size(13.0)
                                            .color(text_color),
                                    )
                                    .default_open(true)
                                    .show(ui, |ui| {
                                        render_toc(
                                            ui,
                                            &entry.children,
                                            scroll_to,
                                            text_color,
                                            muted_color,
                                        );
                                    });
                                });
                            }
                        }
                    }

                    if self.toc.is_empty() {
                        ui.add_space(16.0);
                        ui.centered_and_justified(|ui| {
                            ui.label(
                                egui::RichText::new("No headings found")
                                    .size(12.0)
                                    .color(c.text_muted)
                                    .italics(),
                            );
                        });
                    } else {
                        render_toc(
                            ui,
                            &self.toc,
                            &mut self.scroll_to_node,
                            c.text,
                            c.text_muted,
                        );
                    }
                    ui.add_space(6.0);
                });
            });

        // Top Panel
        egui::TopBottomPanel::top("top_panel")
            .frame(
                egui::Frame::new()
                    .fill(c.surface2)
                    .inner_margin(egui::Margin::symmetric(12, 8))
                    .stroke(egui::Stroke::new(1.0, c.border)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("mq")
                            .size(24.0)
                            .color(c.accent)
                            .strong(),
                    );
                    ui.add_space(8.0);

                    if ui
                        .add(
                            egui::Button::new(egui::RichText::new("Open").size(13.0).color(c.text))
                                .fill(c.surface)
                                .stroke(egui::Stroke::new(1.0, c.border))
                                .corner_radius(4.0),
                        )
                        .clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .add_filter("Markdown", &["md", "markdown"])
                            .pick_file()
                    {
                        self.open_file(path);
                    }

                    if let Some(ref path) = self.file_path {
                        let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                        ui.add_space(4.0);
                        egui::Frame::new()
                            .fill(c.surface)
                            .corner_radius(4.0)
                            .inner_margin(egui::Margin::symmetric(8, 3))
                            .stroke(egui::Stroke::new(1.0, c.border))
                            .show(ui, |ui| {
                                ui.label(
                                    egui::RichText::new(filename).size(12.0).color(c.text_muted),
                                );
                            });
                    } else {
                        ui.label(
                            egui::RichText::new("Drop a markdown file here or click Open")
                                .size(12.0)
                                .color(c.text_muted)
                                .italics(),
                        );
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        egui::widgets::global_theme_preference_buttons(ui);
                    });
                });
            });

        // Bottom Status Panel
        egui::TopBottomPanel::bottom("status_panel")
            .frame(
                egui::Frame::new()
                    .fill(c.surface2)
                    .inner_margin(egui::Margin::symmetric(12, 5))
                    .stroke(egui::Stroke::new(1.0, c.border)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    status_chip(ui, "Nodes", &self.stats.node_count.to_string(), &c);
                    status_chip(ui, "Words", &self.stats.word_count.to_string(), &c);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        status_chip(
                            ui,
                            "Query time",
                            &format!("{:.2?}", self.last_eval_duration),
                            &c,
                        );
                    });
                });
            });

        // Central Panel
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(c.bg).inner_margin(0.0))
            .show(ctx, |ui| {
                // Query bar
                egui::Frame::new()
                    .fill(c.surface)
                    .inner_margin(egui::Margin::symmetric(16, 10))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(">")
                                    .size(14.0)
                                    .color(c.accent)
                                    .strong(),
                            );
                            let response = ui.add(
                                egui::TextEdit::singleline(&mut self.query)
                                    .hint_text("Enter mq query (e.g. .h1, .code, self | .h2)")
                                    .font(egui::FontId::monospace(14.0))
                                    .text_color(c.text)
                                    .desired_width(f32::INFINITY),
                            );
                            if response.changed() {
                                self.run_query();
                            }
                        });
                    });

                // Divider
                ui.painter().line_segment(
                    [
                        ui.cursor().left_top(),
                        egui::pos2(ui.cursor().left_top().x + ui.available_width(), ui.cursor().left_top().y),
                    ],
                    egui::Stroke::new(1.0, c.border),
                );

                // Error banner
                if let Some(error) = self.error.clone() {
                    egui::Frame::new()
                        .fill(c.error_bg)
                        .inner_margin(egui::Margin::symmetric(16, 8))
                        .stroke(egui::Stroke::new(1.0, c.error))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new("⚠ ").color(c.error).size(13.0),
                                );
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(&error)
                                            .color(c.error)
                                            .size(13.0)
                                            .code(),
                                    )
                                    .wrap(),
                                );
                            });
                        });
                }

                // Content scroll area
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let content_width = ui.available_width();
                        ui.set_min_width(content_width);
                        ui.add_space(8.0);

                        egui::Frame::new()
                            .inner_margin(egui::Margin::symmetric(24, 4))
                            .show(ui, |ui| {
                                ui.set_max_width(content_width - 48.0);
                                ui.set_min_width(content_width - 48.0);

                                for (i, node) in self.results.iter().enumerate() {
                                    if self.scroll_to_node == Some(i) {
                                        ui.scroll_to_cursor(Some(egui::Align::TOP));
                                        self.scroll_to_node = None;
                                    }
                                    render_node(ui, node, 0, &c);
                                }

                                if self.results.is_empty() && self.error.is_none() {
                                    ui.add_space(32.0);
                                    ui.centered_and_justified(|ui| {
                                        ui.label(
                                            egui::RichText::new(
                                                "No results — drop a markdown file or modify the query",
                                            )
                                            .size(14.0)
                                            .color(c.text_muted)
                                            .italics(),
                                        );
                                    });
                                }
                            });

                        ui.add_space(24.0);
                    });
            });
    }
}

fn status_chip(ui: &mut egui::Ui, label: &str, value: &str, c: &Colors) {
    egui::Frame::new()
        .fill(c.surface)
        .corner_radius(3.0)
        .inner_margin(egui::Margin::symmetric(6, 2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(label).size(11.0).color(c.text_muted));
                ui.label(
                    egui::RichText::new(value)
                        .size(11.0)
                        .color(c.accent)
                        .strong(),
                );
            });
        });
}

fn render_node(ui: &mut egui::Ui, node: &Node, depth: usize, c: &Colors) {
    match node {
        Node::Heading(h) => {
            let size = match h.depth {
                1 => 30.0,
                2 => 24.0,
                3 => 20.0,
                4 => 17.0,
                5 => 15.0,
                _ => 13.0,
            };
            ui.add_space(if h.depth <= 2 { 14.0 } else { 8.0 });
            ui.label(
                egui::RichText::new(node.value())
                    .size(size)
                    .color(c.text)
                    .strong(),
            );
            if h.depth <= 2 {
                ui.add_space(2.0);
                let rect = ui.available_rect_before_wrap();
                let line_color = if h.depth == 1 { c.accent } else { c.border };
                ui.painter().line_segment(
                    [
                        egui::pos2(rect.left(), rect.top()),
                        egui::pos2(rect.right(), rect.top()),
                    ],
                    egui::Stroke::new(if h.depth == 1 { 2.0 } else { 1.0 }, line_color),
                );
                ui.add_space(4.0);
            } else {
                ui.add_space(2.0);
            }
        }
        Node::Text(t) => {
            ui.add(egui::Label::new(egui::RichText::new(&t.value).color(c.text).size(14.0)).wrap());
        }
        Node::Code(code) => {
            ui.add_space(6.0);
            egui::Frame::new()
                .fill(c.code_bg)
                .corner_radius(6.0)
                .inner_margin(egui::Margin::symmetric(14, 12))
                .stroke(egui::Stroke::new(1.0, c.border))
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    if let Some(lang) = &code.lang
                        && !lang.is_empty()
                    {
                        ui.label(
                            egui::RichText::new(lang.as_str())
                                .size(10.0)
                                .color(c.accent)
                                .strong(),
                        );
                        ui.add_space(4.0);
                    }
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(&code.value)
                                .font(egui::FontId::monospace(13.0))
                                .color(c.code_text),
                        )
                        .wrap(),
                    );
                });
            ui.add_space(6.0);
        }
        Node::CodeInline(code) => {
            egui::Frame::new()
                .fill(c.code_bg)
                .corner_radius(3.0)
                .inner_margin(egui::Margin::symmetric(5, 2))
                .stroke(egui::Stroke::new(1.0, c.border))
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(code.value.as_str())
                            .font(egui::FontId::monospace(12.0))
                            .color(c.code_text),
                    );
                });
        }
        Node::Html(h) => {
            ui.add_space(6.0);
            egui::Frame::new()
                .fill(c.html_bg)
                .corner_radius(6.0)
                .inner_margin(egui::Margin::symmetric(14, 12))
                .stroke(egui::Stroke::new(1.0, c.html_border))
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.label(
                        egui::RichText::new("HTML")
                            .size(10.0)
                            .color(c.html_border)
                            .strong(),
                    );
                    ui.add_space(4.0);
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(&h.value)
                                .font(egui::FontId::monospace(13.0))
                                .color(egui::Color32::from_rgb(180, 83, 9)),
                        )
                        .wrap(),
                    );
                });
            ui.add_space(6.0);
        }
        Node::Yaml(y) => {
            ui.add_space(6.0);
            let (bg, border, label_color, text_color) = if ui.style().visuals.dark_mode {
                (
                    egui::Color32::from_rgb(15, 30, 30),
                    egui::Color32::from_rgb(20, 160, 130),
                    egui::Color32::from_rgb(45, 212, 191),
                    egui::Color32::from_rgb(153, 246, 228),
                )
            } else {
                (
                    egui::Color32::from_rgb(236, 254, 255),
                    egui::Color32::from_rgb(8, 145, 178),
                    egui::Color32::from_rgb(8, 145, 178),
                    egui::Color32::from_rgb(14, 116, 144),
                )
            };
            egui::Frame::new()
                .fill(bg)
                .corner_radius(6.0)
                .inner_margin(egui::Margin::symmetric(14, 12))
                .stroke(egui::Stroke::new(1.0, border))
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.label(
                        egui::RichText::new("FRONTMATTER · YAML")
                            .size(10.0)
                            .color(label_color)
                            .strong(),
                    );
                    ui.add_space(4.0);
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(&y.value)
                                .font(egui::FontId::monospace(13.0))
                                .color(text_color),
                        )
                        .wrap(),
                    );
                });
            ui.add_space(6.0);
        }
        Node::Toml(t) => {
            ui.add_space(6.0);
            let (bg, border, label_color, text_color) = if ui.style().visuals.dark_mode {
                (
                    egui::Color32::from_rgb(20, 15, 30),
                    egui::Color32::from_rgb(120, 80, 200),
                    egui::Color32::from_rgb(167, 139, 250),
                    egui::Color32::from_rgb(216, 180, 254),
                )
            } else {
                (
                    egui::Color32::from_rgb(250, 245, 255),
                    egui::Color32::from_rgb(126, 34, 206),
                    egui::Color32::from_rgb(126, 34, 206),
                    egui::Color32::from_rgb(107, 33, 168),
                )
            };
            egui::Frame::new()
                .fill(bg)
                .corner_radius(6.0)
                .inner_margin(egui::Margin::symmetric(14, 12))
                .stroke(egui::Stroke::new(1.0, border))
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.label(
                        egui::RichText::new("FRONTMATTER · TOML")
                            .size(10.0)
                            .color(label_color)
                            .strong(),
                    );
                    ui.add_space(4.0);
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(&t.value)
                                .font(egui::FontId::monospace(13.0))
                                .color(text_color),
                        )
                        .wrap(),
                    );
                });
            ui.add_space(6.0);
        }
        Node::Math(m) => {
            ui.add_space(6.0);
            let (bg, border, label_color, text_color) = if ui.style().visuals.dark_mode {
                (
                    egui::Color32::from_rgb(10, 20, 35),
                    egui::Color32::from_rgb(56, 189, 248),
                    egui::Color32::from_rgb(56, 189, 248),
                    egui::Color32::from_rgb(186, 230, 253),
                )
            } else {
                (
                    egui::Color32::from_rgb(240, 249, 255),
                    egui::Color32::from_rgb(2, 132, 199),
                    egui::Color32::from_rgb(2, 132, 199),
                    egui::Color32::from_rgb(7, 89, 133),
                )
            };
            egui::Frame::new()
                .fill(bg)
                .corner_radius(6.0)
                .inner_margin(egui::Margin::symmetric(14, 12))
                .stroke(egui::Stroke::new(1.0, border))
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.label(
                        egui::RichText::new("MATH")
                            .size(10.0)
                            .color(label_color)
                            .strong(),
                    );
                    ui.add_space(4.0);
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(&m.value)
                                .font(egui::FontId::monospace(13.0))
                                .color(text_color),
                        )
                        .wrap(),
                    );
                });
            ui.add_space(6.0);
        }
        Node::MathInline(m) => {
            let (bg, border, text_color) = if ui.style().visuals.dark_mode {
                (
                    egui::Color32::from_rgb(10, 20, 35),
                    egui::Color32::from_rgb(56, 189, 248),
                    egui::Color32::from_rgb(186, 230, 253),
                )
            } else {
                (
                    egui::Color32::from_rgb(240, 249, 255),
                    egui::Color32::from_rgb(2, 132, 199),
                    egui::Color32::from_rgb(7, 89, 133),
                )
            };
            egui::Frame::new()
                .fill(bg)
                .corner_radius(3.0)
                .inner_margin(egui::Margin::symmetric(5, 2))
                .stroke(egui::Stroke::new(1.0, border))
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(m.value.as_str())
                            .font(egui::FontId::monospace(12.0))
                            .color(text_color),
                    );
                });
        }
        Node::List(l) => {
            ui.add_space(2.0);
            let marker = if l.ordered {
                format!("{}.", l.index + 1)
            } else {
                "•".to_string()
            };
            ui.horizontal(|ui| {
                ui.add_space(depth as f32 * 16.0 + 4.0);
                ui.label(
                    egui::RichText::new(&marker)
                        .color(c.accent)
                        .size(14.0)
                        .strong(),
                );
                ui.horizontal(|ui| {
                    for child in &l.values {
                        render_node(ui, child, depth + 1, c);
                    }
                });
            });
            ui.add_space(2.0);
        }
        Node::Blockquote(b) => {
            ui.add_space(6.0);
            let accent_tinted =
                egui::Color32::from_rgba_unmultiplied(c.accent.r(), c.accent.g(), c.accent.b(), 18);
            egui::Frame::new()
                .fill(accent_tinted)
                .corner_radius(egui::CornerRadius {
                    nw: 0,
                    ne: 4,
                    sw: 0,
                    se: 4,
                })
                .inner_margin(egui::Margin {
                    left: 16,
                    right: 12,
                    top: 10,
                    bottom: 10,
                })
                .stroke(egui::Stroke::NONE)
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    let rect = ui.available_rect_before_wrap();
                    ui.painter().rect_filled(
                        egui::Rect::from_min_size(
                            egui::pos2(rect.left() - 16.0, rect.top()),
                            egui::vec2(3.0, rect.height().max(20.0)),
                        ),
                        0.0,
                        c.accent,
                    );
                    for child in &b.values {
                        render_node(ui, child, depth + 1, c);
                    }
                });
            ui.add_space(6.0);
        }
        Node::Strong(_) => {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(node.value())
                        .strong()
                        .color(c.text)
                        .size(14.0),
                )
                .wrap(),
            );
        }
        Node::Emphasis(_) => {
            let color = if ui.style().visuals.dark_mode {
                egui::Color32::from_rgb(186, 230, 253)
            } else {
                egui::Color32::from_rgb(7, 89, 133)
            };
            ui.add(
                egui::Label::new(
                    egui::RichText::new(node.value())
                        .italics()
                        .color(color)
                        .size(14.0),
                )
                .wrap(),
            );
        }
        Node::Delete(_) => {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(node.value())
                        .strikethrough()
                        .color(c.text_muted)
                        .size(14.0),
                )
                .wrap(),
            );
        }
        Node::Link(l) => {
            ui.hyperlink_to(
                egui::RichText::new(node.value()).color(c.accent).size(14.0),
                l.url.as_str(),
            );
        }
        Node::Image(i) => {
            ui.add_space(6.0);
            egui::Frame::new()
                .fill(c.surface2)
                .corner_radius(6.0)
                .inner_margin(egui::Margin::symmetric(8, 8))
                .stroke(egui::Stroke::new(1.0, c.border))
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.add(
                        egui::Image::new(&i.url)
                            .max_width(ui.available_width())
                            .corner_radius(4.0),
                    );
                    if !i.alt.is_empty() {
                        ui.add_space(4.0);
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(&i.alt)
                                    .size(11.0)
                                    .color(c.text_muted)
                                    .italics(),
                            )
                            .wrap(),
                        );
                    }
                });
            ui.add_space(6.0);
        }
        Node::HorizontalRule(_) => {
            ui.add_space(8.0);
            let rect = ui.available_rect_before_wrap();
            ui.painter().line_segment(
                [
                    egui::pos2(rect.left(), rect.top()),
                    egui::pos2(rect.right(), rect.top()),
                ],
                egui::Stroke::new(1.0, c.border),
            );
            ui.add_space(10.0);
        }
        Node::Fragment(f) => {
            for child in &f.values {
                render_node(ui, child, depth, c);
            }
        }
        Node::Break(_) => {
            ui.add_space(4.0);
        }
        Node::TableCell(cell) => {
            for child in &cell.values {
                render_node(ui, child, depth, c);
            }
        }
        Node::TableRow(r) => {
            ui.horizontal(|ui| {
                for cell in &r.values {
                    egui::Frame::new()
                        .fill(c.surface)
                        .inner_margin(egui::Margin::symmetric(10, 5))
                        .stroke(egui::Stroke::new(1.0, c.border))
                        .show(ui, |ui| {
                            ui.set_min_width(80.0);
                            render_node(ui, cell, depth, c);
                        });
                }
            });
        }
        Node::TableAlign(_) => {
            let rect = ui.available_rect_before_wrap();
            ui.painter().line_segment(
                [
                    egui::pos2(rect.left(), rect.top()),
                    egui::pos2(rect.right(), rect.top()),
                ],
                egui::Stroke::new(2.0, c.accent),
            );
            ui.add_space(2.0);
        }
        Node::Footnote(f) => {
            ui.add_space(4.0);
            egui::Frame::new()
                .fill(c.surface2)
                .corner_radius(4.0)
                .inner_margin(egui::Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(format!("[{}]", f.ident))
                                .size(11.0)
                                .color(c.accent)
                                .strong(),
                        );
                        for child in &f.values {
                            render_node(ui, child, depth + 1, c);
                        }
                    });
                });
        }
        Node::FootnoteRef(f) => {
            ui.label(
                egui::RichText::new(format!("[{}]", f.ident))
                    .size(11.0)
                    .color(c.accent)
                    .strong()
                    .raised(),
            );
        }
        _ => {
            let text = node.to_string();
            if !text.is_empty() {
                ui.add(egui::Label::new(egui::RichText::new(text).color(c.text).size(14.0)).wrap());
            }
        }
    }
}
