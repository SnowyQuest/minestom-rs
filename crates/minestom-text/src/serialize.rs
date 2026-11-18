//! JSON, MC 26.2 Network NBT, Legacy (`§`/`&`), Plain, and MiniMessage serializers for [`Component`].

use minestom_core::Key;
use minestom_nbt::{NbtCompound, NbtTag};
use serde_json::{Map, Value};

use crate::style::{ClickEvent, HoverEvent, NamedTextColor, Style, TextColor};
use crate::{Component, ComponentContent, TextError};

impl Component {
    /// Strips all formatting and flattens literal text + child components into a plain `String`.
    #[must_use]
    pub fn to_plain_text(&self) -> String {
        let mut out = String::new();
        self.write_plain(&mut out);
        out
    }

    fn write_plain(&self, out: &mut String) {
        match &self.content {
            ComponentContent::Text(s) | ComponentContent::Keybind(s) => out.push_str(s),
            ComponentContent::Translatable { key, fallback, .. } => {
                out.push_str(fallback.as_deref().unwrap_or(key));
            }
        }
        for child in &self.children {
            child.write_plain(out);
        }
    }

    /// Serializes this component to legacy formatting codes using `prefix` (`'§'` or `'&'`).
    #[must_use]
    pub fn to_legacy_string(&self, prefix: char) -> String {
        let mut out = String::new();
        self.write_legacy(&mut out, prefix);
        out
    }

    fn write_legacy(&self, out: &mut String, prefix: char) {
        if let Some(color) = self.style.color {
            match color {
                TextColor::Named(n) => {
                    out.push(prefix);
                    out.push(n.legacy_code());
                }
                TextColor::Rgb(rgb) => {
                    out.push(prefix);
                    out.push('x');
                    for ch in format!("{:06x}", rgb.0 & 0x00FF_FFFF).chars() {
                        out.push(prefix);
                        out.push(ch);
                    }
                }
            }
        }
        if self.style.bold == Some(true) {
            out.push(prefix);
            out.push('l');
        }
        if self.style.strikethrough == Some(true) {
            out.push(prefix);
            out.push('m');
        }
        if self.style.underlined == Some(true) {
            out.push(prefix);
            out.push('n');
        }
        if self.style.italic == Some(true) {
            out.push(prefix);
            out.push('o');
        }
        if self.style.obfuscated == Some(true) {
            out.push(prefix);
            out.push('k');
        }
        match &self.content {
            ComponentContent::Text(s) | ComponentContent::Keybind(s) => out.push_str(s),
            ComponentContent::Translatable { key, fallback, .. } => {
                out.push_str(fallback.as_deref().unwrap_or(key));
            }
        }
        for child in &self.children {
            child.write_legacy(out, prefix);
        }
    }

    /// Parses legacy section/ampersand formatted text (e.g. `"§6Gold §lBold"`).
    #[must_use]
    pub fn from_legacy(input: &str, prefix: char) -> Self {
        let mut root = Self::empty();
        let mut current_style = Style::default();
        let mut current_text = String::new();
        let mut chars = input.chars().peekable();

        while let Some(ch) = chars.next() {
            if ch == prefix
                && let Some(&code) = chars.peek()
            {
                let mut recognized = true;
                if let Some(named) = NamedTextColor::from_legacy_code(code) {
                    if !current_text.is_empty() {
                        root.children.push(Self {
                            content: ComponentContent::Text(std::mem::take(&mut current_text)),
                            style: current_style.clone(),
                            children: Vec::new(),
                        });
                    }
                    current_style = Style {
                        color: Some(TextColor::Named(named)),
                        ..Style::default()
                    };
                } else {
                    match code.to_ascii_lowercase() {
                        'k' | 'l' | 'm' | 'n' | 'o' | 'r' => {
                            if !current_text.is_empty() {
                                root.children.push(Self {
                                    content: ComponentContent::Text(std::mem::take(
                                        &mut current_text,
                                    )),
                                    style: current_style.clone(),
                                    children: Vec::new(),
                                });
                            }
                            match code.to_ascii_lowercase() {
                                'k' => current_style.obfuscated = Some(true),
                                'l' => current_style.bold = Some(true),
                                'm' => current_style.strikethrough = Some(true),
                                'n' => current_style.underlined = Some(true),
                                'o' => current_style.italic = Some(true),
                                'r' => current_style = Style::default(),
                                _ => {}
                            }
                        }
                        _ => recognized = false,
                    }
                }
                if recognized {
                    chars.next();
                    continue;
                }
            }
            current_text.push(ch);
        }

        if !current_text.is_empty() {
            if root.children.is_empty() {
                return Self {
                    content: ComponentContent::Text(current_text),
                    style: current_style,
                    children: Vec::new(),
                };
            }
            root.children.push(Self {
                content: ComponentContent::Text(current_text),
                style: current_style,
                children: Vec::new(),
            });
        }
        root
    }

