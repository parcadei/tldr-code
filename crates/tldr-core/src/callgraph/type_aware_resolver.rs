//! Type-Aware Call Resolver for OOP method resolution.
//!
//! This module provides type-aware resolution for method calls like `obj.method()`.
//! It tracks object types through assignments and annotations to resolve method calls
//! to their target class implementations.
//!
//! # Overview
//!
//! The `TypeAwareCallResolver` bridges type inference and call graph construction by:
//! - Tracking variable types from assignments (`user = User()`)
//! - Tracking variable types from annotations (`user: User`)
//! - Tracking parameter types (`def f(user: User)`)
//! - Resolving method calls to qualified class.method names
//! - Handling inheritance through MRO (Method Resolution Order)
//! - Supporting chained calls like `a.b().c()`
//!
//! # Flow-Sensitive Narrowing Limitation
//!
//! **NOTE**: This resolver uses assignment-based type tracking.
//! Flow-sensitive narrowing (e.g., `if isinstance(x, Foo): x.method()`)
//! is NOT implemented. The type is determined by the last assignment.
//! Full flow-sensitive analysis would require CFG integration (future work).
//!
//! # Example
//!
//! ```rust,ignore
//! use tldr_core::callgraph::type_aware_resolver::{TypeAwareCallResolver, Confidence};
//! use tldr_core::callgraph::module_index::ModuleIndex;
//! use std::path::Path;
//!
//! let index = ModuleIndex::build(Path::new("src"), "python")?;
//! let resolver = TypeAwareCallResolver::new(&index, &func_index, &class_index);
//!
//! // Resolve: user.save() where user = User()
//! let result = resolver.resolve_method_call(
//!     Path::new("src/main.py"),
//!     "process",     // caller function
//!     "user",        // receiver variable
//!     "save",        // method name
//!     10,            // line number
//! );
//!
//! assert!(result.resolved_targets.contains("User.save"));
//! assert_eq!(result.confidence, Confidence::High);
//! ```
//!
//! # Spec Reference
//!
//! See `migration/spec/callgraph-spec.md` Section 7 for the full specification.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::cross_file_types::{ClassDef, FileIR};
use super::module_index::ModuleIndex;

// =============================================================================
// Section 7.1: Confidence
// =============================================================================

/// Confidence level for resolved call targets.
///
/// Indicates how certain the resolver is about the type resolution:
/// - `High`: Type is statically known (annotation, class instantiation)
/// - `Medium`: Type is inferred but may have alternatives (union types)
/// - `Low`: Type is unknown, falling back to original name
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    /// Type is statically known from annotation or class instantiation.
    /// Examples:
    /// - `user: User = User()` -> High (both annotation and instantiation)
    /// - `def f(user: User)` -> High (parameter annotation)
    High,

    /// Type is inferred but may have alternatives.
    /// Examples:
    /// - `user: User | Admin` -> Medium (union type)
    /// - Multiple possible types from control flow
    Medium,

    /// Type is unknown, resolution falls back to original name.
    /// Examples:
    /// - Untyped parameter: `def f(obj): obj.method()`
    /// - External/dynamic type that can't be resolved
    Low,
}

impl Confidence {
    /// Returns true if this confidence level is usable for call resolution.
    pub fn is_usable(&self) -> bool {
        matches!(self, Confidence::High | Confidence::Medium)
    }

    /// Combines two confidence levels, returning the lower of the two.
    pub fn combine(self, other: Confidence) -> Confidence {
        match (self, other) {
            (Confidence::Low, _) | (_, Confidence::Low) => Confidence::Low,
            (Confidence::Medium, _) | (_, Confidence::Medium) => Confidence::Medium,
            (Confidence::High, Confidence::High) => Confidence::High,
        }
    }
}

// =============================================================================
// Section 7.2: TypeSource
// =============================================================================

/// How a variable's type was determined.
///
/// Tracks the source of type information for debugging and confidence scoring.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TypeSource {
    /// Type from assignment: `x = SomeClass()`
    Assignment,
    /// Type from annotation: `x: SomeClass`
    TypeAnnotation,
    /// Type from function parameter: `def f(x: SomeClass)`
    Parameter,
    /// Type from function return: `x = func()` where func returns SomeClass
    Return,
    /// Type inferred from context (less certain)
    Inferred,
}

impl TypeSource {
    /// Returns the confidence level associated with this type source.
    pub fn confidence(&self) -> Confidence {
        match self {
            TypeSource::Assignment => Confidence::High,
            TypeSource::TypeAnnotation => Confidence::High,
            TypeSource::Parameter => Confidence::High,
            TypeSource::Return => Confidence::Medium,
            TypeSource::Inferred => Confidence::Low,
        }
    }
}

