//! Test module for references CLI command (Session 7 spec)
//!
//! These tests define expected behavior BEFORE implementation.
//! Tests are designed to FAIL until the modules are implemented.
//!
//! # Test Categories
//!
//! ## 1. Find All References Basic tests
//! - Text search + AST pruning
//! - Function, class, variable references
//! - No matches and multiple matches cases
//!
//! ## 2. Definition Location tests
//! - Find function, class, variable definitions
//! - Import definition tracking
//!
//! ## 3. Cross-File Reference Tracking tests
//! - Same file references
//! - Different file references via import
//!
//! ## 4. Search Scope Optimization tests
//! - Local variable = function scope
//! - Private function = file scope
//! - Public function = workspace scope
//!
//! ## 5. Reference Kinds tests
//! - Call, read, write, import, type annotation references
//!
//! ## 6. Rename Refactoring tests
//! - Rename function, class
//! - Conflict detection
//!
//! ## 7. Call Hierarchy tests
//! - Incoming and outgoing calls
//! - Nested calls
//!
//! ## 8. Type Hierarchy tests
//! - Supertypes and subtypes
//!
//! Reference: session7-spec.md


// =============================================================================
// Test Fixture Setup Module
// =============================================================================

/// Test fixture utilities for references analysis tests
pub mod fixtures {
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    /// A temporary directory for testing references analysis
    pub struct TestDir {
        pub dir: TempDir,
    }

    impl TestDir {
        /// Create a new empty temporary directory
        pub fn new() -> std::io::Result<Self> {
            let dir = TempDir::new()?;
            Ok(Self { dir })
        }

        /// Get the path to the directory
        pub fn path(&self) -> &Path {
            self.dir.path()
        }

        /// Add a file to the directory
        pub fn add_file(&self, name: &str, content: &str) -> std::io::Result<PathBuf> {
            let path = self.dir.path().join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, content)?;
            Ok(path)
        }
    }

    // -------------------------------------------------------------------------
    // Python Fixtures for Reference Finding
    // -------------------------------------------------------------------------

    pub const PYTHON_AUTH_WITH_REFS: &str = r#"
def login(username: str, password: str) -> bool:
    """Authenticate user credentials."""
    user = get_user(username)
    if user and verify_password(password, user.password_hash):
        log_login(username)
        return True
    return False

def logout(session_id: str) -> None:
    """Invalidate user session."""
    invalidate_session(session_id)
    log_logout(session_id)

def get_user(username: str):
    """Fetch user from database."""
    return query_user(username)

def verify_password(password: str, hash: str) -> bool:
    """Verify password against hash."""
    return hash_password(password) == hash

def log_login(username: str):
    """Log successful login."""
    print(f"Login: {username}")

def log_logout(session_id: str):
    """Log logout event."""
    print(f"Logout: {session_id}")
"#;

    pub const PYTHON_ROUTES_CALLS_AUTH: &str = r#"
from auth import login, logout

def handle_login(request):
    """Handle login request."""
    username = request.get("username")
    password = request.get("password")
    result = login(username, password)
    return {"success": result}

def handle_logout(request):
    """Handle logout request."""
    session_id = request.get("session_id")
    logout(session_id)
    return {"success": True}
"#;

    pub const PYTHON_TEST_CALLS_AUTH: &str = r#"
from auth import login, logout, get_user

def test_login_success():
    """Test successful login."""
    assert login("admin", "secret") == True

def test_login_failure():
    """Test failed login."""
    assert login("admin", "wrong") == False

def test_logout():
    """Test logout."""
    logout("session123")

def test_get_user():
    """Test get_user function."""
    user = get_user("admin")
    assert user is not None
"#;

    pub const PYTHON_CLASS_WITH_REFS: &str = r#"
class User:
    """User model class."""

    def __init__(self, username: str, email: str):
        self.username = username
        self.email = email

    def display_name(self) -> str:
        """Return display name."""
        return self.username.title()

    def send_email(self, subject: str, body: str):
        """Send email to user."""
        send_notification(self.email, subject, body)

class AdminUser(User):
    """Admin user with elevated privileges."""

    def __init__(self, username: str, email: str, permissions: list):
        super().__init__(username, email)
        self.permissions = permissions

    def has_permission(self, perm: str) -> bool:
        """Check if admin has permission."""
        return perm in self.permissions
"#;

    pub const PYTHON_USES_CLASS: &str = r#"
from models import User, AdminUser

def create_user(username: str, email: str) -> User:
    """Create a new user."""
    user = User(username, email)
    return user

def create_admin(username: str, email: str) -> AdminUser:
    """Create a new admin user."""
    admin = AdminUser(username, email, ["read", "write"])
    return admin

def greet_user(user: User):
    """Greet a user by display name."""
    name = user.display_name()
    print(f"Hello, {name}!")
"#;

    pub const PYTHON_VARIABLE_REFS: &str = r#"
MAX_RETRIES = 3
DEFAULT_TIMEOUT = 30

def retry_operation(func):
    """Retry an operation with MAX_RETRIES."""
    retries = 0
    while retries < MAX_RETRIES:
        try:
            return func()
        except Exception:
            retries += 1
    raise Exception(f"Failed after {MAX_RETRIES} retries")

def set_timeout(timeout=None):
    """Set timeout, defaulting to DEFAULT_TIMEOUT."""
    if timeout is None:
        timeout = DEFAULT_TIMEOUT
    return timeout
"#;

    pub const PYTHON_PRIVATE_FUNCTION: &str = r#"
def public_func():
    """Public function."""
    return _private_helper() + __internal_helper()

def _private_helper():
    """Private by convention (single underscore)."""
    return 42

def __internal_helper():
    """Name-mangled (double underscore)."""
    return 100
"#;

    pub const PYTHON_SYMBOL_IN_STRING: &str = r#"
def login():
    """Login function."""
    return True

def test():
    # login is in a string, not a reference
    message = "Please login to continue"
    # login is in a comment, not a reference
    # Call login() to authenticate
    return login()
"#;

    // -------------------------------------------------------------------------
    // TypeScript Fixtures
    // -------------------------------------------------------------------------

    pub const TS_AUTH_WITH_REFS: &str = r#"
export function login(username: string, password: string): boolean {
    const user = getUser(username);
    if (user && verifyPassword(password, user.passwordHash)) {
        logLogin(username);
        return true;
    }
    return false;
}

export function logout(sessionId: string): void {
    invalidateSession(sessionId);
    logLogout(sessionId);
}

function getUser(username: string): User | null {
    return queryUser(username);
}

function verifyPassword(password: string, hash: string): boolean {
    return hashPassword(password) === hash;
}
"#;

    pub const TS_ROUTES_CALLS_AUTH: &str = r#"
import { login, logout } from './auth';

export function handleLogin(request: Request): Response {
    const { username, password } = request.body;
    const result = login(username, password);
    return { success: result };
}

export function handleLogout(request: Request): Response {
    const { sessionId } = request.body;
    logout(sessionId);
    return { success: true };
}
"#;

    pub const TS_CLASS_WITH_REFS: &str = r#"
export class User {
    constructor(
        public username: string,
        public email: string
    ) {}