    /// Parses a concise subset of Adventure MiniMessage (`<red>`, `<#ffaa00>`, `<bold>`, `</bold>`, `<reset>`).
    pub fn from_mini_message(input: &str) -> Result<Self, TextError> {
        let mut root = Self::empty();
        let mut style_stack: Vec<Style> = vec![Style::default()];
        let mut buf = String::new();
        let mut rest = input;

        let flush = |buf: &mut String, style: &Style, children: &mut Vec<Self>| {
            if !buf.is_empty() {
                children.push(Self {
                    content: ComponentContent::Text(std::mem::take(buf)),
                    style: style.clone(),
                    children: Vec::new(),
                });
            }
        };

        while let Some(lt) = rest.find('<') {
            buf.push_str(&rest[..lt]);
            let after_lt = &rest[(lt + 1)..];
            let Some(gt) = after_lt.find('>') else {
                buf.push('<');
                rest = after_lt;
                continue;
            };
            let tag = &after_lt[..gt];
            rest = &after_lt[(gt + 1)..];

            let current = style_stack.last().cloned().unwrap_or_default();
            flush(&mut buf, &current, &mut root.children);

            if tag == "reset" {
                style_stack.truncate(1);
                style_stack[0] = Style::default();
            } else if tag.starts_with('/') {
                if style_stack.len() > 1 {
                    style_stack.pop();
                }
            } else {
                let mut next = current;
                match tag {
                    "bold" | "b" => next.bold = Some(true),
                    "italic" | "i" | "em" => next.italic = Some(true),
                    "underlined" | "u" => next.underlined = Some(true),
                    "strikethrough" | "st" => next.strikethrough = Some(true),
                    "obfuscated" | "obf" => next.obfuscated = Some(true),
                    color_str => {
                        next.color = Some(TextColor::parse(color_str)?);
                    }
                }
                style_stack.push(next);
            }
        }
        buf.push_str(rest);
        let current = style_stack.last().cloned().unwrap_or_default();
        flush(&mut buf, &current, &mut root.children);

        if root.children.len() == 1 && root.style.is_empty() {
            Ok(root.children.remove(0))
        } else {
            Ok(root)
        }
    }

    /// Serializes this component to a `serde_json::Value` following MC 26.2 JSON component rules.
    #[must_use]
    pub fn to_json_value(&self) -> Value {
        if let ComponentContent::Text(s) = &self.content
            && self.style.is_empty()
            && self.children.is_empty()
        {
            return Value::String(s.clone());
        }

        let mut map = Map::new();
        match &self.content {
            ComponentContent::Text(s) => {
                map.insert("text".to_owned(), Value::String(s.clone()));
            }
            ComponentContent::Translatable {
                key,
                fallback,
                args,
            } => {
                map.insert("translate".to_owned(), Value::String(key.clone()));
                if let Some(fb) = fallback {
                    map.insert("fallback".to_owned(), Value::String(fb.clone()));
                }
                if !args.is_empty() {
                    map.insert(
                        "with".to_owned(),
                        Value::Array(args.iter().map(Self::to_json_value).collect()),
                    );
                }
            }
            ComponentContent::Keybind(kb) => {
                map.insert("keybind".to_owned(), Value::String(kb.clone()));
            }
        }

        write_style_json(&mut map, &self.style);

        if !self.children.is_empty() {
            map.insert(
                "extra".to_owned(),
                Value::Array(self.children.iter().map(Self::to_json_value).collect()),
            );
        }

        Value::Object(map)
    }

