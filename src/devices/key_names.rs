//! The key names a SteelSeries device can name an action with, and the codes they stand for.
//!
//! One table for every device: a mouse asks what its buttons send, an onboard binding on a
//! keyboard asks what key or media action to send, and a headset would ask the same of whatever it
//! can bind. Whether a device accepts a name is that device's own question, not this table's.
//!
//! Ported from rivalcfg's `handlers/buttons/layout_qwerty.py` and `layout_multimedia.py` (WTFPL).
//! Names are stored lowercase because rivalcfg matches them case-insensitively. Keyboard codes are
//! HID keyboard usages; multimedia codes are HID consumer usages.

/// QWERTY keys: name -> HID keyboard usage.
const KEYBOARD: &[(&str, u8)] = &[
    // Alphanumeric
    ("a", 0x04),
    ("b", 0x05),
    ("c", 0x06),
    ("d", 0x07),
    ("e", 0x08),
    ("f", 0x09),
    ("g", 0x0A),
    ("h", 0x0B),
    ("i", 0x0C),
    ("j", 0x0D),
    ("k", 0x0E),
    ("l", 0x0F),
    ("m", 0x10),
    ("n", 0x11),
    ("o", 0x12),
    ("p", 0x13),
    ("q", 0x14),
    ("r", 0x15),
    ("s", 0x16),
    ("t", 0x17),
    ("u", 0x18),
    ("v", 0x19),
    ("w", 0x1A),
    ("x", 0x1B),
    ("y", 0x1C),
    ("z", 0x1D),
    ("1", 0x1E),
    ("2", 0x1F),
    ("3", 0x20),
    ("4", 0x21),
    ("5", 0x22),
    ("6", 0x23),
    ("7", 0x24),
    ("8", 0x25),
    ("9", 0x26),
    ("0", 0x27),
    // Editing
    ("enter", 0x28),
    ("escape", 0x29),
    ("backspace", 0x2A),
    ("tab", 0x2B),
    ("space", 0x2C),
    ("delete", 0x4C),
    // Symbols
    ("-", 0x2D),
    ("=", 0x2E),
    ("[", 0x2F),
    ("]", 0x30),
    ("\\", 0x31),
    ("#", 0x32),
    (";", 0x33),
    ("'", 0x34),
    ("`", 0x35),
    (",", 0x36),
    (".", 0x37),
    ("/", 0x38),
    ("\\(inter)", 0x64),
    // Typing mode
    ("capslock", 0x39),
    ("scrolllock", 0x47),
    ("insert", 0x49),
    ("numlock", 0x53),
    // Functions
    ("f1", 0x3A),
    ("f2", 0x3B),
    ("f3", 0x3C),
    ("f4", 0x3D),
    ("f5", 0x3E),
    ("f6", 0x3F),
    ("f7", 0x40),
    ("f8", 0x41),
    ("f9", 0x42),
    ("f10", 0x43),
    ("f11", 0x44),
    ("f12", 0x45),
    ("f13", 0x68),
    ("f14", 0x69),
    ("f15", 0x6A),
    ("f16", 0x6B),
    ("f17", 0x6C),
    ("f18", 0x6D),
    ("f19", 0x6E),
    ("f20", 0x6F),
    ("f21", 0x70),
    ("f22", 0x71),
    ("f23", 0x72),
    ("f24", 0x73),
    // Commands
    ("printscreen", 0x46),
    ("pausebreak", 0x48),
    ("contextmenu", 0x65),
    // Navigation
    ("home", 0x4A),
    ("pageup", 0x4B),
    ("end", 0x4D),
    ("pagedown", 0x4E),
    // Arrows
    ("right", 0x4F),
    ("left", 0x50),
    ("down", 0x51),
    ("up", 0x52),
    // Numpad
    ("keypad/", 0x54),
    ("keypad*", 0x55),
    ("keypad-", 0x56),
    ("keypad+", 0x57),
    ("keypadenter", 0x58),
    ("keypad1", 0x59),
    ("keypad2", 0x5A),
    ("keypad3", 0x5B),
    ("keypad4", 0x5C),
    ("keypad5", 0x5D),
    ("keypad6", 0x5E),
    ("keypad7", 0x5F),
    ("keypad8", 0x60),
    ("keypad9", 0x61),
    ("keypad0", 0x62),
    ("keypad.", 0x63),
    ("keypad,", 0x85),
    ("keypad=", 0x86),
    // Modifiers
    ("leftctrl", 0xE0),
    ("leftshift", 0xE1),
    ("leftalt", 0xE2),
    ("leftsuper", 0xE3),
    ("rightctrl", 0xE4),
    ("rightshift", 0xE5),
    ("rightalt", 0xE6),
    ("rightsuper", 0xE7),
];

