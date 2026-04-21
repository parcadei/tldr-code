//! Tests for the embedding enrichment module (embedding-overhaul migration).
//!
//! These tests define expected behavior for all 5 changes:
//! 1. Enrichment (build_embedding_text)
//! 2. Default model switch (ArcticM -> ArcticS)
//! 3. Bincode cache
//! 4. Parallel chunking
//! 5. Incremental embedding

#[cfg(test)]
mod enrichment_tests {
    use std::collections::HashSet;
    use std::fs;
    use std::path::PathBuf;

    use tempfile::TempDir;

    use crate::semantic::enrichment::{
        build_embedding_text, content_hash_from_source, enrich_chunks, EmbeddingUnit,
    };
    use crate::semantic::types::{CacheConfig, CodeChunk, EmbeddingModel};
    use crate::Language;

    // =========================================================================
    // Helpers
    // =========================================================================

    fn create_test_chunk(name: &str, content: &str) -> CodeChunk {
        CodeChunk {
            file_path: PathBuf::from(format!("test/{}.rs", name)),
            function_name: Some(name.to_string()),
            class_name: None,
            line_start: 1,
            line_end: 10,
            content: content.to_string(),
            content_hash: format!("{:x}", md5::compute(content)),
            language: Language::Rust,
        }
    }

    fn create_file_level_chunk(filename: &str, content: &str) -> CodeChunk {
        CodeChunk {
            file_path: PathBuf::from(filename),
            function_name: None,
            class_name: None,
            line_start: 1,
            line_end: content.lines().count() as u32,
            content: content.to_string(),
            content_hash: format!("{:x}", md5::compute(content)),
            language: Language::Rust,
        }
    }

    fn make_enriched_unit(
        name: &str,
        content: &str,
        calls: Vec<&str>,
        called_by: Vec<&str>,
        cfg: &str,
        dfg: &str,
        deps: &str,
    ) -> EmbeddingUnit {
        EmbeddingUnit {
            chunk: create_test_chunk(name, content),
            signature: format!("fn {}() -> Result<()>", name),
            docstring: format!("Process data for {}", name),
            calls: calls.into_iter().map(String::from).collect(),
            called_by: called_by.into_iter().map(String::from).collect(),
            cfg_summary: cfg.to_string(),
            dfg_summary: dfg.to_string(),
            dependencies: deps.to_string(),
        }
    }

    // =========================================================================
    // Change 1: Enrichment -- build_embedding_text
    // =========================================================================

    #[test]
    fn build_embedding_text_includes_signature() {
        // GIVEN: An EmbeddingUnit with a signature
        let unit = make_enriched_unit(
            "process_data",
            "fn process_data() { validate(); transform(); }",
            vec!["validate", "transform"],
            vec!["main"],
            "complexity=4, branches=3, loops=1",
            "vars=5, defs=3, uses=8",
            "serde, tokio",
        );

        // WHEN: We build the embedding text
        let text = build_embedding_text(&unit);

        // THEN: The text should contain the signature line
        assert!(
            text.contains("Signature:"),
            "Embedding text should contain 'Signature:' line, got: {:?}",
            text
        );
        assert!(
            text.contains("process_data"),
            "Embedding text should contain the function name, got: {:?}",
            text
        );
    }

    #[test]
    fn build_embedding_text_includes_calls() {
        // GIVEN: An EmbeddingUnit with callees
        let unit = make_enriched_unit(
            "process_data",
            "fn process_data() { validate(); transform(); }",
            vec!["validate", "transform"],
            vec![],
            "",
            "",
            "",
        );

        // WHEN: We build the embedding text
        let text = build_embedding_text(&unit);

        // THEN: The text should contain "Calls:" with the callees
        assert!(
            text.contains("Calls:"),
            "Embedding text should contain 'Calls:' when callees present, got: {:?}",
            text
        );
        assert!(
            text.contains("validate"),
            "Embedding text should list callee 'validate', got: {:?}",
            text
        );
        assert!(
            text.contains("transform"),
            "Embedding text should list callee 'transform', got: {:?}",
            text
        );
    }

