mod app;
mod merge;
mod pptx;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([600.0, 500.0])
            .with_min_inner_size([400.0, 300.0]),
        ..Default::default()
    };
    eframe::run_native(
        "pptxMarge - PowerPointファイル結合ツール",
        options,
        Box::new(|_cc| Ok(Box::new(app::PptxMargeApp::default()))),
    )
}