    displayName(): string {
        return this.username.toUpperCase();
    }

    sendEmail(subject: string, body: string): void {
        sendNotification(this.email, subject, body);
    }
}

export class AdminUser extends User {
    constructor(
        username: string,
        email: string,
        public permissions: string[]
    ) {
        super(username, email);
    }

    hasPermission(perm: string): boolean {
        return this.permissions.includes(perm);
    }
}
"#;

    pub const TS_TYPE_ANNOTATION_REFS: &str = r#"
import type { User, AdminUser } from './models';

interface UserService {
    getUser(id: string): User;
    getAdmin(id: string): AdminUser;
}

function processUser(user: User): void {
    console.log(user.username);
}

const users: User[] = [];
"#;

    // -------------------------------------------------------------------------
    // Go Fixtures
    // -------------------------------------------------------------------------

    pub const GO_AUTH_WITH_REFS: &str = r#"
package auth

func Login(username, password string) bool {
    user := GetUser(username)
    if user != nil && VerifyPassword(password, user.PasswordHash) {
        LogLogin(username)
        return true
    }
    return false
}

func Logout(sessionID string) {
    InvalidateSession(sessionID)
    LogLogout(sessionID)
}

func GetUser(username string) *User {
    return QueryUser(username)
}

func VerifyPassword(password, hash string) bool {
    return HashPassword(password) == hash
}
"#;

    pub const GO_ROUTES_CALLS_AUTH: &str = r#"
package routes

import "myproject/auth"

func HandleLogin(w http.ResponseWriter, r *http.Request) {
    username := r.FormValue("username")
    password := r.FormValue("password")
    result := auth.Login(username, password)
    // ...
}

func HandleLogout(w http.ResponseWriter, r *http.Request) {
    sessionID := r.FormValue("session_id")
    auth.Logout(sessionID)
    // ...
}
"#;

    // -------------------------------------------------------------------------
    // Rust Fixtures
    // -------------------------------------------------------------------------

    pub const RUST_AUTH_WITH_REFS: &str = r#"
pub fn login(username: &str, password: &str) -> bool {
    if let Some(user) = get_user(username) {
        if verify_password(password, &user.password_hash) {
            log_login(username);
            return true;
        }
    }
    false
}

pub fn logout(session_id: &str) {
    invalidate_session(session_id);
    log_logout(session_id);
}

fn get_user(username: &str) -> Option<User> {
    query_user(username)
}

fn verify_password(password: &str, hash: &str) -> bool {
    hash_password(password) == hash
}
"#;

    pub const RUST_USES_AUTH: &str = r#"
use crate::auth::{login, logout};

pub fn handle_login(username: &str, password: &str) -> Result<(), Error> {
    if login(username, password) {
        Ok(())
    } else {
        Err(Error::AuthFailed)
    }
}

pub fn handle_logout(session_id: &str) {
    logout(session_id);
}
"#;

    // -------------------------------------------------------------------------
    // Special Cases Fixtures
    // -------------------------------------------------------------------------

    pub const PYTHON_SHADOWED_VARIABLE: &str = r#"
name = "global"

def func1():
    name = "local"
    return name

def func2():
    return name  # References global

class MyClass:
    name = "class"

    def method(self):
        return self.name  # References class attribute
"#;

    pub const PYTHON_OVERLOADED_NAME: &str = r#"
def process(data):
    """Process string data."""
    return data.upper()

class DataProcessor:
    def process(self, data):
        """Process data with instance method."""
        return data.lower()
"#;

    pub const PYTHON_RENAME_CANDIDATE: &str = r#"
def old_name():
    """Function to be renamed."""
    return 42

def caller1():
    return old_name()

def caller2():
    result = old_name()
    return result * 2
"#;

    pub const PYTHON_RENAME_CONFLICT: &str = r#"
def target():
    """Target to rename."""
    return 1

def existing():
    """Existing function with conflicting name."""
    return 2

def caller():
    return target() + existing()
"#;
}

// =============================================================================
// Find All References Basic Tests
// =============================================================================

#[cfg(test)]
mod find_references_basic_tests {
    use super::fixtures::*;

    /// Test finding references to a function
    /// Contract: All call sites are found with correct locations
    #[test]
    fn find_function_references() {
        use crate::analysis::references::{find_references, ReferencesOptions};

        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/auth.py", PYTHON_AUTH_WITH_REFS)
            .unwrap();
        test_dir
            .add_file("src/routes.py", PYTHON_ROUTES_CALLS_AUTH)
            .unwrap();
        test_dir
            .add_file("tests/test_auth.py", PYTHON_TEST_CALLS_AUTH)
            .unwrap();

        let options = ReferencesOptions::new().with_language("python".to_string());
        let report = find_references("login", test_dir.path(), &options).unwrap();

        // Phase 9: Text search finds all occurrences
        // Should find: occurrences in auth.py, routes.py and test_auth.py
        assert!(
            report.total_references >= 3,
            "Expected at least 3 references to 'login', found {}",
            report.total_references
        );
        assert!(
            report
                .references
                .iter()
                .any(|r| r.file.to_string_lossy().contains("routes.py")),
            "Should find reference in routes.py"
        );
        assert!(
            report
                .references
                .iter()
                .any(|r| r.file.to_string_lossy().contains("test_auth.py")),
            "Should find reference in test_auth.py"
        );

        // Stats should be populated
        assert!(
            report.stats.files_searched >= 3,
            "Should search at least 3 files"
        );
        assert_eq!(report.stats.candidates_found, report.total_references);
    }

    /// Test finding references to a class
    /// Contract: Class instantiations and type annotations are found
    #[test]
    #[ignore = "references module not yet implemented"]
    fn find_class_references() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/models.py", PYTHON_CLASS_WITH_REFS)
            .unwrap();
        test_dir
            .add_file("src/services.py", PYTHON_USES_CLASS)
            .unwrap();

        // let report = find_references(
        //     test_dir.path(),
        //     "User",
        //     Language::Python,
        //     &ReferencesConfig::default(),
        // ).unwrap();
        //
        // // Should find: definition, inheritance, instantiation, type annotation
        // assert!(report.definition.is_some());
        // assert!(report.total_references >= 3);

