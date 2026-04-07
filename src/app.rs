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

enum CleanupResult {
    Success,
    NoPython(String),
    Failed(String),
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
                // Try to clean up via python-pptx if available
                match self.try_python_cleanup(&output) {
                    CleanupResult::Success => {
                        self.status = Status::Success(format!(
                            "結合が完了しました！ ({} ファイル → {}) [python-pptx正規化済み]",
                            self.files.len(),
                            output.display(),
                        ));
                    }
                    CleanupResult::NoPython(reason) => {
                        self.status = Status::Success(format!(
                            "結合が完了しました！ ({} ファイル → {})\n※ python-pptx正規化スキップ: {}",
                            self.files.len(),
                            output.display(),
                            reason,
                        ));
                    }
                    CleanupResult::Failed(err) => {
                        self.status = Status::Success(format!(
                            "結合が完了しました！ ({} ファイル → {})\n※ python-pptx正規化失敗: {}",
                            self.files.len(),
                            output.display(),
                            err,
                        ));
                    }
                }
            }
            Err(e) => {
                self.status = Status::Error(format!("{:#}", e));
            }
        }
    }

    /// Try to run python-pptx cleanup on the merged file.
    fn try_python_cleanup(&self, path: &std::path::Path) -> CleanupResult {
        let temp_pptx = path.with_extension("_tmp.pptx");

        // Rename original to temp
        if std::fs::rename(path, &temp_pptx).is_err() {
            return CleanupResult::Failed("ファイルリネーム失敗".to_string());
        }

        // Write Python script to temp directory
        let temp_dir = std::env::temp_dir();
        let script_path = temp_dir.join("pptx_marge_cleanup.py");
        let script =
r#"import sys
try:
    from pptx import Presentation
except ImportError:
    sys.stderr.write("NO_PPTX")
    sys.exit(2)
try:
    p = Presentation(sys.argv[1])
    p.save(sys.argv[2])
except Exception as e:
    sys.stderr.write("ERR:" + str(e))
    sys.exit(1)
"#;

        if std::fs::write(&script_path, script).is_err() {
            let _ = std::fs::rename(&temp_pptx, path);
            return CleanupResult::Failed("スクリプト書き込み失敗".to_string());
        }

        // Build list of Python executables to try
        let mut python_paths: Vec<std::path::PathBuf> = Vec::new();

        // 1. pyenv on Windows: check user home directory
        if cfg!(windows) {
            if let Ok(home) = std::env::var("USERPROFILE") {
                let pyenv_shim = std::path::PathBuf::from(&home)
                    .join(".pyenv").join("pyenv-win").join("shims").join("python.exe");
                if pyenv_shim.exists() {
                    python_paths.push(pyenv_shim);
                }
                // Also check versions directory directly
                let versions_dir = std::path::PathBuf::from(&home)
                    .join(".pyenv").join("pyenv-win").join("versions");
                if let Ok(entries) = std::fs::read_dir(&versions_dir) {
                    for entry in entries.flatten() {
                        let p = entry.path().join("python.exe");
                        if p.exists() {
                            python_paths.push(p);
                        }
                    }
                }
            }
        }

        // 2. Standard commands
        if cfg!(windows) {
            python_paths.push("py".into());
            python_paths.push("python".into());
            python_paths.push("python3".into());
        } else {
            python_paths.push("python3".into());
            python_paths.push("python".into());
        }

        let mut last_err = String::new();
        for cmd in &python_paths {
            let result = std::process::Command::new(cmd)
                .arg(script_path.to_str().unwrap_or(""))
                .arg(temp_pptx.to_str().unwrap_or(""))
                .arg(path.to_str().unwrap_or(""))
                .output();

            match result {
                Ok(output) if output.status.success() && path.exists() => {
                    let _ = std::fs::remove_file(&temp_pptx);
                    let _ = std::fs::remove_file(&script_path);
                    return CleanupResult::Success;
                }
                Ok(output) => {
                    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                    if stderr.contains("NO_PPTX") {
                        let _ = std::fs::rename(&temp_pptx, path);
                        let _ = std::fs::remove_file(&script_path);
                        return CleanupResult::NoPython("pip install python-pptx を実行してください".to_string());
                    }
                    last_err = format!("({}) {}", cmd.display(), stderr);
                }
                Err(_) => continue,
            }
        }

        // Restore original on failure
        if !path.exists() {
            let _ = std::fs::rename(&temp_pptx, path);
        } else {
            let _ = std::fs::remove_file(&temp_pptx);
        }
        let _ = std::fs::remove_file(&script_path);

        if last_err.is_empty() {
            CleanupResult::NoPython("Pythonが見つかりません".to_string())
        } else {
            CleanupResult::Failed(last_err)
        }
    }
}