// =============================================================================
// Section 7.3: ResolvedType
// =============================================================================

/// A resolved type for a variable.
///
/// Contains the type name, optional module, confidence level, and source.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedType {
    /// The type name (e.g., "User", "Admin")
    pub type_name: String,

    /// Module where the type is defined (for cross-file resolution)
    pub module: Option<String>,

    /// Confidence in this type resolution (0.0-1.0)
    pub confidence: f32,

    /// How the type was determined
    pub source: TypeSource,
}

impl ResolvedType {
    /// Creates a new ResolvedType with high confidence.
    pub fn high(type_name: impl Into<String>, source: TypeSource) -> Self {
        Self {
            type_name: type_name.into(),
            module: None,
            confidence: 1.0,
            source,
        }
    }

    /// Creates a new ResolvedType with medium confidence.
    pub fn medium(type_name: impl Into<String>, source: TypeSource) -> Self {
        Self {
            type_name: type_name.into(),
            module: None,
            confidence: 0.7,
            source,
        }
    }

    /// Creates a new ResolvedType with low confidence.
    pub fn low(type_name: impl Into<String>, source: TypeSource) -> Self {
        Self {
            type_name: type_name.into(),
            module: None,
            confidence: 0.3,
            source,
        }
    }

    /// Creates an unknown type (fallback).
    pub fn unknown() -> Self {
        Self {
            type_name: "Unknown".to_string(),
            module: None,
            confidence: 0.0,
            source: TypeSource::Inferred,
        }
    }

    /// Sets the module for this type.
    pub fn with_module(mut self, module: impl Into<String>) -> Self {
        self.module = Some(module.into());
        self
    }

    /// Returns the Confidence enum value based on the float confidence.
    pub fn confidence_level(&self) -> Confidence {
        if self.confidence >= 0.9 {
            Confidence::High
        } else if self.confidence >= 0.5 {
            Confidence::Medium
        } else {
            Confidence::Low
        }
    }

    /// Returns the qualified name including module if present.
    pub fn qualified_name(&self) -> String {
        match &self.module {
            Some(m) => format!("{}.{}", m, self.type_name),
            None => self.type_name.clone(),
        }
    }
}

// =============================================================================
// Section 7.4: ResolvedCall
// =============================================================================

/// A method call resolved to qualified target(s).
///
/// Contains the original call expression, resolved targets, and confidence.
/// Multiple targets are possible for union types.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedCall {
    /// The original call expression (e.g., "obj.save")
    pub original_name: String,

    /// Fully-qualified method names (e.g., {"User.save"})
    /// Multiple targets for union types (e.g., {"User.save", "Admin.save"})
    pub resolved_targets: HashSet<String>,

    /// Confidence level of the resolution
    pub confidence: Confidence,

    /// String representation of the receiver type (for debugging)
    pub receiver_type: Option<String>,
}

impl ResolvedCall {
    /// Creates a new ResolvedCall with a single target.
    pub fn single(
        original: impl Into<String>,
        target: impl Into<String>,
        confidence: Confidence,
        receiver_type: Option<String>,
    ) -> Self {
        let mut targets = HashSet::new();
        targets.insert(target.into());
        Self {
            original_name: original.into(),
            resolved_targets: targets,
            confidence,
            receiver_type,
        }
    }

    /// Creates a new ResolvedCall with multiple targets (union type).
    pub fn multiple(
        original: impl Into<String>,
        targets: impl IntoIterator<Item = impl Into<String>>,
        receiver_type: Option<String>,
    ) -> Self {
        let resolved_targets: HashSet<String> = targets.into_iter().map(Into::into).collect();
        Self {
            original_name: original.into(),
            resolved_targets,
            confidence: Confidence::Medium, // Union types are Medium confidence
            receiver_type,
        }
    }

    /// Creates a fallback ResolvedCall when type is unknown.
    pub fn unknown(original: impl Into<String>) -> Self {
        let original = original.into();
        let mut targets = HashSet::new();
        targets.insert(original.clone());
        Self {
            original_name: original,
            resolved_targets: targets,
            confidence: Confidence::Low,
            receiver_type: None,
        }
    }

    /// Returns true if this resolution has high confidence.
    pub fn is_high_confidence(&self) -> bool {
        self.confidence == Confidence::High
    }

    /// Returns the primary target (first in iteration order, for single-target calls).
    pub fn primary_target(&self) -> Option<&str> {
        self.resolved_targets.iter().next().map(|s| s.as_str())
    }
}

// =============================================================================
// Section 7.5: TypeAwareCallResolver
// =============================================================================

