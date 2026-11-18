//! Adventure-inspired rich text [`Component`] model, styling, and serializers
//! (JSON, MC 26.2 network NBT, legacy `§`/`&`, plain text, and MiniMessage subset).
//!
//! ```
//! use minestom_text::{Component, NamedTextColor};
//!
//! let welcome = Component::text("Welcome to ")
//!     .color(NamedTextColor::Gray)
//!     .append(Component::text("minestom-rs!").color(NamedTextColor::Gold).bold(true));
//! assert_eq!(welcome.to_plain_text(), "Welcome to minestom-rs!");
//! ```

mod serialize;
mod style;

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub use style::{ClickEvent, HoverEvent, NamedTextColor, Style, TextColor};

/// Errors produced when serializing or deserializing text components.
#[derive(Debug, thiserror::Error)]
pub enum TextError {
    /// JSON component error.
    #[error("JSON component serialization error: {0}")]
    Json(#[from] serde_json::Error),
    /// NBT component error.
    #[error("NBT component error: {0}")]
    Nbt(#[from] minestom_nbt::NbtError),
    /// Invalid color or component structure.
    #[error("invalid component format: {0}")]
    InvalidFormat(String),
}

/// Core content variant of a [`Component`].
#[derive(Debug, Clone, PartialEq)]
pub enum ComponentContent {
    /// Literal UTF-8 text (`"text"`).
    Text(String),
    /// Client-localized translation key (`"translate"`) with optional fallback and `with` arguments.
    Translatable {
        /// Translation key (e.g. `"chat.type.text"`).
        key: String,
        /// Optional fallback string if client lacks `key`.
        fallback: Option<String>,
        /// Positional substitution arguments.
        args: Vec<Component>,
    },
    /// Client keybind identifier (`"keybind"`, e.g. `"key.jump"`).
    Keybind(String),
}

/// Immutable, composable Adventure-style rich text node.
#[derive(Debug, Clone, PartialEq)]
pub struct Component {
    /// Primary content of this node.
    pub content: ComponentContent,
    /// Visual and interactive styling applied to this node and inherited by `children`.
    pub style: Style,
    /// Appended sibling/child components (`"extra"`).
    pub children: Vec<Self>,
}

impl Default for Component {
    fn default() -> Self {
        Self::empty()
    }
}

impl Component {
    /// Empty literal text component (`""`).
    #[must_use]
    pub fn empty() -> Self {
        Self::text("")
    }

    /// Creates a literal text [`Component`].
    #[must_use]
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            content: ComponentContent::Text(content.into()),
            style: Style::default(),
            children: Vec::new(),
        }
    }

    /// Creates a translatable [`Component`] with no arguments.
    #[must_use]
    pub fn translatable(key: impl Into<String>) -> Self {
        Self {
            content: ComponentContent::Translatable {
                key: key.into(),
                fallback: None,
                args: Vec::new(),
            },
            style: Style::default(),
            children: Vec::new(),
        }
    }

    /// Creates a translatable [`Component`] with substitution arguments.
    #[must_use]
    pub fn translatable_with(key: impl Into<String>, args: Vec<Self>) -> Self {
        Self {
            content: ComponentContent::Translatable {
                key: key.into(),
                fallback: None,
                args,
            },
            style: Style::default(),
            children: Vec::new(),
        }
    }

    /// Creates a keybind [`Component`].
    #[must_use]
    pub fn keybind(keybind: impl Into<String>) -> Self {
        Self {
            content: ComponentContent::Keybind(keybind.into()),
            style: Style::default(),
            children: Vec::new(),
        }
    }

    /// Appends a child [`Component`] (`extra`).
    #[must_use]
    pub fn append(mut self, child: impl Into<Self>) -> Self {
        self.children.push(child.into());
        self
    }

    /// Sets the foreground [`TextColor`].
    #[must_use]
    pub fn color(mut self, color: impl Into<TextColor>) -> Self {
        self.style.color = Some(color.into());
        self
    }

    /// Sets the `bold` decoration state.
    #[must_use]
    pub const fn bold(mut self, state: bool) -> Self {
        self.style.bold = Some(state);
        self
    }

    /// Sets the `italic` decoration state.
    #[must_use]
    pub const fn italic(mut self, state: bool) -> Self {
        self.style.italic = Some(state);
        self
    }

    /// Sets the `underlined` decoration state.
    #[must_use]
    pub const fn underlined(mut self, state: bool) -> Self {
        self.style.underlined = Some(state);
        self
    }

    /// Sets the `strikethrough` decoration state.
    #[must_use]
    pub const fn strikethrough(mut self, state: bool) -> Self {
        self.style.strikethrough = Some(state);
        self
    }

    /// Sets the `obfuscated` decoration state.
    #[must_use]
    pub const fn obfuscated(mut self, state: bool) -> Self {
        self.style.obfuscated = Some(state);
        self
    }

    /// Attaches a [`ClickEvent`].
    #[must_use]
    pub fn click_event(mut self, event: ClickEvent) -> Self {
        self.style.click_event = Some(event);
        self
    }

    /// Attaches a [`HoverEvent`].
    #[must_use]
    pub fn hover_event(mut self, event: HoverEvent) -> Self {
        self.style.hover_event = Some(event);
        self
    }
}

impl From<&str> for Component {
    fn from(s: &str) -> Self {
        Self::text(s)
    }
}

impl From<String> for Component {
    fn from(s: String) -> Self {
        Self::text(s)
    }
}

impl fmt::Display for Component {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_plain_text())
    }
}

impl FromIterator<Self> for Component {
    fn from_iter<I: IntoIterator<Item = Self>>(iter: I) -> Self {
        let mut root = Self::empty();
        root.children.extend(iter);
        root
    }
}

impl Serialize for Component {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_json_value().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Component {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let val = serde_json::Value::deserialize(deserializer)?;
        Self::from_json_value(&val).map_err(serde::de::Error::custom)
    }
}
