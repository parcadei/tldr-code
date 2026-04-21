//! Test coverage for tldr-core limits module
//!
//! Tests all public types and functions from:
//! - crates/tldr-core/src/limits.rs

use std::time::Duration;

use tldr_core::limits::*;
use tldr_core::TldrError;

// =============================================================================
// AnalysisLimits Tests
// =============================================================================

#[test]
fn test_analysis_limits_default() {
    let limits = AnalysisLimits::default();

    assert_eq!(limits.max_nodes, 50_000);
    assert_eq!(limits.max_edges, 100_000);
    assert_eq!(limits.max_diamond_paths, 1_000);
    assert_eq!(limits.max_patterns, 10_000);
    assert_eq!(limits.timeout_secs, 30);
}

#[test]
fn test_analysis_limits_with_timeout() {
    let limits = AnalysisLimits::default().with_timeout(60);

    assert_eq!(limits.timeout_secs, 60);
    // Other fields unchanged
    assert_eq!(limits.max_nodes, 50_000);
}

#[test]
fn test_analysis_limits_with_timeout_zero() {
    let limits = AnalysisLimits::default().with_timeout(0);

    assert_eq!(limits.timeout_secs, 0);
}

#[test]
fn test_analysis_limits_with_max_nodes() {
    let limits = AnalysisLimits::default().with_max_nodes(100_000);

    assert_eq!(limits.max_nodes, 100_000);
    assert_eq!(limits.max_edges, 100_000); // Unchanged
}

#[test]
fn test_analysis_limits_with_max_edges() {
    let limits = AnalysisLimits::default().with_max_edges(200_000);

    assert_eq!(limits.max_edges, 200_000);
    assert_eq!(limits.max_nodes, 50_000); // Unchanged
}

#[test]
fn test_analysis_limits_unlimited() {
    let limits = AnalysisLimits::unlimited();

    assert_eq!(limits.max_nodes, usize::MAX);
    assert_eq!(limits.max_edges, usize::MAX);
    assert_eq!(limits.max_diamond_paths, usize::MAX);
    assert_eq!(limits.max_patterns, usize::MAX);
    assert_eq!(limits.timeout_secs, 0);
}

#[test]
fn test_analysis_limits_chaining() {
    let limits = AnalysisLimits::default()
        .with_timeout(120)
        .with_max_nodes(10_000)
        .with_max_edges(20_000);

    assert_eq!(limits.timeout_secs, 120);
    assert_eq!(limits.max_nodes, 10_000);
    assert_eq!(limits.max_edges, 20_000);
}

#[test]
fn test_analysis_limits_check_nodes_under_limit() {
    let limits = AnalysisLimits::default().with_max_nodes(100);

    assert!(limits.check_nodes(50).is_ok());
    assert!(limits.check_nodes(99).is_ok());
    assert!(limits.check_nodes(100).is_ok());
}

#[test]
fn test_analysis_limits_check_nodes_over_limit() {
    let limits = AnalysisLimits::default().with_max_nodes(100);

    let result = limits.check_nodes(101);
    assert!(result.is_err());

    match result.unwrap_err() {
        LimitExceeded::MaxNodes { limit, actual } => {
            assert_eq!(limit, 100);
            assert_eq!(actual, 101);
        }
        _ => panic!("Expected MaxNodes error"),
    }
}

#[test]
fn test_analysis_limits_check_nodes_unlimited() {
    let limits = AnalysisLimits::unlimited();

    assert!(limits.check_nodes(usize::MAX - 1).is_ok());
}

#[test]
fn test_analysis_limits_check_edges_under_limit() {
    let limits = AnalysisLimits::default().with_max_edges(100);

    assert!(limits.check_edges(50).is_ok());
    assert!(limits.check_edges(100).is_ok());
}