/// Maximum number of targets for union type expansion.
/// Beyond this limit, we return Medium confidence with truncated targets.
pub const MAX_UNION_EXPANSION: usize = 10;

/// Type-aware call resolver for OOP languages.
///
/// Resolves method calls like `obj.method()` by tracking object types
/// through assignments and inferring the target method.
///
/// # Thread Safety
///
/// This struct is NOT thread-safe. For concurrent use, wrap in a Mutex
/// or create per-thread instances.
pub struct TypeAwareCallResolver<'a> {
    /// Module index for file path resolution (stored for future cross-file resolution)
    _index: &'a ModuleIndex,

    /// Function index: (module, func) -> file path (stored for future cross-file resolution)
    _func_index: &'a HashMap<(String, String), PathBuf>,

    /// Class index: (module, class) -> file path (stored for future cross-file resolution)
    _class_index: &'a HashMap<(String, String), PathBuf>,

    /// Variable types: (file, func, var) -> ResolvedType
    /// Tracks the most recently assigned type for each variable in each function
    var_types: HashMap<(PathBuf, String, String), ResolvedType>,

    /// Class definitions by name for inheritance lookup
    class_defs: HashMap<String, ClassDef>,

    /// File IR cache for looking up class methods
    file_ir_cache: HashMap<PathBuf, FileIR>,
}

impl<'a> TypeAwareCallResolver<'a> {
    /// Creates a new TypeAwareCallResolver.
    ///
    /// # Arguments
    ///
    /// * `index` - Module index for file path resolution
    /// * `func_index` - Function index mapping (module, func) to file paths
    /// * `class_index` - Class index mapping (module, class) to file paths
    pub fn new(
        index: &'a ModuleIndex,
        func_index: &'a HashMap<(String, String), PathBuf>,
        class_index: &'a HashMap<(String, String), PathBuf>,
    ) -> Self {
        Self {
            _index: index,
            _func_index: func_index,
            _class_index: class_index,
            var_types: HashMap::new(),
            class_defs: HashMap::new(),
            file_ir_cache: HashMap::new(),
        }
    }

    /// Adds a class definition for inheritance resolution.
    pub fn add_class_def(&mut self, class_def: ClassDef) {
        self.class_defs.insert(class_def.name.clone(), class_def);
    }

    /// Adds a FileIR to the cache for method lookup.
    pub fn add_file_ir(&mut self, file: PathBuf, ir: FileIR) {
        // Also extract class definitions from the FileIR
        for class in &ir.classes {
            self.class_defs.insert(class.name.clone(), class.clone());
        }
        self.file_ir_cache.insert(file, ir);
    }

    /// Track type from assignment (e.g., `x = SomeClass()`).
    ///
    /// # Arguments
    ///
    /// * `file` - File path where the assignment occurs
    /// * `func` - Function name containing the assignment
    /// * `var` - Variable being assigned
    /// * `type_name` - Type name being assigned
    /// * `module` - Optional module where the type is defined
    pub fn track_assignment(
        &mut self,
        file: &Path,
        func: &str,
        var: &str,
        type_name: &str,
        module: Option<&str>,
    ) {
        let resolved = ResolvedType::high(type_name, TypeSource::Assignment);
        let resolved = match module {
            Some(m) => resolved.with_module(m),
            None => resolved,
        };
        self.var_types.insert(
            (file.to_path_buf(), func.to_string(), var.to_string()),
            resolved,
        );
    }

    /// Track type from annotation (e.g., `x: SomeClass`).
    ///
    /// # Arguments
    ///
    /// * `file` - File path where the annotation occurs
    /// * `func` - Function name containing the annotation
    /// * `var` - Variable being annotated
    /// * `type_name` - Type name from annotation
    pub fn track_annotation(&mut self, file: &Path, func: &str, var: &str, type_name: &str) {
        let key = (file.to_path_buf(), func.to_string(), var.to_string());

        // If we already have an assignment, keep it (higher confidence than annotation alone)
        if let Some(existing) = self.var_types.get(&key) {
            if existing.source == TypeSource::Assignment {
                return;
            }
        }

        let resolved = ResolvedType::high(type_name, TypeSource::TypeAnnotation);
        self.var_types.insert(key, resolved);
    }

    /// Track type from parameter (e.g., `def f(x: SomeClass)`).
    ///
    /// # Arguments
    ///
    /// * `file` - File path where the function is defined
    /// * `func` - Function name
    /// * `param` - Parameter name
    /// * `type_name` - Type name from parameter annotation
    pub fn track_parameter(&mut self, file: &Path, func: &str, param: &str, type_name: &str) {
        let resolved = ResolvedType::high(type_name, TypeSource::Parameter);
        self.var_types.insert(
            (file.to_path_buf(), func.to_string(), param.to_string()),
            resolved,
        );
    }

