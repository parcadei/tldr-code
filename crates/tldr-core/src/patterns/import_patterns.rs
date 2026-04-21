//! Import organization pattern detection
//!
//! Detects import patterns:
//! - Absolute vs relative imports
//! - Import grouping (stdlib, third-party, local)
//! - Star import usage
//! - Common alias conventions (np, pd, etc.)

use std::collections::HashMap;

use super::signals::PatternSignals;
use crate::types::{
    AliasConvention, Evidence, ImportGrouping, ImportPattern, ImportStyle, StarImportUsage,
};

/// Convert signals to import pattern
pub fn signals_to_pattern(
    signals: &PatternSignals,
    evidence_limit: usize,
) -> Option<ImportPattern> {
    let import_patterns = &signals.import_patterns;

    if !import_patterns.has_signals() {
        return None;
    }

    // Determine absolute vs relative preference
    let absolute_count = import_patterns.absolute_imports.len();
    let relative_count = import_patterns.relative_imports.len();
    let total_imports = absolute_count + relative_count;

    let absolute_vs_relative = if total_imports == 0 {
        ImportStyle::Mixed
    } else {
        let ratio = absolute_count as f64 / total_imports as f64;
        if ratio >= 0.8 {
            ImportStyle::Absolute
        } else if ratio <= 0.2 {
            ImportStyle::Relative
        } else {
            ImportStyle::Mixed
        }
    };

    // Determine star import usage
    let star_import_count = import_patterns.star_imports.len();
    let star_imports = if star_import_count == 0 {
        StarImportUsage::None
    } else if star_import_count <= 2 {
        StarImportUsage::Rare
    } else {
        StarImportUsage::Common
    };

    // Detect grouping style from collected groupings
    let grouping_style = detect_grouping_style(&import_patterns.groupings);

    // Convert aliases to AliasConvention
    let alias_conventions = convert_aliases(&import_patterns.aliases);

    // Collect evidence (limited)
    let evidence: Vec<Evidence> = import_patterns
        .star_imports
        .iter()
        .take(evidence_limit)
        .cloned()
        .collect();

    Some(ImportPattern {
        grouping_style,
        absolute_vs_relative,
        star_imports,
        alias_conventions,
        evidence,
    })
}

/// Detect the import grouping style from collected groupings
fn detect_grouping_style(groupings: &[super::signals::ImportGrouping]) -> ImportGrouping {
    if groupings.is_empty() {
        return ImportGrouping::Ungrouped;
    }

    // Count patterns observed across files
    let mut stdlib_first_count = 0;
    let mut local_first_count = 0;
    let mut third_party_first_count = 0;

    for grouping in groupings {
        // Determine which type appears first (non-empty)
        if !grouping.stdlib_imports.is_empty() {
            if grouping.third_party_imports.is_empty() || !grouping.local_imports.is_empty() {
                stdlib_first_count += 1;
            }
        } else if !grouping.third_party_imports.is_empty() {
            third_party_first_count += 1;
        } else if !grouping.local_imports.is_empty() {
            local_first_count += 1;
        }
    }

    // Determine majority pattern
    if stdlib_first_count >= third_party_first_count && stdlib_first_count >= local_first_count {
        if stdlib_first_count > 0 {
            ImportGrouping::StdlibFirst
        } else {
            ImportGrouping::Ungrouped
        }
    } else if third_party_first_count >= local_first_count {
        ImportGrouping::ThirdPartyFirst
    } else {
        ImportGrouping::LocalFirst
    }
}

