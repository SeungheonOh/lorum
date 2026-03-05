use thiserror::Error;

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("database error: {0}")]
    Database(String),
    #[error("serialization error: {0}")]
    Serialization(String),
    #[error("invalid credential data: {0}")]
    InvalidCredential(String),
    #[error("oauth provider not registered: {0}")]
    MissingOAuthProvider(String),
    #[error("internal synchronization error")]
    Internal,
}
