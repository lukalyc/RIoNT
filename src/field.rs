//! Product-side field resolution: maps RIONT's `config.json` onto the
//! `riont-field` engine crate. All geometry lives in the crate; only this
//! config coupling is product code.

pub use riont_field::*;

pub fn resolve(config: &crate::config::Config) -> (FieldMap, Option<String>) {
    riont_field::resolve(riont_field::FieldSettingsRef {
        walls_file: config.field.walls_file.as_deref(),
        map: &config.field.map,
    })
}