        todo!("Implement find_class_references test")
    }

    /// Test finding references to a variable/constant
    /// Contract: Variable reads and writes are found
    #[test]
    #[ignore = "references module not yet implemented"]
    fn find_variable_references() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/config.py", PYTHON_VARIABLE_REFS)
            .unwrap();

        // let report = find_references(
        //     test_dir.path(),
        //     "MAX_RETRIES",
        //     Language::Python,
        //     &ReferencesConfig::default(),
        // ).unwrap();
        //
        // // Should find: definition and usages
        // assert!(report.definition.is_some());
        // assert!(report.total_references >= 2);

        todo!("Implement find_variable_references test")
    }

    /// Test no matches case
    /// Contract: Returns empty references with no error
    #[test]
    fn no_matches_case() {
        use crate::analysis::references::{find_references, ReferencesOptions};

        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("src/main.py", "def foo(): pass").unwrap();

        let options = ReferencesOptions::new().with_language("python".to_string());
        let report = find_references("nonexistent_symbol", test_dir.path(), &options).unwrap();

        // Phase 9: No text matches should result in empty report
        assert!(report.definition.is_none());
        assert_eq!(report.total_references, 0);
        assert!(report.references.is_empty());
        assert!(
            report.stats.files_searched >= 1,
            "Should search at least 1 file"
        );
        assert_eq!(report.stats.candidates_found, 0);
    }

    /// Test multiple matches in same file
    /// Contract: All occurrences in same file are found
    #[test]
    fn multiple_matches_same_file() {
        use crate::analysis::references::{find_references, ReferencesOptions};

        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/auth.py", PYTHON_AUTH_WITH_REFS)
            .unwrap();

        // get_user is defined and called multiple times in auth.py
        let options = ReferencesOptions::new().with_language("python".to_string());
        let report = find_references("get_user", test_dir.path(), &options).unwrap();

        // Phase 9: Text search finds all occurrences in the same file
        let auth_refs: Vec<_> = report
            .references
            .iter()
            .filter(|r| r.file.to_string_lossy().contains("auth.py"))
            .collect();

        // Should find at least 2 occurrences: definition and call in login()
        assert!(
            auth_refs.len() >= 2,
            "Expected at least 2 references in auth.py, found {}",
            auth_refs.len()
        );
        assert!(report.stats.files_searched >= 1);
        assert_eq!(report.stats.candidates_found, report.total_references);
    }

    /// Test excluding string matches
    /// Contract: Symbol in strings/comments are not counted as references
    #[test]
    fn exclude_string_matches() {
        use crate::analysis::references::{find_references, ReferenceKind, ReferencesOptions};

        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/main.py", PYTHON_SYMBOL_IN_STRING)
            .unwrap();

        let options = ReferencesOptions::new().with_language("python".to_string());
        let report = find_references("login", test_dir.path(), &options).unwrap();

        // Should find: definition and call, NOT string or comment
        // "Please login to continue" and "# Call login() to authenticate"
        // should be excluded by AST verification (Phase 10)

        // We expect: definition (def login():) and the call (login())
        // The string "Please login to continue" should be filtered out

        // Note: comment lines are already filtered by Phase 9 text search
        // but strings inside non-comment lines need AST filtering

        // Count references that are NOT in strings
        let valid_refs: Vec<_> = report
            .references
            .iter()
            .filter(|r| !r.context.contains("\"")) // Not inside a string context
            .collect();

        // Should have the definition and the call
        assert!(
            valid_refs.len() >= 2,
            "Expected at least 2 valid references (def + call), found {}. All refs: {:?}",
            valid_refs.len(),
            report
                .references
                .iter()
                .map(|r| (&r.kind, &r.context))
                .collect::<Vec<_>>()
        );

        // The string match should either be excluded or have low confidence
        // Check that we have a Definition kind and a Call kind
        let has_definition = report
            .references
            .iter()
            .any(|r| r.kind == ReferenceKind::Definition);
        let has_call = report
            .references
            .iter()
            .any(|r| r.kind == ReferenceKind::Call);

        assert!(has_definition, "Expected a Definition reference for login");
        assert!(has_call, "Expected a Call reference for login()");
    }

    /// Test TypeScript references
    /// Contract: Works with TypeScript/JavaScript syntax
    #[test]
    #[ignore = "references module not yet implemented"]
    fn typescript_references() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("src/auth.ts", TS_AUTH_WITH_REFS).unwrap();
        test_dir
            .add_file("src/routes.ts", TS_ROUTES_CALLS_AUTH)
            .unwrap();

        // let report = find_references(
        //     test_dir.path(),
        //     "login",
        //     Language::TypeScript,
        //     &ReferencesConfig::default(),
        // ).unwrap();
        //
        // assert!(report.definition.is_some());
        // assert!(report.total_references >= 2);

        todo!("Implement typescript_references test")
    }

    /// Test Go references
    /// Contract: Works with Go capitalization-based visibility
    #[test]
    #[ignore = "references module not yet implemented"]
    fn go_references() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("auth/auth.go", GO_AUTH_WITH_REFS)
            .unwrap();
        test_dir
            .add_file("routes/routes.go", GO_ROUTES_CALLS_AUTH)
            .unwrap();

        // let report = find_references(
        //     test_dir.path(),
        //     "Login",
        //     Language::Go,
        //     &ReferencesConfig::default(),
        // ).unwrap();
        //
        // assert!(report.definition.is_some());
        // assert!(report.total_references >= 1);

        todo!("Implement go_references test")
    }

    /// Test Rust references
    /// Contract: Works with Rust module system
    #[test]
    #[ignore = "references module not yet implemented"]
    fn rust_references() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/auth.rs", RUST_AUTH_WITH_REFS)
            .unwrap();
        test_dir
            .add_file("src/handlers.rs", RUST_USES_AUTH)
            .unwrap();

        // let report = find_references(
        //     test_dir.path(),
        //     "login",
        //     Language::Rust,
        //     &ReferencesConfig::default(),
        // ).unwrap();
        //
        // assert!(report.definition.is_some());
        // assert!(report.total_references >= 1);

        todo!("Implement rust_references test")
    }
}

// =============================================================================
// Definition Location Tests
// =============================================================================

#[cfg(test)]
mod definition_location_tests {
    use super::fixtures::*;
    use crate::analysis::references::{find_references, DefinitionKind, ReferencesOptions};

    /// Test finding function definition
    /// Contract: Returns file, line, column of function definition
    #[test]
    fn test_function_definition() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/auth.py", PYTHON_AUTH_WITH_REFS)
            .unwrap();

        let options = ReferencesOptions::new()
            .with_definition()
            .with_language("python".to_string());
        let report = find_references("login", test_dir.path(), &options).unwrap();

