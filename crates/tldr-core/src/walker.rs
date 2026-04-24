//! Shared project walker built on `ignore::WalkBuilder`.
//!
//! Honors `.gitignore`, skips hidden dirs, skips vendor/build dirs by default,
//! does not follow symlinks. Every project-wide filesystem walk in tldr
//! should go through this module instead of using `walkdir::WalkDir` directly.
//!
//! # Why this exists
//!
//! Raw `walkdir::WalkDir` doesn't honor `.gitignore`, doesn't skip vendor dirs
//! (like `node_modules`, `target`, `dist`), and by default follows symlinks.
//! In pnpm monorepos `node_modules/.pnpm/` is a symlink forest that causes
//! infinite loops (`tldr smells` on a 2GB pnpm repo ran for 10+ minutes
//! before being killed) and produces false findings inside vendored code.
//!
//! # Typical usage
//!
//! ```rust,ignore
//! use tldr_core::walker::walk_project;
//!
//! for entry in walk_project("src") {
//!     // `entry` is an `ignore::DirEntry` yielded only for non-ignored files.
//! }
//! ```
//!
//! Or with more control:
//!
//! ```rust,ignore
//! use tldr_core::walker::ProjectWalker;
//!
//! let files: Vec<_> = ProjectWalker::new("src")
//!     .max_depth(10)
//!     .extensions(&["rs"])
//!     .iter()
//!     .collect();
//! ```

use std::path::{Path, PathBuf};

use ignore::{DirEntry, WalkBuilder};

/// Directories skipped by default regardless of `.gitignore` presence.
///
/// Commands that explicitly need to scan vendored code (e.g. auditing
/// dependencies) can disable this list via
/// [`ProjectWalker::no_default_ignore`].
pub const DEFAULT_EXCLUDE_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    "__pycache__",
    "vendor",
    ".git",
];

/// Builder for project walks.
///
/// Produces an iterator of [`ignore::DirEntry`]s after applying:
/// - `.gitignore` / global gitignore / `.git/info/exclude` (default on)
/// - hidden-file filtering (always on)
/// - the [`DEFAULT_EXCLUDE_DIRS`] list (default on, disable via
///   [`ProjectWalker::no_default_ignore`])
/// - `follow_links(false)` (always — critical for pnpm symlink forests)
/// - optional max depth
/// - optional extension allow-list
pub struct ProjectWalker {
    root: PathBuf,
    respect_gitignore: bool,
    default_ignore: bool,
    max_depth: Option<usize>,
    extensions: Option<Vec<&'static str>>,
}

impl ProjectWalker {
    /// Create a walker rooted at `root` with all default filters on.
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
            respect_gitignore: true,
            default_ignore: true,
            max_depth: None,
            extensions: None,
        }
    }

    /// Disable the [`DEFAULT_EXCLUDE_DIRS`] list.
    ///
    /// Use when a command explicitly needs to scan vendored code
    /// (e.g. `node_modules`, `target`). `.gitignore` is still honored
    /// unless [`ProjectWalker::respect_gitignore(false)`] is also set.
    pub fn no_default_ignore(mut self) -> Self {
        self.default_ignore = false;
        self
    }

    /// Control whether `.gitignore` rules are honored. Default: `true`.
    pub fn respect_gitignore(mut self, yes: bool) -> Self {
        self.respect_gitignore = yes;
        self
    }

    /// Limit recursion depth.
    pub fn max_depth(mut self, n: usize) -> Self {
        self.max_depth = Some(n);
        self
    }

    /// Only yield files with these extensions (e.g. `&["rs", "ts", "tsx"]`).
    ///
    /// Extensions should NOT include the leading dot. Matching is
    /// case-sensitive. Callers that want language-aware filtering should
    /// prefer `Language::from_path` after the walk.
    pub fn extensions(mut self, exts: &[&'static str]) -> Self {
        self.extensions = Some(exts.to_vec());
        self
    }

    /// Iterate yielded entries.
    ///
    /// Errors during traversal (permission denied, broken symlinks, etc.)
    /// are silently skipped — the caller gets only successful `DirEntry`s.
    pub fn iter(self) -> impl Iterator<Item = DirEntry> {
        let default_ignore = self.default_ignore;
        let extensions = self.extensions.clone();

        let mut builder = WalkBuilder::new(&self.root);
        builder
            .hidden(true) // skip .hidden files/dirs
            .git_ignore(self.respect_gitignore)
            .git_global(self.respect_gitignore)
            .git_exclude(self.respect_gitignore)
            .parents(self.respect_gitignore)
            .follow_links(false); // CRITICAL: avoid pnpm symlink loops

        if let Some(depth) = self.max_depth {
            builder.max_depth(Some(depth));
        }

        if default_ignore {
            builder.filter_entry(|entry| {
                // Only filter directory entries by the exclude list; files
                // named "node_modules" are fine to yield (edge case).
                let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
                if !is_dir {
                    return true;
                }
                match entry.file_name().to_str() {
                    Some(name) => !DEFAULT_EXCLUDE_DIRS.contains(&name),
                    None => true,
                }
            });
        }

        builder.build().filter_map(move |res| {
            let entry = res.ok()?;
            if let Some(ref allowed) = extensions {
                // Only apply extension filter to files; directories must
                // still pass through so we can descend into them.
                let is_file = entry.file_type().map(|ft| ft.is_file()).unwrap_or(false);
                if is_file {
                    let ext = entry.path().extension().and_then(|s| s.to_str());
                    match ext {
                        Some(e) if allowed.contains(&e) => Some(entry),
                        _ => None,
                    }
                } else {
                    Some(entry)
                }
            } else {
                Some(entry)
            }
        })
    }
}

