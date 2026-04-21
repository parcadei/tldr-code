//! Base class resolution
//!
//! Resolves whether base classes are:
//! - Project-internal (found in scanned files)
//! - Stdlib (known standard library classes)
//! - Unresolved (external packages)

use crate::types::{InheritanceGraph, Language};
use crate::TldrResult;
use std::path::Path;

/// Python stdlib classes commonly used as base classes
pub const PYTHON_STDLIB_CLASSES: &[&str] = &[
    // Built-in exceptions
    "Exception",
    "BaseException",
    "ValueError",
    "TypeError",
    "RuntimeError",
    "KeyError",
    "IndexError",
    "AttributeError",
    "NotImplementedError",
    "StopIteration",
    "GeneratorExit",
    "SystemExit",
    "KeyboardInterrupt",
    "AssertionError",
    "ImportError",
    "ModuleNotFoundError",
    "OSError",
    "IOError",
    "FileNotFoundError",
    "PermissionError",
    "TimeoutError",
    "ConnectionError",
    "BrokenPipeError",
    "ChildProcessError",
    "FileExistsError",
    "InterruptedError",
    "IsADirectoryError",
    "NotADirectoryError",
    "ProcessLookupError",
    // ABC module
    "ABC",
    "ABCMeta",
    // typing module
    "Protocol",
    "Generic",
    "TypedDict",
    "NamedTuple",
    // collections
    "UserDict",
    "UserList",
    "UserString",
    "OrderedDict",
    "Counter",
    "ChainMap",
    "defaultdict",
    "deque",
    // collections.abc
    "Mapping",
    "MutableMapping",
    "Sequence",
    "MutableSequence",
    "Set",
    "MutableSet",
    "Iterator",
    "Iterable",
    "Callable",
    "Hashable",
    "Reversible",
    "Collection",
    "Container",
    "Coroutine",
    "Awaitable",
    "AsyncIterable",
    "AsyncIterator",
    "AsyncGenerator",
    // enum
    "Enum",
    "IntEnum",
    "Flag",
    "IntFlag",
    "StrEnum",
    "auto",
    // dataclasses
    "Field",
    // contextlib
    "ContextDecorator",
    "AbstractContextManager",
    "AbstractAsyncContextManager",
    // io
    "IOBase",
    "RawIOBase",
    "BufferedIOBase",
    "TextIOBase",
    "StringIO",
    "BytesIO",
    // threading
    "Thread",
    "Timer",
    "Lock",
    "RLock",
    "Condition",
    "Semaphore",
    "Event",
    "Barrier",
    // logging
    "Handler",
    "Formatter",
    "Filter",
    "LogRecord",
    "Logger",
    "LoggerAdapter",
    // unittest
    "TestCase",
    "TestSuite",
    "TestLoader",
    "TestResult",
    // pathlib
    "Path",
    "PurePath",
    "PosixPath",
    "WindowsPath",
    // Built-in types (rarely subclassed but possible)
    "object",
    "type",
    "int",
    "float",
    "str",
    "bytes",
    "list",
    "dict",
    "set",
    "frozenset",
    "tuple",
    "bool",
    "complex",
];

/// TypeScript/JavaScript stdlib classes
pub const TYPESCRIPT_STDLIB_CLASSES: &[&str] = &[
    "Error",
    "TypeError",
    "RangeError",
    "ReferenceError",
    "SyntaxError",
    "URIError",
    "EvalError",
    "AggregateError",
    "Object",
    "Array",
    "Map",
    "Set",
    "WeakMap",
    "WeakSet",
    "Promise",
    "Date",
    "RegExp",
    "Function",
    "EventTarget",
    "Event",
    "HTMLElement",
    "Element",
    "Node",
    "Document",
    "Iterable",
    "Iterator",
    "IterableIterator",
    "AsyncIterable",
    "AsyncIterator",
    "ArrayBuffer",
    "DataView",
    "TypedArray",
    "Int8Array",
    "Uint8Array",
    "Int16Array",
    "Uint16Array",
    "Int32Array",
    "Uint32Array",
    "Float32Array",
    "Float64Array",
    "BigInt64Array",
    "BigUint64Array",
];

