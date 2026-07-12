//! Spec fact store (knowledge-accumulation-v1).
//!
//! A two-tier knowledge system for the project:
//! - **Tier 1 (raw)**: `.ctl/facts.jsonl` — an append-only evidence index of
//!   atomic verified facts captured during conversations. Each fact has a
//!   statement, a source (where it was verified), and a category.
//! - **Tier 2 (curated)**: `.ctl/spec/**/*.md` — the existing human-authored
//!   spec documents. `promote` copies a fact from the raw stream into a curated
//!   markdown file, transforming a one-liner into processed knowledge.
//!
//! The loop: a conversation discovers an objective fact → `ctl spec fact add`
//! → it persists → `ctl hook context` injects a digest into every subsequent
//! session → the model sees accumulated knowledge. Facts are evidence, not
//! state: no events, no reducer, no gating. Record-and-disclose, L0 content.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

/// One atomic verified fact in the project knowledge base.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Fact {
    pub fact_id: String,
    pub statement: String,
    /// Where the fact was verified: a file:line, a command, or a URL. Required —
    /// a fact without provenance is an opinion, not knowledge.
    pub source: String,
    /// Free-text category for filtering (e.g. "boundary", "domain", "gotcha").
    pub category: Option<String>,
    /// ISO 8601 timestamp — stamped by the control layer (the fact content is
    /// time-free evidence).
    pub recorded_at: String,
    /// The envelope actor at record time (unattested principal).
    pub recorded_by: String,
}

/// A compact summary of one fact for the context digest.
#[derive(Debug, Clone, Serialize)]
pub struct FactSummary {
    pub fact_id: String,
    pub statement: String,
    pub category: Option<String>,
}

/// A digest of the entire fact store, injected into session context so every
/// subsequent conversation sees accumulated knowledge. Carries counts (never a
/// verdict) and the most recent facts as one-liners.
#[derive(Debug, Clone, Serialize)]
pub struct FactsDigest {
    pub total: usize,
    pub categories: BTreeMap<String, usize>,
    pub recent: Vec<FactSummary>,
}

/// Path to the facts evidence index (cross-task, at the `.ctl/` root).
pub fn facts_path(project_root: &Path) -> std::path::PathBuf {
    project_root.join(".ctl").join("facts.jsonl")
}

/// Read all facts in append order. Skips blank lines; parse errors carry the
/// line number. Returns an empty vec when the file does not exist yet.
pub fn read_all_facts(project_root: &Path) -> Result<Vec<Fact>> {
    use std::io::BufRead;
    let path = facts_path(project_root);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = fs::File::open(&path)?;
    let reader = std::io::BufReader::new(file);
    let mut facts = Vec::new();
    for (i, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let fact: Fact = serde_json::from_str(&line)
            .map_err(|e| anyhow!("{} line {}: parse error: {}", path.display(), i + 1, e))?;
        facts.push(fact);
    }
    Ok(facts)
}

/// Compute the next fact id (F-001, F-002, ...) by scanning existing facts for
/// the highest numeric suffix.
pub fn next_fact_id(facts: &[Fact]) -> String {
    let max = facts
        .iter()
        .filter_map(|f| {
            f.fact_id
                .strip_prefix("F-")
                .and_then(|n| n.parse::<u32>().ok())
        })
        .max()
        .unwrap_or(0);
    format!("F-{:03}", max + 1)
}

/// Filter facts by optional category and/or case-insensitive search term.
/// Category matches exactly (case-insensitive); search matches statement OR
/// source. Returns facts in append order.
pub fn filter_facts<'a>(
    facts: &'a [Fact],
    category: Option<&str>,
    search: Option<&str>,
) -> Vec<&'a Fact> {
    let cat_lower = category.map(|c| c.to_ascii_lowercase());
    let search_lower = search.map(|s| s.to_ascii_lowercase());
    facts
        .iter()
        .filter(|f| {
            let cat_ok = match &cat_lower {
                Some(c) => f
                    .category
                    .as_ref()
                    .map(|fc| fc.to_ascii_lowercase() == *c)
                    .unwrap_or(false),
                None => true,
            };
            let search_ok = match &search_lower {
                Some(s) => {
                    f.statement.to_ascii_lowercase().contains(s)
                        || f.source.to_ascii_lowercase().contains(s)
                }
                None => true,
            };
            cat_ok && search_ok
        })
        .collect()
}