    /// Track type from function return (e.g., `x = func()` where func returns SomeClass).
    ///
    /// # Arguments
    ///
    /// * `file` - File path where the assignment occurs
    /// * `func` - Function name containing the assignment
    /// * `var` - Variable being assigned
    /// * `type_name` - Return type of the called function
    pub fn track_return(&mut self, file: &Path, func: &str, var: &str, type_name: &str) {
        let resolved = ResolvedType::medium(type_name, TypeSource::Return);
        self.var_types.insert(
            (file.to_path_buf(), func.to_string(), var.to_string()),
            resolved,
        );
    }

    /// Look up the type of a variable.
    ///
    /// # Arguments
    ///
    /// * `file` - File path
    /// * `func` - Function name
    /// * `var` - Variable name
    ///
    /// # Returns
    ///
    /// The resolved type if known, or None if unknown.
    pub fn lookup_type(&self, file: &Path, func: &str, var: &str) -> Option<&ResolvedType> {
        self.var_types
            .get(&(file.to_path_buf(), func.to_string(), var.to_string()))
    }

    /// Resolve a method call to its target(s).
    ///
    /// Given `receiver.method()`, determines which class defines `method`
    /// based on the type of `receiver`.
    ///
    /// # Arguments
    ///
    /// * `file` - File containing the call
    /// * `func` - Function making the call
    /// * `receiver` - Variable name of the receiver (e.g., "obj")
    /// * `method` - Method name being called (e.g., "save")
    /// * `line` - Line number of the call (for better diagnostics)
    ///
    /// # Returns
    ///
    /// A `ResolvedCall` with the resolved target(s) and confidence.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // For: user.save() where user = User()
    /// let result = resolver.resolve_method_call(
    ///     Path::new("main.py"), "process", "user", "save", 10
    /// );
    /// assert!(result.resolved_targets.contains("User.save"));
    /// assert_eq!(result.confidence, Confidence::High);
    /// ```
    pub fn resolve_method_call(
        &self,
        file: &Path,
        func: &str,
        receiver: &str,
        method: &str,
        _line: u32,
    ) -> ResolvedCall {
        let original = format!("{}.{}", receiver, method);

        // Special case: "self" resolves to the enclosing class
        if receiver == "self" {
            if let Some(class_name) = self.find_enclosing_class(file, func) {
                let target = format!("{}.{}", class_name, method);

                // Try to find the method in the class or its bases
                if let Some(defining_class) =
                    self.find_method_in_class_or_bases(&class_name, method)
                {
                    return ResolvedCall::single(
                        &original,
                        format!("{}.{}", defining_class, method),
                        Confidence::High,
                        Some(class_name),
                    );
                }

                // Method not found but we know the class
                return ResolvedCall::single(&original, target, Confidence::High, Some(class_name));
            }
            // self but no enclosing class found
            return ResolvedCall::unknown(&original);
        }

        // Look up the receiver's type
        let resolved_type = match self.lookup_type(file, func, receiver) {
            Some(t) => t,
            None => return ResolvedCall::unknown(&original),
        };

        // Check if this is a union type (contains |)
        if resolved_type.type_name.contains(" | ") || resolved_type.type_name.contains('|') {
            return self.resolve_union_method_call(&original, &resolved_type.type_name, method);
        }

        let type_name = &resolved_type.type_name;
        let confidence = resolved_type.confidence_level();

        // Try to find the method in the class or its bases
        if let Some(defining_class) = self.find_method_in_class_or_bases(type_name, method) {
            return ResolvedCall::single(
                &original,
                format!("{}.{}", defining_class, method),
                confidence,
                Some(type_name.clone()),
            );
        }

        // Type is known but method location unknown - use the type name
        ResolvedCall::single(
            &original,
            format!("{}.{}", type_name, method),
            confidence,
            Some(type_name.clone()),
        )
    }

