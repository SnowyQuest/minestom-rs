//! Text colors, formatting `Style`, `ClickEvent`, and `HoverEvent` types.

use minestom_core::{Color, Key};

use crate::{Component, TextError};

/// Standard 16 Minecraft named text colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NamedTextColor {
    /// `#000000` (`§0`).
    Black,
    /// `#0000aa` (`§1`).
    DarkBlue,
    /// `#00aa00` (`§2`).
    DarkGreen,
    /// `#00aaaa` (`§3`).
    DarkAqua,
    /// `#aa0000` (`§4`).
    DarkRed,
    /// `#aa00aa` (`§5`).
    DarkPurple,
    /// `#ffaa00` (`§6`).
    Gold,
    /// `#aaaaaa` (`§7`).
    Gray,
    /// `#555555` (`§8`).
    DarkGray,
    /// `#5555ff` (`§9`).
    Blue,
    /// `#55ff55` (`§a`).
    Green,
    /// `#55ffff` (`§b`).
    Aqua,
    /// `#ff5555` (`§c`).
    Red,
    /// `#ff55ff` (`§d`).
    LightPurple,
    /// `#ffff55` (`§e`).
    Yellow,
    /// `#ffffff` (`§f`).
    White,
}

impl NamedTextColor {
    /// Canonical lowercase protocol name (e.g. `"gold"`, `"dark_aqua"`).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Black => "black",
            Self::DarkBlue => "dark_blue",
            Self::DarkGreen => "dark_green",
            Self::DarkAqua => "dark_aqua",
            Self::DarkRed => "dark_red",
            Self::DarkPurple => "dark_purple",
            Self::Gold => "gold",
            Self::Gray => "gray",
            Self::DarkGray => "dark_gray",
            Self::Blue => "blue",
            Self::Green => "green",
            Self::Aqua => "aqua",
            Self::Red => "red",
            Self::LightPurple => "light_purple",
            Self::Yellow => "yellow",
            Self::White => "white",
        }
    }

    /// Parses a named color from its lowercase protocol identifier.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "black" => Some(Self::Black),
            "dark_blue" => Some(Self::DarkBlue),
            "dark_green" => Some(Self::DarkGreen),
            "dark_aqua" => Some(Self::DarkAqua),
            "dark_red" => Some(Self::DarkRed),
            "dark_purple" => Some(Self::DarkPurple),
            "gold" => Some(Self::Gold),
            "gray" => Some(Self::Gray),
            "dark_gray" => Some(Self::DarkGray),
            "blue" => Some(Self::Blue),
            "green" => Some(Self::Green),
            "aqua" => Some(Self::Aqua),
            "red" => Some(Self::Red),
            "light_purple" => Some(Self::LightPurple),
            "yellow" => Some(Self::Yellow),
            "white" => Some(Self::White),
            _ => None,
        }
    }

    /// Legacy formatting character (`'0'..'9' | 'a'..'f'`).
    pub const fn legacy_code(self) -> char {
        match self {
            Self::Black => '0',
            Self::DarkBlue => '1',
            Self::DarkGreen => '2',
            Self::DarkAqua => '3',
            Self::DarkRed => '4',
            Self::DarkPurple => '5',
            Self::Gold => '6',
            Self::Gray => '7',
            Self::DarkGray => '8',
            Self::Blue => '9',
            Self::Green => 'a',
            Self::Aqua => 'b',
            Self::Red => 'c',
            Self::LightPurple => 'd',
            Self::Yellow => 'e',
            Self::White => 'f',
        }
    }

    /// Looks up a named color from a legacy hex code character (`0-9`, `a-f`).
    pub const fn from_legacy_code(ch: char) -> Option<Self> {
        match ch.to_ascii_lowercase() {
            '0' => Some(Self::Black),
            '1' => Some(Self::DarkBlue),
            '2' => Some(Self::DarkGreen),
            '3' => Some(Self::DarkAqua),
            '4' => Some(Self::DarkRed),
            '5' => Some(Self::DarkPurple),
            '6' => Some(Self::Gold),
            '7' => Some(Self::Gray),
            '8' => Some(Self::DarkGray),
            '9' => Some(Self::Blue),
            'a' => Some(Self::Green),
            'b' => Some(Self::Aqua),
            'c' => Some(Self::Red),
            'd' => Some(Self::LightPurple),
            'e' => Some(Self::Yellow),
            'f' => Some(Self::White),
            _ => None,
        }
    }

    /// Exact RGB value of this named color.
    #[must_use]
    pub const fn to_rgb(self) -> Color {
        match self {
            Self::Black => Color::from_rgb(0x00, 0x00, 0x00),
            Self::DarkBlue => Color::from_rgb(0x00, 0x00, 0xAA),
            Self::DarkGreen => Color::from_rgb(0x00, 0xAA, 0x00),
            Self::DarkAqua => Color::from_rgb(0x00, 0xAA, 0xAA),
            Self::DarkRed => Color::from_rgb(0xAA, 0x00, 0x00),
            Self::DarkPurple => Color::from_rgb(0xAA, 0x00, 0xAA),
            Self::Gold => Color::from_rgb(0xFF, 0xAA, 0x00),
            Self::Gray => Color::from_rgb(0xAA, 0xAA, 0xAA),
            Self::DarkGray => Color::from_rgb(0x55, 0x55, 0x55),
            Self::Blue => Color::from_rgb(0x55, 0x55, 0xFF),
            Self::Green => Color::from_rgb(0x55, 0xFF, 0x55),
            Self::Aqua => Color::from_rgb(0x55, 0xFF, 0xFF),
            Self::Red => Color::from_rgb(0xFF, 0x55, 0x55),
            Self::LightPurple => Color::from_rgb(0xFF, 0x55, 0xFF),
            Self::Yellow => Color::from_rgb(0xFF, 0xFF, 0x55),
            Self::White => Color::from_rgb(0xFF, 0xFF, 0xFF),
        }
    }
}