/// Build the context digest: total count, per-category counts, and the N most
/// recent facts as one-liner summaries.
pub fn facts_digest(facts: &[Fact], recent_count: usize) -> FactsDigest {
    let mut categories: BTreeMap<String, usize> = BTreeMap::new();
    for f in facts {
        let cat = f
            .category
            .clone()
            .unwrap_or_else(|| "uncategorized".to_string());
        *categories.entry(cat).or_default() += 1;
    }
    let recent: Vec<FactSummary> = facts
        .iter()
        .rev()
        .take(recent_count)
        .map(|f| FactSummary {
            fact_id: f.fact_id.clone(),
            statement: f.statement.clone(),
            category: f.category.clone(),
        })
        .collect();
    FactsDigest {
        total: facts.len(),
        categories,
        recent,
    }
}

/// Format a fact as a markdown block for promotion into a curated spec file.
pub fn format_fact_for_promote(fact: &Fact) -> String {
    let cat = fact.category.as_deref().unwrap_or("uncategorized");
    format!(
        "\n### Fact {} (category: {})\n**Source**: {}\n**Verified**: {}\n\n{}\n",
        fact.fact_id, cat, fact.source, fact.recorded_at, fact.statement
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fact(id: &str, stmt: &str, src: &str, cat: Option<&str>) -> Fact {
        Fact {
            fact_id: id.into(),
            statement: stmt.into(),
            source: src.into(),
            category: cat.map(String::from),
            recorded_at: "2026-07-11T00:00:00Z".into(),
            recorded_by: "human".into(),
        }
    }

    #[test]
    fn next_fact_id_starts_at_001_when_empty() {
        assert_eq!(next_fact_id(&[]), "F-001");
    }

    #[test]
    fn next_fact_id_increments_past_max() {
        let facts = vec![
            fact("F-001", "a", "s", None),
            fact("F-003", "c", "s", None),
            fact("F-002", "b", "s", None),
        ];
        assert_eq!(next_fact_id(&facts), "F-004");
    }

    #[test]
    fn filter_by_category_case_insensitive() {
        let facts = vec![
            fact("F-001", "alpha", "src/a", Some("Boundary")),
            fact("F-002", "beta", "src/b", Some("domain")),
        ];
        let filtered = filter_facts(&facts, Some("boundary"), None);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].fact_id, "F-001");
    }

    #[test]
    fn filter_by_search_matches_statement_or_source() {
        let facts = vec![
            fact("F-001", "normalizer canonicalizes", "src/norm.rs", None),
            fact("F-002", "reducer is pure", "src/reducer.rs", None),
        ];
        let by_stmt = filter_facts(&facts, None, Some("canonicalizes"));
        assert_eq!(by_stmt.len(), 1);
        assert_eq!(by_stmt[0].fact_id, "F-001");

        let by_src = filter_facts(&facts, None, Some("reducer.rs"));
        assert_eq!(by_src.len(), 1);
        assert_eq!(by_src[0].fact_id, "F-002");
    }

    #[test]
    fn digest_counts_categories_and_returns_recent() {
        let facts = vec![
            fact("F-001", "a", "s", Some("boundary")),
            fact("F-002", "b", "s", Some("domain")),
            fact("F-003", "c", "s", Some("boundary")),
        ];
        let digest = facts_digest(&facts, 2);
        assert_eq!(digest.total, 3);
        assert_eq!(digest.categories.get("boundary"), Some(&2));
        assert_eq!(digest.categories.get("domain"), Some(&1));
        assert_eq!(digest.recent.len(), 2);
        // Most recent first (reverse append order).
        assert_eq!(digest.recent[0].fact_id, "F-003");
        assert_eq!(digest.recent[1].fact_id, "F-002");
    }

    #[test]
    fn digest_uncategorized_when_no_category() {
        let facts = vec![fact("F-001", "a", "s", None)];
        let digest = facts_digest(&facts, 5);
        assert_eq!(digest.categories.get("uncategorized"), Some(&1));
    }

    #[test]
    fn promote_format_includes_id_source_and_statement() {
        let f = fact("F-007", "some fact", "src/x.rs:42", Some("gotcha"));
        let md = format_fact_for_promote(&f);
        assert!(md.contains("Fact F-007"));
        assert!(md.contains("category: gotcha"));
        assert!(md.contains("src/x.rs:42"));
        assert!(md.contains("some fact"));
    }

    #[test]
    fn read_all_facts_empty_when_no_file() {
        let dir = std::env::temp_dir().join(format!("spec-test-{}", uuid_stamp()));
        let facts = read_all_facts(&dir).unwrap();
        assert!(facts.is_empty());
    }

    fn uuid_stamp() -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos().to_string())
            .unwrap_or_default()
    }
}
