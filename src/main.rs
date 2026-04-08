#![windows_subsystem = "windows"]

mod app;
mod merge;
mod pptx;

fn main() -> eframe::Result {
    let icon = load_icon();

    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_inner_size([600.0, 500.0])
        .with_min_inner_size([400.0, 300.0]);

    if let Some(icon_data) = icon {
        viewport = viewport.with_icon(icon_data);
    }

    let options = eframe::NativeOptions {
        viewport,
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

/// Load icon.png from next to the executable or current directory.
fn load_icon() -> Option<eframe::egui::IconData> {
    let icon_bytes = find_and_read_icon()?;
    let img = image::load_from_memory(&icon_bytes).ok()?.into_rgba8();
    let (w, h) = img.dimensions();
    Some(eframe::egui::IconData {
        rgba: img.into_raw(),
        width: w,
        height: h,
    })
}

fn find_and_read_icon() -> Option<Vec<u8>> {
    // Check next to executable
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p = dir.join("icon.png");
            if let Ok(data) = std::fs::read(&p) {
                return Some(data);
            }
        }
    }
    // Check current directory
    std::fs::read("icon.png").ok()
}

fn configure_fonts(ctx: &eframe::egui::Context) {
    let mut fonts = eframe::egui::FontDefinitions::default();

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
