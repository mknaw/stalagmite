use std::io::Write;

use liquid_core::runtime::Variable;
use liquid_core::{
    Error, Expression, Language, ObjectView, ParseTag, Renderable, Result, Runtime, TagReflection,
    TagTokenIter, ValueView,
};

use crate::renderer::STATIC_ASSET_MAP_TEMPLATE_VAR;

#[derive(Copy, Clone, Debug, Default)]
pub struct StaticAssetTag;

// TODO might be best to somehow ignore this tag the first time around, save that version to DB
// and only sub in the cache busted version on the second pass, if nothing else has changed.
impl TagReflection for StaticAssetTag {
    fn tag(&self) -> &str {
        "static_asset"
    }

    fn description(&self) -> &str {
        "Render an inclusion tag for a static asset."
    }
}

impl ParseTag for StaticAssetTag {
    fn parse(
        &self,
        mut arguments: TagTokenIter,
        _options: &Language,
    ) -> Result<Box<dyn Renderable>> {
        let filename = arguments.expect_next("Filename expected.")?;
        let filename = filename.expect_value().into_result()?;
        arguments.expect_nothing()?;
        Ok(Box::new(StaticAsset { filename }))
    }

    fn reflection(&self) -> &dyn TagReflection {
        self
    }
}

#[derive(Debug)]
struct StaticAsset {
    filename: Expression,
}

fn find_cache_busted_filename(static_assets: &dyn ObjectView, filename: &str) -> Result<String> {
    let cache_busted_name = static_assets
        .get(filename)
        .map_or(filename.to_string(), |v| v.to_kstr().as_str().to_string());
    Ok(cache_busted_name)
}

impl Renderable for StaticAsset {
    fn render_to(&self, writer: &mut dyn Write, runtime: &dyn Runtime) -> Result<()> {
        let value = self.filename.evaluate(runtime)?;
        let filename = value
            .as_scalar()
            .ok_or(Error::with_msg("Expected a filename"))?
            .into_string()
            .into_string();

        // TODO this bit seems like a lot of fanfare to get the value of the var!
        let static_asset_map_path = Variable::with_literal(STATIC_ASSET_MAP_TEMPLATE_VAR);
        let static_asset_map_path = static_asset_map_path.evaluate(runtime)?;
        // TODO has to be an `as_object`, but we could be more civil with an Err message.
        let static_asset_map = runtime.get(&static_asset_map_path)?;
        let static_asset_map = static_asset_map.as_object().unwrap();

        let cache_busted_filename = format!(
            "static/{}",
            find_cache_busted_filename(static_asset_map, &filename)?
        );

        // TODO really should convert to a liquid::Error
        writer.write_all(cache_busted_filename.as_bytes()).unwrap();
        Ok(())
    }
}