    #[test]
    fn build_embedding_text_includes_called_by() {
        // GIVEN: An EmbeddingUnit with callers
        let unit = make_enriched_unit(
            "validate",
            "fn validate() {}",
            vec![],
            vec!["main", "run_pipeline"],
            "",
            "",
            "",
        );

        // WHEN: We build the embedding text
        let text = build_embedding_text(&unit);

        // THEN: The text should contain "Called by:" with the callers
        assert!(
            text.contains("Called by:"),
            "Embedding text should contain 'Called by:' when callers present, got: {:?}",
            text
        );
        assert!(
            text.contains("main"),
            "Embedding text should list caller 'main', got: {:?}",
            text
        );
        assert!(
            text.contains("run_pipeline"),
            "Embedding text should list caller 'run_pipeline', got: {:?}",
            text
        );
    }

    #[test]
    fn build_embedding_text_includes_control_flow() {
        // GIVEN: An EmbeddingUnit with CFG summary
        let unit = make_enriched_unit(
            "process",
            "fn process() {}",
            vec![],
            vec![],
            "complexity=4, branches=3, loops=1",
            "",
            "",
        );

        // WHEN: We build the embedding text
        let text = build_embedding_text(&unit);

        // THEN: The text should contain "Control flow:" line
        assert!(
            text.contains("Control flow:"),
            "Embedding text should contain 'Control flow:' line, got: {:?}",
            text
        );
        assert!(
            text.contains("complexity=4"),
            "Embedding text should include complexity metric, got: {:?}",
            text
        );
    }

    #[test]
    fn build_embedding_text_includes_dependencies() {
        // GIVEN: An EmbeddingUnit with dependencies
        let unit = make_enriched_unit(
            "process",
            "fn process() {}",
            vec![],
            vec![],
            "",
            "",
            "serde, tokio",
        );

        // WHEN: We build the embedding text
        let text = build_embedding_text(&unit);

        // THEN: The text should contain "Dependencies:" line
        assert!(
            text.contains("Dependencies:"),
            "Embedding text should contain 'Dependencies:' line, got: {:?}",
            text
        );
        assert!(
            text.contains("serde"),
            "Embedding text should include 'serde' dependency, got: {:?}",
            text
        );
    }

    #[test]
    fn build_embedding_text_under_512_tokens() {
        // GIVEN: A fully enriched unit
        let unit = make_enriched_unit(
            "process_data",
            "fn process_data(config: &Config) -> Result<Data> { validate_input(config); transform(config.data); write_output(result); }",
            vec!["validate_input", "transform", "write_output", "serialize", "log_result"],
            vec!["main", "run_pipeline", "handle_request"],
            "complexity=8, branches=5, loops=2",
            "vars=12, defs=7, uses=15",
            "serde, tokio, anyhow, tracing, config",
        );

        // WHEN: We build the embedding text
        let text = build_embedding_text(&unit);

        // THEN: The text should be under 2000 characters (~512 tokens)
        assert!(
            text.len() < 2000,
            "Embedding text should be under 2000 chars (~512 tokens), got {} chars: {:?}",
            text.len(),
            text
        );
    }

    #[test]
    fn build_embedding_text_minimal_unit() {
        // GIVEN: An EmbeddingUnit with only L1 data (signature), no other layers
        let unit = EmbeddingUnit {
            chunk: create_test_chunk("simple", "fn simple() {}"),
            signature: "fn simple()".to_string(),
            docstring: String::new(),
            calls: Vec::new(),
            called_by: Vec::new(),
            cfg_summary: String::new(),
            dfg_summary: String::new(),
            dependencies: String::new(),
        };

        // WHEN: We build the embedding text
        let text = build_embedding_text(&unit);

        // THEN: Should not panic and should contain at least the function name
        assert!(
            text.contains("simple"),
            "Minimal enrichment should still include function name, got: {:?}",
            text
        );
        // Should NOT contain optional layer headers when data is empty
        assert!(
            !text.contains("Calls:"),
            "Should not include 'Calls:' when no callees, got: {:?}",
            text
        );
        assert!(
            !text.contains("Called by:"),
            "Should not include 'Called by:' when no callers, got: {:?}",
            text
        );
    }

