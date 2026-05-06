use godot::classes::{Control, IControl};
use godot::prelude::*;

use crate::game_state::Screen;

#[derive(GodotClass)]
#[class(base=Control)]
pub struct UiManager {
    base: Base<Control>,
}

#[godot_api]
impl IControl for UiManager {
    fn init(base: Base<Control>) -> Self {
        Self { base }
    }

    fn ready(&mut self) {}
}

impl UiManager {
    pub fn set_screen(&mut self, s: Screen, is_mobile: bool) {
        let mut mobile = self.base_mut().get_node_as::<Control>("Mobile");
        let mut desktop = self
            .base_mut()
            .get_node_as::<Control>("CenterContainer/VBoxContainer/Desktop");
        let mut desktop_host = self
            .base_mut()
            .get_node_as::<Control>("CenterContainer/VBoxContainer/DesktopHost");
        let mut info = self.base_mut().get_node_as::<Control>("Info");
        match s {
            Screen::MainMenu => {
                if is_mobile {
                    desktop.set_visible(false);
                    desktop_host.set_visible(false);
                    info.set_visible(false);
                    mobile.set_visible(true);
                } else {
                    desktop.set_visible(true);
                    desktop_host.set_visible(false);
                    info.set_visible(false);
                    mobile.set_visible(false);
                }
            }
            Screen::Host => {
                desktop.set_visible(false);
                desktop_host.set_visible(true);
                info.set_visible(false);
                mobile.set_visible(false);
            }
            Screen::Info => {
                desktop.set_visible(false);
                desktop_host.set_visible(false);
                mobile.set_visible(false);
                info.set_visible(true);
            }
            Screen::Game => {
                desktop.set_visible(false);
                desktop_host.set_visible(false);
                mobile.set_visible(false);
                info.set_visible(false);
            }
        }
    }
}