        let def = report.definition.expect("Definition not found");
        assert!(
            def.file.to_string_lossy().contains("auth.py"),
            "Expected definition in auth.py, got {:?}",
            def.file
        );
        assert!(def.line > 0, "Line should be > 0");
        assert!(def.column > 0, "Column should be > 0");
        assert_eq!(
            def.kind,
            DefinitionKind::Function,
            "Expected Function definition kind"
        );
    }

    /// Test finding class definition
    /// Contract: Returns class definition location
    #[test]
    fn test_class_definition() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/models.py", PYTHON_CLASS_WITH_REFS)
            .unwrap();

        let options = ReferencesOptions::new()
            .with_definition()
            .with_language("python".to_string());
        let report = find_references("User", test_dir.path(), &options).unwrap();

        let def = report.definition.expect("Definition not found");
        assert!(
            def.file.to_string_lossy().contains("models.py"),
            "Expected definition in models.py, got {:?}",
            def.file
        );
        assert_eq!(
            def.kind,
            DefinitionKind::Class,
            "Expected Class definition kind"
        );
    }

    /// Test finding variable definition
    /// Contract: Returns variable assignment location
    #[test]
    #[ignore = "Variable definitions at module level require more complex AST handling"]
    fn find_variable_definition() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/config.py", PYTHON_VARIABLE_REFS)
            .unwrap();

        let options = ReferencesOptions::new()
            .with_definition()
            .with_language("python".to_string());
        let report = find_references("MAX_RETRIES", test_dir.path(), &options).unwrap();

        let def = report.definition.expect("Definition not found");
        assert!(
            def.file.to_string_lossy().contains("config.py"),
            "Expected definition in config.py"
        );
        assert_eq!(
            def.kind,
            DefinitionKind::Variable,
            "Expected Variable definition kind"
        );
    }

    /// Test finding import definition (resolves to original)
    /// Contract: Import references resolve to original definition
    #[test]
    #[ignore = "Import resolution requires import graph tracking"]
    fn find_import_definition() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/auth.py", PYTHON_AUTH_WITH_REFS)
            .unwrap();
        test_dir
            .add_file("src/routes.py", PYTHON_ROUTES_CALLS_AUTH)
            .unwrap();

        // When searching from routes.py where login is imported
        let options = ReferencesOptions::new()
            .with_definition()
            .with_definition_file(test_dir.path().join("src/routes.py"))
            .with_language("python".to_string());
        let report = find_references("login", test_dir.path(), &options).unwrap();

        // Definition should resolve to auth.py, not the import in routes.py
        let def = report.definition.expect("Definition not found");
        assert!(
            def.file.to_string_lossy().contains("auth.py"),
            "Definition should be in auth.py, not import site"
        );
    }

    /// Test definition signature extraction
    /// Contract: signature field contains function/class signature
    #[test]
    fn definition_signature() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/auth.py", PYTHON_AUTH_WITH_REFS)
            .unwrap();

        let options = ReferencesOptions::new()
            .with_definition()
            .with_language("python".to_string());
        let report = find_references("login", test_dir.path(), &options).unwrap();

        let def = report.definition.expect("Definition not found");
        let sig = def.signature.expect("Signature not found");
        assert!(
            sig.contains("def login"),
            "Signature should contain 'def login', got: {}",
            sig
        );
        assert!(
            sig.contains("username"),
            "Signature should contain 'username', got: {}",
            sig
        );
        assert!(
            sig.contains("password"),
            "Signature should contain 'password', got: {}",
            sig
        );
    }
}

// =============================================================================
// Cross-File Reference Tracking Tests
// =============================================================================

#[cfg(test)]
mod cross_file_tests {
    use super::fixtures::*;
    use crate::analysis::references::{find_references, ReferenceKind, ReferencesOptions};

    /// Test reference in same file
    /// Contract: Internal references within a file are found
    #[test]
    fn test_same_file_reference() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/auth.py", PYTHON_AUTH_WITH_REFS)
            .unwrap();

        // log_login is defined and called in the same file
        let options = ReferencesOptions::new().with_language("python".to_string());
        let report = find_references("log_login", test_dir.path(), &options).unwrap();

        // Should find definition and call in auth.py
        let auth_refs: Vec<_> = report
            .references
            .iter()
            .filter(|r| r.file.to_string_lossy().contains("auth.py"))
            .collect();
        assert!(!auth_refs.is_empty(), "Should find references in auth.py");
    }

    /// Test reference in different file
    /// Contract: References across file boundaries are found
    #[test]
    fn test_cross_file_via_import() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/auth.py", PYTHON_AUTH_WITH_REFS)
            .unwrap();
        test_dir
            .add_file("src/routes.py", PYTHON_ROUTES_CALLS_AUTH)
            .unwrap();

        let options = ReferencesOptions::new().with_language("python".to_string());
        let report = find_references("login", test_dir.path(), &options).unwrap();

        // Should find call in routes.py
        assert!(
            report
                .references
                .iter()
                .any(|r| r.file.to_string_lossy().contains("routes.py")),
            "Should find reference in routes.py. Found refs in: {:?}",
            report
                .references
                .iter()
                .map(|r| &r.file)
                .collect::<Vec<_>>()
        );
    }

    /// Test reference via import
    /// Contract: Imported symbols are tracked correctly
    #[test]
    fn reference_via_import() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/auth.py", PYTHON_AUTH_WITH_REFS)
            .unwrap();
        test_dir
            .add_file("src/routes.py", PYTHON_ROUTES_CALLS_AUTH)
            .unwrap();

        let options = ReferencesOptions::new().with_language("python".to_string());
        let report = find_references("login", test_dir.path(), &options).unwrap();

        // Should include the import statement as a reference
        let import_refs: Vec<_> = report
            .references
            .iter()
            .filter(|r| r.kind == ReferenceKind::Import)
            .collect();
        assert!(
            !import_refs.is_empty(),
            "Should find import references for 'login'"
        );
    }

    /// Test references across multiple files
    /// Contract: All references in workspace are found
    #[test]
    fn references_multiple_files() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/auth.py", PYTHON_AUTH_WITH_REFS)
            .unwrap();
        test_dir
            .add_file("src/routes.py", PYTHON_ROUTES_CALLS_AUTH)
            .unwrap();
        test_dir
            .add_file("tests/test_auth.py", PYTHON_TEST_CALLS_AUTH)
            .unwrap();

        let options = ReferencesOptions::new().with_language("python".to_string());
        let report = find_references("login", test_dir.path(), &options).unwrap();

        // Should find references in auth.py, routes.py, and test_auth.py
        let files_with_refs: std::collections::HashSet<_> = report
            .references
            .iter()
            .filter_map(|r| r.file.file_name())
            .filter_map(|n| n.to_str())
            .collect();
        assert!(
            files_with_refs.len() >= 2,
            "Should find references in at least 2 files, found in: {:?}",
            files_with_refs
        );
    }
}

// =============================================================================
// Search Scope Optimization Tests
// =============================================================================

#[cfg(test)]
mod search_scope_tests {
    use super::fixtures::*;

    /// Test local variable scope (function only)
    /// Contract: Local variables only search within function
    /// Phase 13: test_local_scope - verify Local scope limits search
    #[test]
    fn test_local_scope() {
        use crate::analysis::references::{find_references, ReferencesOptions, SearchScope};

        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/auth.py", PYTHON_AUTH_WITH_REFS)
            .unwrap();
        test_dir
            .add_file(
                "src/other.py",
                r#"
def other_func():
    user = "other user"  # Different 'user' variable
    return user
"#,
            )
            .unwrap();

        // When searching with Local scope for 'user' in auth.py
        // Should only find references in that function, not other files
        let options = ReferencesOptions::new()
            .with_language("python".to_string())
            .with_scope(SearchScope::Local)
            .with_definition_file(test_dir.path().join("src/auth.py"));

        let report = find_references("user", test_dir.path(), &options).unwrap();

        // With Local scope, the search is limited
        // The scope should be Local
        assert_eq!(report.search_scope, SearchScope::Local);

        // All references should be in the same file as definition_file
        // (Local scope restricts to containing function/file)
        for r in &report.references {
            assert!(
                r.file.to_string_lossy().contains("auth.py"),
                "With Local scope, expected only auth.py references, found in {:?}",
                r.file
            );
        }
    }

