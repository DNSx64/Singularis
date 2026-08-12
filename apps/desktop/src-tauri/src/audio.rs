use serde::Serialize;
use thiserror::Error;

const MAX_ROOM_ID_BYTES: usize = 64;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct AudioStatus {
    pub muted: bool,
    pub deafened: bool,
    pub push_to_talk: bool,
    pub ptt_pressed: bool,
    pub joined_room: Option<String>,
}

#[derive(Debug)]
pub(crate) struct AudioState {
    muted: bool,
    deafened: bool,
    push_to_talk: bool,
    ptt_pressed: bool,
    joined_room: Option<String>,
}

#[derive(Debug, Error)]
pub(crate) enum AudioError {
    #[error("invalid voice room identifier")]
    InvalidRoom,
}

impl AudioError {
    pub(crate) const fn code(&self) -> &'static str {
        match self {
            Self::InvalidRoom => "invalid_audio_room",
        }
    }

    pub(crate) const fn public_message(&self) -> &'static str {
        match self {
            Self::InvalidRoom => "Der ausgewaehlte Sprachraum ist ungueltig.",
        }
    }
}

impl AudioState {
    pub(crate) fn new() -> Self {
        Self {
            muted: false,
            deafened: false,
            push_to_talk: false,
            ptt_pressed: false,
            joined_room: None,
        }
    }

    pub(crate) fn status(&self) -> AudioStatus {
        AudioStatus {
            muted: self.deafened || self.muted || (self.push_to_talk && !self.ptt_pressed),
            deafened: self.deafened,
            push_to_talk: self.push_to_talk,
            ptt_pressed: self.push_to_talk && self.ptt_pressed,
            joined_room: self.joined_room.clone(),
        }
    }

    pub(crate) fn set_muted(&mut self, muted: bool) -> AudioStatus {
        self.muted = muted;
        self.status()
    }

    pub(crate) fn set_deafened(&mut self, deafened: bool) -> AudioStatus {
        self.deafened = deafened;
        self.status()
    }

    pub(crate) fn set_push_to_talk(&mut self, enabled: bool) -> AudioStatus {
        self.push_to_talk = enabled;
        if !enabled {
            self.ptt_pressed = false;
        }
        self.status()
    }

    pub(crate) fn set_ptt_pressed(&mut self, pressed: bool) -> AudioStatus {
        self.ptt_pressed = self.push_to_talk && pressed;
        self.status()
    }

    pub(crate) fn join_room(&mut self, room_id: &str) -> Result<AudioStatus, AudioError> {
        validate_room_id(room_id)?;
        self.joined_room = Some(room_id.to_owned());
        Ok(self.status())
    }

    pub(crate) fn leave_room(&mut self) -> AudioStatus {
        self.joined_room = None;
        self.ptt_pressed = false;
        self.status()
    }
}

fn validate_room_id(room_id: &str) -> Result<(), AudioError> {
    if room_id.is_empty() || room_id.len() > MAX_ROOM_ID_BYTES {
        return Err(AudioError::InvalidRoom);
    }
    let allowed = room_id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-');
    if !allowed {
        return Err(AudioError::InvalidRoom);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deafening_forces_mute_and_blocks_unmute() {
        let mut state = AudioState::new();
        let deafened = state.set_deafened(true);
        assert!(deafened.deafened);
        assert!(deafened.muted);

        let unmute_attempt = state.set_muted(false);
        assert!(unmute_attempt.deafened);
        assert!(unmute_attempt.muted);
    }

    #[test]
    fn undeafen_restores_manual_mute_control() {
        let mut state = AudioState::new();
        state.set_deafened(true);
        let status = state.set_deafened(false);
        assert!(!status.deafened);
        assert!(!status.muted);

        let muted = state.set_muted(true);
        assert!(!muted.deafened);
        assert!(muted.muted);

        let unmuted = state.set_muted(false);
        assert!(!unmuted.muted);
    }

    #[test]
    fn joining_and_leaving_voice_rooms_is_persistent() {
        let mut state = AudioState::new();
        let joined = state.join_room("lounge").unwrap();
        assert_eq!(joined.joined_room.as_deref(), Some("lounge"));

        let switched = state.join_room("pairing").unwrap();
        assert_eq!(switched.joined_room.as_deref(), Some("pairing"));

        let left = state.leave_room();
        assert_eq!(left.joined_room, None);
    }

    #[test]
    fn invalid_room_identifiers_are_rejected() {
        let mut state = AudioState::new();
        assert!(matches!(state.join_room(""), Err(AudioError::InvalidRoom)));
        assert!(matches!(
            state.join_room("room with spaces"),
            Err(AudioError::InvalidRoom)
        ));
        assert!(matches!(
            state.join_room(&"a".repeat(MAX_ROOM_ID_BYTES + 1)),
            Err(AudioError::InvalidRoom)
        ));
    }

    #[test]
    fn push_to_talk_requires_a_pressed_key() {
        let mut state = AudioState::new();
        state.join_room("lounge").unwrap();
        let enabled = state.set_push_to_talk(true);
        assert!(enabled.push_to_talk);
        assert!(enabled.muted);
        assert!(!enabled.ptt_pressed);

        let pressed = state.set_ptt_pressed(true);
        assert!(pressed.push_to_talk);
        assert!(pressed.ptt_pressed);
        assert!(!pressed.muted);

        let released = state.set_ptt_pressed(false);
        assert!(released.push_to_talk);
        assert!(!released.ptt_pressed);
        assert!(released.muted);
    }

    #[test]
    fn disabling_push_to_talk_keeps_manual_mute_state() {
        let mut state = AudioState::new();
        state.join_room("pairing").unwrap();
        state.set_push_to_talk(true);
        state.set_ptt_pressed(true);
        state.set_muted(true);
        let disabled = state.set_push_to_talk(false);

        assert!(!disabled.push_to_talk);
        assert!(!disabled.ptt_pressed);
        assert!(disabled.muted);
    }

    #[test]
    fn audio_state_round_trip_matches_ui_expectations() {
        let mut state = AudioState::new();
        assert_eq!(state.status().joined_room, None);
        assert!(!state.status().muted);
        assert!(!state.status().deafened);
        assert!(!state.status().push_to_talk);
        assert!(!state.status().ptt_pressed);

        state.join_room("lounge").unwrap();
        state.set_muted(true);
        assert_eq!(state.status().joined_room.as_deref(), Some("lounge"));
        assert!(state.status().muted);

        state.set_deafened(true);
        assert!(state.status().deafened);
        assert!(state.status().muted);

        state.set_deafened(false);
        state.set_muted(false);
        let left = state.leave_room();
        assert_eq!(left.joined_room, None);
        assert!(!left.deafened);
        assert!(!left.muted);
        assert!(!left.push_to_talk);
        assert!(!left.ptt_pressed);
    }
}
