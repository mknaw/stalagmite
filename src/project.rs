use std::io::Write;
use std::{fs, io};

pub fn initialize() -> io::Result<()> {
    fs::create_dir("pages")?;
    let mut layout_file = fs::File::create("layout.liquid")?;
    layout_file.write_all(include_str!("../assets/layout.liquid").as_bytes())?;
    Ok(())
}