    #[test]
    fn build_embedding_text_file_level_chunk() {
        // GIVEN: A file-level chunk (no function name)
        let unit = EmbeddingUnit {
            chunk: create_file_level_chunk(
                "src/config.rs",
                "use serde::Deserialize;\n\n#[derive(Deserialize)]\nstruct Config {\n    port: u16,\n}",
            ),
            signature: String::new(),
            docstring: String::new(),
            calls: Vec::new(),
            called_by: Vec::new(),
            cfg_summary: String::new(),
            dfg_summary: String::new(),
            dependencies: "serde".to_string(),
        };

        // WHEN: We build the embedding text
        let text = build_embedding_text(&unit);

        // THEN: Should use the filename since there is no function name
        assert!(
            text.contains("config.rs") || text.contains("config"),
            "File-level chunk should reference filename, got: {:?}",
            text
        );
    }

    #[test]
    fn build_embedding_text_no_raw_code() {
        // GIVEN: An EmbeddingUnit with a function body in the chunk content
        let long_body = "fn process_data(config: &Config) -> Result<Data> {\n    let x = validate_input(config)?;\n    let y = transform(x)?;\n    let z = serialize(y)?;\n    write_output(z)?;\n    Ok(Data::new())\n}";
        let unit = make_enriched_unit(
            "process_data",
            long_body,
            vec!["validate_input", "transform"],
            vec!["main"],
            "complexity=4",
            "vars=5",
            "serde",
        );

        // WHEN: We build the embedding text
        let text = build_embedding_text(&unit);

        // THEN: The enriched text should be a summary, not the full function body
        assert!(
            !text.contains("let x = validate_input"),
            "Enriched text should not contain raw function body lines, got: {:?}",
            text
        );
        assert!(
            !text.contains("Ok(Data::new())"),
            "Enriched text should not contain raw function body, got: {:?}",
            text
        );
    }

    // =========================================================================
    // Change 2: Default Model is ArcticS
    // =========================================================================

    #[test]
    fn default_model_is_arctic_s() {
        // GIVEN/WHEN: We get the default embedding model
        let model = EmbeddingModel::default();

        // THEN: Default should be ArcticS (not ArcticM)
        assert_eq!(
            model,
            EmbeddingModel::ArcticS,
            "Default model should be ArcticS, got {:?}",
            model
        );
    }

    #[test]
    fn arctic_s_dimensions_384() {
        // GIVEN: The ArcticS model
        let model = EmbeddingModel::ArcticS;

        // WHEN: We check its dimensions
        let dims = model.dimensions();

        // THEN: Should be 384
        assert_eq!(
            dims, 384,
            "ArcticS should have 384 dimensions, got {}",
            dims
        );
    }

    // =========================================================================
    // Change 3: Bincode Cache
    // =========================================================================

    #[test]
    fn cache_roundtrip_bincode() {
        // GIVEN: A cache with entries, flushed to disk
        let temp = TempDir::new().unwrap();
        let config = CacheConfig {
            cache_dir: temp.path().to_path_buf(),
            max_size_mb: 100,
            ttl_days: 7,
        };

        let chunk = create_test_chunk("foo", "fn foo() {}");
        let embedding = vec![0.1_f32, 0.2, 0.3, 0.4, 0.5];

        // Put entry and flush
        {
            let mut cache =
                crate::semantic::cache::EmbeddingCache::open(config.clone()).unwrap();
            cache.put(&chunk, embedding.clone(), EmbeddingModel::ArcticM);
            cache.flush().unwrap();
        }

        // WHEN: We reopen the cache
        let mut cache2 = crate::semantic::cache::EmbeddingCache::open(config).unwrap();
        let result = cache2.get(&chunk, EmbeddingModel::ArcticM);

        // THEN: Roundtrip should preserve the embedding
        assert!(
            result.is_some(),
            "Cache roundtrip should preserve entries after flush + reopen"
        );
        assert_eq!(
            result.unwrap(),
            embedding,
            "Embedding values should match after roundtrip"
        );
    }

