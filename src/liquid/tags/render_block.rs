use std::io::Write;

use liquid_core::runtime::{StackFrame, Variable};
use liquid_core::{
    Error, Expression, Language, ObjectView, ParseTag, Renderable, Result, Runtime, TagReflection,
    TagTokenIter, ValueView,
};

use crate::renderer::BLOCK_RULES_TEMPLATE_VAR;

#[derive(Copy, Clone, Debug, Default)]
pub struct RenderBlockTag;

impl TagReflection for RenderBlockTag {
    fn tag(&self) -> &str {
        "render_block"
    }

    fn description(&self) -> &str {
        "Render a block in accordance with the rules of this directory."
    }
}

impl ParseTag for RenderBlockTag {
    fn parse(
        &self,
        mut arguments: TagTokenIter,
        _options: &Language,
    ) -> Result<Box<dyn Renderable>> {
        let block = arguments.expect_next("Identifier or literal expected.")?;

        let block = block.expect_value().into_result()?;

        arguments.expect_nothing()?;

        Ok(Box::new(RenderBlock { block }))
    }

    fn reflection(&self) -> &dyn TagReflection {
        self
    }
}

#[derive(Debug)]
struct RenderBlock {
    block: Expression,
    // TODO could allow for an ad-hoc override
}

fn find_partial_name(block_rules: &dyn ObjectView, kind: &str) -> Result<String> {
    let partial_name = block_rules
        .get(kind)
        .map_or(kind.to_string(), |v| v.to_kstr().as_str().to_string());
    Ok(format!("blocks/{}.liquid", partial_name))
}

// TODO deduplicate this functionality vis-a-vis `render.rs`.
fn render_block(
    runtime: &dyn Runtime,
    block: &dyn ObjectView,
    block_rules: &dyn ObjectView,
) -> Result<String> {
    let value = block
        .get("kind")
        .ok_or(Error::with_msg("Malformed block - missing `kind`"))?;
    let kind = value.to_kstr().as_str().to_string();
    let tokens = block
        .get("tokens")
        .ok_or(Error::with_msg("Malformed block - missing `tokens`"))?;
    let tokens = tokens
        .as_array()
        .ok_or(Error::with_msg("Malformed block - `tokens` not an array"))?;

    let content = tokens
        .values()
        .filter_map(|token| {
            let token = token.as_object().unwrap();
            if let Some(literal) = token.get("Literal") {
                Some(literal.to_kstr().as_str().into())
            } else if let Some(nested_block) = token.get("Block") {
                let nested_block = nested_block.as_object().unwrap();
                render_block(runtime, nested_block, block_rules).ok()
            } else {
                // TODO or error out here, because this would imply something is malformed...
                None
            }
        })
        .collect::<Vec<String>>()
        .join("");

    let partial_name = find_partial_name(block_rules, &kind)?;
    let partial = runtime.partials().get(&partial_name)?;
    let pass_through = liquid::object!({
        "content": content,
    });
    let scope = StackFrame::new(runtime, &pass_through);
    partial.render(&scope)
}

impl Renderable for RenderBlock {
    fn render_to(&self, writer: &mut dyn Write, runtime: &dyn Runtime) -> Result<()> {
        let value = self.block.evaluate(runtime)?;
        let block = value
            .as_object()
            .ok_or(Error::with_msg("Can only render blocks"))?;

        // TODO this bit seems like a lot of fanfare to get the value of the var!
        let rules_expr = Variable::with_literal(BLOCK_RULES_TEMPLATE_VAR);
        let rules_path = rules_expr.evaluate(runtime)?;
        // TODO has to be an `as_object`, but we could be more civil with an Err message.
        let block_rules = runtime.get(&rules_path)?;
        let block_rules = block_rules.as_object().unwrap();

        // TODO really should convert to a liquid::Error
        writer
            .write_all(render_block(runtime, block, &block_rules)?.as_bytes())
            .unwrap();
        Ok(())
    }
}