/// Go stdlib interfaces
pub const GO_STDLIB_CLASSES: &[&str] = &[
    "error",
    "Stringer",
    "Reader",
    "Writer",
    "Closer",
    "ReadWriter",
    "ReadCloser",
    "WriteCloser",
    "ReadWriteCloser",
    "ReaderAt",
    "WriterAt",
    "ReaderFrom",
    "WriterTo",
    "Seeker",
    "ByteReader",
    "ByteWriter",
    "ByteScanner",
    "RuneReader",
    "RuneScanner",
    "StringWriter",
    "Marshaler",
    "Unmarshaler",
    "MarshalJSON",
    "UnmarshalJSON",
    "MarshalText",
    "UnmarshalText",
    "Handler",
    "HandlerFunc",
    "ResponseWriter",
    "Flusher",
    "Hijacker",
    "Driver",
    "Conn",
    "Scanner",
    "Formatter",
    "GoStringer",
    "State",
    "ScanState",
];

/// Rust stdlib traits
pub const RUST_STDLIB_CLASSES: &[&str] = &[
    "Clone",
    "Copy",
    "Debug",
    "Display",
    "Default",
    "Eq",
    "PartialEq",
    "Ord",
    "PartialOrd",
    "Hash",
    "Send",
    "Sync",
    "Sized",
    "Unpin",
    "Drop",
    "Fn",
    "FnMut",
    "FnOnce",
    "Iterator",
    "IntoIterator",
    "ExactSizeIterator",
    "DoubleEndedIterator",
    "Extend",
    "FromIterator",
    "AsRef",
    "AsMut",
    "Borrow",
    "BorrowMut",
    "From",
    "Into",
    "TryFrom",
    "TryInto",
    "Read",
    "Write",
    "Seek",
    "BufRead",
    "Error",
    "Future",
    "Stream",
    "Deref",
    "DerefMut",
    "Index",
    "IndexMut",
    "Add",
    "Sub",
    "Mul",
    "Div",
    "Rem",
    "Neg",
    "Not",
    "BitAnd",
    "BitOr",
    "BitXor",
    "Shl",
    "Shr",
    "AddAssign",
    "SubAssign",
    "MulAssign",
    "DivAssign",
    "Serialize",
    "Deserialize",
    "ToOwned",
    "ToString",
    "Any",
];

/// Java stdlib classes commonly used as base classes/interfaces
pub const JAVA_STDLIB_CLASSES: &[&str] = &[
    // java.lang
    "Object",
    "Throwable",
    "Exception",
    "RuntimeException",
    "Error",
    "StackOverflowError",
    "OutOfMemoryError",
    "NullPointerException",
    "IllegalArgumentException",
    "IllegalStateException",
    "UnsupportedOperationException",
    "IndexOutOfBoundsException",
    "ArrayIndexOutOfBoundsException",
    "ClassCastException",
    "ClassNotFoundException",
    "IOException",
    "Thread",
    "Runnable",
    "Comparable",
    "Iterable",
    "AutoCloseable",
    "Cloneable",
    "Enum",
    "Number",
    "CharSequence",
    // java.util
    "AbstractCollection",
    "AbstractList",
    "AbstractMap",
    "AbstractSet",
    "AbstractQueue",
    "ArrayList",
    "HashMap",
    "HashSet",
    "LinkedList",
    "TreeMap",
    "TreeSet",
    "Collection",
    "List",
    "Map",
    "Set",
    "Queue",
    "Deque",
    "Iterator",
    "ListIterator",
    "Comparator",
    "EventListener",
    "EventObject",
    "Observable",
    "Observer",
    // java.io
    "Serializable",
    "InputStream",
    "OutputStream",
    "Reader",
    "Writer",
    "FilterInputStream",
    "FilterOutputStream",
    "BufferedReader",
    "BufferedWriter",
    "Closeable",
    "Flushable",
    // java.util.function
    "Function",
    "Predicate",
    "Consumer",
    "Supplier",
    "BiFunction",
    "BiPredicate",
    "BiConsumer",
    "UnaryOperator",
    "BinaryOperator",
    // java.util.concurrent
    "Callable",
    "Future",
    "Executor",
    "ExecutorService",
    "AbstractExecutorService",
    // java.util.stream
    "Stream",
    "Collector",
];