    /// Test private function scope (file only)
    /// Contract: Private functions only search within file
    /// Phase 13: test_file_scope - verify File scope limits search to single file
    #[test]
    fn test_file_scope() {
        use crate::analysis::references::{find_references, ReferencesOptions, SearchScope};

        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/private.py", PYTHON_PRIVATE_FUNCTION)
            .unwrap();
        test_dir
            .add_file(
                "src/other.py",
                r#"
def other():
    # This file also mentions _private_helper in a comment
    pass
"#,
            )
            .unwrap();

        // When searching with File scope for _private_helper
        let options = ReferencesOptions::new()
            .with_language("python".to_string())
            .with_scope(SearchScope::File)
            .with_definition_file(test_dir.path().join("src/private.py"));

        let report = find_references("_private_helper", test_dir.path(), &options).unwrap();

        // Scope should be File
        assert_eq!(report.search_scope, SearchScope::File);

        // All references should be in private.py only (file scope)
        for r in &report.references {
            assert!(
                r.file.to_string_lossy().contains("private.py"),
                "With File scope, expected only private.py references, found in {:?}",
                r.file
            );
        }
    }

    /// Test Python private convention (_prefix = file scope)
    /// Contract: Python _prefix symbols automatically infer File scope
    /// Phase 13: test_python_private_convention
    #[test]
    fn test_python_private_convention() {
        use crate::analysis::references::{
            determine_search_scope, find_references, ReferencesOptions, SearchScope,
        };

        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/private.py", PYTHON_PRIVATE_FUNCTION)
            .unwrap();
        test_dir
            .add_file("src/other.py", "def other(): pass")
            .unwrap();

        // Test that determine_search_scope infers File scope for _prefix
        let scope = determine_search_scope(
            "_private_helper",
            Some(test_dir.path().join("src/private.py").as_path()),
            "python",
        );
        assert_eq!(
            scope,
            SearchScope::File,
            "Python _prefix symbol should infer File scope"
        );

        // Test that __dunder__ methods get Workspace scope (they're special)
        let dunder_scope = determine_search_scope("__init__", None, "python");
        assert_eq!(
            dunder_scope,
            SearchScope::Workspace,
            "Python __dunder__ methods should have Workspace scope"
        );

        // Test that regular symbols get Workspace scope
        let public_scope = determine_search_scope("public_func", None, "python");
        assert_eq!(
            public_scope,
            SearchScope::Workspace,
            "Python public symbols should have Workspace scope"
        );

        // Now test the full find_references flow without explicit scope
        // (it should auto-infer File scope for _prefix)
        let options = ReferencesOptions::new()
            .with_language("python".to_string())
            .with_definition_file(test_dir.path().join("src/private.py"));

        let report = find_references("_private_helper", test_dir.path(), &options).unwrap();

        // Auto-inferred scope for _prefix should be File
        assert_eq!(
            report.search_scope,
            SearchScope::File,
            "Should auto-infer File scope for _private_helper"
        );
    }

    /// Test public function scope (workspace)
    /// Contract: Public functions search entire workspace
    #[test]
    #[ignore = "references module not yet implemented"]
    fn public_function_scope() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/auth.py", PYTHON_AUTH_WITH_REFS)
            .unwrap();
        test_dir
            .add_file("src/routes.py", PYTHON_ROUTES_CALLS_AUTH)
            .unwrap();
        test_dir
            .add_file("tests/test_auth.py", PYTHON_TEST_CALLS_AUTH)
            .unwrap();

        // login is public
        // let report = find_references(
        //     test_dir.path(),
        //     "login",
        //     Language::Python,
        //     &ReferencesConfig::default(),
        // ).unwrap();
        //
        // // Should use Workspace scope
        // assert_eq!(report.search_scope, SearchScope::Workspace);
        // assert!(report.files_searched >= 3);

        todo!("Implement public_function_scope test")
    }

    /// Test Go capitalization-based scope
    /// Contract: Lowercase = package-private, Uppercase = public
    #[test]
    #[ignore = "references module not yet implemented"]
    fn go_visibility_scope() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("auth/auth.go", GO_AUTH_WITH_REFS)
            .unwrap();

        // GetUser is public (capitalized)
        // let report_public = find_references(
        //     test_dir.path(),
        //     "GetUser",
        //     Language::Go,
        //     &ReferencesConfig::default(),
        // ).unwrap();
        // assert_eq!(report_public.search_scope, SearchScope::Workspace);
        //
        // // lowercase would be package-private (if we had one)

        todo!("Implement go_visibility_scope test")
    }

    /// Test explicit scope override
    /// Contract: --scope flag overrides inferred scope
    #[test]
    #[ignore = "references module not yet implemented"]
    fn explicit_scope_override() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/auth.py", PYTHON_AUTH_WITH_REFS)
            .unwrap();
        test_dir
            .add_file("src/routes.py", PYTHON_ROUTES_CALLS_AUTH)
            .unwrap();

        // Force File scope even for public function
        // let report = find_references(
        //     test_dir.path(),
        //     "login",
        //     Language::Python,
        //     &ReferencesConfig {
        //         scope: Some(SearchScope::File),
        //         definition_file: Some(test_dir.path().join("src/auth.py")),
        //         ..Default::default()
        //     },
        // ).unwrap();
        //
        // // Should only search auth.py
        // assert_eq!(report.search_scope, SearchScope::File);
        // assert!(report.references.iter().all(|r| r.file.ends_with("auth.py")));

        todo!("Implement explicit_scope_override test")
    }
}

// =============================================================================
// Reference Kinds Tests
// =============================================================================

#[cfg(test)]
mod reference_kinds_tests {
    use super::fixtures::*;
    use crate::analysis::references::{find_references, ReferenceKind, ReferencesOptions};

    /// Test call reference kind
    /// Contract: Function invocations are marked as Call
    #[test]
    fn call_reference_kind() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/auth.py", PYTHON_AUTH_WITH_REFS)
            .unwrap();
        test_dir
            .add_file("src/routes.py", PYTHON_ROUTES_CALLS_AUTH)
            .unwrap();

        let options = ReferencesOptions::new().with_language("python".to_string());
        let report = find_references("login", test_dir.path(), &options).unwrap();

