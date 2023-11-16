use liquid_core::{
    Display_filter, Expression, Filter, FilterParameters, FilterReflection, FromFilterParameters,
    ParseFilter, Result, Runtime, Value, ValueView,
};

#[derive(Debug, FilterParameters)]
struct KindArgs {
    #[parameter(description = "The kind of the block to find.", arg_type = "str")]
    kind: Expression,
}

#[derive(Clone, ParseFilter, FilterReflection)]
#[filter(
    name = "first_block_of_kind",
    description = "Returns the first block of the matching kind.",
    parameters(KindArgs),
    parsed(FirstBlockOfKindFilter)
)]
pub struct FirstBlockOfKind;

#[derive(Debug, FromFilterParameters, Display_filter)]
#[name = "first_block_of_kind"]
struct FirstBlockOfKindFilter {
    #[parameters]
    args: KindArgs,
}

impl Filter for FirstBlockOfKindFilter {
    fn evaluate(&self, input: &dyn ValueView, runtime: &dyn Runtime) -> Result<Value> {
        if let Some(blocks) = input
            .as_object()
            .and_then(|entry| entry.get("blocks"))
            .and_then(|blocks| blocks.as_array())
        {
            let args = self.args.evaluate(runtime)?;

            if let Some(block) = blocks
                .values()
                .flat_map(|b| b.as_object())
                .find(|b| b.get("kind").map(|v| v.to_kstr()).as_ref() == Some(&args.kind))
            {
                return Ok(block.to_value());
            }
        }
        Err(liquid_core::Error::with_msg("Block not found"))
    }
}