/// Foreground color of a text [`Component`]: either a [`NamedTextColor`] or custom 24-bit RGB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextColor {
    /// Named palette color.
    Named(NamedTextColor),
    /// Arbitrary `#rrggbb` RGB color.
    Rgb(Color),
}

impl TextColor {
    /// Constructs a custom RGB `TextColor`.
    #[must_use]
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self::Rgb(Color::from_rgb(r, g, b))
    }

    /// Parses either a named color (`"gold"`) or hex color (`"#ffaa00"`).
    pub fn parse(s: &str) -> Result<Self, TextError> {
        if let Some(named) = NamedTextColor::from_name(s) {
            return Ok(Self::Named(named));
        }
        if let Some(hex) = s.strip_prefix('#')
            && hex.len() == 6
            && let Ok(val) = u32::from_str_radix(hex, 16)
        {
            return Ok(Self::Rgb(Color(val)));
        }
        Err(TextError::InvalidFormat(format!(
            "invalid text color `{s}`"
        )))
    }

    /// Serializes to protocol string (`"gold"` or `"#ffaa00"`).
    #[must_use]
    pub fn to_protocol_string(self) -> String {
        match self {
            Self::Named(n) => n.as_str().to_owned(),
            Self::Rgb(c) => c.to_hex(),
        }
    }
}

impl From<NamedTextColor> for TextColor {
    fn from(n: NamedTextColor) -> Self {
        Self::Named(n)
    }
}

impl From<Color> for TextColor {
    fn from(c: Color) -> Self {
        Self::Rgb(c)
    }
}

/// Interactive click action attached to a [`Style`] (`click_event` in MC 26.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClickEvent {
    /// Opens a URL in the user's browser (`"open_url"`).
    OpenUrl(String),
    /// Runs a command (`"run_command"`).
    RunCommand(String),
    /// Fills the chat input box (`"suggest_command"`).
    SuggestCommand(String),
    /// Switches to a book page (`"change_page"`).
    ChangePage(i32),
    /// Copies text to the clipboard (`"copy_to_clipboard"`).
    CopyToClipboard(String),
}

/// Interactive hover tooltip attached to a [`Style`] (`hover_event` in MC 26.2).
#[derive(Debug, Clone, PartialEq)]
pub enum HoverEvent {
    /// Displays a rich [`Component`] tooltip (`"show_text"`).
    ShowText(Box<Component>),
}

/// Visual and interactive style attributes of a [`Component`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Style {
    /// Optional foreground color.
    pub color: Option<TextColor>,
    /// Optional shadow ARGB color (`shadow_color`).
    pub shadow_color: Option<i32>,
    /// Optional custom font resource key.
    pub font: Option<Key>,
    /// Optional `bold` override (`None` inherits from parent).
    pub bold: Option<bool>,
    /// Optional `italic` override.
    pub italic: Option<bool>,
    /// Optional `underlined` override.
    pub underlined: Option<bool>,
    /// Optional `strikethrough` override.
    pub strikethrough: Option<bool>,
    /// Optional `obfuscated` override.
    pub obfuscated: Option<bool>,
    /// Optional shift-click chat insertion string.
    pub insertion: Option<String>,
    /// Optional click action.
    pub click_event: Option<ClickEvent>,
    /// Optional hover tooltip.
    pub hover_event: Option<HoverEvent>,
}

impl Style {
    /// Returns `true` if every style field is unset (`None`).
    pub const fn is_empty(&self) -> bool {
        self.color.is_none()
            && self.shadow_color.is_none()
            && self.font.is_none()
            && self.bold.is_none()
            && self.italic.is_none()
            && self.underlined.is_none()
            && self.strikethrough.is_none()
            && self.obfuscated.is_none()
            && self.insertion.is_none()
            && self.click_event.is_none()
            && self.hover_event.is_none()
    }
}