    /// Resolve a chained call like `a.b().c()`.
    ///
    /// Walks the chain, tracking the type at each step using return types.
    ///
    /// # Arguments
    ///
    /// * `file` - File containing the call
    /// * `func` - Function making the call
    /// * `chain` - Chain elements (e.g., ["a", "b", "c"])
    /// * `line` - Line number of the call
    ///
    /// # Returns
    ///
    /// A vector of `ResolvedCall` for each step in the chain.
    /// The last element is the final call resolution.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // For: builder.with_name("x").build().save()
    /// let results = resolver.resolve_call_chain(
    ///     Path::new("main.py"),
    ///     "process",
    ///     &["builder", "with_name", "build", "save"],
    ///     10,
    /// );
    /// // results[-1] contains the resolution for save()
    /// ```
    pub fn resolve_call_chain(
        &self,
        file: &Path,
        func: &str,
        chain: &[&str],
        _line: u32,
    ) -> Vec<ResolvedCall> {
        if chain.is_empty() {
            return vec![];
        }

        if chain.len() == 1 {
            // Single element, not really a chain
            return vec![ResolvedCall::unknown(chain[0])];
        }

        let mut results = Vec::with_capacity(chain.len() - 1);
        let receiver = chain[0];

        // Get the initial type
        let mut current_type: Option<String> = self
            .lookup_type(file, func, receiver)
            .map(|t| t.type_name.clone());

        // Walk the chain starting from the first method call
        for i in 1..chain.len() {
            let method = chain[i];
            let original = format!("{}.{}", chain[i - 1], method);

            let type_name_owned = current_type.clone();
            let resolved = match type_name_owned {
                Some(ref type_name) => {
                    // Try to find the method and its return type
                    if let Some(defining_class) =
                        self.find_method_in_class_or_bases(type_name, method)
                    {
                        let target = format!("{}.{}", defining_class, method);

                        // Try to get the return type for the next iteration
                        current_type = self.get_method_return_type(&defining_class, method);

                        ResolvedCall::single(
                            &original,
                            target,
                            Confidence::High,
                            Some(type_name.clone()),
                        )
                    } else {
                        // Method not found but we know the type
                        let target = format!("{}.{}", type_name, method);

                        // Lost type info for next step
                        current_type = None;

                        ResolvedCall::single(
                            &original,
                            target,
                            Confidence::Medium,
                            Some(type_name.clone()),
                        )
                    }
                }
                None => {
                    // Type is unknown - return low confidence
                    current_type = None;
                    ResolvedCall::unknown(&original)
                }
            };

            results.push(resolved);
        }

        results
    }

    /// Clear all tracked types (useful for re-analysis).
    pub fn clear(&mut self) {
        self.var_types.clear();
    }

    // -------------------------------------------------------------------------
    // Private Helper Methods
    // -------------------------------------------------------------------------

    /// Find the enclosing class for a method (using function name pattern).
    fn find_enclosing_class(&self, file: &Path, func: &str) -> Option<String> {
        // Check if the function is a method by looking at FileIR
        if let Some(ir) = self.file_ir_cache.get(file) {
            for func_def in &ir.funcs {
                if func_def.name == func && func_def.is_method {
                    return func_def.class_name.clone();
                }
            }
        }

        // Also check class_defs for classes that have this method
        for (class_name, class_def) in &self.class_defs {
            if class_def.has_method(func) {
                return Some(class_name.clone());
            }
        }

        None
    }

    /// Find which class defines a method, checking bases via MRO.
    ///
    /// Uses a visited set internally to prevent infinite recursion on
    /// circular inheritance (e.g., A extends B, B extends A).
    fn find_method_in_class_or_bases(&self, class_name: &str, method: &str) -> Option<String> {
        let mut visited = HashSet::new();
        self.find_method_in_class_or_bases_inner(class_name, method, &mut visited)
    }

    /// Inner recursive helper with visited set for cycle detection.
    fn find_method_in_class_or_bases_inner(
        &self,
        class_name: &str,
        method: &str,
        visited: &mut HashSet<String>,
    ) -> Option<String> {
        // Guard against circular inheritance
        if !visited.insert(class_name.to_string()) {
            return None;
        }

        // Check if the class directly has the method
        if let Some(class_def) = self.class_defs.get(class_name) {
            if class_def.has_method(method) {
                return Some(class_name.to_string());
            }

            // Check base classes (simplified MRO - depth-first)
            let bases: Vec<String> = class_def.bases.clone();
            for base in &bases {
                if let Some(defining_class) =
                    self.find_method_in_class_or_bases_inner(base, method, visited)
                {
                    return Some(defining_class);
                }
            }
        }

        // Class not found in our definitions
        None
    }

    /// Get the return type of a method (if known).
    fn get_method_return_type(&self, class_name: &str, method: &str) -> Option<String> {
        // Look for the function definition in our caches
        for ir in self.file_ir_cache.values() {
            for func_def in &ir.funcs {
                if func_def.class_name.as_deref() == Some(class_name) && func_def.name == method {
                    return func_def.return_type.clone();
                }
            }
        }
        None
    }