/// Check if a class name is a known stdlib class for the given language
pub fn is_stdlib_class(name: &str, lang: Language) -> bool {
    let classes = match lang {
        Language::Python => PYTHON_STDLIB_CLASSES,
        Language::TypeScript | Language::JavaScript => TYPESCRIPT_STDLIB_CLASSES,
        Language::Go => GO_STDLIB_CLASSES,
        Language::Rust => RUST_STDLIB_CLASSES,
        Language::Java => JAVA_STDLIB_CLASSES,
        _ => return false,
    };

    classes.contains(&name)
}

/// Resolve base class to determine its origin
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaseOrigin {
    /// Found in project files
    Project,
    /// Known standard library class
    Stdlib,
    /// External package (not found)
    External,
}

/// Resolve a base class
pub fn resolve_base(base_name: &str, graph: &InheritanceGraph, lang: Language) -> BaseOrigin {
    // Check if in project
    if graph.nodes.contains_key(base_name) {
        return BaseOrigin::Project;
    }

    // Check if stdlib
    if is_stdlib_class(base_name, lang) {
        return BaseOrigin::Stdlib;
    }

    // Otherwise external
    BaseOrigin::External
}

/// Resolve all base classes in the graph
pub fn resolve_all_bases(_graph: &mut InheritanceGraph, _project_root: &Path) -> TldrResult<()> {
    // Nothing to mutate on the graph itself for resolution
    // The resolution is done when building edges in the main module
    // This function exists for future enhancement like scanning external packages
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::InheritanceNode;
    use std::path::PathBuf;

    #[test]
    fn test_python_stdlib_detection() {
        assert!(is_stdlib_class("Exception", Language::Python));
        assert!(is_stdlib_class("ABC", Language::Python));
        assert!(is_stdlib_class("Protocol", Language::Python));
        assert!(is_stdlib_class("TestCase", Language::Python));
        assert!(!is_stdlib_class("MyCustomClass", Language::Python));
    }

    #[test]
    fn test_typescript_stdlib_detection() {
        assert!(is_stdlib_class("Error", Language::TypeScript));
        assert!(is_stdlib_class("HTMLElement", Language::TypeScript));
        assert!(is_stdlib_class("Promise", Language::TypeScript));
        assert!(!is_stdlib_class("MyComponent", Language::TypeScript));
    }

    #[test]
    fn test_go_stdlib_detection() {
        assert!(is_stdlib_class("error", Language::Go));
        assert!(is_stdlib_class("Reader", Language::Go));
        assert!(is_stdlib_class("Writer", Language::Go));
        assert!(!is_stdlib_class("MyService", Language::Go));
    }

    #[test]
    fn test_rust_stdlib_detection() {
        assert!(is_stdlib_class("Clone", Language::Rust));
        assert!(is_stdlib_class("Debug", Language::Rust));
        assert!(is_stdlib_class("Iterator", Language::Rust));
        assert!(is_stdlib_class("Serialize", Language::Rust));
        assert!(!is_stdlib_class("MyTrait", Language::Rust));
    }

    #[test]
    fn test_resolve_base_project() {
        let mut graph = InheritanceGraph::new();
        graph.add_node(InheritanceNode::new(
            "Animal",
            PathBuf::from("test.py"),
            1,
            Language::Python,
        ));

        let origin = resolve_base("Animal", &graph, Language::Python);
        assert_eq!(origin, BaseOrigin::Project);
    }

    #[test]
    fn test_resolve_base_stdlib() {
        let graph = InheritanceGraph::new();

        let origin = resolve_base("Exception", &graph, Language::Python);
        assert_eq!(origin, BaseOrigin::Stdlib);
    }

    #[test]
    fn test_resolve_base_external() {
        let graph = InheritanceGraph::new();

        let origin = resolve_base("FlaskView", &graph, Language::Python);
        assert_eq!(origin, BaseOrigin::External);
    }
}