#[test]
fn test_analysis_limits_check_edges_over_limit() {
    let limits = AnalysisLimits::default().with_max_edges(100);

    let result = limits.check_edges(101);
    assert!(result.is_err());

    match result.unwrap_err() {
        LimitExceeded::MaxEdges { limit, actual } => {
            assert_eq!(limit, 100);
            assert_eq!(actual, 101);
        }
        _ => panic!("Expected MaxEdges error"),
    }
}

#[test]
fn test_analysis_limits_check_edges_unlimited() {
    let limits = AnalysisLimits::unlimited();

    assert!(limits.check_edges(1_000_000).is_ok());
}

#[test]
fn test_analysis_limits_serde_roundtrip() {
    let limits = AnalysisLimits::default()
        .with_timeout(60)
        .with_max_nodes(100_000);

    let json = serde_json::to_string(&limits).unwrap();
    let parsed: AnalysisLimits = serde_json::from_str(&json).unwrap();

    assert_eq!(limits.max_nodes, parsed.max_nodes);
    assert_eq!(limits.max_edges, parsed.max_edges);
    assert_eq!(limits.timeout_secs, parsed.timeout_secs);
    assert_eq!(limits.max_diamond_paths, parsed.max_diamond_paths);
    assert_eq!(limits.max_patterns, parsed.max_patterns);
}

// =============================================================================
// LimitExceeded Tests
// =============================================================================

#[test]
fn test_limit_exceeded_max_nodes_display() {
    let err = LimitExceeded::MaxNodes {
        limit: 1000,
        actual: 2000,
    };
    let msg = err.to_string();

    assert!(msg.contains("Node limit exceeded"));
    assert!(msg.contains("2000"));
    assert!(msg.contains("1000"));
    assert!(msg.contains("--max-nodes"));
}

#[test]
fn test_limit_exceeded_max_edges_display() {
    let err = LimitExceeded::MaxEdges {
        limit: 5000,
        actual: 6000,
    };
    let msg = err.to_string();

    assert!(msg.contains("Edge limit exceeded"));
    assert!(msg.contains("6000"));
    assert!(msg.contains("5000"));
    assert!(msg.contains("--max-files"));
}

#[test]
fn test_limit_exceeded_max_diamond_paths_display() {
    let err = LimitExceeded::MaxDiamondPaths {
        limit: 100,
        actual: 150,
    };
    let msg = err.to_string();

    assert!(msg.contains("Diamond path limit exceeded"));
    assert!(msg.contains("150"));
    assert!(msg.contains("100"));
    assert!(msg.contains("--no-patterns"));
}

#[test]
fn test_limit_exceeded_max_patterns_display() {
    let err = LimitExceeded::MaxPatterns {
        limit: 1000,
        actual: 2000,
    };
    let msg = err.to_string();

    assert!(msg.contains("Pattern limit exceeded"));
    assert!(msg.contains("2000"));
    assert!(msg.contains("1000"));
}

#[test]
fn test_limit_exceeded_timeout_display() {
    let err = LimitExceeded::Timeout {
        elapsed_secs: 30,
        limit_secs: 30,
    };
    let msg = err.to_string();

    assert!(msg.contains("Analysis timed out"));
    assert!(msg.contains("30s"));
    assert!(msg.contains("--max-files"));
    assert!(msg.contains("--timeout"));
}

#[test]
fn test_limit_exceeded_error_trait() {
    let err = LimitExceeded::MaxNodes {
        limit: 100,
        actual: 200,
    };
    let _: &dyn std::error::Error = &err;
}

