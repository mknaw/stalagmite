use std::io::Write;

use liquid_core::runtime::Variable;
use liquid_core::{
    Language, ParseTag, Renderable, Result, Runtime, TagReflection, TagTokenIter, ValueView,
};

use crate::renderer::TAILWIND_FILENAME_TEMPLATE_VAR;

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
    fn render_to(&self, writer: &mut dyn Write, runtime: &dyn Runtime) -> Result<()> {
        // TODO this bit seems like a lot of fanfare to get the value of the var!
        let tailwind_expr = Variable::with_literal(TAILWIND_FILENAME_TEMPLATE_VAR);
        let tailwind_path = tailwind_expr.evaluate(runtime)?;
        // TODO has to be an `as_object`, but we could be more civil with an Err message.
        let tailwind_filename = runtime.get(&tailwind_path)?;
        let tailwind_filename = tailwind_filename.as_scalar().unwrap();
        let tailwind_filename = tailwind_filename.to_kstr().to_string();

        // TODO probably should actually leverage liquid for this, but this is quicker for now.
        let include = format!(
            r#"<link rel="stylesheet" href="/static/{}">"#,
            tailwind_filename
        );
        // TODO really should convert to a liquid::Error
        writer.write_all(include.as_bytes()).unwrap();
        Ok(())
    }
}
