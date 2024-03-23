use std::io::Write;

use liquid_core::{Language, ParseTag, Renderable, Result, Runtime, TagReflection, TagTokenIter};

use crate::liquid::tags::static_asset::get_actual_filename;

#[derive(Copy, Clone, Debug, Default)]
pub struct TailwindTag;

impl TagReflection for TailwindTag {
    fn tag(&self) -> &str {
        "tailwind"
    }

    fn description(&self) -> &str {
        "Include generated tailwind stylesheet."
    }
}

impl ParseTag for TailwindTag {
    fn parse(
        &self,
        mut arguments: TagTokenIter,
        _options: &Language,
    ) -> Result<Box<dyn Renderable>> {
        arguments.expect_nothing()?;
        Ok(Box::new(Tailwind {}))
    }

    fn reflection(&self) -> &dyn TagReflection {
        self
    }
}

#[derive(Debug)]
struct Tailwind;

impl Renderable for Tailwind {
    /// Include the tailwind stylesheet.
    fn render_to(&self, writer: &mut dyn Write, runtime: &dyn Runtime) -> Result<()> {
        let tailwind_filename = get_actual_filename("tw.css", runtime)?;
        let include_tag = format!(r#"<link rel="stylesheet" href="{}">"#, tailwind_filename);
        // TODO really should convert to a liquid::Error
        writer.write_all(include_tag.as_bytes()).unwrap();
        Ok(())
    }
}