#[test]
fn test_limit_exceeded_serde_roundtrip() {
    let errors = vec![
        LimitExceeded::MaxNodes {
            limit: 100,
            actual: 200,
        },
        LimitExceeded::MaxEdges {
            limit: 500,
            actual: 600,
        },
        LimitExceeded::MaxDiamondPaths {
            limit: 50,
            actual: 100,
        },
        LimitExceeded::MaxPatterns {
            limit: 1000,
            actual: 2000,
        },
        LimitExceeded::Timeout {
            elapsed_secs: 30,
            limit_secs: 30,
        },
    ];

    for err in errors {
        let json = serde_json::to_string(&err).unwrap();
        let parsed: LimitExceeded = serde_json::from_str(&json).unwrap();

        // Verify the variant matches
        match (&err, &parsed) {
            (LimitExceeded::MaxNodes { .. }, LimitExceeded::MaxNodes { .. }) => {}
            (LimitExceeded::MaxEdges { .. }, LimitExceeded::MaxEdges { .. }) => {}
            (LimitExceeded::MaxDiamondPaths { .. }, LimitExceeded::MaxDiamondPaths { .. }) => {}
            (LimitExceeded::MaxPatterns { .. }, LimitExceeded::MaxPatterns { .. }) => {}
            (LimitExceeded::Timeout { .. }, LimitExceeded::Timeout { .. }) => {}
            _ => panic!("Mismatched variants after serde"),
        }
    }
}

// =============================================================================
// TimeoutContext Tests
// =============================================================================

#[test]
fn test_timeout_context_new() {
    let ctx = TimeoutContext::new(30);

    assert!(ctx.check().is_ok());
    assert!(!ctx.is_cancelled());
    assert_eq!(ctx.nodes_processed(), 0);
}

#[test]
fn test_timeout_context_new_zero() {
    let ctx = TimeoutContext::new(0);

    // Zero means no timeout, so should always be ok
    assert!(ctx.check().is_ok());
    assert!(!ctx.is_cancelled());
}

#[test]
fn test_timeout_context_no_timeout() {
    let ctx = TimeoutContext::no_timeout();

    assert!(ctx.check().is_ok());
    assert!(!ctx.is_cancelled());
}

#[test]
fn test_timeout_context_default() {
    let ctx: TimeoutContext = Default::default();

    // Default is 30 seconds
    assert!(ctx.check().is_ok());
}

#[test]
fn test_timeout_context_cancel() {
    let ctx = TimeoutContext::new(30);

    assert!(!ctx.is_cancelled());
    ctx.cancel();
    assert!(ctx.is_cancelled());
}

#[test]
fn test_timeout_context_check_cancelled() {
    let ctx = TimeoutContext::new(30);

    ctx.cancel();
    let result = ctx.check();

    assert!(result.is_err());
    match result.unwrap_err() {
        LimitExceeded::Timeout { .. } => {}
        _ => panic!("Expected Timeout error"),
    }
}

#[test]
fn test_timeout_context_elapsed() {
    let ctx = TimeoutContext::new(30);

    // Should be very small elapsed time
    let elapsed = ctx.elapsed();
    assert!(elapsed.as_secs() < 1);
    assert!(elapsed.as_millis() < 100);
}

#[test]
fn test_timeout_context_nodes_processed() {
    let ctx = TimeoutContext::new(30);

    assert_eq!(ctx.nodes_processed(), 0);

    // Simulate some work
    let _ = ctx.check_periodic(1);
    let _ = ctx.check_periodic(1);
    let _ = ctx.check_periodic(1);

    assert_eq!(ctx.nodes_processed(), 3);
}

#[test]
fn test_timeout_context_check_periodic() {
    let ctx = TimeoutContext::new(30);

    // Check every 5 nodes
    assert!(ctx.check_periodic(5).unwrap()); // Node 0 - check
    assert!(!ctx.check_periodic(5).unwrap()); // Node 1 - skip
    assert!(!ctx.check_periodic(5).unwrap()); // Node 2 - skip
    assert!(!ctx.check_periodic(5).unwrap()); // Node 3 - skip
    assert!(!ctx.check_periodic(5).unwrap()); // Node 4 - skip
    assert!(ctx.check_periodic(5).unwrap()); // Node 5 - check
}

#[test]
fn test_timeout_context_check_periodic_with_cancel() {
    let ctx = TimeoutContext::new(30);

    ctx.cancel();

    let result = ctx.check_periodic(5);
    assert!(result.is_err());
}