    /// Serializes this component to a compact JSON string (used in `Status` MOTD and `LoginDisconnect`).
    pub fn to_json_string(&self) -> Result<String, TextError> {
        Ok(serde_json::to_string(&self.to_json_value())?)
    }

    /// Deserializes a [`Component`] from a JSON string.
    pub fn from_json_str(json: &str) -> Result<Self, TextError> {
        let val: Value = serde_json::from_str(json)?;
        Self::from_json_value(&val)
    }

    /// Deserializes a [`Component`] from a `serde_json::Value`.
    pub fn from_json_value(value: &Value) -> Result<Self, TextError> {
        match value {
            Value::String(s) => Ok(Self::text(s.clone())),
            Value::Bool(b) => Ok(Self::text(b.to_string())),
            Value::Number(n) => Ok(Self::text(n.to_string())),
            Value::Array(arr) => {
                if arr.is_empty() {
                    return Ok(Self::empty());
                }
                let mut first = Self::from_json_value(&arr[0])?;
                for item in &arr[1..] {
                    first.children.push(Self::from_json_value(item)?);
                }
                Ok(first)
            }
            Value::Object(map) => {
                let content = if let Some(t) = map.get("text").and_then(Value::as_str) {
                    ComponentContent::Text(t.to_owned())
                } else if let Some(tr) = map.get("translate").and_then(Value::as_str) {
                    let fallback = map
                        .get("fallback")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    let args = if let Some(with_arr) = map.get("with").and_then(Value::as_array) {
                        with_arr
                            .iter()
                            .map(Self::from_json_value)
                            .collect::<Result<Vec<_>, _>>()?
                    } else {
                        Vec::new()
                    };
                    ComponentContent::Translatable {
                        key: tr.to_owned(),
                        fallback,
                        args,
                    }
                } else if let Some(kb) = map.get("keybind").and_then(Value::as_str) {
                    ComponentContent::Keybind(kb.to_owned())
                } else {
                    ComponentContent::Text(String::new())
                };

                let style = read_style_json(map)?;
                let children = if let Some(extra) = map.get("extra").and_then(Value::as_array) {
                    extra
                        .iter()
                        .map(Self::from_json_value)
                        .collect::<Result<Vec<_>, _>>()?
                } else {
                    Vec::new()
                };

                Ok(Self {
                    content,
                    style,
                    children,
                })
            }
            Value::Null => Err(TextError::InvalidFormat("null JSON component".to_owned())),
        }
    }

    /// Serializes this [`Component`] into MC 26.2 nameless network [`NbtTag`] (`TAG_Compound`
    /// matching Minestom's `ComponentNetworkBufferTypeImpl`).
    #[must_use]
    pub fn to_nbt(&self) -> NbtTag {
        NbtTag::Compound(self.to_nbt_compound())
    }

    /// Serializes this [`Component`] into an [`NbtCompound`] following MC 26.2 format.
    #[must_use]
    pub fn to_nbt_compound(&self) -> NbtCompound {
        let mut map = NbtCompound::new();
        match &self.content {
            ComponentContent::Text(s) => {
                map.insert("type", "text");
                map.insert("text", s.clone());
            }
            ComponentContent::Translatable {
                key,
                fallback,
                args,
            } => {
                map.insert("type", "translatable");
                map.insert("translate", key.clone());
                if let Some(fb) = fallback {
                    map.insert("fallback", fb.clone());
                }
                if !args.is_empty() {
                    let list = args
                        .iter()
                        .map(|a| NbtTag::Compound(a.to_nbt_compound()))
                        .collect::<Vec<_>>();
                    map.insert("with", NbtTag::List(list));
                }
            }
            ComponentContent::Keybind(kb) => {
                map.insert("type", "keybind");
                map.insert("keybind", kb.clone());
            }
        }

        if !self.children.is_empty() {
            let extra = self
                .children
                .iter()
                .map(|c| NbtTag::Compound(c.to_nbt_compound()))
                .collect::<Vec<_>>();
            map.insert("extra", NbtTag::List(extra));
        }

        write_style_nbt(&mut map, &self.style);
        map
    }

