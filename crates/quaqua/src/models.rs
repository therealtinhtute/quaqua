//! Parse the routing table in `models.md` so `config validate` can flag a kind
//! whose harness/model/effort has drifted from the table a human maintains.
//! The table is fixed-shape markdown; the routing cell (2nd column) is picked by
//! position, not by matching text, then split on the `·` separator.

/// One row of the `models.md` routing table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelsKind {
    pub harness: String,
    pub model: String,
    pub effort: Option<String>,
}

/// Parse `models.md` text into `name -> routing`. Header and separator rows and
/// any row whose routing cell has fewer than two `·`-separated segments are skipped.
pub fn parse_models_md(text: &str) -> std::collections::BTreeMap<String, ModelsKind> {
    let mut out = std::collections::BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with('|') {
            continue;
        }
        let cols: Vec<&str> = line.split('|').map(str::trim).collect();
        // "| name | harness · model[ · effort] | ... |" splits to
        // ["", name, routing, ...]; anything shorter is not a data row.
        if cols.len() < 3 {
            continue;
        }
        let name = cols[1];
        if name.is_empty() || name == "name" || name.starts_with('-') {
            continue;
        }
        let segs: Vec<&str> = cols[2].split('·').map(str::trim).collect();
        if segs.len() < 2 || segs[0].is_empty() || segs[1].is_empty() {
            continue;
        }
        out.insert(
            name.to_string(),
            ModelsKind {
                harness: segs[0].to_string(),
                model: segs[1].to_string(),
                effort: segs.get(2).filter(|s| !s.is_empty()).map(|s| s.to_string()),
            },
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
| name | harness · model · effort | Use for | Seen |
|------|--------------------------|---------|------|
| tieude | opencode · opencode-go/deepseek-v4.1-flash | default | – |
| sude | omp · google-antigravity/gemini-3.8-flash · high | hard impl | – |
";

    #[test]
    fn parses_rows_by_position() {
        let m = parse_models_md(SAMPLE);
        assert_eq!(m.len(), 2);
        assert_eq!(m["tieude"].harness, "opencode");
        assert_eq!(m["tieude"].model, "opencode-go/deepseek-v4.1-flash");
        assert_eq!(m["tieude"].effort, None);
        assert_eq!(m["sude"].effort.as_deref(), Some("high"));
    }
}