#[test]
fn test_timeout_context_clone() {
    let ctx = TimeoutContext::new(30);

    // Simulate some work
    let _ = ctx.check_periodic(1);

    let cloned = ctx.clone();

    // Both should see the same node count (Arc shared)
    assert_eq!(ctx.nodes_processed(), cloned.nodes_processed());

    // Cancelling one should affect the other (shared atomic)
    ctx.cancel();
    assert!(cloned.is_cancelled());
}

// =============================================================================
// with_timeout Tests
// =============================================================================

#[test]
fn test_with_timeout_completes_successfully() {
    let result = with_timeout(Duration::from_secs(5), || 42);

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 42);
}

#[test]
fn test_with_timeout_returns_complex_value() {
    let result = with_timeout(Duration::from_secs(5), || vec![1, 2, 3, 4, 5]);

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), vec![1, 2, 3, 4, 5]);
}

#[test]
fn test_with_timeout_times_out() {
    let result = with_timeout(Duration::from_secs(1), || {
        std::thread::sleep(Duration::from_secs(5));
        42
    });

    assert!(result.is_err());
    match result.unwrap_err() {
        TldrError::Timeout(msg) => {
            assert!(msg.contains("timed out"));
            assert!(msg.contains("1s"));
        }
        _ => panic!("Expected Timeout error"),
    }
}

#[test]
fn test_with_timeout_no_timeout() {
    let result = with_timeout(Duration::from_secs(1), || {
        std::thread::sleep(Duration::from_millis(10));
        "done"
    });

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "done");
}

#[test]
fn test_with_timeout_result_completes_successfully() {
    let result = with_timeout_result(Duration::from_secs(5), || -> Result<i32, std::io::Error> {
        Ok(42)
    });

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 42);
}

#[test]
fn test_with_timeout_result_inner_error() {
    let result = with_timeout_result(Duration::from_secs(5), || -> Result<i32, std::io::Error> {
        Err(std::io::Error::other(
            "inner error",
        ))
    });

    assert!(result.is_err());
    match result.unwrap_err() {
        TldrError::Timeout(msg) => {
            assert!(msg.contains("Analysis failed"));
            assert!(msg.contains("inner error"));
        }
        _ => panic!("Expected Timeout error with inner error message"),
    }
}

#[test]
fn test_with_timeout_result_times_out() {
    let result = with_timeout_result(
        Duration::from_millis(10),
        || -> Result<i32, std::io::Error> {
            std::thread::sleep(Duration::from_secs(1));
            Ok(42)
        },
    );

    assert!(result.is_err());
    match result.unwrap_err() {
        TldrError::Timeout(msg) => {
            assert!(msg.contains("timed out"));
        }
        _ => panic!("Expected Timeout error"),
    }
}

#[test]
fn test_with_timeout_panic_handling() {
    let result = with_timeout(Duration::from_secs(1), || {
        panic!("intentional panic");
    });

    // Panic causes disconnect, which is reported as timeout
    assert!(result.is_err());
    match result.unwrap_err() {
        TldrError::Timeout(msg) => {
            assert!(msg.contains("panicked"));
        }
        _ => panic!("Expected Timeout error with panic message"),
    }
}

// =============================================================================
// AnalysisProgress Tests
// =============================================================================

#[test]
fn test_analysis_progress_new() {
    let progress = AnalysisProgress::new();

    assert_eq!(progress.files_scanned, 0);
    assert_eq!(progress.files_skipped, 0);
    assert_eq!(progress.nodes_processed, 0);
    assert_eq!(progress.edges_processed, 0);
    assert!(!progress.truncated);
    assert!(progress.truncation_reason.is_none());
    assert_eq!(progress.elapsed_ms, 0);
}

#[test]
fn test_analysis_progress_default() {
    let progress: AnalysisProgress = Default::default();

    assert_eq!(progress.files_scanned, 0);
    assert!(!progress.truncated);
}

