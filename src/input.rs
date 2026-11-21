//! Mouse capture/release and menu input state machine matching Perfect-World-Explorer.
//!
//! Controls:
//! - Left-click in the 3D viewport captures and hides the mouse cursor for looking.
//! - Escape releases the mouse and opens the ESC menu (`MenuPage::Main`).
//! - Escape from Settings / Modules / Teleport List returns to `MenuPage::Main`.
//! - Escape from `MenuPage::Main` (or `WorldMap`) closes the menu and returns to the world.
//! - `M` toggles the World Map (releasing mouse capture if active).
//! - Camera movement and mouse look are ONLY applied while the mouse is captured, the
//!   window is focused, the menu is closed, and no world load is in progress.

use crate::camera::{CameraInput, FreeCamera};

/// In-world menu state matching `Perfect-World-Explorer/src/gpu.rs` `MenuPage`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuPage {
    Closed,
    Main,
    Modules,
    Settings,
    Teleports,
    WorldMap,
}

impl MenuPage {
    pub fn escape(self) -> Self {
        match self {
            Self::Closed => Self::Main,
            Self::Main => Self::Closed,
            Self::Modules | Self::Settings | Self::Teleports => Self::Main,
            Self::WorldMap => Self::Closed,
        }
    }

    pub fn is_open(self) -> bool {
        self != Self::Closed
    }
}

/// Window cursor-capture transition requested by the input state machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureAction {
    None,
    Capture,
    Release,
}

/// Pure, headless-testable state machine governing mouse capture, menu navigation,
/// and camera input gating.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputStateMachine {
    pub menu_page: MenuPage,
    pub mouse_captured: bool,
    pub window_focused: bool,
    pub world_loading: bool,
}

impl Default for InputStateMachine {
    fn default() -> Self {
        Self {
            // Matches PW Explorer: menu starts open when entering the world, cursor free.
            menu_page: MenuPage::Main,
            mouse_captured: false,
            window_focused: true,
            world_loading: false,
        }
    }
}

impl InputStateMachine {
    pub fn new_world_ready() -> Self {
        Self {
            menu_page: MenuPage::Closed,
            mouse_captured: false,
            window_focused: true,
            world_loading: false,
        }
    }

    /// True only when normal 3D world camera controls (mouse look + WASD/flight) may run.
    pub fn allows_camera_input(&self) -> bool {
        self.window_focused
            && !self.menu_page.is_open()
            && !self.world_loading
            && self.mouse_captured
    }

    /// Left-click inside the 3D world viewport.
    pub fn on_left_click_in_viewport(&mut self) -> CaptureAction {
        if self.window_focused
            && !self.menu_page.is_open()
            && !self.world_loading
            && !self.mouse_captured
        {
            self.mouse_captured = true;
            CaptureAction::Capture
        } else {
            CaptureAction::None
        }
    }

    /// Escape key pressed (non-repeat).
    pub fn on_escape(&mut self) -> CaptureAction {
        if self.world_loading {
            return CaptureAction::None;
        }
        self.menu_page = self.menu_page.escape();
        if self.mouse_captured {
            self.mouse_captured = false;
            CaptureAction::Release
        } else {
            CaptureAction::None
        }
    }

    /// `M` key pressed (non-repeat) to toggle World Map.
    pub fn on_toggle_world_map(&mut self) -> CaptureAction {
        if self.world_loading || self.menu_page == MenuPage::Teleports {
            return CaptureAction::None;
        }
        self.menu_page = if self.menu_page == MenuPage::WorldMap {
            MenuPage::Closed
        } else {
            MenuPage::WorldMap
        };
        if self.mouse_captured {
            self.mouse_captured = false;
            CaptureAction::Release
        } else {
            CaptureAction::None
        }
    }

    /// Open a specific menu page (e.g., from an ESC menu button).
    pub fn on_set_menu_page(&mut self, page: MenuPage) -> CaptureAction {
        self.menu_page = page;
        if page.is_open() && self.mouse_captured {
            self.mouse_captured = false;
            CaptureAction::Release
        } else {
            CaptureAction::None
        }
    }

    /// Window focus gained or lost. Focus loss always releases mouse capture.
    pub fn on_focus_changed(&mut self, focused: bool) -> CaptureAction {
        self.window_focused = focused;
        if !focused && self.mouse_captured {
            self.mouse_captured = false;
            CaptureAction::Release
        } else {
            CaptureAction::None
        }
    }

