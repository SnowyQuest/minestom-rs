//! Brigadier-compatible declarative command tree, typed arguments, dynamic suggestions,
//! sender conditions, and `DeclareCommands` graph serialization.

/// Errors returned when parsing or executing commands.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CommandError {
    /// Command name was not registered.
    #[error("unknown command `{0}`")]
    UnknownCommand(String),
    /// Argument failed syntax validation.
    #[error("invalid command syntax at `{input}`: {reason}")]
    SyntaxError {
        /// Offending token or substring.
        input: String,
        /// Human-readable explanation.
        reason: String,
    },
    /// Sender failed the command's execution condition.
    #[error("insufficient permissions to execute command")]
    ConditionFailed,
}