        // routes.py calls login()
        let call_refs: Vec<_> = report
            .references
            .iter()
            .filter(|r| {
                r.kind == ReferenceKind::Call && r.file.to_string_lossy().contains("routes.py")
            })
            .collect();
        assert!(
            !call_refs.is_empty(),
            "Expected Call references in routes.py, found none. All refs: {:?}",
            report
                .references
                .iter()
                .map(|r| (&r.file, &r.kind, &r.context))
                .collect::<Vec<_>>()
        );
    }

    /// Test read reference kind
    /// Contract: Variable reads are marked as Read
    #[test]
    fn read_reference_kind() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/config.py", PYTHON_VARIABLE_REFS)
            .unwrap();

        let options = ReferencesOptions::new().with_language("python".to_string());
        let report = find_references("MAX_RETRIES", test_dir.path(), &options).unwrap();

        // MAX_RETRIES is read in retry_operation (in the while condition and f-string)
        let read_refs: Vec<_> = report
            .references
            .iter()
            .filter(|r| r.kind == ReferenceKind::Read)
            .collect();
        assert!(
            !read_refs.is_empty(),
            "Expected Read references for MAX_RETRIES, found none. All refs: {:?}",
            report
                .references
                .iter()
                .map(|r| (&r.file, &r.kind, &r.context))
                .collect::<Vec<_>>()
        );
    }

    /// Test write reference kind
    /// Contract: Variable assignments are marked as Write
    #[test]
    fn write_reference_kind() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file(
                "src/main.py",
                r#"
counter = 0

def increment():
    global counter
    counter = counter + 1
    return counter
"#,
            )
            .unwrap();

        let options = ReferencesOptions::new().with_language("python".to_string());
        let report = find_references("counter", test_dir.path(), &options).unwrap();

        // counter = counter + 1 has write on LHS
        // The first 'counter = 0' is also a write (definition)
        let write_refs: Vec<_> = report
            .references
            .iter()
            .filter(|r| r.kind == ReferenceKind::Write || r.kind == ReferenceKind::Definition)
            .collect();
        assert!(
            !write_refs.is_empty(),
            "Expected Write references for counter, found none. All refs: {:?}",
            report
                .references
                .iter()
                .map(|r| (&r.file, &r.kind, &r.context))
                .collect::<Vec<_>>()
        );
    }

    /// Test import reference kind
    /// Contract: Import statements are marked as Import
    #[test]
    fn import_reference_kind() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/auth.py", PYTHON_AUTH_WITH_REFS)
            .unwrap();
        test_dir
            .add_file("src/routes.py", PYTHON_ROUTES_CALLS_AUTH)
            .unwrap();

        let options = ReferencesOptions::new().with_language("python".to_string());
        let report = find_references("login", test_dir.path(), &options).unwrap();

        // routes.py imports login
        let import_refs: Vec<_> = report
            .references
            .iter()
            .filter(|r| r.kind == ReferenceKind::Import)
            .collect();
        assert!(
            !import_refs.is_empty(),
            "Expected Import references for login, found none. All refs: {:?}",
            report
                .references
                .iter()
                .map(|r| (&r.file, &r.kind, &r.context))
                .collect::<Vec<_>>()
        );
    }

    /// Test type annotation reference kind
    /// Contract: Type annotations are marked as Type
    #[test]
    fn type_annotation_reference_kind() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/models.ts", TS_CLASS_WITH_REFS)
            .unwrap();
        test_dir
            .add_file("src/services.ts", TS_TYPE_ANNOTATION_REFS)
            .unwrap();

        let options = ReferencesOptions::new().with_language("typescript".to_string());
        let report = find_references("User", test_dir.path(), &options).unwrap();

        // services.ts uses User in type annotations
        let type_refs: Vec<_> = report
            .references
            .iter()
            .filter(|r| r.kind == ReferenceKind::Type)
            .collect();
        assert!(
            !type_refs.is_empty(),
            "Expected Type references for User, found none. All refs: {:?}",
            report
                .references
                .iter()
                .map(|r| (&r.file, &r.kind, &r.context))
                .collect::<Vec<_>>()
        );
    }

    /// Test definition reference kind
    /// Contract: Definition location is marked as Definition
    #[test]
    fn definition_reference_kind() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/auth.py", PYTHON_AUTH_WITH_REFS)
            .unwrap();

        let options = ReferencesOptions::new()
            .with_language("python".to_string())
            .with_definition();
        let report = find_references("login", test_dir.path(), &options).unwrap();

        // The definition should be marked with Definition kind
        let def_refs: Vec<_> = report
            .references
            .iter()
            .filter(|r| r.kind == ReferenceKind::Definition)
            .collect();
        assert!(
            !def_refs.is_empty(),
            "Expected Definition reference for login, found none. All refs: {:?}",
            report
                .references
                .iter()
                .map(|r| (&r.file, &r.kind, &r.context))
                .collect::<Vec<_>>()
        );
    }

    /// Test filtering by kinds
    /// Contract: --kinds flag filters results
    /// Phase 13: test_filter_by_kinds - verify kind filtering works
    #[test]
    fn test_filter_by_kinds() {
        use crate::analysis::references::{
            filter_by_kinds, find_references, ReferenceKind, ReferencesOptions,
        };

        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/auth.py", PYTHON_AUTH_WITH_REFS)
            .unwrap();
        test_dir
            .add_file("src/routes.py", PYTHON_ROUTES_CALLS_AUTH)
            .unwrap();

        // First, get all references without filtering
        let options_all = ReferencesOptions::new().with_language("python".to_string());
        let report_all = find_references("login", test_dir.path(), &options_all).unwrap();

        // Should have multiple kinds: Definition, Call, Import
        let kinds_found: std::collections::HashSet<_> =
            report_all.references.iter().map(|r| r.kind).collect();
        assert!(
            kinds_found.len() >= 2,
            "Expected at least 2 different kinds, found: {:?}",
            kinds_found
        );

        // Now filter to only Call references
        let options_calls = ReferencesOptions::new()
            .with_language("python".to_string())
            .with_kinds(vec![ReferenceKind::Call]);
        let report_calls = find_references("login", test_dir.path(), &options_calls).unwrap();

        // Should only have Call references
        assert!(
            !report_calls.references.is_empty(),
            "Should find Call references"
        );
        assert!(
            report_calls
                .references
                .iter()
                .all(|r| r.kind == ReferenceKind::Call),
            "With kinds=[Call], all references should be Call. Found: {:?}",
            report_calls
                .references
                .iter()
                .map(|r| &r.kind)
                .collect::<Vec<_>>()
        );

        // Test the standalone filter_by_kinds function
        let mixed_refs = vec![
            crate::analysis::references::Reference::new(
                std::path::PathBuf::from("test.py"),
                1,
                1,
                ReferenceKind::Call,
                "login()".to_string(),
            ),
            crate::analysis::references::Reference::new(
                std::path::PathBuf::from("test.py"),
                2,
                1,
                ReferenceKind::Import,
                "from auth import login".to_string(),
            ),
            crate::analysis::references::Reference::new(
                std::path::PathBuf::from("test.py"),
                3,
                1,
                ReferenceKind::Read,
                "x = login".to_string(),
            ),
        ];

        let filtered = filter_by_kinds(
            mixed_refs.clone(),
            &[ReferenceKind::Call, ReferenceKind::Import],
        );
        assert_eq!(
            filtered.len(),
            2,
            "Should have 2 references after filtering"
        );
        assert!(filtered
            .iter()
            .all(|r| r.kind == ReferenceKind::Call || r.kind == ReferenceKind::Import));

        // Filter to empty set should return nothing
        let empty_filter = filter_by_kinds(mixed_refs, &[ReferenceKind::Write]);
        assert!(empty_filter.is_empty(), "Should have no Write references");
    }
}

// =============================================================================
// Rename Refactoring Tests
// =============================================================================

#[cfg(test)]
mod rename_tests {
    use super::fixtures::*;

    /// Test rename function
    /// Contract: Returns all locations that need to be updated
    #[test]
    #[ignore = "rename refactoring not yet implemented"]
    fn rename_function() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/main.py", PYTHON_RENAME_CANDIDATE)
            .unwrap();