    /// Deserializes a [`Component`] from an MC 26.2 network [`NbtTag`].
    pub fn from_nbt(tag: &NbtTag) -> Result<Self, TextError> {
        match tag {
            NbtTag::String(s) => Ok(Self::text(s.clone())),
            NbtTag::Compound(map) => Self::from_nbt_compound(map),
            other => Err(TextError::InvalidFormat(format!(
                "unsupported root NBT tag for Component: {:?}",
                other.tag_type()
            ))),
        }
    }

    /// Deserializes a [`Component`] from an [`NbtCompound`].
    pub fn from_nbt_compound(map: &NbtCompound) -> Result<Self, TextError> {
        let content = if let Some(t) = map.get_str("text") {
            ComponentContent::Text(t.to_owned())
        } else if let Some(tr) = map.get_str("translate") {
            let fallback = map.get_str("fallback").map(str::to_owned);
            let args = if let Some(with_list) = map.get_list("with") {
                with_list
                    .iter()
                    .map(Self::from_nbt)
                    .collect::<Result<Vec<_>, _>>()?
            } else {
                Vec::new()
            };
            ComponentContent::Translatable {
                key: tr.to_owned(),
                fallback,
                args,
            }
        } else if let Some(kb) = map.get_str("keybind") {
            ComponentContent::Keybind(kb.to_owned())
        } else if let Some(empty_wrapper) = map.get_str("") {
            ComponentContent::Text(empty_wrapper.to_owned())
        } else {
            ComponentContent::Text(String::new())
        };

        let style = read_style_nbt(map)?;
        let children = if let Some(extra_list) = map.get_list("extra") {
            extra_list
                .iter()
                .map(Self::from_nbt)
                .collect::<Result<Vec<_>, _>>()?
        } else {
            Vec::new()
        };

        Ok(Self {
            content,
            style,
            children,
        })
    }
}

fn write_style_json(map: &mut Map<String, Value>, style: &Style) {
    if let Some(c) = style.color {
        map.insert("color".to_owned(), Value::String(c.to_protocol_string()));
    }
    if let Some(sc) = style.shadow_color {
        map.insert("shadow_color".to_owned(), Value::Number(sc.into()));
    }
    if let Some(font) = &style.font {
        map.insert("font".to_owned(), Value::String(font.as_str().to_owned()));
    }
    if let Some(v) = style.bold {
        map.insert("bold".to_owned(), Value::Bool(v));
    }
    if let Some(v) = style.italic {
        map.insert("italic".to_owned(), Value::Bool(v));
    }
    if let Some(v) = style.underlined {
        map.insert("underlined".to_owned(), Value::Bool(v));
    }
    if let Some(v) = style.strikethrough {
        map.insert("strikethrough".to_owned(), Value::Bool(v));
    }
    if let Some(v) = style.obfuscated {
        map.insert("obfuscated".to_owned(), Value::Bool(v));
    }
    if let Some(ins) = &style.insertion {
        map.insert("insertion".to_owned(), Value::String(ins.clone()));
    }
    if let Some(click) = &style.click_event {
        let mut cm = Map::new();
        match click {
            ClickEvent::OpenUrl(url) => {
                cm.insert("action".to_owned(), Value::String("open_url".to_owned()));
                cm.insert("url".to_owned(), Value::String(url.clone()));
            }
            ClickEvent::RunCommand(cmd) => {
                cm.insert("action".to_owned(), Value::String("run_command".to_owned()));
                cm.insert("command".to_owned(), Value::String(cmd.clone()));
            }
            ClickEvent::SuggestCommand(cmd) => {
                cm.insert(
                    "action".to_owned(),
                    Value::String("suggest_command".to_owned()),
                );
                cm.insert("command".to_owned(), Value::String(cmd.clone()));
            }
            ClickEvent::ChangePage(p) => {
                cm.insert("action".to_owned(), Value::String("change_page".to_owned()));
                cm.insert("page".to_owned(), Value::Number((*p).into()));
            }
            ClickEvent::CopyToClipboard(val) => {
                cm.insert(
                    "action".to_owned(),
                    Value::String("copy_to_clipboard".to_owned()),
                );
                cm.insert("value".to_owned(), Value::String(val.clone()));
            }
        }
        map.insert("click_event".to_owned(), Value::Object(cm));
    }
    if let Some(HoverEvent::ShowText(comp)) = &style.hover_event {
        let mut hm = Map::new();
        hm.insert("action".to_owned(), Value::String("show_text".to_owned()));
        hm.insert("value".to_owned(), comp.to_json_value());
        map.insert("hover_event".to_owned(), Value::Object(hm));
    }
}

