use std::fmt;

#[derive(Debug)]
pub enum AppError {
    Config(String),
    Database(String),
    Network(String),
    Io(String),
    Yaml(String),
    JSON(String),
    NotFound(String),
    Auth(String),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(msg) => write!(f, "Config error: {}", msg),
            Self::Database(msg) => write!(f, "Database error: {}", msg),
            Self::Network(msg) => write!(f, "Network error: {}", msg),
            Self::Io(msg) => write!(f, "IO error: {}", msg),
            Self::Yaml(msg) => write!(f, "YAML error: {}", msg),
            Self::JSON(msg) => write!(f, "JSON error: {}", msg),
            Self::NotFound(msg) => write!(f, "Not found: {}", msg),
            Self::Auth(msg) => write!(f, "Auth error: {}", msg),
        }
    }
}

impl std::error::Error for AppError {}

impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Database(e.to_string())
    }
}

impl From<reqwest::Error> for AppError {
    fn from(e: reqwest::Error) -> Self {
        Self::Network(e.to_string())
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        Self::JSON(e.to_string())
    }
}

impl From<serde_yaml::Error> for AppError {
    fn from(e: serde_yaml::Error) -> Self {
        Self::Yaml(e.to_string())
    }
}

impl serde::Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}
