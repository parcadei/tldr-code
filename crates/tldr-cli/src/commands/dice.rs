//! Dice command - Compare similarity between two code fragments
//!
//! Computes the Dice coefficient between two code targets.
//! Targets can be files, file::function, or file:start:end (line ranges).

use std::path::PathBuf;

use anyhow::{anyhow, Result};
use clap::Args;
use serde::Serialize;

use tldr_core::analysis::{
    compute_dice_similarity, interpret_similarity, normalize_tokens, NormalizationMode,
};

use crate::output::{OutputFormat, OutputWriter};

/// Compare similarity between two code fragments
#[derive(Debug, Args)]
pub struct DiceArgs {
    /// First target: file, file::function, or file:start:end
    pub target1: String,

    /// Second target: file, file::function, or file:start:end
    pub target2: String,

    /// Normalization mode: none, identifiers, literals, all (default: all)
    #[arg(long, default_value = "all")]
    pub normalize: String,

    /// Language hint (auto-detected if not specified)
    #[arg(long = "language")]
    pub language: Option<String>,

    /// Output format: json, text (default: json)
    #[arg(short, long, default_value = "json")]
    pub output: String,
}

/// Parsed target specification
#[derive(Debug)]
enum Target {
    File(PathBuf),
    Function(PathBuf, String),
    Block(PathBuf, usize, usize),
}

/// Similarity report for dice command
#[derive(Debug, Serialize)]
struct DiceSimilarityReport {
    /// First target as specified
    target1: String,
    /// Second target as specified
    target2: String,
    /// Dice coefficient (0.0 - 1.0)
    dice_coefficient: f64,
    /// Human-readable interpretation
    interpretation: String,
    /// Token count in first fragment
    tokens1_count: usize,
    /// Token count in second fragment
    tokens2_count: usize,
}

impl DiceArgs {
    /// Run the dice command
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        let writer = OutputWriter::new(format, quiet);

        writer.progress(&format!(
            "Comparing similarity between {} and {}...",
            self.target1, self.target2
        ));

        let target1 = parse_target(&self.target1)?;
        let target2 = parse_target(&self.target2)?;

        let normalization =
            NormalizationMode::parse(&self.normalize).unwrap_or(NormalizationMode::All);

        // Get source and language for each target
        let (source1, lang1) = get_source(&target1, self.language.as_deref())?;
        let (source2, lang2) = get_source(&target2, self.language.as_deref())?;

        // Tokenize and normalize using tldr-core's normalize_tokens
        let tokens1 = normalize_tokens(&source1, &lang1, normalization)
            .map_err(|e| anyhow!("Failed to tokenize target1: {}", e))?;
        let tokens2 = normalize_tokens(&source2, &lang2, normalization)
            .map_err(|e| anyhow!("Failed to tokenize target2: {}", e))?;

        // Compute similarity
        let dice = compute_dice_similarity(&tokens1, &tokens2);

        let report = DiceSimilarityReport {
            target1: self.target1.clone(),
            target2: self.target2.clone(),
            dice_coefficient: dice,
            interpretation: interpret_similarity(dice),
            tokens1_count: tokens1.len(),
            tokens2_count: tokens2.len(),
        };

        // Determine output format
        let effective_format = match self.output.as_str() {
            "text" => OutputFormat::Text,
            "json" => format,
            _ => format,
        };

        if matches!(effective_format, OutputFormat::Text) {
            let text = format_dice_text(&report);
            writer.write_text(&text)?;
        } else {
            writer.write(&report)?;
        }

        Ok(())
    }
}

/// Parse a target string into a Target enum
fn parse_target(s: &str) -> Result<Target> {
    // Check for function specifier (::)
    if let Some((path, func)) = s.split_once("::") {
        return Ok(Target::Function(PathBuf::from(path), func.to_string()));
    }

    // Check for line range (file:start:end)
    // Need to be careful with Windows paths (C:\...) - only split if we have exactly 3 parts
    // and the last two are numbers
    let parts: Vec<&str> = s.rsplitn(3, ':').collect();

    if parts.len() == 3 {
        // parts are [end, start, path] due to rsplitn
        if let (Ok(end), Ok(start)) = (parts[0].parse::<usize>(), parts[1].parse::<usize>()) {
            return Ok(Target::Block(PathBuf::from(parts[2]), start, end));
        }
    }

    // Default to file
    Ok(Target::File(PathBuf::from(s)))
}

/// Get source code and language for a target
fn get_source(target: &Target, lang_hint: Option<&str>) -> Result<(String, String)> {
    match target {
        Target::File(path) => {
            let source = std::fs::read_to_string(path)
                .map_err(|e| anyhow!("Failed to read {}: {}", path.display(), e))?;
            let lang = lang_hint
                .map(String::from)
                .or_else(|| detect_language(path))
                .ok_or_else(|| anyhow!("Could not detect language for {}", path.display()))?;
            Ok((source, lang))
        }
        Target::Function(path, _func_name) => {
            // For now, return full file - function extraction requires more work
            // TODO: Extract function body using tree-sitter
            let source = std::fs::read_to_string(path)
                .map_err(|e| anyhow!("Failed to read {}: {}", path.display(), e))?;
            let lang = lang_hint
                .map(String::from)
                .or_else(|| detect_language(path))
                .ok_or_else(|| anyhow!("Could not detect language"))?;
            Ok((source, lang))
        }
        Target::Block(path, start, end) => {
            let source = std::fs::read_to_string(path)
                .map_err(|e| anyhow!("Failed to read {}: {}", path.display(), e))?;
            let lines: Vec<&str> = source.lines().collect();

            // Convert to 0-indexed and handle bounds
            let start_idx = start.saturating_sub(1);
            let end_idx = (*end).min(lines.len());

            let block = lines
                .get(start_idx..end_idx)
                .map(|l| l.join("\n"))
                .unwrap_or_default();

            let lang = lang_hint
                .map(String::from)
                .or_else(|| detect_language(path))
                .ok_or_else(|| anyhow!("Could not detect language"))?;

            Ok((block, lang))
        }
    }
}

/// Detect language from file extension (delegates to Language::from_path for all 18 languages)
fn detect_language(path: &std::path::Path) -> Option<String> {
    tldr_core::Language::from_path(path).map(|l| l.to_string())
}

/// Format dice similarity report as human-readable text
fn format_dice_text(report: &DiceSimilarityReport) -> String {
    use std::fmt::Write;

    let mut output = String::new();

    writeln!(output, "Similarity Comparison").unwrap();
    writeln!(output, "=====================").unwrap();
    writeln!(output).unwrap();
    writeln!(
        output,
        "Target 1: {} ({} tokens)",
        report.target1, report.tokens1_count
    )
    .unwrap();
    writeln!(
        output,
        "Target 2: {} ({} tokens)",
        report.target2, report.tokens2_count
    )
    .unwrap();
    writeln!(output).unwrap();
    writeln!(
        output,
        "Dice coefficient: {:.2}%",
        report.dice_coefficient * 100.0
    )
    .unwrap();
    writeln!(output, "Interpretation: {}", report.interpretation).unwrap();

    output
}
