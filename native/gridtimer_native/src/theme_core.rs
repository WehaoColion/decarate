//! Rust-owned semantic color decisions for the Android theme.
//!
//! Compose owns rendering and system appearance observation. This module is
//! deliberately platform-free: one call resolves a complete immutable token
//! set, and Android maps those tokens onto Material 3 roles.

pub const THEME_TOKEN_JNI_FIELD_COUNT: usize = 37;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemePreference {
    System,
    Light,
    Dark,
    Oled,
}

impl ThemePreference {
    pub fn from_code(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::System),
            1 => Some(Self::Light),
            2 => Some(Self::Dark),
            3 => Some(Self::Oled),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeTokens {
    pub is_dark: bool,
    pub is_oled: bool,

    pub background: i64,
    pub on_background: i64,
    pub surface: i64,
    pub on_surface: i64,
    pub surface_variant: i64,
    pub on_surface_variant: i64,
    pub surface_dim: i64,
    pub surface_bright: i64,
    pub surface_container_lowest: i64,
    pub surface_container_low: i64,
    pub surface_container: i64,
    pub surface_container_high: i64,
    pub surface_container_highest: i64,

    pub primary: i64,
    pub on_primary: i64,
    pub primary_container: i64,
    pub on_primary_container: i64,
    pub secondary: i64,
    pub on_secondary: i64,
    pub secondary_container: i64,
    pub on_secondary_container: i64,
    pub tertiary: i64,
    pub on_tertiary: i64,
    pub tertiary_container: i64,
    pub on_tertiary_container: i64,

    pub outline: i64,
    pub outline_variant: i64,
    pub error: i64,
    pub on_error: i64,
    pub error_container: i64,
    pub on_error_container: i64,
    pub inverse_surface: i64,
    pub inverse_on_surface: i64,
    pub inverse_primary: i64,
    pub scrim: i64,
}

impl ThemeTokens {
    /// Stable JNI payload consumed by the Rust-generated Compose bridge.
    pub fn to_jni_values(&self) -> [i64; THEME_TOKEN_JNI_FIELD_COUNT] {
        [
            i64::from(self.is_dark),
            i64::from(self.is_oled),
            self.background,
            self.on_background,
            self.surface,
            self.on_surface,
            self.surface_variant,
            self.on_surface_variant,
            self.surface_dim,
            self.surface_bright,
            self.surface_container_lowest,
            self.surface_container_low,
            self.surface_container,
            self.surface_container_high,
            self.surface_container_highest,
            self.primary,
            self.on_primary,
            self.primary_container,
            self.on_primary_container,
            self.secondary,
            self.on_secondary,
            self.secondary_container,
            self.on_secondary_container,
            self.tertiary,
            self.on_tertiary,
            self.tertiary_container,
            self.on_tertiary_container,
            self.outline,
            self.outline_variant,
            self.error,
            self.on_error,
            self.error_container,
            self.on_error_container,
            self.inverse_surface,
            self.inverse_on_surface,
            self.inverse_primary,
            self.scrim,
        ]
    }
}

const fn color(value: u32) -> i64 {
    value as i64
}

pub fn resolve_theme(preference: ThemePreference, system_is_dark: bool) -> ThemeTokens {
    match preference {
        ThemePreference::Light => light_tokens(),
        ThemePreference::Dark => dark_tokens(),
        ThemePreference::Oled => oled_tokens(),
        ThemePreference::System if system_is_dark => dark_tokens(),
        ThemePreference::System => light_tokens(),
    }
}

fn light_tokens() -> ThemeTokens {
    ThemeTokens {
        is_dark: false,
        is_oled: false,

        background: color(0xFFFAF9FC),
        on_background: color(0xFF1A1C20),
        surface: color(0xFFF7F7FA),
        on_surface: color(0xFF1A1C20),
        surface_variant: color(0xFFE0E3E8),
        on_surface_variant: color(0xFF44474F),
        surface_dim: color(0xFFDAD9DD),
        surface_bright: color(0xFFFAF9FC),
        surface_container_lowest: color(0xFFFFFFFF),
        surface_container_low: color(0xFFF7F7FA),
        surface_container: color(0xFFF3F4F7),
        surface_container_high: color(0xFFEAECF1),
        surface_container_highest: color(0xFFE0E3E8),

        primary: color(0xFF3F608F),
        on_primary: color(0xFFFFFFFF),
        primary_container: color(0xFFD6E3FF),
        on_primary_container: color(0xFF244875),
        secondary: color(0xFF526072),
        on_secondary: color(0xFFFFFFFF),
        secondary_container: color(0xFFD6E4F7),
        on_secondary_container: color(0xFF3B4859),
        tertiary: color(0xFF8A4B08),
        on_tertiary: color(0xFFFFFFFF),
        tertiary_container: color(0xFFFFDDBF),
        on_tertiary_container: color(0xFF2D1600),

        outline: color(0xFF74777F),
        outline_variant: color(0xFFC4C6D0),
        error: color(0xFFBA1A1A),
        on_error: color(0xFFFFFFFF),
        error_container: color(0xFFFFDAD6),
        on_error_container: color(0xFF410002),
        inverse_surface: color(0xFF2F3035),
        inverse_on_surface: color(0xFFF1F0F4),
        inverse_primary: color(0xFFAFC6FF),
        scrim: color(0x99000000),
    }
}

fn dark_tokens() -> ThemeTokens {
    ThemeTokens {
        is_dark: true,
        is_oled: false,

        background: color(0xFF101114),
        on_background: color(0xFFE7E9ED),
        surface: color(0xFF13151A),
        on_surface: color(0xFFE7E9ED),
        surface_variant: color(0xFF3A404B),
        on_surface_variant: color(0xFFADB3BD),
        surface_dim: color(0xFF101114),
        surface_bright: color(0xFF353B46),
        surface_container_lowest: color(0xFF0B0C0F),
        surface_container_low: color(0xFF13151A),
        surface_container: color(0xFF171A20),
        surface_container_high: color(0xFF252A33),
        surface_container_highest: color(0xFF2D333D),

        primary: color(0xFFAFC6FF),
        on_primary: color(0xFF0B2E5A),
        primary_container: color(0xFF244A78),
        on_primary_container: color(0xFFD7E3FF),
        secondary: color(0xFFA7D5C2),
        on_secondary: color(0xFF10372A),
        secondary_container: color(0xFF274E40),
        on_secondary_container: color(0xFFC2F1DD),
        tertiary: color(0xFFFFB77D),
        on_tertiary: color(0xFF4B2500),
        tertiary_container: color(0xFF6A3A08),
        on_tertiary_container: color(0xFFFFDDBF),

        outline: color(0xFF747B87),
        outline_variant: color(0xFF3A404B),
        error: color(0xFFFFB4AB),
        on_error: color(0xFF690005),
        error_container: color(0xFF93000A),
        on_error_container: color(0xFFFFDAD6),
        inverse_surface: color(0xFFE7E9ED),
        inverse_on_surface: color(0xFF2D333D),
        inverse_primary: color(0xFF3F608F),
        scrim: color(0xB3000000),
    }
}

fn oled_tokens() -> ThemeTokens {
    let mut tokens = dark_tokens();
    tokens.is_oled = true;

    // Only the lowest planes are pure black. Cards, dialogs and selection
    // surfaces keep separation so the interface does not collapse visually.
    tokens.background = color(0xFF000000);
    tokens.surface = color(0xFF000000);
    tokens.surface_dim = color(0xFF000000);
    tokens.surface_container_lowest = color(0xFF000000);
    tokens.surface_container_low = color(0xFF08090B);
    tokens.surface_container = color(0xFF0D0F12);
    tokens.surface_container_high = color(0xFF111419);
    tokens.surface_container_highest = color(0xFF1B1F25);
    tokens.surface_bright = color(0xFF242A32);
    tokens
}

pub fn contrast_ratio(foreground: i64, background: i64) -> f64 {
    let foreground_luminance = relative_luminance(foreground);
    let background_luminance = relative_luminance(background);
    let lighter = foreground_luminance.max(background_luminance);
    let darker = foreground_luminance.min(background_luminance);
    (lighter + 0.05) / (darker + 0.05)
}

fn relative_luminance(argb: i64) -> f64 {
    let value = argb as u32;
    let red = ((value >> 16) & 0xFF) as u8;
    let green = ((value >> 8) & 0xFF) as u8;
    let blue = (value & 0xFF) as u8;

    0.2126 * linear_channel(red) + 0.7152 * linear_channel(green) + 0.0722 * linear_channel(blue)
}

fn linear_channel(channel: u8) -> f64 {
    let value = channel as f64 / 255.0;
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_text_contrast(tokens: &ThemeTokens) {
        let pairs = [
            (
                "on_background/background",
                tokens.on_background,
                tokens.background,
            ),
            ("on_surface/surface", tokens.on_surface, tokens.surface),
            (
                "on_surface_variant/surface_container_high",
                tokens.on_surface_variant,
                tokens.surface_container_high,
            ),
            ("on_primary/primary", tokens.on_primary, tokens.primary),
            (
                "on_primary_container/primary_container",
                tokens.on_primary_container,
                tokens.primary_container,
            ),
            (
                "on_secondary/secondary",
                tokens.on_secondary,
                tokens.secondary,
            ),
            (
                "on_secondary_container/secondary_container",
                tokens.on_secondary_container,
                tokens.secondary_container,
            ),
            ("on_tertiary/tertiary", tokens.on_tertiary, tokens.tertiary),
            (
                "on_tertiary_container/tertiary_container",
                tokens.on_tertiary_container,
                tokens.tertiary_container,
            ),
            ("on_error/error", tokens.on_error, tokens.error),
            (
                "on_error_container/error_container",
                tokens.on_error_container,
                tokens.error_container,
            ),
        ];

        for (name, foreground, background) in pairs {
            let ratio = contrast_ratio(foreground, background);
            assert!(
                ratio >= 4.5,
                "{name} contrast ratio was {ratio:.2}, expected at least 4.5"
            );
        }
    }

    #[test]
    fn all_palettes_have_accessible_text_contrast() {
        assert_text_contrast(&light_tokens());
        assert_text_contrast(&dark_tokens());
        assert_text_contrast(&oled_tokens());
    }

    #[test]
    fn system_mode_tracks_android_appearance() {
        assert!(!resolve_theme(ThemePreference::System, false).is_dark);
        assert!(resolve_theme(ThemePreference::System, true).is_dark);
    }

    #[test]
    fn oled_keeps_only_the_lowest_planes_black() {
        let tokens = resolve_theme(ThemePreference::Oled, false);
        assert!(tokens.is_dark);
        assert!(tokens.is_oled);
        assert_eq!(tokens.background, color(0xFF000000));
        assert_eq!(tokens.surface, color(0xFF000000));
        assert_ne!(tokens.surface_container_high, color(0xFF000000));
    }

    #[test]
    fn jni_contract_has_stable_width_and_field_order() {
        let values = light_tokens().to_jni_values();
        assert_eq!(THEME_TOKEN_JNI_FIELD_COUNT, values.len());
        assert_eq!(0, values[0]);
        assert_eq!(0, values[1]);
        assert_eq!(color(0xFFFAF9FC), values[2], "background");
        assert_eq!(color(0xFF1A1C20), values[5], "on_surface");
        assert_eq!(color(0xFFEAECF1), values[13], "surface_container_high");
        assert_eq!(color(0xFF3F608F), values[15], "primary");
        assert_eq!(color(0xFF526072), values[19], "secondary");
        assert_eq!(color(0xFF8A4B08), values[23], "tertiary");
        assert_eq!(color(0xFF74777F), values[27], "outline");
        assert_eq!(color(0xFFBA1A1A), values[29], "error");
        assert_eq!(color(0xFF2F3035), values[33], "inverse_surface");
        assert_eq!(color(0x99000000), values[36], "scrim");
    }
}
