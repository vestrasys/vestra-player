//! Slint-facing code: the main window and its views, plugin UI values,
//! composition and slot claims, menus, themes, settings, window state,
//! picture-in-picture, drops, dialogs and the bottom-bar buttons.

pub(crate) mod bar_buttons;
pub mod composition;
pub(crate) mod confirm;
pub(crate) mod drops;
pub(crate) mod menu;
pub(crate) mod pip;
pub(crate) mod settings_window;
#[cfg(test)]
mod slint_lists;
pub(crate) mod slot_claims;
pub(crate) mod theme;
pub(crate) mod ui_fonts;
pub(crate) mod ui_image;
pub(crate) mod ui_markdown;
pub(crate) mod ui_patch;
pub(crate) mod ui_rows;
#[cfg(test)]
pub(crate) mod ui_shots;
pub(crate) mod ui_state;
#[cfg(test)]
pub(crate) mod ui_types;
pub(crate) mod ui_values;
pub(crate) mod view;
#[cfg(test)]
mod widgets_tests;
pub mod window_control;
pub mod window_state;