#[test]
fn test_analysis_progress_truncate() {
    let mut progress = AnalysisProgress::new();

    assert!(!progress.truncated);
    assert!(progress.truncation_reason.is_none());

    progress.truncate("Max nodes exceeded");

    assert!(progress.truncated);
    assert_eq!(
        progress.truncation_reason,
        Some("Max nodes exceeded".to_string())
    );
}

#[test]
fn test_analysis_progress_set_elapsed() {
    let mut progress = AnalysisProgress::new();

    progress.set_elapsed(Duration::from_secs(5));

    assert_eq!(progress.elapsed_ms, 5000);
}

#[test]
fn test_analysis_progress_set_elapsed_milliseconds() {
    let mut progress = AnalysisProgress::new();

    progress.set_elapsed(Duration::from_millis(1500));

    assert_eq!(progress.elapsed_ms, 1500);
}

#[test]
fn test_analysis_progress_serde_roundtrip() {
    let mut progress = AnalysisProgress::new();
    progress.files_scanned = 100;
    progress.files_skipped = 5;
    progress.nodes_processed = 1000;
    progress.edges_processed = 5000;
    progress.truncate("Memory limit");
    progress.elapsed_ms = 5000;

    let json = serde_json::to_string(&progress).unwrap();
    let parsed: AnalysisProgress = serde_json::from_str(&json).unwrap();

    assert_eq!(progress.files_scanned, parsed.files_scanned);
    assert_eq!(progress.files_skipped, parsed.files_skipped);
    assert_eq!(progress.nodes_processed, parsed.nodes_processed);
    assert_eq!(progress.edges_processed, parsed.edges_processed);
    assert_eq!(progress.truncated, parsed.truncated);
    assert_eq!(progress.truncation_reason, parsed.truncation_reason);
    assert_eq!(progress.elapsed_ms, parsed.elapsed_ms);
}

// =============================================================================
// Integration Tests
// =============================================================================

#[test]
fn test_limits_workflow_nodes() {
    let limits = AnalysisLimits::default().with_max_nodes(100);
    let mut progress = AnalysisProgress::new();

    // Simulate processing
    for i in 0..50 {
        if let Err(e) = limits.check_nodes(i) {
            progress.truncate(e.to_string());
            break;
        }
        progress.nodes_processed = i;
    }

    assert!(!progress.truncated);
    assert_eq!(progress.nodes_processed, 49);
}

#[test]
fn test_limits_workflow_truncate() {
    let limits = AnalysisLimits::default().with_max_nodes(100);
    let mut progress = AnalysisProgress::new();

    // Try to process more than limit
    match limits.check_nodes(150) {
        Ok(_) => panic!("Expected error"),
        Err(e) => {
            progress.truncate(format!("Stopped: {}", e));
        }
    }

    assert!(progress.truncated);
    assert!(progress
        .truncation_reason
        .as_ref()
        .unwrap()
        .contains("Stopped"));
}

#[test]
fn test_timeout_with_context() {
    let ctx = TimeoutContext::new(30);

    // Simulate work
    for _ in 0..100 {
        if ctx.check().is_err() {
            break;
        }
        // Do work
    }

    // Should complete without timeout
    assert!(!ctx.is_cancelled());
}

#[test]
fn test_timeout_context_periodic_checking() {
    let ctx = TimeoutContext::new(30);

    // Process many nodes, checking periodically
    for i in 0..1000 {
        if let Ok(true) = ctx.check_periodic(100) {
            // This is a check point (every 100 nodes)
            assert!(i % 100 == 0);
        }
    }

    assert_eq!(ctx.nodes_processed(), 1000);
}

#[test]
fn test_send_sync_bounds() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<AnalysisLimits>();
    assert_send_sync::<LimitExceeded>();
    assert_send_sync::<TimeoutContext>();
    assert_send_sync::<AnalysisProgress>();
}

#[test]
fn test_clone_bounds() {
    fn assert_clone<T: Clone>() {}
    assert_clone::<AnalysisLimits>();
    assert_clone::<LimitExceeded>();
    assert_clone::<TimeoutContext>();
    assert_clone::<AnalysisProgress>();
}
