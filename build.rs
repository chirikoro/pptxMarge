fn main() {
    #[cfg(windows)]
    {
        // Convert icon.png to icon.ico if needed, then embed in exe
        let ico_path = std::path::Path::new("icon.ico");
        let png_path = std::path::Path::new("icon.png");

        if png_path.exists() && !ico_path.exists() {
            // Convert PNG to ICO using the image crate at build time
            // We'll create a simple script approach instead
            println!("cargo:warning=icon.png found. Please convert to icon.ico for Windows icon embedding.");
            println!("cargo:warning=You can use: magick convert icon.png icon.ico");
        }

        if ico_path.exists() {
            let mut res = winresource::WindowsResource::new();
            res.set_icon("icon.ico");
            if let Err(e) = res.compile() {
                println!("cargo:warning=Failed to set icon: {}", e);
            }
        }
    }
}
