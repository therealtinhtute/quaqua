use crate::layout::model::Layout;

pub const BUILTIN_NAMES: [&str; 5] = ["slp", "lead-peer", "advisor", "dual-lens", "council"];

pub fn is_builtin(name: &str) -> bool {
    BUILTIN_NAMES.contains(&name)
}

fn builtin_toml_str(name: &str) -> Option<&'static str> {
    match name {
        "slp" => Some(include_str!("../../../../templates/layouts/slp.toml")),
        "lead-peer" => Some(include_str!("../../../../templates/layouts/lead-peer.toml")),
        "advisor" => Some(include_str!("../../../../templates/layouts/advisor.toml")),
        "dual-lens" => Some(include_str!("../../../../templates/layouts/dual-lens.toml")),
        "council" => Some(include_str!("../../../../templates/layouts/council.toml")),
        _ => None,
    }
}

pub fn get(name: &str) -> Option<Layout> {
    let toml_str = builtin_toml_str(name)?;
    match toml::from_str(toml_str) {
        Ok(layout) => Some(layout),
        Err(e) => panic!("built-in layout '{name}' failed to parse: {e}"),
    }
}

pub fn load(name: &str) -> Result<Layout, String> {
    let toml_str =
        builtin_toml_str(name).ok_or_else(|| format!("unknown built-in layout '{name}'"))?;
    toml::from_str(toml_str).map_err(|e| format!("built-in layout '{name}' failed to parse: {e}"))
}