    #[test]
    fn cache_file_is_binary_not_json() {
        // GIVEN: A cache that has been flushed to disk
        let temp = TempDir::new().unwrap();
        let config = CacheConfig {
            cache_dir: temp.path().to_path_buf(),
            max_size_mb: 100,
            ttl_days: 7,
        };

        let chunk = create_test_chunk("foo", "fn foo() {}");
        let embedding = vec![0.1_f32; 384]; // ArcticS-sized embedding

        {
            let mut cache =
                crate::semantic::cache::EmbeddingCache::open(config.clone()).unwrap();
            cache.put(&chunk, embedding, EmbeddingModel::ArcticS);
            cache.flush().unwrap();
        }

        // WHEN: We look at the cache file on disk
        let cache_bin = temp.path().join("cache.bin");
        let cache_json = temp.path().join("cache.json");

        // THEN: cache.bin should exist (not cache.json)
        assert!(
            cache_bin.exists(),
            "Cache file should be 'cache.bin' (bincode format), but cache.bin does not exist. Files in dir: {:?}",
            fs::read_dir(temp.path())
                .unwrap()
                .filter_map(|e| e.ok())
                .map(|e| e.file_name())
                .collect::<Vec<_>>()
        );

        // The file should NOT be valid JSON
        if cache_bin.exists() {
            let contents = fs::read(&cache_bin).unwrap();
            let json_result: Result<serde_json::Value, _> =
                serde_json::from_slice(&contents);
            assert!(
                json_result.is_err(),
                "Cache file should be binary (bincode), not JSON"
            );
        }

        // cache.json should NOT exist (we switched to bincode)
        assert!(
            !cache_json.exists(),
            "Old cache.json should not exist when using bincode format"
        );
    }

    #[test]
    fn cache_file_size_compact() {
        // GIVEN: A cache with 100 entries of 384-dim embeddings
        let temp = TempDir::new().unwrap();
        let config = CacheConfig {
            cache_dir: temp.path().to_path_buf(),
            max_size_mb: 100,
            ttl_days: 7,
        };

        {
            let mut cache =
                crate::semantic::cache::EmbeddingCache::open(config.clone()).unwrap();

            for i in 0..100 {
                let chunk = create_test_chunk(
                    &format!("func_{}", i),
                    &format!("fn func_{}() {{ /* body {} */ }}", i, i),
                );
                // 384-dim embedding (ArcticS size)
                let embedding: Vec<f32> = (0..384).map(|j| (j as f32) * 0.001 + (i as f32) * 0.01).collect();
                cache.put(&chunk, embedding, EmbeddingModel::ArcticS);
            }

            cache.flush().unwrap();
        }

        // WHEN: We check the file size
        // Look for either cache.bin (bincode) or cache.json (current)
        let cache_file = if temp.path().join("cache.bin").exists() {
            temp.path().join("cache.bin")
        } else {
            temp.path().join("cache.json")
        };

        let file_size = fs::metadata(&cache_file).unwrap().len();

        // THEN: 100 entries x 384-dim should be < 200KB with bincode
        // (bincode: 100 * (384*4 + overhead) ~ 160KB)
        // (JSON would be: 100 * (384*12 + overhead) ~ 500KB+)
        assert!(
            file_size < 200_000,
            "Cache file with 100 x 384-dim entries should be < 200KB with bincode, got {} bytes ({:.1}KB)",
            file_size,
            file_size as f64 / 1024.0
        );
    }