/// Convert alias map to AliasConvention list, filtering out identity aliases
/// where the alias name equals the original module name (e.g. `echo -> echo`).
fn convert_aliases(aliases: &HashMap<String, String>) -> Vec<AliasConvention> {
    aliases
        .iter()
        .filter(|(module, alias)| module != alias)
        .map(|(module, alias)| AliasConvention {
            module: module.clone(),
            alias: alias.clone(),
            count: 1,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_signals_returns_none() {
        let signals = PatternSignals::default();
        assert!(signals_to_pattern(&signals, 3).is_none());
    }

    #[test]
    fn test_absolute_imports_preferred() {
        let mut signals = PatternSignals::default();
        // Add 8 absolute imports
        for i in 0..8 {
            signals
                .import_patterns
                .absolute_imports
                .push((format!("module_{}", i), "file.py".to_string()));
        }
        // Add 2 relative imports
        signals
            .import_patterns
            .relative_imports
            .push((".local".to_string(), "file.py".to_string()));
        signals
            .import_patterns
            .relative_imports
            .push((".utils".to_string(), "file.py".to_string()));

        let pattern = signals_to_pattern(&signals, 3).unwrap();
        assert_eq!(pattern.absolute_vs_relative, ImportStyle::Absolute);
    }

    #[test]
    fn test_relative_imports_preferred() {
        let mut signals = PatternSignals::default();
        // Add 2 absolute imports
        signals
            .import_patterns
            .absolute_imports
            .push(("os".to_string(), "file.py".to_string()));
        signals
            .import_patterns
            .absolute_imports
            .push(("sys".to_string(), "file.py".to_string()));
        // Add 8 relative imports
        for i in 0..8 {
            signals
                .import_patterns
                .relative_imports
                .push((format!(".module_{}", i), "file.py".to_string()));
        }

        let pattern = signals_to_pattern(&signals, 3).unwrap();
        assert_eq!(pattern.absolute_vs_relative, ImportStyle::Relative);
    }

    #[test]
    fn test_star_imports_detected() {
        let mut signals = PatternSignals::default();
        signals
            .import_patterns
            .absolute_imports
            .push(("module".to_string(), "file.py".to_string()));
        signals.import_patterns.star_imports.push(Evidence::new(
            "file.py",
            5,
            "from module import *",
        ));

        let pattern = signals_to_pattern(&signals, 3).unwrap();
        assert_eq!(pattern.star_imports, StarImportUsage::Rare);
    }

    #[test]
    fn test_alias_conventions_detected() {
        let mut signals = PatternSignals::default();
        signals
            .import_patterns
            .absolute_imports
            .push(("numpy".to_string(), "file.py".to_string()));
        signals
            .import_patterns
            .aliases
            .insert("numpy".to_string(), "np".to_string());

        let pattern = signals_to_pattern(&signals, 3).unwrap();
        assert!(!pattern.alias_conventions.is_empty());
        assert_eq!(pattern.alias_conventions[0].module, "numpy");
        assert_eq!(pattern.alias_conventions[0].alias, "np");
    }

    #[test]
    fn test_identity_aliases_filtered_out() {
        let mut signals = PatternSignals::default();
        signals
            .import_patterns
            .absolute_imports
            .push(("click".to_string(), "file.py".to_string()));
        // Identity alias: module name == alias name (should be filtered)
        signals
            .import_patterns
            .aliases
            .insert("echo".to_string(), "echo".to_string());
        signals
            .import_patterns
            .aliases
            .insert("style".to_string(), "style".to_string());
        // Non-identity alias: module name != alias name (should be kept)
        signals
            .import_patterns
            .aliases
            .insert("typing".to_string(), "t".to_string());
        signals
            .import_patterns
            .aliases
            .insert("collections.abc".to_string(), "cabc".to_string());

        let pattern = signals_to_pattern(&signals, 3).unwrap();
        // Only the non-identity aliases should remain
        assert_eq!(pattern.alias_conventions.len(), 2);
        let modules: Vec<&str> = pattern
            .alias_conventions
            .iter()
            .map(|a| a.module.as_str())
            .collect();
        assert!(modules.contains(&"typing"));
        assert!(modules.contains(&"collections.abc"));
        // Identity aliases should NOT be present
        assert!(!modules.contains(&"echo"));
        assert!(!modules.contains(&"style"));
    }

    #[test]
    fn test_all_identity_aliases_results_in_empty_list() {
        let mut signals = PatternSignals::default();
        signals
            .import_patterns
            .absolute_imports
            .push(("click".to_string(), "file.py".to_string()));
        // All identity aliases
        signals
            .import_patterns
            .aliases
            .insert("echo".to_string(), "echo".to_string());
        signals
            .import_patterns
            .aliases
            .insert("option".to_string(), "option".to_string());

        let pattern = signals_to_pattern(&signals, 3).unwrap();
        assert!(pattern.alias_conventions.is_empty());
    }
}
