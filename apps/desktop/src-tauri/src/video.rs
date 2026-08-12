use serde::Serialize;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct VideoStatus {
    pub camera_enabled: bool,
    pub screen_share_enabled: bool,
}

#[derive(Debug)]
pub(crate) struct VideoState {
    camera_enabled: bool,
    screen_share_enabled: bool,
}

impl VideoState {
    pub(crate) fn new() -> Self {
        Self {
            camera_enabled: false,
            screen_share_enabled: false,
        }
    }

    pub(crate) fn status(&self) -> VideoStatus {
        VideoStatus {
            camera_enabled: self.camera_enabled,
            screen_share_enabled: self.screen_share_enabled,
        }
    }

    pub(crate) fn set_camera_enabled(&mut self, enabled: bool) -> VideoStatus {
        self.camera_enabled = enabled;
        if enabled {
            self.screen_share_enabled = false;
        }
        self.status()
    }

    pub(crate) fn set_screen_share_enabled(&mut self, enabled: bool) -> VideoStatus {
        self.screen_share_enabled = enabled;
        if enabled {
            self.camera_enabled = false;
        }
        self.status()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enabling_camera_disables_screen_share() {
        let mut state = VideoState::new();
        state.set_screen_share_enabled(true);

        let status = state.set_camera_enabled(true);

        assert!(status.camera_enabled);
        assert!(!status.screen_share_enabled);
    }

    #[test]
    fn enabling_screen_share_disables_camera() {
        let mut state = VideoState::new();
        state.set_camera_enabled(true);

        let status = state.set_screen_share_enabled(true);

        assert!(!status.camera_enabled);
        assert!(status.screen_share_enabled);
    }

    #[test]
    fn disabling_flags_keeps_other_stream_off() {
        let mut state = VideoState::new();

        let camera_off = state.set_camera_enabled(false);
        assert!(!camera_off.camera_enabled);
        assert!(!camera_off.screen_share_enabled);

        let share_off = state.set_screen_share_enabled(false);
        assert!(!share_off.camera_enabled);
        assert!(!share_off.screen_share_enabled);
    }
}
