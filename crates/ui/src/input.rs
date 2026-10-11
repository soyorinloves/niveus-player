//! Keyboard input to [`Action`], via the keymap.
//!
//! The keymap is the only source of bindings, so what the status bar prints and
//! what actually fires cannot disagree.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use niveus_config::{Action, Key, Keymap};

/// Translates a terminal key press into the keymap's own key vocabulary.
///
/// Returns `None` for anything the keymap cannot express, which is how
/// unsupported combinations (Alt-modified keys, media keys) fall through
/// harmlessly.
pub fn to_key(event: KeyEvent) -> Option<Key> {
    // Alt combinations are reserved: nothing is bound to them, and on many
    // terminals they are indistinguishable from escape sequences.
    if event.modifiers.contains(KeyModifiers::ALT) {
        return None;
    }
    let ctrl = event.modifiers.contains(KeyModifiers::CONTROL);
    // Shift is deliberately ignored: it is already reflected in the character
    // itself, which is what keeps `j` and `J` distinct bindings.
    match event.code {
        KeyCode::Char(c) if ctrl => Some(Key::Ctrl(c)),
        // The space bar arrives as a character, but is bound by name.
        KeyCode::Char(' ') => Some(Key::Space),
        KeyCode::Char(c) => Some(Key::Char(c)),
        KeyCode::Enter => Some(Key::Enter),
        KeyCode::Esc => Some(Key::Esc),
        KeyCode::Backspace => Some(Key::Backspace),
        KeyCode::Tab => Some(Key::Tab),
        KeyCode::Delete => Some(Key::Delete),
        KeyCode::Insert => Some(Key::Insert),
        KeyCode::Home => Some(Key::Home),
        KeyCode::End => Some(Key::End),
        KeyCode::PageUp => Some(Key::PageUp),
        KeyCode::PageDown => Some(Key::PageDown),
        KeyCode::Up => Some(Key::Up),
        KeyCode::Down => Some(Key::Down),
        KeyCode::Left => Some(Key::Left),
        KeyCode::Right => Some(Key::Right),
        KeyCode::F(n) if (1..=12).contains(&n) => Some(Key::Function(n)),
        _ => None,
    }
}

/// The action bound to a key press, if any.
pub fn map_key(event: KeyEvent, keymap: &Keymap) -> Option<Action> {
    keymap.action_for(to_key(event)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn default_bindings_resolve_to_their_actions() {
        let keymap = Keymap::default();
        assert_eq!(
            map_key(press(KeyCode::Char('1')), &keymap),
            Some(Action::ShowPlaylist)
        );
        assert_eq!(
            map_key(press(KeyCode::Char(' ')), &keymap),
            Some(Action::PlayPause)
        );
        assert_eq!(map_key(press(KeyCode::Esc), &keymap), Some(Action::Back));
        assert_eq!(
            map_key(press(KeyCode::Enter), &keymap),
            Some(Action::Select)
        );
        assert_eq!(map_key(press(KeyCode::Up), &keymap), Some(Action::MoveUp));
    }

    #[test]
    fn letter_case_survives_translation() {
        let keymap = Keymap::default();
        assert_eq!(
            map_key(press(KeyCode::Char('m')), &keymap),
            Some(Action::ToggleMetadata)
        );
        // Upper case is a different key and is not bound by default.
        assert_eq!(map_key(press(KeyCode::Char('M')), &keymap), None);
    }

    #[test]
    fn shift_is_folded_into_the_character() {
        let event = KeyEvent::new(KeyCode::Char('L'), KeyModifiers::SHIFT);
        assert_eq!(to_key(event), Some(Key::Char('L')));
    }

    #[test]
    fn control_combinations_become_control_keys() {
        let event = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(to_key(event), Some(Key::Ctrl('c')));
        // Ctrl+C is not in the keymap; the app handles it as a fixed fallback.
        assert_eq!(map_key(event, &Keymap::default()), None);
    }

    #[test]
    fn alt_combinations_and_unknown_keys_are_ignored() {
        let keymap = Keymap::default();
        let alt = KeyEvent::new(KeyCode::Char('1'), KeyModifiers::ALT);
        assert_eq!(map_key(alt, &keymap), None);
        assert_eq!(map_key(press(KeyCode::Null), &keymap), None);
    }

    #[test]
    fn function_keys_are_carried_across() {
        assert_eq!(to_key(press(KeyCode::F(5))), Some(Key::Function(5)));
        assert_eq!(to_key(press(KeyCode::F(13))), None);
    }

    #[test]
    fn a_rebound_key_reports_the_new_action() {
        let mut keymap = Keymap::default();
        keymap.rebind(Action::Quit, Key::Char('x')).unwrap();
        assert_eq!(
            map_key(press(KeyCode::Char('x')), &keymap),
            Some(Action::Quit)
        );
        // The old binding is gone, so the hint and the behaviour stay in step.
        assert_eq!(map_key(press(KeyCode::Char('q')), &keymap), None);
    }
}
