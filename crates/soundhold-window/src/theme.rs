use gtk::gdk::RGBA;
use material_colors::color::Rgb;
use material_colors::theme::ThemeBuilder;

const STYLE: &str = include_str!("style.css");

fn hex(color: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", color.red, color.green, color.blue)
}

fn channel(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn tokens(source: Rgb, dark: bool) -> String {
    let theme = ThemeBuilder::with_source(source).build();
    let s = if dark { theme.schemes.dark } else { theme.schemes.light };
    [
        ("primary", s.primary),
        ("on_primary", s.on_primary),
        ("secondary_container", s.secondary_container),
        ("on_secondary_container", s.on_secondary_container),
        ("surface", s.surface),
        ("surface_container", s.surface_container),
        ("surface_container_highest", s.surface_container_highest),
        ("on_surface", s.on_surface),
        ("on_surface_variant", s.on_surface_variant),
        ("outline", s.outline),
        ("outline_variant", s.outline_variant),
        ("error", s.error),
    ]
    .iter()
    .map(|(name, color)| format!("@define-color m3_{name} {};\n", hex(*color)))
    .collect()
}

/// The window stylesheet with Material 3 colours generated from `accent`.
pub fn stylesheet(accent: RGBA, dark: bool) -> String {
    let source = Rgb::new(channel(accent.red()), channel(accent.green()), channel(accent.blue()));
    tokens(source, dark) + STYLE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_colour_in_the_stylesheet_is_defined() {
        for dark in [false, true] {
            let defined = tokens(Rgb::new(58, 148, 74), dark);
            for used in STYLE.split("@m3_").skip(1) {
                let name: String = used.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                assert!(defined.contains(&format!("@define-color m3_{name} ")), "{name}");
            }
        }
    }

    #[test]
    fn accent_colour_changes_the_primary_colour() {
        let green = tokens(Rgb::new(58, 148, 74), true);
        let red = tokens(Rgb::new(230, 45, 66), true);
        assert_ne!(green, red);
    }
}