fn read_style_json(map: &Map<String, Value>) -> Result<Style, TextError> {
    let mut style = Style::default();
    if let Some(c) = map.get("color").and_then(Value::as_str) {
        style.color = Some(TextColor::parse(c)?);
    }
    if let Some(sc) = map.get("shadow_color").and_then(Value::as_i64) {
        style.shadow_color = Some(sc as i32);
    }
    if let Some(font) = map.get("font").and_then(Value::as_str) {
        style.font = Some(
            Key::parse(font)
                .map_err(|e| TextError::InvalidFormat(format!("invalid font key: {e}")))?,
        );
    }
    style.bold = map.get("bold").and_then(Value::as_bool);
    style.italic = map.get("italic").and_then(Value::as_bool);
    style.underlined = map.get("underlined").and_then(Value::as_bool);
    style.strikethrough = map.get("strikethrough").and_then(Value::as_bool);
    style.obfuscated = map.get("obfuscated").and_then(Value::as_bool);
    style.insertion = map
        .get("insertion")
        .and_then(Value::as_str)
        .map(str::to_owned);

    if let Some(cm) = map.get("click_event").and_then(Value::as_object)
        && let Some(action) = cm.get("action").and_then(Value::as_str)
    {
        style.click_event = match action {
            "open_url" => cm
                .get("url")
                .and_then(Value::as_str)
                .map(|s| ClickEvent::OpenUrl(s.to_owned())),
            "run_command" => cm
                .get("command")
                .and_then(Value::as_str)
                .map(|s| ClickEvent::RunCommand(s.to_owned())),
            "suggest_command" => cm
                .get("command")
                .and_then(Value::as_str)
                .map(|s| ClickEvent::SuggestCommand(s.to_owned())),
            "change_page" => cm
                .get("page")
                .and_then(Value::as_i64)
                .map(|p| ClickEvent::ChangePage(p as i32)),
            "copy_to_clipboard" => cm
                .get("value")
                .and_then(Value::as_str)
                .map(|s| ClickEvent::CopyToClipboard(s.to_owned())),
            _ => None,
        };
    }

    if let Some(hm) = map.get("hover_event").and_then(Value::as_object)
        && hm.get("action").and_then(Value::as_str) == Some("show_text")
        && let Some(val) = hm.get("value")
    {
        style.hover_event = Some(HoverEvent::ShowText(Box::new(Component::from_json_value(
            val,
        )?)));
    }
    Ok(style)
}

fn write_style_nbt(map: &mut NbtCompound, style: &Style) {
    if let Some(c) = style.color {
        map.insert("color", c.to_protocol_string());
    }
    if let Some(sc) = style.shadow_color {
        map.insert("shadow_color", sc);
    }
    if let Some(font) = &style.font {
        map.insert("font", font.as_str());
    }
    if let Some(v) = style.bold {
        map.insert("bold", v);
    }
    if let Some(v) = style.italic {
        map.insert("italic", v);
    }
    if let Some(v) = style.underlined {
        map.insert("underlined", v);
    }
    if let Some(v) = style.strikethrough {
        map.insert("strikethrough", v);
    }
    if let Some(v) = style.obfuscated {
        map.insert("obfuscated", v);
    }
    if let Some(ins) = &style.insertion {
        map.insert("insertion", ins.clone());
    }
    if let Some(click) = &style.click_event {
        let mut cm = NbtCompound::new();
        match click {
            ClickEvent::OpenUrl(url) => {
                cm.insert("action", "open_url");
                cm.insert("url", url.clone());
            }
            ClickEvent::RunCommand(cmd) => {
                cm.insert("action", "run_command");
                cm.insert("command", cmd.clone());
            }
            ClickEvent::SuggestCommand(cmd) => {
                cm.insert("action", "suggest_command");
                cm.insert("command", cmd.clone());
            }
            ClickEvent::ChangePage(p) => {
                cm.insert("action", "change_page");
                cm.insert("page", *p);
            }
            ClickEvent::CopyToClipboard(val) => {
                cm.insert("action", "copy_to_clipboard");
                cm.insert("value", val.clone());
            }
        }
        map.insert("click_event", cm);
    }
    if let Some(HoverEvent::ShowText(comp)) = &style.hover_event {
        let hm = NbtCompound::new()
            .with("action", "show_text")
            .with("value", comp.to_nbt_compound());
        map.insert("hover_event", hm);
    }
}

