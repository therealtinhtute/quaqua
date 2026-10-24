use crate::layout::model::Layout;

pub fn get(name: &str) -> Option<Layout> {
    let toml_str = match name {
        "slp" => include_str!("../../../../templates/layouts/slp.toml"),
        "lead-peer" => include_str!("../../../../templates/layouts/lead-peer.toml"),
        "advisor" => include_str!("../../../../templates/layouts/advisor.toml"),
        "dual-lens" => include_str!("../../../../templates/layouts/dual-lens.toml"),
        "council" => include_str!("../../../../templates/layouts/council.toml"),
        _ => return None,
    };
    toml::from_str(toml_str).ok()
}
