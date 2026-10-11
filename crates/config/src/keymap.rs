//! The keymap: the single source of truth for keyboard shortcuts.
//!
//! `Keyboard Input -> Keymap -> Action -> AppState -> UI`. The UI reads the same
//! map to render its `[X]` hints, so a displayed key can never drift from the
//! binding that actually fires.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

/// A key as bound in `config.toml`.
///
/// Case is significant: `j` and `J` are different keys, which is what lets the
/// queue's reorder bindings sit alongside plain list movement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Key {
    Char(char),
    Space,
    Enter,
    Esc,
    Backspace,
    Tab,
    Delete,
    Insert,
    Home,
    End,
    PageUp,
    PageDown,
    Up,
    Down,
    Left,
    Right,
    Function(u8),
    Ctrl(char),
}

impl Key {
    /// Label used in the UI's `[X]` hints. Letters are upper-cased to match the
    /// document's conventions (`[L]`, `[M]`, `[O]`).
    pub fn label(self) -> String {
        match self {
            Key::Char(c) if c.is_alphabetic() => c.to_uppercase().to_string(),
            Key::Char(c) => c.to_string(),
            Key::Space => "Space".into(),
            Key::Enter => "Enter".into(),
            Key::Esc => "Esc".into(),
            Key::Backspace => "Bksp".into(),
            Key::Tab => "Tab".into(),
            Key::Delete => "Del".into(),
            Key::Insert => "Ins".into(),
            Key::Home => "Home".into(),
            Key::End => "End".into(),
            Key::PageUp => "PgUp".into(),
            Key::PageDown => "PgDn".into(),
            Key::Up => "↑".into(),
            Key::Down => "↓".into(),
            Key::Left => "←".into(),
            Key::Right => "→".into(),
            Key::Function(n) => format!("F{n}"),
            Key::Ctrl(c) => format!("Ctrl+{}", c.to_uppercase()),
        }
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label())
    }
}

impl Key {
    /// The key as it is written in `config.toml`.
    ///
    /// The inverse of [`FromStr`], and deliberately not the label the UI shows:
    /// that one is for people reading the screen, this one for people editing a
    /// file — and a round trip through the file has to come back unchanged.
    pub fn as_config(self) -> String {
        match self {
            Key::Char(c) => c.to_string(),
            Key::Space => "space".into(),
            Key::Enter => "enter".into(),
            Key::Esc => "esc".into(),
            Key::Backspace => "backspace".into(),
            Key::Tab => "tab".into(),
            Key::Delete => "delete".into(),
            Key::Insert => "insert".into(),
            Key::Home => "home".into(),
            Key::End => "end".into(),
            Key::PageUp => "pageup".into(),
            Key::PageDown => "pagedown".into(),
            Key::Up => "up".into(),
            Key::Down => "down".into(),
            Key::Left => "left".into(),
            Key::Right => "right".into(),
            Key::Function(n) => format!("f{n}"),
            Key::Ctrl(c) => format!("ctrl+{c}"),
        }
    }
}

impl FromStr for Key {
    type Err = KeymapError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(KeymapError::UnknownKey(raw.to_string()));
        }
        if let Some(rest) = trimmed
            .strip_prefix("ctrl+")
            .or_else(|| trimmed.strip_prefix("Ctrl+"))
        {
            let mut chars = rest.chars();
            return match (chars.next(), chars.next()) {
                (Some(c), None) => Ok(Key::Ctrl(c)),
                _ => Err(KeymapError::UnknownKey(raw.to_string())),
            };
        }

        let named = match trimmed.to_ascii_lowercase().as_str() {
            "space" => Some(Key::Space),
            "enter" | "return" => Some(Key::Enter),
            "esc" | "escape" => Some(Key::Esc),
            "backspace" => Some(Key::Backspace),
            "tab" => Some(Key::Tab),
            "delete" => Some(Key::Delete),
            "insert" => Some(Key::Insert),
            "home" => Some(Key::Home),
            "end" => Some(Key::End),
            "pageup" => Some(Key::PageUp),
            "pagedown" => Some(Key::PageDown),
            "up" => Some(Key::Up),
            "down" => Some(Key::Down),
            "left" => Some(Key::Left),
            "right" => Some(Key::Right),
            _ => None,
        };
        if let Some(key) = named {
            return Ok(key);
        }

        let lowered = trimmed.to_ascii_lowercase();
        if let Some(digits) = lowered.strip_prefix('f') {
            if let Ok(n) = digits.parse::<u8>() {
                if (1..=12).contains(&n) {
                    return Ok(Key::Function(n));
                }
            }
        }

        let mut chars = trimmed.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => Ok(Key::Char(c)),
            _ => Err(KeymapError::UnknownKey(raw.to_string())),
        }
    }
}