fn read_style_nbt(map: &NbtCompound) -> Result<Style, TextError> {
    let mut style = Style::default();
    if let Some(c) = map.get_str("color") {
        style.color = Some(TextColor::parse(c)?);
    }
    style.shadow_color = map.get_int("shadow_color");
    if let Some(font) = map.get_str("font") {
        style.font = Some(
            Key::parse(font)
                .map_err(|e| TextError::InvalidFormat(format!("invalid font key: {e}")))?,
        );
    }
    style.bold = map.get_bool("bold");
    style.italic = map.get_bool("italic");
    style.underlined = map.get_bool("underlined");
    style.strikethrough = map.get_bool("strikethrough");
    style.obfuscated = map.get_bool("obfuscated");
    style.insertion = map.get_str("insertion").map(str::to_owned);

    if let Some(cm) = map.get_compound("click_event")
        && let Some(action) = cm.get_str("action")
    {
        style.click_event = match action {
            "open_url" => cm.get_str("url").map(|s| ClickEvent::OpenUrl(s.to_owned())),
            "run_command" => cm
                .get_str("command")
                .map(|s| ClickEvent::RunCommand(s.to_owned())),
            "suggest_command" => cm
                .get_str("command")
                .map(|s| ClickEvent::SuggestCommand(s.to_owned())),
            "change_page" => cm.get_int("page").map(ClickEvent::ChangePage),
            "copy_to_clipboard" => cm
                .get_str("value")
                .map(|s| ClickEvent::CopyToClipboard(s.to_owned())),
            _ => None,
        };
    }

    if let Some(hm) = map.get_compound("hover_event")
        && hm.get_str("action") == Some("show_text")
        && let Some(val) = hm.get("value")
    {
        style.hover_event = Some(HoverEvent::ShowText(Box::new(Component::from_nbt(val)?)));
    }
    Ok(style)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_and_nbt_component_roundtrip() {
        let comp = Component::text("Hello ")
            .color(NamedTextColor::Aqua)
            .append(
                Component::text("World")
                    .color(TextColor::rgb(0xFF, 0xAA, 0x00))
                    .bold(true)
                    .click_event(ClickEvent::RunCommand("/spawn".to_owned()))
                    .hover_event(HoverEvent::ShowText(Box::new(Component::text("Click me!")))),
            );

        let json = comp.to_json_string().expect("serialize JSON");
        let from_json = Component::from_json_str(&json).expect("deserialize JSON");
        assert_eq!(from_json, comp);

        let nbt = comp.to_nbt();
        let from_nbt = Component::from_nbt(&nbt).expect("deserialize NBT");
        assert_eq!(from_nbt, comp);
    }

    #[test]
    fn mini_message_and_legacy_roundtrip() {
        let mm = Component::from_mini_message("<gold><bold>Minestom</bold> <#55ff55>Rust</gold>")
            .expect("parse MiniMessage");
        assert_eq!(mm.to_plain_text(), "Minestom Rust");

        let legacy = Component::from_legacy("§6Gold §lBold", '§');
        assert_eq!(legacy.to_plain_text(), "Gold Bold");
        assert!(legacy.to_legacy_string('§').contains("§6"));
    }
}