    /// Resolve a method call on a union type.
    fn resolve_union_method_call(
        &self,
        original: &str,
        union_type: &str,
        method: &str,
    ) -> ResolvedCall {
        // Parse union type: "User | Admin" or "User|Admin"
        let type_names: Vec<&str> = union_type
            .split('|')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();

        if type_names.is_empty() {
            return ResolvedCall::unknown(original);
        }

        // Resolve method for each type in the union
        let mut targets = HashSet::new();
        let mut type_strs = Vec::new();

        for type_name in &type_names {
            if targets.len() >= MAX_UNION_EXPANSION {
                break;
            }

            // Filter out None type (it doesn't have methods)
            if *type_name == "None" {
                continue;
            }

            let target = if let Some(defining_class) =
                self.find_method_in_class_or_bases(type_name, method)
            {
                format!("{}.{}", defining_class, method)
            } else {
                format!("{}.{}", type_name, method)
            };

            targets.insert(target);
            type_strs.push(type_name.to_string());
        }

        if targets.is_empty() {
            return ResolvedCall::unknown(original);
        }

        ResolvedCall {
            original_name: original.to_string(),
            resolved_targets: targets,
            confidence: Confidence::Medium,
            receiver_type: Some(type_strs.join(" | ")),
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::callgraph::cross_file_types::FuncDef;

    fn create_test_resolver<'a>(
        index: &'a ModuleIndex,
        func_index: &'a HashMap<(String, String), PathBuf>,
        class_index: &'a HashMap<(String, String), PathBuf>,
    ) -> TypeAwareCallResolver<'a> {
        TypeAwareCallResolver::new(index, func_index, class_index)
    }

    #[test]
    fn test_confidence_variants_exist() {
        let high = Confidence::High;
        let medium = Confidence::Medium;
        let low = Confidence::Low;

        assert_ne!(high, medium);
        assert_ne!(medium, low);
        assert_ne!(high, low);
    }

    #[test]
    fn test_confidence_is_usable() {
        assert!(Confidence::High.is_usable());
        assert!(Confidence::Medium.is_usable());
        assert!(!Confidence::Low.is_usable());
    }

    #[test]
    fn test_confidence_combine() {
        assert_eq!(Confidence::High.combine(Confidence::High), Confidence::High);
        assert_eq!(
            Confidence::High.combine(Confidence::Medium),
            Confidence::Medium
        );
        assert_eq!(Confidence::High.combine(Confidence::Low), Confidence::Low);
        assert_eq!(Confidence::Medium.combine(Confidence::Low), Confidence::Low);
    }

    #[test]
    fn test_type_source_confidence() {
        assert_eq!(TypeSource::Assignment.confidence(), Confidence::High);
        assert_eq!(TypeSource::TypeAnnotation.confidence(), Confidence::High);
        assert_eq!(TypeSource::Parameter.confidence(), Confidence::High);
        assert_eq!(TypeSource::Return.confidence(), Confidence::Medium);
        assert_eq!(TypeSource::Inferred.confidence(), Confidence::Low);
    }

    #[test]
    fn test_resolved_type_creation() {
        let high = ResolvedType::high("User", TypeSource::Assignment);
        assert_eq!(high.type_name, "User");
        assert_eq!(high.confidence, 1.0);
        assert_eq!(high.confidence_level(), Confidence::High);

        let medium = ResolvedType::medium("User", TypeSource::Return);
        assert_eq!(medium.confidence, 0.7);
        assert_eq!(medium.confidence_level(), Confidence::Medium);

        let low = ResolvedType::low("User", TypeSource::Inferred);
        assert_eq!(low.confidence, 0.3);
        assert_eq!(low.confidence_level(), Confidence::Low);
    }

    #[test]
    fn test_resolved_type_with_module() {
        let t = ResolvedType::high("User", TypeSource::Assignment).with_module("models");
        assert_eq!(t.module, Some("models".to_string()));
        assert_eq!(t.qualified_name(), "models.User");
    }

    #[test]
    fn test_resolved_call_single() {
        let call = ResolvedCall::single(
            "obj.save",
            "User.save",
            Confidence::High,
            Some("User".to_string()),
        );
        assert_eq!(call.original_name, "obj.save");
        assert!(call.resolved_targets.contains("User.save"));
        assert_eq!(call.resolved_targets.len(), 1);
        assert_eq!(call.confidence, Confidence::High);
        assert!(call.is_high_confidence());
    }

    #[test]
    fn test_resolved_call_multiple() {
        let call = ResolvedCall::multiple(
            "obj.save",
            vec!["User.save", "Admin.save"],
            Some("User | Admin".to_string()),
        );
        assert!(call.resolved_targets.contains("User.save"));
        assert!(call.resolved_targets.contains("Admin.save"));
        assert_eq!(call.resolved_targets.len(), 2);
        assert_eq!(call.confidence, Confidence::Medium);
    }