/// Everything the user can trigger from the keyboard.
///
/// A single global namespace, as the first release has no page- or
/// component-scoped keymaps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    // Navigation between top-level pages.
    ShowPlaylist,
    ShowFavorites,
    ShowHistory,
    OpenSettings,
    // Panels and views. The metadata panel is a switch rather than a view of
    // its own, so there is one action here, not two (SPEC §12.1, §24).
    ToggleMetadata,
    ToggleQueue,
    Back,
    // List movement and activation.
    MoveUp,
    MoveDown,
    Select,
    // Queue management, in the queue view only.
    DeleteSong,
    ReorderUp,
    ReorderDown,
    // Playback control.
    PlayPause,
    Next,
    Previous,
    SeekForward,
    SeekBackward,
    VolumeUp,
    VolumeDown,
    CyclePlaybackMode,
    PlayNext,
    // Leave.
    Quit,
}

impl Action {
    /// Every action, used by keymap validation.
    pub const ALL: [Action; 23] = [
        Action::ShowPlaylist,
        Action::ShowFavorites,
        Action::ShowHistory,
        Action::OpenSettings,
        Action::ToggleMetadata,
        Action::ToggleQueue,
        Action::Back,
        Action::MoveUp,
        Action::MoveDown,
        Action::Select,
        Action::DeleteSong,
        Action::ReorderUp,
        Action::ReorderDown,
        Action::PlayPause,
        Action::Next,
        Action::Previous,
        Action::SeekForward,
        Action::SeekBackward,
        Action::VolumeUp,
        Action::VolumeDown,
        Action::CyclePlaybackMode,
        Action::PlayNext,
        Action::Quit,
    ];

