//! Adventure-inspired rich text `Component` model, styling, and serializers
//! (JSON, nameless network NBT, legacy `§`/`&`, plain text, and MiniMessage subset).

/// Errors produced when serializing or deserializing text components.
#[derive(Debug, thiserror::Error)]
pub enum TextError {
    /// JSON component error.
    #[error("JSON component serialization error: {0}")]
    Json(#[from] serde_json::Error),
    /// NBT component error.
    #[error("NBT component error: {0}")]
    Nbt(#[from] minestom_nbt::NbtError),
    /// Invalid color or style specification.
    #[error("invalid component format: {0}")]
    InvalidFormat(String),
}
