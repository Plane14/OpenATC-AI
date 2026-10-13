//! Theme colours shared by the simulator and desktop interface.
use imgui::{ColorStackToken, StyleColor, Ui};
use std::sync::OnceLock;

pub fn apply(ui: &Ui, light: bool) -> [ColorStackToken<'_>; StyleColor::COUNT] {
    static DARK: OnceLock<[[f32; 4]; StyleColor::COUNT]> = OnceLock::new();
    static LIGHT: OnceLock<[[f32; 4]; StyleColor::COUNT]> = OnceLock::new();
    let colours = if light { &LIGHT } else { &DARK }.get_or_init(|| {
        let mut style = ui.clone_style();
        style.use_dark_colors();
        crate::widgets::configure_style(&mut style);
        if light {
            style.use_light_colors();
            for (slot, colour) in [
                (StyleColor::Text, [0.09, 0.14, 0.19, 1.0]),
                (StyleColor::TextDisabled, [0.34, 0.39, 0.44, 1.0]),
                (StyleColor::WindowBg, [0.94, 0.96, 0.97, 1.0]),
                (StyleColor::ChildBg, [0.86, 0.90, 0.93, 1.0]),
                (StyleColor::PopupBg, [0.97, 0.98, 0.99, 1.0]),
                (StyleColor::Border, [0.55, 0.64, 0.70, 0.6]),
                (StyleColor::FrameBg, [0.98, 0.99, 1.0, 1.0]),
                (StyleColor::FrameBgHovered, [0.85, 0.94, 0.97, 1.0]),
                (StyleColor::FrameBgActive, [0.80, 0.91, 0.95, 1.0]),
                (StyleColor::Button, [0.82, 0.88, 0.91, 1.0]),
                (StyleColor::ButtonHovered, [0.66, 0.84, 0.90, 1.0]),
                (StyleColor::ButtonActive, [0.53, 0.76, 0.85, 1.0]),
                (StyleColor::Header, [0.74, 0.85, 0.90, 1.0]),
                (StyleColor::HeaderHovered, [0.65, 0.82, 0.89, 1.0]),
                (StyleColor::HeaderActive, [0.55, 0.76, 0.85, 1.0]),
                (StyleColor::Tab, [0.83, 0.89, 0.92, 1.0]),
                (StyleColor::TabHovered, [0.66, 0.83, 0.89, 1.0]),
                (StyleColor::TabActive, [0.67, 0.83, 0.89, 1.0]),
                (StyleColor::Separator, [0.46, 0.58, 0.65, 0.5]),
                (StyleColor::ScrollbarBg, [0.82, 0.86, 0.89, 0.6]),
                (StyleColor::ScrollbarGrab, [0.42, 0.53, 0.61, 1.0]),
                (StyleColor::CheckMark, [0.0, 0.38, 0.51, 1.0]),
                (StyleColor::SliderGrab, [0.0, 0.40, 0.53, 1.0]),
                (StyleColor::TableHeaderBg, [0.77, 0.84, 0.89, 1.0]),
                (StyleColor::TableRowBg, [0.88, 0.92, 0.95, 0.4]),
                (StyleColor::TableRowBgAlt, [0.75, 0.85, 0.90, 0.3]),
            ] {
                style.colors[slot as usize] = colour;
            }
        }
        style.colors
    });
    std::array::from_fn(|index| ui.push_style_color(StyleColor::VARIANTS[index], colours[index]))
}
