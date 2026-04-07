use std::path::PathBuf;

use eframe::egui;

use crate::merge;

pub struct PptxMargeApp {
    files: Vec<PathBuf>,
    output_path: Option<PathBuf>,
    status: Status,
}

enum Status {
    Idle,
    Success(String),
    Error(String),
}

impl Default for PptxMargeApp {
    fn default() -> Self {
        Self {
            files: Vec::new(),
            output_path: None,
            status: Status::Idle,
        }
    }
}

impl eframe::App for PptxMargeApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("pptxMarge - PowerPointファイル結合ツール");
            ui.add_space(8.0);

            // Add files button
            if ui.button("📂 ファイルを追加...").clicked() {
                if let Some(paths) = rfd::FileDialog::new()
                    .add_filter("PowerPoint", &["pptx"])
                    .pick_files()
                {
                    for path in paths {
                        if !self.files.contains(&path) {
                            self.files.push(path);
                        }
                    }
                    self.update_default_output();
                }
            }

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(4.0);

            // File list
            ui.label(format!("ファイル一覧 ({}件):", self.files.len()));

            let mut action: Option<ListAction> = None;

            egui::ScrollArea::vertical()
                .max_height(250.0)
                .show(ui, |ui| {
                    for i in 0..self.files.len() {
                        ui.horizontal(|ui| {
                            // Number
                            ui.label(format!("{}.", i + 1));

                            // File name
                            let name = self.files[i]
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_else(|| self.files[i].display().to_string());
                            ui.label(&name).on_hover_text(self.files[i].display().to_string());

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                // Delete button
                                if ui.button("✕").on_hover_text("削除").clicked() {
                                    action = Some(ListAction::Remove(i));
                                }
                                // Move down
                                if i < self.files.len() - 1 {
                                    if ui.button("▼").on_hover_text("下に移動").clicked() {
                                        action = Some(ListAction::MoveDown(i));
                                    }
                                }
                                // Move up
                                if i > 0 {
                                    if ui.button("▲").on_hover_text("上に移動").clicked() {
                                        action = Some(ListAction::MoveUp(i));
                                    }
                                }
                            });
                        });
                        ui.separator();
                    }

                    if self.files.is_empty() {
                        ui.label("ファイルが追加されていません。「ファイルを追加」ボタンをクリックしてください。");
                    }
                });

            // Apply action
            match action {
                Some(ListAction::MoveUp(i)) => {
                    self.files.swap(i, i - 1);
                }
                Some(ListAction::MoveDown(i)) => {
                    self.files.swap(i, i + 1);
                }
                Some(ListAction::Remove(i)) => {
                    self.files.remove(i);
                    self.update_default_output();
                }
                None => {}
            }

            ui.add_space(8.0);

            // Output path
            ui.horizontal(|ui| {
                ui.label("出力先:");
                let display = self
                    .output_path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "未設定".to_string());
                ui.label(&display);
                if ui.button("変更...").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("PowerPoint", &["pptx"])
                        .set_file_name("merged.pptx")
                        .save_file()
                    {
                        self.output_path = Some(path);
                    }
                }
            });

            ui.add_space(12.0);

            // Merge button
            let can_merge = self.files.len() >= 2 && self.output_path.is_some();
            if ui.add_enabled(can_merge, egui::Button::new("🔗 結合する")).clicked() {
                self.do_merge();
            }

            if !can_merge && !self.files.is_empty() {
                if self.files.len() < 2 {
                    ui.small("※ 2つ以上のファイルを追加してください");
                }
            }

            ui.add_space(8.0);

            // Status
            match &self.status {
                Status::Idle => {}
                Status::Success(msg) => {
                    ui.colored_label(egui::Color32::from_rgb(0, 160, 0), msg);
                }
                Status::Error(msg) => {
                    ui.colored_label(egui::Color32::from_rgb(220, 0, 0), format!("エラー: {}", msg));
                }
            }
        });
    }
}

enum ListAction {
    MoveUp(usize),
    MoveDown(usize),
    Remove(usize),
}

impl PptxMargeApp {
    fn update_default_output(&mut self) {
        if self.output_path.is_none() || self.files.is_empty() {
            self.output_path = self.files.first().and_then(|f| f.parent()).map(|dir| dir.join("merged.pptx"));
        }
    }

    fn do_merge(&mut self) {
        let output = match &self.output_path {
            Some(p) => p.clone(),
            None => return,
        };

        match merge::merge_pptx_files(&self.files, &output) {
            Ok(()) => {
                self.status = Status::Success(format!(
                    "結合が完了しました！ ({} ファイル → {})",
                    self.files.len(),
                    output.display()
                ));
            }
            Err(e) => {
                self.status = Status::Error(format!("{:#}", e));
            }
        }
    }
}
