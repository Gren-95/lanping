use super::Message;

/// Maps Ctrl+F, Enter and Esc to messages. Esc is handled even when a text field
/// has consumed it, so it also clears the filters from inside the search box.
pub(super) fn shortcut_message(
    event: iced::Event,
    status: iced::event::Status,
    _window: iced::window::Id,
) -> Option<Message> {
    use iced::keyboard::{Event, Key, key::Named};
    let iced::Event::Keyboard(Event::KeyPressed { key, modifiers, .. }) = event else {
        return None;
    };
    match key.as_ref() {
        Key::Named(Named::Escape) => Some(Message::EscapePressed),
        Key::Character("f") if modifiers.command() => Some(Message::FocusSearch),
        Key::Named(Named::Enter) if status == iced::event::Status::Ignored => {
            Some(Message::EnterPressed)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(key: iced::keyboard::Key, modifiers: iced::keyboard::Modifiers) -> iced::Event {
        use iced::keyboard::{Event, Location, key::Physical};
        iced::Event::Keyboard(Event::KeyPressed {
            key: key.clone(),
            modified_key: key,
            physical_key: Physical::Unidentified(iced::keyboard::key::NativeCode::Unidentified),
            location: Location::Standard,
            modifiers,
            text: None,
            repeat: false,
        })
    }

    #[test]
    fn shortcuts_map_to_messages() {
        use iced::event::Status::{Captured, Ignored};
        use iced::keyboard::{Key, Modifiers, key::Named};
        let id = iced::window::Id::unique();
        let map = |key, modifiers, status| shortcut_message(press(key, modifiers), status, id);
        assert!(matches!(
            map(Key::Named(Named::Escape), Modifiers::empty(), Captured),
            Some(Message::EscapePressed)
        ));
        assert!(matches!(
            map(Key::Character("f".into()), Modifiers::CTRL, Ignored),
            Some(Message::FocusSearch)
        ));
        assert!(map(Key::Character("f".into()), Modifiers::empty(), Ignored).is_none());
        assert!(matches!(
            map(Key::Named(Named::Enter), Modifiers::empty(), Ignored),
            Some(Message::EnterPressed)
        ));
        assert!(map(Key::Named(Named::Enter), Modifiers::empty(), Captured).is_none());
    }
}
