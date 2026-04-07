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
        "pptxMarge",
        options,
        Box::new(|cc| {
            configure_fonts(&cc.egui_ctx);
            Ok(Box::new(app::PptxMargeApp::default()))
        }),
    )
}

fn configure_fonts(ctx: &eframe::egui::Context) {
    let mut fonts = eframe::egui::FontDefinitions::default();

    // Try to load a Japanese font from the system
    let font_paths = [
        "/usr/share/fonts/opentype/ipafont-gothic/ipagp.ttf",
        "/usr/share/fonts/opentype/ipafont-gothic/ipag.ttf",
        "/usr/share/fonts/truetype/fonts-japanese-gothic.ttf",
        // macOS
        "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc",
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
        // Windows
        "C:\\Windows\\Fonts\\msgothic.ttc",
        "C:\\Windows\\Fonts\\meiryo.ttc",
        "C:\\Windows\\Fonts\\YuGothR.ttc",
    ];

    for path in &font_paths {
        if let Ok(font_data) = std::fs::read(path) {
            fonts.font_data.insert(
                "japanese_font".to_string(),
                eframe::egui::FontData::from_owned(font_data).into(),
            );
            // Add Japanese font as fallback for proportional and monospace
            if let Some(family) = fonts
                .families
                .get_mut(&eframe::egui::FontFamily::Proportional)
            {
                family.push("japanese_font".to_string());
            }
            if let Some(family) = fonts
                .families
                .get_mut(&eframe::egui::FontFamily::Monospace)
            {
                family.push("japanese_font".to_string());
            }
            break;
        }
    }

    ctx.set_fonts(fonts);
}