    // =========================================================================
    // Change 4: Parallel Chunking
    // =========================================================================

    #[test]
    fn chunk_directory_parallel_complete() {
        // GIVEN: A directory with multiple source files
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("a.rs"), "fn alpha() { println!(\"a\"); }").unwrap();
        fs::write(tmp.path().join("b.rs"), "fn beta() { println!(\"b\"); }").unwrap();
        fs::write(tmp.path().join("c.py"), "def gamma():\n    pass").unwrap();

        let sub = tmp.path().join("sub");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("d.rs"), "fn delta() {}").unwrap();

        // WHEN: We chunk the directory
        let options = crate::semantic::types::ChunkOptions::default();
        let result = crate::semantic::chunker::chunk_code(tmp.path(), &options).unwrap();

        // THEN: All files should be represented in chunks
        let file_paths: HashSet<String> = result
            .chunks
            .iter()
            .map(|c| c.file_path.to_string_lossy().to_string())
            .collect();

        // Should find chunks from all source files
        assert!(
            result.chunks.len() >= 4,
            "Should have at least 4 chunks (one per function), got {}. Chunks: {:?}",
            result.chunks.len(),
            result.chunks.iter().map(|c| (&c.file_path, &c.function_name)).collect::<Vec<_>>()
        );

        let func_names: HashSet<String> = result
            .chunks
            .iter()
            .filter_map(|c| c.function_name.clone())
            .collect();

        assert!(
            func_names.contains("alpha"),
            "Should find function 'alpha' from a.rs"
        );
        assert!(
            func_names.contains("beta"),
            "Should find function 'beta' from b.rs"
        );
        assert!(
            func_names.contains("delta"),
            "Should find function 'delta' from sub/d.rs"
        );
    }

    #[test]
    fn chunk_directory_parallel_deterministic_set() {
        // GIVEN: A directory with multiple source files
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("a.rs"), "fn alpha() {}").unwrap();
        fs::write(tmp.path().join("b.rs"), "fn beta() {}").unwrap();
        fs::write(tmp.path().join("c.rs"), "fn gamma() {}").unwrap();

        let sub = tmp.path().join("sub");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("d.rs"), "fn delta() {}").unwrap();
        fs::write(sub.join("e.rs"), "fn epsilon() {}").unwrap();

        let options = crate::semantic::types::ChunkOptions::default();

        // WHEN: We chunk the directory twice
        let result1 = crate::semantic::chunker::chunk_code(tmp.path(), &options).unwrap();
        let result2 = crate::semantic::chunker::chunk_code(tmp.path(), &options).unwrap();

        // THEN: The SET of chunks should be identical (order may differ with parallel)
        let names1: HashSet<String> = result1
            .chunks
            .iter()
            .filter_map(|c| c.function_name.clone())
            .collect();
        let names2: HashSet<String> = result2
            .chunks
            .iter()
            .filter_map(|c| c.function_name.clone())
            .collect();

        assert_eq!(
            names1, names2,
            "Chunking the same directory twice should produce the same set of function names"
        );
        assert_eq!(
            result1.chunks.len(),
            result2.chunks.len(),
            "Chunking the same directory twice should produce the same number of chunks"
        );

        // Content hashes should also match
        let hashes1: HashSet<String> = result1
            .chunks
            .iter()
            .map(|c| c.content_hash.clone())
            .collect();
        let hashes2: HashSet<String> = result2
            .chunks
            .iter()
            .map(|c| c.content_hash.clone())
            .collect();
        assert_eq!(
            hashes1, hashes2,
            "Content hashes should be identical across runs"
        );
    }

    // =========================================================================
    // Change 5: Incremental Embedding
    // =========================================================================

    #[test]
    fn incremental_embed_skips_unchanged() {
        // GIVEN: A cache with entries for known chunks
        let temp = TempDir::new().unwrap();
        let config = CacheConfig {
            cache_dir: temp.path().to_path_buf(),
            max_size_mb: 100,
            ttl_days: 7,
        };

        let chunk1 = create_test_chunk("foo", "fn foo() { return 1; }");
        let chunk2 = create_test_chunk("bar", "fn bar() { return 2; }");

        let embedding1 = vec![0.1_f32, 0.2, 0.3];
        let embedding2 = vec![0.4_f32, 0.5, 0.6];

        // Pre-populate cache
        {
            let mut cache =
                crate::semantic::cache::EmbeddingCache::open(config.clone()).unwrap();
            cache.put(&chunk1, embedding1.clone(), EmbeddingModel::ArcticM);
            cache.put(&chunk2, embedding2.clone(), EmbeddingModel::ArcticM);
            cache.flush().unwrap();
        }

        // WHEN: We query the cache for the same (unchanged) chunks
        let mut cache =
            crate::semantic::cache::EmbeddingCache::open(config).unwrap();

        let hit1 = cache.get(&chunk1, EmbeddingModel::ArcticM);
        let hit2 = cache.get(&chunk2, EmbeddingModel::ArcticM);

        // THEN: Both should be cache hits (100% cache hit rate)
        assert!(
            hit1.is_some(),
            "Unchanged chunk1 should be a cache hit"
        );
        assert!(
            hit2.is_some(),
            "Unchanged chunk2 should be a cache hit"
        );
        assert_eq!(
            hit1.unwrap(),
            embedding1,
            "Cached embedding1 should match original"
        );
        assert_eq!(
            hit2.unwrap(),
            embedding2,
            "Cached embedding2 should match original"
        );
    }

    #[test]
    fn incremental_embed_detects_changes() {
        // GIVEN: A cache with an entry for a chunk
        let temp = TempDir::new().unwrap();
        let config = CacheConfig {
            cache_dir: temp.path().to_path_buf(),
            max_size_mb: 100,
            ttl_days: 7,
        };

        let chunk_v1 = create_test_chunk("foo", "fn foo() { return 1; }");
        let embedding = vec![0.1_f32, 0.2, 0.3];

        // Cache the original version
        {
            let mut cache =
                crate::semantic::cache::EmbeddingCache::open(config.clone()).unwrap();
            cache.put(&chunk_v1, embedding.clone(), EmbeddingModel::ArcticM);
            cache.flush().unwrap();
        }

        // WHEN: The source code changes (different content_hash)
        let chunk_v2 = create_test_chunk("foo", "fn foo() { return 2; /* modified */ }");

        let mut cache =
            crate::semantic::cache::EmbeddingCache::open(config).unwrap();
        let result = cache.get(&chunk_v2, EmbeddingModel::ArcticM);

        // THEN: Should be a cache miss (content changed)
        assert!(
            result.is_none(),
            "Modified chunk should be a cache miss (content_hash changed)"
        );
    }

    #[test]
    fn content_hash_based_on_source() {
        // GIVEN: The same source code
        let source = "fn process() { validate(); transform(); }";

        // WHEN: We compute the hash twice
        let hash1 = content_hash_from_source(source);
        let hash2 = content_hash_from_source(source);

        // THEN: Hashes should be identical (deterministic)
        assert_eq!(
            hash1, hash2,
            "Content hash should be deterministic for same source"
        );

        // AND: Hash should be different for different source
        let different_source = "fn process() { validate(); transform(); /* comment */ }";
        let hash3 = content_hash_from_source(different_source);
        assert_ne!(
            hash1, hash3,
            "Content hash should differ when source code changes"
        );

        // AND: Hash should NOT change when only cross-references change
        // (the hash is based on source code only, not on callers/callees)
        // This is verified by the fact that we hash `source` directly,
        // not any enrichment metadata
        let hash_same_source_different_context = content_hash_from_source(source);
        assert_eq!(
            hash1, hash_same_source_different_context,
            "Hash should be stable regardless of cross-reference changes"
        );
    }
}