/// Alternative names for QWERTY keys: alias -> name in [`KEYBOARD`].
const KEYBOARD_ALIASES: &[(&str, &str)] = &[
    ("esc", "escape"),
    ("bksp", "backspace"),
    ("bkspace", "backspace"),
    ("del", "delete"),
    ("dash", "-"),
    ("minus", "-"),
    ("equal", "="),
    ("eq", "="),
    ("leftbracket", "["),
    ("rightbracket", "]"),
    ("backslash", "\\"),
    ("hash", "#"),
    ("semicolon", ";"),
    ("semi", ";"),
    ("quote", "'"),
    ("backtick", "`"),
    ("backquote", "`"),
    ("comma", ","),
    ("dot", "."),
    ("point", "."),
    ("slash", "/"),
    ("capslck", "capslock"),
    ("capslk", "capslock"),
    ("cpslck", "capslock"),
    ("cpslk", "capslock"),
    ("scrolllck", "scrolllock"),
    ("scrolllk", "scrolllock"),
    ("scrllck", "scrolllock"),
    ("scrllk", "scrolllock"),
    ("scrlck", "scrolllock"),
    ("scrlk", "scrolllock"),
    ("ins", "insert"),
    ("num", "numlock"),
    ("numlck", "numlock"),
    ("numlk", "numlock"),
    ("prntscr", "printscreen"),
    ("prtscr", "printscreen"),
    ("prtsc", "printscreen"),
    ("psbrk", "pausebreak"),
    ("psbr", "pausebreak"),
    ("ctx", "contextmenu"),
    ("menu", "contextmenu"),
    ("ctxmenu", "contextmenu"),
    ("ctxmn", "contextmenu"),
    ("pgup", "pageup"),
    ("pgdown", "pagedown"),
    ("pgdwn", "pagedown"),
    ("pgdn", "pagedown"),
    ("lctrl", "leftctrl"),
    ("lshift", "leftshift"),
    ("lalt", "leftalt"),
    ("alt", "leftalt"),
    ("super", "leftsuper"),
    ("lsuper", "leftsuper"),
    ("windows", "leftsuper"),
    ("leftwindows", "leftsuper"),
    ("win", "leftsuper"),
    ("lwin", "leftsuper"),
    ("command", "leftsuper"),
    ("leftcommand", "leftsuper"),
    ("cmd", "leftsuper"),
    ("lcmd", "leftsuper"),
    ("rctrl", "rightctrl"),
    ("rshift", "rightshift"),
    ("ralt", "rightalt"),
    ("altgr", "rightalt"),
    ("rsuper", "rightsuper"),
    ("rightwindows", "rightsuper"),
    ("rwin", "rightsuper"),
    ("rightcommand", "rightsuper"),
    ("rcmd", "rightsuper"),
];

/// Multimedia keys: name -> HID consumer usage.
const MULTIMEDIA: &[(&str, u8)] = &[
    ("mute", 0xE2),
    ("next", 0xB5),
    ("playpause", 0xCD),
    ("previous", 0xB6),
    ("volumeup", 0xE9),
    ("volumedown", 0xEA),
];

