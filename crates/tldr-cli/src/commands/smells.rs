//! Smells command - Detect code smells
//!
//! Identifies common code smells like God Class, Long Method, etc.
//! Auto-routes through daemon when available for ~35x speedup.

use std::path::PathBuf;

use anyhow::Result;
use clap::Args;

use tldr_core::{
    analyze_smells_aggregated, detect_smells, SmellType, SmellsReport, ThresholdPreset,
};

use crate::commands::daemon_router::{params_with_path, try_daemon_route};
use crate::output::{format_smells_text, OutputFormat, OutputWriter};

/// Detect code smells
#[derive(Debug, Args)]
pub struct SmellsArgs {
    /// Path to analyze (file or directory)
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Threshold preset
    #[arg(long, short = 't', default_value = "default")]
    pub threshold: ThresholdPresetArg,

    /// Filter by smell type
    #[arg(long, short = 's')]
    pub smell_type: Option<SmellTypeArg>,

    /// Include suggestions for fixing
    #[arg(long)]
    pub suggest: bool,

    /// Deep analysis: aggregate findings from cohesion, coupling, dead code,
    /// similarity, and cognitive complexity analyzers in addition to the
    /// standard smell detectors
    #[arg(long)]
    pub deep: bool,
}

/// CLI wrapper for threshold preset
#[derive(Debug, Clone, Copy, Default, clap::ValueEnum)]
pub enum ThresholdPresetArg {
    /// Strict thresholds for high-quality codebases
    Strict,
    /// Default thresholds (recommended)
    #[default]
    Default,
    /// Relaxed thresholds for legacy code
    Relaxed,
}

impl From<ThresholdPresetArg> for ThresholdPreset {
    fn from(arg: ThresholdPresetArg) -> Self {
        match arg {
            ThresholdPresetArg::Strict => ThresholdPreset::Strict,
            ThresholdPresetArg::Default => ThresholdPreset::Default,
            ThresholdPresetArg::Relaxed => ThresholdPreset::Relaxed,
        }
    }
}

/// CLI wrapper for smell type
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum SmellTypeArg {
    /// God Class (>20 methods or >500 LOC)
    GodClass,
    /// Long Method (>50 LOC or cyclomatic >10)
    LongMethod,
    /// Long Parameter List (>5 parameters)
    LongParameterList,
    /// Feature Envy
    FeatureEnvy,
    /// Data Clumps
    DataClumps,
    /// Low Cohesion (LCOM4 >= 2) -- requires --deep
    LowCohesion,
    /// Tight Coupling (score >= 0.6) -- requires --deep
    TightCoupling,
    /// Dead Code (unreachable functions) -- requires --deep
    DeadCode,
    /// Code Clone (similar functions) -- requires --deep
    CodeClone,
    /// High Cognitive Complexity (>= 15) -- requires --deep
    HighCognitiveComplexity,
    /// Deep Nesting (nesting depth >= 5)
    DeepNesting,
    /// Data Class (many fields, few/no methods)
    DataClass,
    /// Lazy Element (class with only 1 method and 0-1 fields)
    LazyElement,
    /// Message Chain (long method call chains > 3)
    MessageChain,
    /// Primitive Obsession (many primitive-typed parameters)
    PrimitiveObsession,
    /// Middle Man (>60% delegation) -- requires --deep
    MiddleMan,
    /// Refused Bequest (<33% inherited usage) -- requires --deep
    RefusedBequest,
    /// Inappropriate Intimacy (bidirectional coupling) -- requires --deep
    InappropriateIntimacy,
}

impl From<SmellTypeArg> for SmellType {
    fn from(arg: SmellTypeArg) -> Self {
        match arg {
            SmellTypeArg::GodClass => SmellType::GodClass,
            SmellTypeArg::LongMethod => SmellType::LongMethod,
            SmellTypeArg::LongParameterList => SmellType::LongParameterList,
            SmellTypeArg::FeatureEnvy => SmellType::FeatureEnvy,
            SmellTypeArg::DataClumps => SmellType::DataClumps,
            SmellTypeArg::LowCohesion => SmellType::LowCohesion,
            SmellTypeArg::TightCoupling => SmellType::TightCoupling,
            SmellTypeArg::DeadCode => SmellType::DeadCode,
            SmellTypeArg::CodeClone => SmellType::CodeClone,
            SmellTypeArg::HighCognitiveComplexity => SmellType::HighCognitiveComplexity,
            SmellTypeArg::DeepNesting => SmellType::DeepNesting,
            SmellTypeArg::DataClass => SmellType::DataClass,
            SmellTypeArg::LazyElement => SmellType::LazyElement,
            SmellTypeArg::MessageChain => SmellType::MessageChain,
            SmellTypeArg::PrimitiveObsession => SmellType::PrimitiveObsession,
            SmellTypeArg::MiddleMan => SmellType::MiddleMan,
            SmellTypeArg::RefusedBequest => SmellType::RefusedBequest,
            SmellTypeArg::InappropriateIntimacy => SmellType::InappropriateIntimacy,
        }
    }
}

impl SmellsArgs {
    /// Run the smells command
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        let writer = OutputWriter::new(format, quiet);

        // Try daemon first for cached result
        if let Some(report) = try_daemon_route::<SmellsReport>(
            &self.path,
            "smells",
            params_with_path(Some(&self.path)),
        ) {
            // Output based on format
            if writer.is_text() {
                let text = format_smells_text(&report);
                writer.write_text(&text)?;
                return Ok(());
            } else {
                writer.write(&report)?;
                return Ok(());
            }
        }

        // Fallback to direct compute
        writer.progress(&format!(
            "Scanning for code smells in {}{}...",
            self.path.display(),
            if self.deep { " (deep analysis)" } else { "" }
        ));

        // Detect smells - use aggregated analysis when --deep is set
        let report = if self.deep {
            analyze_smells_aggregated(
                &self.path,
                self.threshold.into(),
                self.smell_type.map(|s| s.into()),
                self.suggest,
            )?
        } else {
            detect_smells(
                &self.path,
                self.threshold.into(),
                self.smell_type.map(|s| s.into()),
                self.suggest,
            )?
        };

        // Output based on format
        if writer.is_text() {
            let text = format_smells_text(&report);
            writer.write_text(&text)?;
        } else {
            writer.write(&report)?;
        }

        Ok(())
    }
}