    #[test]
    fn test_resolved_call_unknown() {
        let call = ResolvedCall::unknown("obj.method");
        assert!(call.resolved_targets.contains("obj.method"));
        assert_eq!(call.confidence, Confidence::Low);
    }

    #[test]
    fn test_track_assignment() {
        let index = ModuleIndex::new(PathBuf::new(), "python");
        let func_index = HashMap::new();
        let class_index = HashMap::new();
        let mut resolver = create_test_resolver(&index, &func_index, &class_index);

        resolver.track_assignment(Path::new("test.py"), "process", "user", "User", None);

        let resolved = resolver.lookup_type(Path::new("test.py"), "process", "user");
        assert!(resolved.is_some());
        let t = resolved.unwrap();
        assert_eq!(t.type_name, "User");
        assert_eq!(t.source, TypeSource::Assignment);
    }

    #[test]
    fn test_track_annotation() {
        let index = ModuleIndex::new(PathBuf::new(), "python");
        let func_index = HashMap::new();
        let class_index = HashMap::new();
        let mut resolver = create_test_resolver(&index, &func_index, &class_index);

        resolver.track_annotation(Path::new("test.py"), "process", "user", "User");

        let resolved = resolver.lookup_type(Path::new("test.py"), "process", "user");
        assert!(resolved.is_some());
        assert_eq!(resolved.unwrap().source, TypeSource::TypeAnnotation);
    }

    #[test]
    fn test_assignment_overrides_annotation() {
        let index = ModuleIndex::new(PathBuf::new(), "python");
        let func_index = HashMap::new();
        let class_index = HashMap::new();
        let mut resolver = create_test_resolver(&index, &func_index, &class_index);

        // First add annotation
        resolver.track_annotation(Path::new("test.py"), "process", "user", "User");

        // Then add assignment - should override since both are high confidence
        resolver.track_assignment(Path::new("test.py"), "process", "user", "Admin", None);

        let resolved = resolver.lookup_type(Path::new("test.py"), "process", "user");
        assert!(resolved.is_some());
        assert_eq!(resolved.unwrap().type_name, "Admin");
    }

    #[test]
    fn test_resolve_unknown_type() {
        let index = ModuleIndex::new(PathBuf::new(), "python");
        let func_index = HashMap::new();
        let class_index = HashMap::new();
        let resolver = create_test_resolver(&index, &func_index, &class_index);

        let result =
            resolver.resolve_method_call(Path::new("test.py"), "process", "obj", "method", 10);

        assert_eq!(result.confidence, Confidence::Low);
        assert!(result.resolved_targets.contains("obj.method"));
    }

    #[test]
    fn test_resolve_known_type() {
        let index = ModuleIndex::new(PathBuf::new(), "python");
        let func_index = HashMap::new();
        let class_index = HashMap::new();
        let mut resolver = create_test_resolver(&index, &func_index, &class_index);

        resolver.track_assignment(Path::new("test.py"), "process", "user", "User", None);

        let result =
            resolver.resolve_method_call(Path::new("test.py"), "process", "user", "save", 10);

        assert_eq!(result.confidence, Confidence::High);
        assert!(result.resolved_targets.contains("User.save"));
        assert_eq!(result.receiver_type, Some("User".to_string()));
    }

    #[test]
    fn test_resolve_union_type() {
        let index = ModuleIndex::new(PathBuf::new(), "python");
        let func_index = HashMap::new();
        let class_index = HashMap::new();
        let mut resolver = create_test_resolver(&index, &func_index, &class_index);

        resolver.var_types.insert(
            (
                PathBuf::from("test.py"),
                "process".to_string(),
                "user".to_string(),
            ),
            ResolvedType::medium("User | Admin", TypeSource::TypeAnnotation),
        );

        let result =
            resolver.resolve_method_call(Path::new("test.py"), "process", "user", "save", 10);

        assert_eq!(result.confidence, Confidence::Medium);
        assert!(result.resolved_targets.contains("User.save"));
        assert!(result.resolved_targets.contains("Admin.save"));
    }

    #[test]
    fn test_resolve_inherited_method() {
        let index = ModuleIndex::new(PathBuf::new(), "python");
        let func_index = HashMap::new();
        let class_index = HashMap::new();
        let mut resolver = create_test_resolver(&index, &func_index, &class_index);

        // Add class definitions with inheritance
        let user_class = ClassDef::new("User".to_string(), 1, 20, vec!["save".to_string()], vec![]);
        let admin_class = ClassDef::new(
            "Admin".to_string(),
            25,
            40,
            vec!["promote".to_string()], // Admin doesn't define save
            vec!["User".to_string()],    // But inherits from User
        );

        resolver.add_class_def(user_class);
        resolver.add_class_def(admin_class);

        resolver.track_assignment(Path::new("test.py"), "process", "admin", "Admin", None);

        let result =
            resolver.resolve_method_call(Path::new("test.py"), "process", "admin", "save", 30);

        // save should resolve to User.save via inheritance
        assert_eq!(result.confidence, Confidence::High);
        assert!(result.resolved_targets.contains("User.save"));
    }