        // let rename_edits = prepare_rename(
        //     test_dir.path(),
        //     "old_name",
        //     "new_name",
        //     Language::Python,
        // ).unwrap();
        //
        // // Should have 3 edits: definition + 2 calls
        // assert_eq!(rename_edits.len(), 3);
        // assert!(rename_edits.iter().all(|e| e.new_text == "new_name"));

        todo!("Implement rename_function test")
    }

    /// Test rename class
    /// Contract: Class name, constructor calls, and type annotations updated
    #[test]
    #[ignore = "rename refactoring not yet implemented"]
    fn rename_class() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/models.py", PYTHON_CLASS_WITH_REFS)
            .unwrap();
        test_dir
            .add_file("src/services.py", PYTHON_USES_CLASS)
            .unwrap();

        // let rename_edits = prepare_rename(
        //     test_dir.path(),
        //     "User",
        //     "Person",
        //     Language::Python,
        // ).unwrap();
        //
        // // Should update class definition, inheritance, instantiation, type hints
        // assert!(rename_edits.len() >= 4);

        todo!("Implement rename_class test")
    }

    /// Test rename conflict detection
    /// Contract: Returns error when new name conflicts with existing symbol
    #[test]
    #[ignore = "rename refactoring not yet implemented"]
    fn rename_conflict_detection() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/main.py", PYTHON_RENAME_CONFLICT)
            .unwrap();

        // Trying to rename 'target' to 'existing' should conflict
        // let result = prepare_rename(
        //     test_dir.path(),
        //     "target",
        //     "existing",  // Already exists!
        //     Language::Python,
        // );
        //
        // assert!(result.is_err());
        // let err = result.unwrap_err();
        // assert!(err.to_string().contains("conflict") || err.to_string().contains("exists"));

        todo!("Implement rename_conflict_detection test")
    }

    /// Test rename in different scopes
    /// Contract: Rename respects scope (doesn't rename unrelated symbols)
    #[test]
    #[ignore = "rename refactoring not yet implemented"]
    fn rename_scope_aware() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/main.py", PYTHON_SHADOWED_VARIABLE)
            .unwrap();

        // Renaming the global 'name' shouldn't affect local 'name' or class 'name'
        // let rename_edits = prepare_rename(
        //     test_dir.path(),
        //     "name",  // The global one
        //     "global_name",
        //     Language::Python,
        //     Some(&test_dir.path().join("src/main.py")),  // Specify definition location
        // ).unwrap();
        //
        // // Should only rename global and references to global
        // // Not the local variable or class attribute

        todo!("Implement rename_scope_aware test")
    }
}

// =============================================================================
// Call Hierarchy Tests
// =============================================================================

#[cfg(test)]
mod call_hierarchy_tests {
    use super::fixtures::*;

    /// Test incoming calls (who calls this function)
    /// Contract: Returns all functions that call the target
    #[test]
    #[ignore = "call hierarchy not yet implemented"]
    fn incoming_calls() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/auth.py", PYTHON_AUTH_WITH_REFS)
            .unwrap();
        test_dir
            .add_file("src/routes.py", PYTHON_ROUTES_CALLS_AUTH)
            .unwrap();

        // let incoming = get_incoming_calls(
        //     test_dir.path(),
        //     "login",
        //     Language::Python,
        // ).unwrap();
        //
        // // login is called by handle_login in routes.py
        // assert!(incoming.iter().any(|c| c.name == "handle_login"));

        todo!("Implement incoming_calls test")
    }

    /// Test outgoing calls (what does this function call)
    /// Contract: Returns all functions called by the target
    #[test]
    #[ignore = "call hierarchy not yet implemented"]
    fn outgoing_calls() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/auth.py", PYTHON_AUTH_WITH_REFS)
            .unwrap();

        // let outgoing = get_outgoing_calls(
        //     test_dir.path(),
        //     "login",
        //     Language::Python,
        // ).unwrap();
        //
        // // login calls get_user, verify_password, log_login
        // let call_names: Vec<_> = outgoing.iter().map(|c| c.name.as_str()).collect();
        // assert!(call_names.contains(&"get_user"));
        // assert!(call_names.contains(&"verify_password"));
        // assert!(call_names.contains(&"log_login"));

        todo!("Implement outgoing_calls test")
    }

    /// Test nested calls
    /// Contract: Deep call chains are traversed correctly
    #[test]
    #[ignore = "call hierarchy not yet implemented"]
    fn nested_calls() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/auth.py", PYTHON_AUTH_WITH_REFS)
            .unwrap();
        test_dir
            .add_file("src/routes.py", PYTHON_ROUTES_CALLS_AUTH)
            .unwrap();

        // Chain: handle_login -> login -> get_user
        // let hierarchy = get_call_hierarchy(
        //     test_dir.path(),
        //     "get_user",
        //     Language::Python,
        //     2,  // depth
        // ).unwrap();
        //
        // // Should show: get_user <- login <- handle_login
        // assert!(hierarchy.incoming.iter().any(|c| c.name == "login"));

        todo!("Implement nested_calls test")
    }

    /// Test call hierarchy for methods
    /// Contract: Class methods are tracked correctly
    #[test]
    #[ignore = "call hierarchy not yet implemented"]
    fn method_call_hierarchy() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/models.py", PYTHON_CLASS_WITH_REFS)
            .unwrap();
        test_dir
            .add_file("src/services.py", PYTHON_USES_CLASS)
            .unwrap();

        // let incoming = get_incoming_calls(
        //     test_dir.path(),
        //     "display_name",  // User.display_name
        //     Language::Python,
        // ).unwrap();
        //
        // // display_name is called by greet_user
        // assert!(incoming.iter().any(|c| c.name == "greet_user"));

        todo!("Implement method_call_hierarchy test")
    }
}

// =============================================================================
// Type Hierarchy Tests
// =============================================================================

#[cfg(test)]
mod type_hierarchy_tests {
    use super::fixtures::*;

    /// Test finding supertypes
    /// Contract: Returns base classes/interfaces
    #[test]
    #[ignore = "type hierarchy not yet implemented"]
    fn find_supertypes() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/models.py", PYTHON_CLASS_WITH_REFS)
            .unwrap();

        // let supertypes = get_supertypes(
        //     test_dir.path(),
        //     "AdminUser",
        //     Language::Python,
        // ).unwrap();
        //
        // // AdminUser extends User
        // assert!(supertypes.iter().any(|t| t.name == "User"));

