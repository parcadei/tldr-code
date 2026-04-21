/// A simple configuration holder.
#[derive(Debug, Clone)]
pub struct Config {
    /// The application name.
    pub name: String,
    /// The port number.
    pub port: u16,
}

impl Config {
    /// Create a new Config with the given name and port.
    pub fn new(name: &str, port: u16) -> Self {
        Config {
            name: name.to_string(),
            port,
        }
    }

    /// Get the address string.
    pub fn address(&self) -> String {
        format!("{}:{}", self.name, self.port)
    }

    fn internal_helper(&self) -> bool {
        true
    }
}

/// A public helper function.
pub fn greet(name: &str) -> String {
    format!("Hello, {}!", name)
}

/// Maximum retry count.
pub const MAX_RETRIES: u32 = 3;

fn private_function() -> i32 {
    42
}

/// A simple trait for greeting.
pub trait Greeter {
    /// Produce a greeting message.
    fn greet(&self) -> String;
}

/// Status of a request.
pub enum Status {
    /// Everything is fine.
    Ok,
    /// Something went wrong.
    Error(String),
}