    /// Name used as the key in `config.toml`'s `[keybindings]` table.
    pub fn name(self) -> &'static str {
        match self {
            Action::ShowPlaylist => "playlist",
            Action::ShowFavorites => "favorites",
            Action::ShowHistory => "history",
            Action::OpenSettings => "settings",
            Action::ToggleMetadata => "metadata",
            Action::ToggleQueue => "queue",
            Action::Back => "back",
            Action::MoveUp => "move_up",
            Action::MoveDown => "move_down",
            Action::Select => "select",
            Action::DeleteSong => "delete_song",
            Action::ReorderUp => "reorder_up",
            Action::ReorderDown => "reorder_down",
            Action::PlayPause => "play_pause",
            Action::Next => "next",
            Action::Previous => "previous",
            Action::SeekForward => "seek_forward",
            Action::SeekBackward => "seek_backward",
            Action::VolumeUp => "volume_up",
            Action::VolumeDown => "volume_down",
            Action::CyclePlaybackMode => "playback_mode",
            Action::PlayNext => "play_next",
            Action::Quit => "quit",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|a| a.name() == name)
    }
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum KeymapError {
    #[error("未知按键: {0}")]
    UnknownKey(String),
    #[error("未知操作: {0}")]
    UnknownAction(String),
    #[error("按键冲突: {key} 已绑定到 {existing}，不能同时绑定 {incoming}")]
    Conflict {
        key: Key,
        existing: Action,
        incoming: Action,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keymap {
    bindings: BTreeMap<Action, Key>,
}

impl Default for Keymap {
    /// The documented default bindings.
    fn default() -> Self {
        use Action::*;
        let entries = [
            (ShowPlaylist, "1"),
            (ShowFavorites, "2"),
            (ShowHistory, "3"),
            (OpenSettings, "4"),
            (ToggleMetadata, "m"),
            (ToggleQueue, "u"),
            (Back, "esc"),
            (MoveUp, "up"),
            (MoveDown, "down"),
            (Select, "enter"),
            // Queue management. Case matters: `d` deletes, while `J` and `K`
            // reorder, which leaves the plain letters free for anything else.
            (DeleteSong, "d"),
            (ReorderUp, "K"),
            (ReorderDown, "J"),
            (PlayPause, "space"),
            (Next, "n"),
            (Previous, "p"),
            (SeekForward, "right"),
            (SeekBackward, "left"),
            (VolumeUp, "="),
            (VolumeDown, "-"),
            (CyclePlaybackMode, "o"),
            (PlayNext, "insert"),
            (Quit, "q"),
        ];
        let mut bindings = BTreeMap::new();
        for (action, key) in entries {
            let key = Key::from_str(key).expect("default bindings are valid keys");
            bindings.insert(action, key);
        }
        Self { bindings }
    }
}

impl Keymap {
    pub fn key_for(&self, action: Action) -> Option<Key> {
        self.bindings.get(&action).copied()
    }

    /// The action bound to `key`, if any.
    pub fn action_for(&self, key: Key) -> Option<Action> {
        self.bindings
            .iter()
            .find(|(_, bound)| **bound == key)
            .map(|(action, _)| *action)
    }

    /// Rebinds `action`, rejecting a key already held by a different action.
    pub fn rebind(&mut self, action: Action, key: Key) -> Result<(), KeymapError> {
        if let Some(existing) = self.action_for(key) {
            if existing != action {
                return Err(KeymapError::Conflict {
                    key,
                    existing,
                    incoming: action,
                });
            }
        }
        self.bindings.insert(action, key);
        Ok(())
    }

    pub fn iter(&self) -> impl Iterator<Item = (Action, Key)> + '_ {
        self.bindings.iter().map(|(a, k)| (*a, *k))
    }

    /// Applies `[keybindings]` entries read from `config.toml`.
    ///
    /// Unknown action names and malformed keys are rejected individually so one
    /// bad line cannot cost the user the rest of their configuration.
    /// Applies `[keybindings]` entries read from `config.toml`.
    ///
    /// The entries are taken as a **set**, not one after another. Two of them
    /// can only make sense together — an action taking a key another action is
    /// giving up — and applying them in turn would reject whichever came
    /// second, which would depend on the order a hash map happened to hand them
    /// over in. So every overridden action lets go of its key first, and only
    /// then are the new ones taken.
    ///
    /// Unknown action names and malformed keys are rejected individually so one
    /// bad line cannot cost the user the rest of their configuration.
    pub fn apply_overrides(
        &mut self,
        overrides: &std::collections::HashMap<String, String>,
    ) -> Vec<KeymapError> {
        let mut errors = Vec::new();
        let mut wanted: Vec<(Action, Key)> = Vec::new();

        for (name, raw_key) in overrides {
            let Some(action) = Action::from_name(name) else {
                errors.push(KeymapError::UnknownAction(name.clone()));
                continue;
            };
            match Key::from_str(raw_key) {
                Ok(key) => wanted.push((action, key)),
                Err(_) => errors.push(KeymapError::UnknownKey(raw_key.clone())),
            }
        }

        // Sorted, so that the same file always produces the same map: which of
        // two entries claiming one key wins must not be a matter of chance.
        wanted.sort_by_key(|(action, _)| *action);

        // Let go of the old keys first — all of them, before any new one is
        // taken.
        let mut released: Vec<(Action, Key)> = Vec::new();
        for (action, _) in &wanted {
            if let Some(key) = self.bindings.remove(action) {
                released.push((*action, key));
            }
        }

        for (action, key) in wanted {
            if let Some(existing) = self.action_for(key) {
                if existing != action {
                    errors.push(KeymapError::Conflict {
                        key,
                        existing,
                        incoming: action,
                    });
                    continue;
                }
            }
            self.bindings.insert(action, key);
        }

        // An entry that could not be honoured — the key is held by an action
        // the file says nothing about, say — leaves its action with the binding
        // it had, so that one bad line costs only that line.
        for (action, key) in released {
            if !self.bindings.contains_key(&action) && self.action_for(key).is_none() {
                self.bindings.insert(action, key);
            }
        }

        errors
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn defaults_match_the_documented_bindings() {
        let map = Keymap::default();
        assert_eq!(map.key_for(Action::ShowPlaylist), Some(Key::Char('1')));
        assert_eq!(map.key_for(Action::Back), Some(Key::Esc));
        assert_eq!(map.key_for(Action::PlayPause), Some(Key::Space));
        assert_eq!(map.key_for(Action::VolumeUp), Some(Key::Char('=')));
        assert_eq!(map.key_for(Action::VolumeDown), Some(Key::Char('-')));
        assert_eq!(map.key_for(Action::PlayNext), Some(Key::Insert));
        assert_eq!(map.key_for(Action::SeekForward), Some(Key::Right));
    }

    /// The three queue-management keys the design fixes in its queue view:
    /// delete, move up, move down. Their case is what keeps them from colliding
    /// with one another and with the rest of the map.
    #[test]
    fn the_queue_management_keys_match_the_design() {
        let map = Keymap::default();
        assert_eq!(map.key_for(Action::DeleteSong), Some(Key::Char('d')));
        assert_eq!(map.key_for(Action::ReorderUp), Some(Key::Char('K')));
        assert_eq!(map.key_for(Action::ReorderDown), Some(Key::Char('J')));
    }

    #[test]
    fn every_action_has_a_default_binding() {
        let map = Keymap::default();
        for action in Action::ALL {
            assert!(map.key_for(action).is_some(), "{action} is unbound");
        }
    }

    #[test]
    fn no_default_binding_collides() {
        let map = Keymap::default();
        let mut seen = std::collections::HashSet::new();
        for (action, key) in map.iter() {
            assert!(seen.insert(key), "{action} reuses a key");
        }
    }

    #[test]
    fn action_names_round_trip() {
        for action in Action::ALL {
            assert_eq!(Action::from_name(action.name()), Some(action));
        }
    }

    #[test]
    fn key_parsing_covers_named_keys_and_letters() {
        assert_eq!(Key::from_str("space").unwrap(), Key::Space);
        assert_eq!(Key::from_str("ESC").unwrap(), Key::Esc);
        assert_eq!(Key::from_str("insert").unwrap(), Key::Insert);
        assert_eq!(Key::from_str("f5").unwrap(), Key::Function(5));
        assert_eq!(Key::from_str("=").unwrap(), Key::Char('='));
        assert_eq!(Key::from_str("ctrl+c").unwrap(), Key::Ctrl('c'));
        assert!(Key::from_str("not-a-key").is_err());
        assert!(Key::from_str("").is_err());
    }

    #[test]
    fn case_is_significant() {
        assert_ne!(Key::from_str("j").unwrap(), Key::from_str("J").unwrap());
    }

    #[test]
    fn a_key_written_to_the_file_reads_back_as_itself() {
        // The file the program writes has to be one it can read: a key that
        // came back as something else would silently rebind on the next start.
        for action in Action::ALL {
            let key = Keymap::default().key_for(action).unwrap();
            assert_eq!(
                Key::from_str(&key.as_config()),
                Ok(key),
                "{action} does not survive a round trip through the file"
            );
        }
        for key in [
            Key::Space,
            Key::Function(7),
            Key::Ctrl('c'),
            Key::PageDown,
            Key::Char('='),
            Key::Char('J'),
        ] {
            assert_eq!(Key::from_str(&key.as_config()), Ok(key), "{key:?}");
        }
    }

    #[test]
    fn labels_follow_the_ui_convention() {
        assert_eq!(Key::Char('l').label(), "L");
        assert_eq!(Key::Space.label(), "Space");
        assert_eq!(Key::Char('=').label(), "=");
        assert_eq!(Key::Right.label(), "→");
    }

    #[test]
    fn rebinding_reports_a_conflict_instead_of_shadowing() {
        let mut map = Keymap::default();
        let err = map.rebind(Action::Next, Key::Char('q')).unwrap_err();
        assert_eq!(
            err,
            KeymapError::Conflict {
                key: Key::Char('q'),
                existing: Action::Quit,
                incoming: Action::Next,
            }
        );
        // The original binding is untouched.
        assert_eq!(map.key_for(Action::Quit), Some(Key::Char('q')));
    }

    #[test]
    fn rebinding_to_the_same_action_is_a_no_op() {
        let mut map = Keymap::default();
        assert!(map.rebind(Action::Quit, Key::Char('q')).is_ok());
    }

    #[test]
    fn overrides_are_taken_as_a_set_rather_than_one_at_a_time() {
        // `next` moves onto the key `quit` is leaving. Applied one after the
        // other against the defaults, whichever came second would be rejected
        // — and which that was would depend on the iteration order.
        let mut overrides = std::collections::HashMap::new();
        overrides.insert("quit".to_string(), "x".to_string());
        overrides.insert("next".to_string(), "q".to_string());

        let mut map = Keymap::default();
        let errors = map.apply_overrides(&overrides);

        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(map.key_for(Action::Quit), Some(Key::Char('x')));
        assert_eq!(map.key_for(Action::Next), Some(Key::Char('q')));
    }

    #[test]
    fn a_key_already_held_by_an_untouched_action_is_still_refused() {
        // `1` belongs to the playlist page, which the file says nothing about.
        let mut overrides = std::collections::HashMap::new();
        overrides.insert("next".to_string(), "1".to_string());

        let mut map = Keymap::default();
        let errors = map.apply_overrides(&overrides);

        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(map.key_for(Action::ShowPlaylist), Some(Key::Char('1')));
        // And the action that was refused keeps the binding it had, rather
        // than being left with none at all.
        assert_eq!(map.key_for(Action::Next), Some(Key::Char('n')));
    }

    #[test]
    fn a_file_that_contradicts_itself_is_reported_rather_than_guessed_at() {
        let mut overrides = std::collections::HashMap::new();
        overrides.insert("quit".to_string(), "z".to_string());
        overrides.insert("next".to_string(), "z".to_string());

        let mut map = Keymap::default();
        let errors = map.apply_overrides(&overrides);

        assert_eq!(errors.len(), 1, "{errors:?}");
        // One of the two was honoured, and never both: two actions sharing a
        // key is the one thing the map must not contain.
        let holders = [Action::Quit, Action::Next]
            .iter()
            .filter(|action| map.key_for(**action) == Some(Key::Char('z')))
            .count();
        assert_eq!(holders, 1, "{:?}", map.iter().collect::<Vec<_>>());
    }

    #[test]
    fn overrides_report_bad_entries_without_discarding_good_ones() {
        let mut overrides = std::collections::HashMap::new();
        overrides.insert("quit".to_string(), "x".to_string());
        overrides.insert("nonsense".to_string(), "z".to_string());
        overrides.insert("next".to_string(), "not-a-key".to_string());

        let mut map = Keymap::default();
        let errors = map.apply_overrides(&overrides);

        assert_eq!(errors.len(), 2);
        assert!(errors.contains(&KeymapError::UnknownAction("nonsense".into())));
        assert!(errors.contains(&KeymapError::UnknownKey("not-a-key".into())));
        assert_eq!(map.key_for(Action::Quit), Some(Key::Char('x')));
        // The rejected binding left the default in place.
        assert_eq!(map.key_for(Action::Next), Some(Key::Char('n')));
    }
}