    /// Apply keyboard/mouse camera input ONLY when captured and allowed.
    /// Returns `true` if input was applied to `camera`.
    pub fn apply_camera_input(
        &self,
        camera: &mut FreeCamera,
        input: &CameraInput,
        dt_seconds: f32,
    ) -> bool {
        if !self.allows_camera_input() {
            return false;
        }
        camera.step(input, dt_seconds);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn left_click_captures_mouse_and_gates_camera_input() {
        let mut sm = InputStateMachine::new_world_ready();
        let mut cam = FreeCamera::default();
        let start_pos = cam.position;
        let start_yaw = cam.yaw;

        // Before capture: mouse motion and WASD must NOT move or rotate the camera.
        let move_and_look = CameraInput {
            forward: true,
            mouse_dx: 40.0,
            mouse_dy: -15.0,
            ..Default::default()
        };
        assert!(!sm.allows_camera_input());
        assert!(!sm.apply_camera_input(&mut cam, &move_and_look, 0.1));
        assert_eq!(cam.position, start_pos);
        assert_eq!(cam.yaw, start_yaw);

        // Left-click in viewport captures mouse.
        assert_eq!(sm.on_left_click_in_viewport(), CaptureAction::Capture);
        assert!(sm.mouse_captured);
        assert!(sm.allows_camera_input());

        // Second click while already captured is a no-op.
        assert_eq!(sm.on_left_click_in_viewport(), CaptureAction::None);

        // Now camera moves and rotates on mouse delta + WASD.
        assert!(sm.apply_camera_input(&mut cam, &move_and_look, 0.1));
        assert_ne!(cam.position, start_pos);
        assert_ne!(cam.yaw, start_yaw);
    }

    #[test]
    fn escape_releases_mouse_opens_menu_and_unwinds_subpages() {
        let mut sm = InputStateMachine::new_world_ready();
        assert_eq!(sm.on_left_click_in_viewport(), CaptureAction::Capture);

        // Escape releases mouse and opens Main ESC menu.
        assert_eq!(sm.on_escape(), CaptureAction::Release);
        assert!(!sm.mouse_captured);
        assert_eq!(sm.menu_page, MenuPage::Main);
        assert!(!sm.allows_camera_input());

        // Clicking while menu is open must NOT capture mouse.
        assert_eq!(sm.on_left_click_in_viewport(), CaptureAction::None);

        // Open Settings -> Escape returns to Main -> Escape again closes menu.
        sm.on_set_menu_page(MenuPage::Settings);
        assert_eq!(sm.menu_page, MenuPage::Settings);
        assert_eq!(sm.on_escape(), CaptureAction::None);
        assert_eq!(sm.menu_page, MenuPage::Main);
        assert_eq!(sm.on_escape(), CaptureAction::None);
        assert_eq!(sm.menu_page, MenuPage::Closed);

        // Modules & Teleports also return to Main on Escape.
        for sub in [MenuPage::Modules, MenuPage::Teleports] {
            sm.on_set_menu_page(sub);
            assert_eq!(sm.on_escape(), CaptureAction::None);
            assert_eq!(sm.menu_page, MenuPage::Main);
        }
    }

    #[test]
    fn world_map_toggle_and_focus_loss_release_capture() {
        let mut sm = InputStateMachine::new_world_ready();
        assert_eq!(sm.on_left_click_in_viewport(), CaptureAction::Capture);

        // M opens World Map and releases mouse.
        assert_eq!(sm.on_toggle_world_map(), CaptureAction::Release);
        assert_eq!(sm.menu_page, MenuPage::WorldMap);
        assert!(!sm.mouse_captured);

        // M again closes World Map.
        assert_eq!(sm.on_toggle_world_map(), CaptureAction::None);
        assert_eq!(sm.menu_page, MenuPage::Closed);

        // Focus loss releases mouse capture and blocks input until refocused & clicked.
        assert_eq!(sm.on_left_click_in_viewport(), CaptureAction::Capture);
        assert_eq!(sm.on_focus_changed(false), CaptureAction::Release);
        assert!(!sm.mouse_captured);
        assert_eq!(sm.on_left_click_in_viewport(), CaptureAction::None);
        assert_eq!(sm.on_focus_changed(true), CaptureAction::None);
        assert_eq!(sm.on_left_click_in_viewport(), CaptureAction::Capture);
    }
}
