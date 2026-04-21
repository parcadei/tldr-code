//! C# (dotnet build) diagnostic output parser.
//!
//! Parses MSBuild text output format from `dotnet build`:
//! ```text
//! file.cs(line,col): error CODE: message [project.csproj]
//! ```
//!
//! The `[project.csproj]` suffix is optional and stripped during parsing.

use crate::diagnostics::{Diagnostic, Severity};
use crate::error::TldrError;
use regex::Regex;
use std::path::PathBuf;

/// Parse dotnet build MSBuild text output into unified Diagnostic structs.
///
/// MSBuild output format:
/// `file.cs(line,col): severity CODE: message [project.csproj]`
///
/// The project reference in brackets at the end is optional and ignored.
///
/// # Arguments
/// * `output` - The raw text output from `dotnet build`
///
/// # Returns
/// A vector of Diagnostic structs. Malformed lines are skipped.
pub fn parse_dotnet_build_output(output: &str) -> Result<Vec<Diagnostic>, TldrError> {
    if output.trim().is_empty() {
        return Ok(Vec::new());
    }

    // Pattern: file.cs(line,col): severity CODE: message [optional project]
    // The code format is typically CS#### or CA#### for analyzers
    let regex = Regex::new(
        r"^\s*(.+?)\((\d+),(\d+)\):\s*(error|warning|info)\s+([A-Z]+\d+):\s*(.+?)(?:\s*\[.+\])?\s*$"
    ).expect("Invalid dotnet build regex pattern");

    let mut diagnostics = Vec::new();

    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        if let Some(captures) = regex.captures(line) {
            let file = captures.get(1).map(|m| m.as_str()).unwrap_or("");
            let line_num: u32 = captures
                .get(2)
                .and_then(|m| m.as_str().parse().ok())
                .unwrap_or(1);
            let column: u32 = captures
                .get(3)
                .and_then(|m| m.as_str().parse().ok())
                .unwrap_or(1);
            let severity_str = captures.get(4).map(|m| m.as_str()).unwrap_or("error");
            let code = captures.get(5).map(|m| m.as_str().to_string());
            let message = captures
                .get(6)
                .map(|m| m.as_str())
                .unwrap_or("")
                .to_string();

            let severity = match severity_str {
                "error" => Severity::Error,
                "warning" => Severity::Warning,
                "info" => Severity::Information,
                _ => Severity::Error,
            };

            diagnostics.push(Diagnostic {
                file: PathBuf::from(file),
                line: line_num,
                column,
                end_line: None,
                end_column: None,
                severity,
                message,
                code,
                source: "dotnet build".to_string(),
                url: None,
            });
        }
    }

    Ok(diagnostics)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_error_with_project() {
        let output = "Program.cs(10,5): error CS0103: The name 'foo' does not exist in the current context [MyApp.csproj]";

        let result = parse_dotnet_build_output(output).unwrap();
        assert_eq!(result.len(), 1);

        let d = &result[0];
        assert_eq!(d.file, PathBuf::from("Program.cs"));
        assert_eq!(d.line, 10);
        assert_eq!(d.column, 5);
        assert_eq!(d.severity, Severity::Error);
        assert_eq!(d.code, Some("CS0103".to_string()));
        assert_eq!(
            d.message,
            "The name 'foo' does not exist in the current context"
        );
        assert_eq!(d.source, "dotnet build");
    }

    #[test]
    fn test_parse_error_without_project() {
        let output = "Program.cs(10,5): error CS0103: The name 'foo' does not exist";

        let result = parse_dotnet_build_output(output).unwrap();
        assert_eq!(result.len(), 1);

        let d = &result[0];
        assert_eq!(d.code, Some("CS0103".to_string()));
        assert_eq!(d.message, "The name 'foo' does not exist");
    }

    #[test]
    fn test_parse_warning() {
        let output = "Utils.cs(25,1): warning CS0168: The variable 'x' is declared but never used [MyApp.csproj]";

        let result = parse_dotnet_build_output(output).unwrap();
        assert_eq!(result.len(), 1);

        let d = &result[0];
        assert_eq!(d.severity, Severity::Warning);
        assert_eq!(d.code, Some("CS0168".to_string()));
    }

    #[test]
    fn test_parse_analyzer_warning() {
        let output = "Service.cs(42,8): warning CA1822: Member 'DoWork' does not access instance data [MyApp.csproj]";

        let result = parse_dotnet_build_output(output).unwrap();
        assert_eq!(result.len(), 1);

        let d = &result[0];
        assert_eq!(d.severity, Severity::Warning);
        assert_eq!(d.code, Some("CA1822".to_string()));
    }

    #[test]
    fn test_parse_multiple() {
        let output = r#"Program.cs(10,5): error CS0103: Name does not exist [MyApp.csproj]
Utils.cs(25,1): warning CS0168: Variable never used [MyApp.csproj]
Service.cs(42,8): error CS1002: ; expected [MyApp.csproj]"#;

        let result = parse_dotnet_build_output(output).unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].severity, Severity::Error);
        assert_eq!(result[1].severity, Severity::Warning);
        assert_eq!(result[2].severity, Severity::Error);
    }

    #[test]
    fn test_parse_empty() {
        let result = parse_dotnet_build_output("").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_malformed() {
        let output = "Build succeeded.\n    0 Warning(s)\n    0 Error(s)";
        let result = parse_dotnet_build_output(output).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_with_path_in_file() {
        let output = "src/Controllers/HomeController.cs(15,20): warning CS0219: unused variable [src/MyApp.csproj]";

        let result = parse_dotnet_build_output(output).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].file,
            PathBuf::from("src/Controllers/HomeController.cs")
        );
    }
}
