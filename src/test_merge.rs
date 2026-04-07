mod merge;
mod pptx;

use std::path::PathBuf;

fn main() {
    let files: Vec<PathBuf> = vec![
        PathBuf::from("test_data/a.pptx"),
        PathBuf::from("test_data/b.pptx"),
        PathBuf::from("test_data/c.pptx"),
    ];
    let output = PathBuf::from("test_data/merged.pptx");

    match merge::merge_pptx_files(&files, &output) {
        Ok(()) => println!("Merge successful: {}", output.display()),
        Err(e) => {
            eprintln!("Merge failed: {:#}", e);
            std::process::exit(1);
        }
    }
}