/// Convenience free function: walk project with all defaults on.
///
/// Equivalent to `ProjectWalker::new(root).iter()`. Use [`ProjectWalker`]
/// directly for finer control (extension filters, max depth, opt-outs).
pub fn walk_project(root: impl AsRef<Path>) -> impl Iterator<Item = DirEntry> {
    ProjectWalker::new(root).iter()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn write_file(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    fn collect_rel_files(root: &Path, walker: impl Iterator<Item = DirEntry>) -> Vec<String> {
        let mut out: Vec<String> = walker
            .filter(|e| e.file_type().map(|ft| ft.is_file()).unwrap_or(false))
            .map(|e| {
                e.path()
                    .strip_prefix(root)
                    .unwrap_or(e.path())
                    .to_string_lossy()
                    .replace('\\', "/")
                    .to_string()
            })
            .collect();
        out.sort();
        out
    }

    #[test]
    fn test_skips_node_modules_by_default() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        write_file(&root.join("foo.rs"), "fn main() {}");
        write_file(&root.join("node_modules/bad.py"), "import os");

        let files = collect_rel_files(root, walk_project(root));
        assert_eq!(files, vec!["foo.rs".to_string()]);
    }

    #[test]
    fn test_skips_target_dist_build_cache() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        write_file(&root.join("src/lib.rs"), "fn main() {}");
        write_file(&root.join("target/debug/x.rs"), "fn x() {}");
        write_file(&root.join("dist/bundle.js"), "// bundled");
        write_file(&root.join("build/out.o"), "binary");
        write_file(&root.join("__pycache__/cached.pyc"), "binary");
        write_file(&root.join(".next/cache.js"), "// cached");
        write_file(&root.join("vendor/dep.go"), "package v");

        let files = collect_rel_files(root, walk_project(root));
        assert_eq!(files, vec!["src/lib.rs".to_string()]);
    }

    #[test]
    fn test_respects_gitignore() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        // Gotcha: ignore crate only activates gitignore under a git repo or
        // if we register a custom ignore. Create a .git dir so it's treated
        // as a repo root.
        fs::create_dir_all(root.join(".git")).unwrap();
        write_file(&root.join(".gitignore"), "secret/\n");
        write_file(&root.join("foo.rs"), "fn main() {}");
        write_file(&root.join("secret/x.rs"), "fn x() {}");

        let files = collect_rel_files(root, walk_project(root));
        assert_eq!(files, vec!["foo.rs".to_string()]);
    }

    #[test]
    fn test_hidden_dirs_skipped() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        write_file(&root.join("visible.rs"), "fn main() {}");
        write_file(&root.join(".hidden/secret.rs"), "fn secret() {}");

        let files = collect_rel_files(root, walk_project(root));
        assert_eq!(files, vec!["visible.rs".to_string()]);
    }

    #[test]
    fn test_does_not_follow_symlinks_into_loop() {
        // Build root/a.rs plus root/loop -> root to exercise the symlink
        // guard. On systems where symlinks aren't supported the call errors
        // out; just skip those.
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        write_file(&root.join("a.rs"), "fn a() {}");

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            // Point a child dir back to root -> would loop if followed.
            let loop_path = root.join("loop");
            symlink(root, &loop_path).unwrap();
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::symlink_dir;
            let loop_path = root.join("loop");
            // May fail without dev-mode; swallow the error so the rest of
            // the test still exercises normal traversal.
            let _ = symlink_dir(root, &loop_path);
        }

        // Traversal must terminate. Collect with a reasonable cap to
        // prevent a runaway test from hanging CI for infinity.
        let files: Vec<_> = walk_project(root).take(10_000).collect();
        // Must find a.rs exactly once; symlink target must not be
        // descended into.
        let count_a = files.iter().filter(|e| e.file_name() == "a.rs").count();
        assert_eq!(count_a, 1, "expected exactly one a.rs, got {}", count_a);
    }

    #[test]
    fn test_no_default_ignore_walks_node_modules() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        write_file(&root.join("foo.rs"), "fn main() {}");
        write_file(&root.join("node_modules/bad.py"), "import os");

        let files = collect_rel_files(root, ProjectWalker::new(root).no_default_ignore().iter());
        assert!(
            files.contains(&"foo.rs".to_string()),
            "missing foo.rs: {files:?}"
        );
        assert!(
            files.contains(&"node_modules/bad.py".to_string()),
            "expected node_modules/bad.py to be walked with no_default_ignore: {files:?}"
        );
    }

    #[test]
    fn test_extensions_filter() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        write_file(&root.join("a.rs"), "fn a() {}");
        write_file(&root.join("b.py"), "def b(): pass");
        write_file(&root.join("c.ts"), "function c() {}");

        let files = collect_rel_files(root, ProjectWalker::new(root).extensions(&["rs"]).iter());
        assert_eq!(files, vec!["a.rs".to_string()]);
    }

    #[test]
    fn test_max_depth_limits_recursion() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        write_file(&root.join("top.rs"), "fn top() {}");
        write_file(&root.join("a/b/deep.rs"), "fn deep() {}");

        // max_depth(1) should include entries exactly one level deep, i.e.
        // files immediately under root (top.rs and the `a` directory) but
        // not a/b/deep.rs.
        let files = collect_rel_files(root, ProjectWalker::new(root).max_depth(1).iter());
        assert!(files.contains(&"top.rs".to_string()), "{files:?}");
        assert!(
            !files.contains(&"a/b/deep.rs".to_string()),
            "max_depth=1 should have excluded deep file: {files:?}"
        );
    }
}