    #[test]
    fn test_resolve_self_method() {
        let index = ModuleIndex::new(PathBuf::new(), "python");
        let func_index = HashMap::new();
        let class_index = HashMap::new();
        let mut resolver = create_test_resolver(&index, &func_index, &class_index);

        // Add class definition
        let user_class = ClassDef::new(
            "User".to_string(),
            1,
            30,
            vec!["save".to_string(), "validate".to_string()],
            vec![],
        );
        resolver.add_class_def(user_class.clone());

        // Add FileIR with method definition
        let mut ir = FileIR::new(PathBuf::from("test.py"));
        ir.funcs.push(FuncDef::method("save", "User", 5, 15));
        ir.funcs.push(FuncDef::method("validate", "User", 17, 25));
        ir.classes.push(user_class);
        resolver.add_file_ir(PathBuf::from("test.py"), ir);

        // Resolve self.validate() inside the save method
        let result = resolver.resolve_method_call(
            Path::new("test.py"),
            "save", // We're inside the save method
            "self",
            "validate",
            10,
        );

        assert_eq!(result.confidence, Confidence::High);
        assert!(result.resolved_targets.contains("User.validate"));
    }

    #[test]
    fn test_resolve_call_chain() {
        let index = ModuleIndex::new(PathBuf::new(), "python");
        let func_index = HashMap::new();
        let class_index = HashMap::new();
        let mut resolver = create_test_resolver(&index, &func_index, &class_index);

        // Add class definitions for the chain
        let builder_class = ClassDef::new(
            "Builder".to_string(),
            1,
            20,
            vec!["with_name".to_string(), "build".to_string()],
            vec![],
        );
        let result_class = ClassDef::new(
            "Result".to_string(),
            25,
            40,
            vec!["save".to_string()],
            vec![],
        );

        resolver.add_class_def(builder_class);
        resolver.add_class_def(result_class);

        // Add function with return type
        let mut ir = FileIR::new(PathBuf::from("test.py"));
        ir.funcs.push(FuncDef {
            name: "with_name".to_string(),
            line: 5,
            end_line: 10,
            is_method: true,
            class_name: Some("Builder".to_string()),
            return_type: Some("Builder".to_string()),
            parent_function: None,
        });
        ir.funcs.push(FuncDef {
            name: "build".to_string(),
            line: 12,
            end_line: 18,
            is_method: true,
            class_name: Some("Builder".to_string()),
            return_type: Some("Result".to_string()),
            parent_function: None,
        });
        resolver.add_file_ir(PathBuf::from("test.py"), ir);

        // Track initial variable type
        resolver.track_assignment(Path::new("test.py"), "process", "builder", "Builder", None);

        // Resolve chain: builder.with_name("test").build().save()
        let results = resolver.resolve_call_chain(
            Path::new("test.py"),
            "process",
            &["builder", "with_name", "build", "save"],
            10,
        );

        assert_eq!(results.len(), 3);

        // First call: builder.with_name() -> Builder.with_name
        assert!(results[0].resolved_targets.contains("Builder.with_name"));

        // Second call: .build() -> Builder.build
        assert!(results[1].resolved_targets.contains("Builder.build"));

        // Third call: .save() -> Result.save (via return type of build)
        assert!(results[2].resolved_targets.contains("Result.save"));
    }

    #[test]
    fn test_chain_lost_type_info() {
        let index = ModuleIndex::new(PathBuf::new(), "python");
        let func_index = HashMap::new();
        let class_index = HashMap::new();
        let mut resolver = create_test_resolver(&index, &func_index, &class_index);

        // Class without return type info
        let class_def = ClassDef::new(
            "Something".to_string(),
            1,
            20,
            vec!["get_something".to_string()],
            vec![],
        );
        resolver.add_class_def(class_def);

        // Track initial type but no return type info
        resolver.track_assignment(Path::new("test.py"), "main", "a", "Something", None);

        // Resolve chain where type info is lost
        let results = resolver.resolve_call_chain(
            Path::new("test.py"),
            "main",
            &["a", "get_something", "process"],
            10,
        );

        assert_eq!(results.len(), 2);

        // First call resolves with medium confidence (no return type)
        // Second call should have low confidence (lost type info)
        assert_eq!(results[1].confidence, Confidence::Low);
    }
}