/// Alternative names for multimedia keys: alias -> name in [`MULTIMEDIA`].
const MULTIMEDIA_ALIASES: &[(&str, &str)] = &[
    ("play", "playpause"),
    ("pause", "playpause"),
    ("prev", "previous"),
    ("volup", "volumeup"),
    ("volume+", "volumeup"),
    ("vol+", "volumeup"),
    ("volumeplus", "volumeup"),
    ("volplus", "volumeup"),
    ("voldown", "volumedown"),
    ("voldwn", "volumedown"),
    ("voldn", "volumedown"),
    ("volume-", "volumedown"),
    ("vol-", "volumedown"),
    ("volumeminus", "volumedown"),
    ("volminus", "volumedown"),
];

fn lookup(names: &[(&str, u8)], aliases: &[(&str, &str)], name: &str) -> Option<u8> {
    let name = name.to_ascii_lowercase();
    let canonical = aliases
        .iter()
        .find(|(alias, _)| *alias == name)
        .map_or(name.as_str(), |(_, target)| *target);
    names.iter().find(|(key, _)| *key == canonical).map(|(_, code)| *code)
}

/// HID keyboard usage of a key name or alias (case-insensitive).
pub fn keyboard_code(name: &str) -> Option<u8> {
    lookup(KEYBOARD, KEYBOARD_ALIASES, name)
}

/// HID consumer usage of a multimedia key name or alias (case-insensitive).
pub fn multimedia_code(name: &str) -> Option<u8> {
    lookup(MULTIMEDIA, MULTIMEDIA_ALIASES, name)
}

/// Every keyboard key name and alias, lowercase.
pub fn keyboard_names() -> impl Iterator<Item = &'static str> {
    KEYBOARD
        .iter()
        .map(|(name, _)| *name)
        .chain(KEYBOARD_ALIASES.iter().map(|(alias, _)| *alias))
}

/// Every multimedia key name and alias, lowercase.
pub fn multimedia_names() -> impl Iterator<Item = &'static str> {
    MULTIMEDIA
        .iter()
        .map(|(name, _)| *name)
        .chain(MULTIMEDIA_ALIASES.iter().map(|(alias, _)| *alias))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// rivalcfg's `build_layout` refuses aliases that point nowhere; ours must not have any.
    #[test]
    fn every_alias_resolves() {
        for (alias, target) in KEYBOARD_ALIASES {
            assert!(KEYBOARD.iter().any(|(k, _)| k == target), "{alias} -> {target}");
        }
        for (alias, target) in MULTIMEDIA_ALIASES {
            assert!(MULTIMEDIA.iter().any(|(k, _)| k == target), "{alias} -> {target}");
        }
    }

    #[test]
    fn names_are_lowercase_and_unique() {
        let all: Vec<&str> = keyboard_names().chain(multimedia_names()).collect();
        for name in &all {
            assert_eq!(*name, name.to_ascii_lowercase());
            assert_eq!(all.iter().filter(|n| *n == name).count(), 1, "{name} defined twice");
        }
    }

    /// Spot checks against rivalcfg's layout_qwerty.py / layout_multimedia.py.
    #[test]
    fn codes_match_rivalcfg_layouts() {
        assert_eq!(keyboard_code("A"), Some(0x04));
        assert_eq!(keyboard_code("Keypad="), Some(0x86));
        assert_eq!(keyboard_code("AltGr"), Some(0xE6));
        assert_eq!(keyboard_code("\\(inter)"), Some(0x64));
        assert_eq!(keyboard_code("PageDown"), Some(0x4E));
        assert_eq!(multimedia_code("VolumeDown"), Some(0xEA));
        assert_eq!(multimedia_code("pause"), Some(0xCD));
        assert_eq!(keyboard_code("mute"), None);
        assert_eq!(multimedia_code("a"), None);
    }
}
