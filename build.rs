use {std::io, winres::WindowsResource};

fn main() -> io::Result<()> {
    #[cfg(target_os = "windows")]
    WindowsResource::new()
        .set_icon("assets/icon.ico")
        .compile()?;
    Ok(())
}