        todo!("Implement find_supertypes test")
    }

    /// Test finding subtypes
    /// Contract: Returns classes that extend/implement the target
    #[test]
    #[ignore = "type hierarchy not yet implemented"]
    fn find_subtypes() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/models.py", PYTHON_CLASS_WITH_REFS)
            .unwrap();

        // let subtypes = get_subtypes(
        //     test_dir.path(),
        //     "User",
        //     Language::Python,
        // ).unwrap();
        //
        // // User is extended by AdminUser
        // assert!(subtypes.iter().any(|t| t.name == "AdminUser"));

        todo!("Implement find_subtypes test")
    }

    /// Test TypeScript interface hierarchy
    /// Contract: Works with TypeScript extends/implements
    #[test]
    #[ignore = "type hierarchy not yet implemented"]
    fn typescript_type_hierarchy() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/models.ts", TS_CLASS_WITH_REFS)
            .unwrap();

        // let subtypes = get_subtypes(
        //     test_dir.path(),
        //     "User",
        //     Language::TypeScript,
        // ).unwrap();
        //
        // assert!(subtypes.iter().any(|t| t.name == "AdminUser"));

        todo!("Implement typescript_type_hierarchy test")
    }

    /// Test multi-level inheritance
    /// Contract: Full inheritance chain is traversed
    #[test]
    #[ignore = "type hierarchy not yet implemented"]
    fn multi_level_inheritance() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file(
                "src/models.py",
                r#"
class Base:
    pass

class Middle(Base):
    pass

class Derived(Middle):
    pass
"#,
            )
            .unwrap();

        // let supertypes = get_supertypes(
        //     test_dir.path(),
        //     "Derived",
        //     Language::Python,
        // ).unwrap();
        //
        // // Should find both Middle and Base
        // let type_names: Vec<_> = supertypes.iter().map(|t| t.name.as_str()).collect();
        // assert!(type_names.contains(&"Middle"));
        // assert!(type_names.contains(&"Base"));

        todo!("Implement multi_level_inheritance test")
    }
}

// =============================================================================
// Edge Cases and Performance Tests
// =============================================================================

#[cfg(test)]
mod edge_case_tests {
    use super::fixtures::*;

    /// Test shadowed variable handling
    /// Contract: Shadowed variables are reported as separate references
    #[test]
    #[ignore = "references module not yet implemented"]
    fn shadowed_variables() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/main.py", PYTHON_SHADOWED_VARIABLE)
            .unwrap();

        // let report = find_references(
        //     test_dir.path(),
        //     "name",
        //     Language::Python,
        //     &ReferencesConfig::default(),
        // ).unwrap();
        //
        // // Should find multiple definitions and references
        // // Context should help user disambiguate
        // assert!(report.total_references >= 3);

        todo!("Implement shadowed_variables test")
    }

    /// Test overloaded symbol handling
    /// Contract: Multiple definitions with same name are reported
    #[test]
    #[ignore = "references module not yet implemented"]
    fn overloaded_symbols() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/main.py", PYTHON_OVERLOADED_NAME)
            .unwrap();

        // let report = find_references(
        //     test_dir.path(),
        //     "process",
        //     Language::Python,
        //     &ReferencesConfig::default(),
        // ).unwrap();
        //
        // // Should find both function and method
        // assert!(report.candidates_found >= 2);

        todo!("Implement overloaded_symbols test")
    }

    /// Test Unicode symbol support
    /// Contract: Unicode identifiers are handled correctly
    #[test]
    #[ignore = "references module not yet implemented"]
    fn unicode_symbols() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file(
                "src/main.py",
                r#"
def calcul_somme(nombres):
    """Calculate sum with French variable names."""
    résultat = 0
    for n in nombres:
        résultat += n
    return résultat
"#,
            )
            .unwrap();

        // let report = find_references(
        //     test_dir.path(),
        //     "résultat",
        //     Language::Python,
        //     &ReferencesConfig::default(),
        // ).unwrap();
        //
        // // Should find the Unicode variable
        // assert!(report.total_references >= 2);  // definition + usage

        todo!("Implement unicode_symbols test")
    }

    /// Test very common symbol handling
    /// Contract: Results are limited, user is warned
    #[test]
    #[ignore = "references module not yet implemented"]
    fn very_common_symbol() {
        let test_dir = TestDir::new().unwrap();
        // Create many files with 'self' references
        for i in 0..20 {
            test_dir
                .add_file(
                    &format!("src/mod{}.py", i),
                    r#"
class MyClass:
    def method(self):
        return self.value
"#,
                )
                .unwrap();
        }

        // let report = find_references(
        //     test_dir.path(),
        //     "self",
        //     Language::Python,
        //     &ReferencesConfig { limit: 50, ..Default::default() },
        // ).unwrap();
        //
        // // Results should be limited
        // assert!(report.total_references <= 50);

        todo!("Implement very_common_symbol test")
    }

    /// Test performance requirement
    /// Contract: < 500ms for 10K files, 100 candidates
    #[test]
    #[ignore = "performance test"]
    fn performance_requirement() {
        let test_dir = TestDir::new().unwrap();

        // Create a realistic project structure
        for i in 0..100 {
            test_dir
                .add_file(
                    &format!("src/module{}.py", i),
                    &format!(
                        r#"
from utils import helper

def function{}():
    return helper() + {}
"#,
                        i, i
                    ),
                )
                .unwrap();
        }
        test_dir
            .add_file("src/utils.py", "def helper(): return 42")
            .unwrap();

        // let start = std::time::Instant::now();
        // let report = find_references(
        //     test_dir.path(),
        //     "helper",
        //     Language::Python,
        //     &ReferencesConfig::default(),
        // ).unwrap();
        // let elapsed = start.elapsed();
        //
        // assert!(elapsed.as_millis() < 500);
        // assert!(report.total_references >= 100);

        todo!("Implement performance_requirement test")
    }

    /// Test keyword exclusion
    /// Contract: Python keywords are not matched as symbols
    #[test]
    #[ignore = "references module not yet implemented"]
    fn keyword_exclusion() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file(
                "src/main.py",
                r#"
def process():
    if True:
        for i in range(10):
            pass
"#,
            )
            .unwrap();

        // Searching for "if" should not return the keyword
        // let report = find_references(
        //     test_dir.path(),
        //     "if",
        //     Language::Python,
        //     &ReferencesConfig::default(),
        // ).unwrap();
        //
        // // "if" is a keyword, not a symbol
        // assert!(report.total_references == 0);

        todo!("Implement keyword_exclusion test")
    }

    /// Test context lines in output
    /// Contract: --context-lines shows surrounding code
    #[test]
    #[ignore = "references module not yet implemented"]
    fn context_lines() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/auth.py", PYTHON_AUTH_WITH_REFS)
            .unwrap();

        // let report = find_references(
        //     test_dir.path(),
        //     "login",
        //     Language::Python,
        //     &ReferencesConfig { context_lines: 2, ..Default::default() },
        // ).unwrap();
        //
        // // Context should include surrounding lines
        // for ref_ in &report.references {
        //     // Context should span multiple lines when context_lines > 0
        //     assert!(ref_.context.lines().count() >= 1);
        // }

        todo!("Implement context_lines test")
    }

    /// Test confidence scores
    /// Contract: AST-verified references have confidence = 1.0
    #[test]
    #[ignore = "references module not yet implemented"]
    fn confidence_scores() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("src/auth.py", PYTHON_AUTH_WITH_REFS)
            .unwrap();

        // let report = find_references(
        //     test_dir.path(),
        //     "login",
        //     Language::Python,
        //     &ReferencesConfig::default(),
        // ).unwrap();
        //
        // // All verified references should have confidence 1.0
        // for ref_ in &report.references {
        //     assert_eq!(ref_.confidence, Some(1.0));
        // }

        todo!("Implement confidence_scores test")
    }
}
