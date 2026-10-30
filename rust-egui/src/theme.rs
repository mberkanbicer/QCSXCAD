//! `Precision Workbench` design tokens.
//!
//! One place owns every colour, radius, row height and text style the shell
//! uses. Widgets read a [`Palette`] through [`palette`] instead of hardcoding
//! RGB values, so switching the theme is a single call and cannot leave a
//! stray light-on-dark control behind.
//!
//! Dark is the recommended default (DESIGN.md §3). Light and a high-contrast
//! fallback are complete alternatives, not inversions: each palette is checked
//! for text contrast by the tests below.

use egui::{Color32, CornerRadius, FontId, Id, Margin, Stroke, TextStyle, Visuals};

/// Which palette the shell is rendering.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThemeMode {
    /// Recommended default.
    #[default]
    Dark,
    Light,
    /// Black background, pure-white text, yellow focus/selection.
    HighContrast,
}

impl ThemeMode {
    pub const ALL: [Self; 3] = [Self::Dark, Self::Light, Self::HighContrast];

    pub fn label(self) -> &'static str {
        match self {
            Self::Dark => "Dark",
            Self::Light => "Light",
            Self::HighContrast => "High contrast",
        }
    }

    /// Stable identifier for the settings file.
    pub fn name(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
            Self::HighContrast => "high-contrast",
        }
    }

    /// Parse a persisted identifier; unknown values fall back to dark.
    pub fn from_name(name: &str) -> Self {
        match name.trim().to_ascii_lowercase().as_str() {
            "light" => Self::Light,
            "high-contrast" | "highcontrast" | "contrast" => Self::HighContrast,
            _ => Self::Dark,
        }
    }

    pub fn palette(self) -> &'static Palette {
        match self {
            Self::Dark => &DARK,
            Self::Light => &LIGHT,
            Self::HighContrast => &HIGH_CONTRAST,
        }
    }
}

/// Semantic colours. Field names mirror the token table in DESIGN.md.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub app_bg: Color32,
    pub chrome_bg: Color32,
    pub panel_bg: Color32,
    pub panel_raised: Color32,
    pub field_bg: Color32,
    pub viewport_bg: Color32,
    pub border: Color32,
    pub border_strong: Color32,
    pub text: Color32,
    pub text_secondary: Color32,
    pub text_tertiary: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub selection: Color32,
    pub hover: Color32,
    pub focus: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub danger: Color32,
    pub info: Color32,
    /// X/Y/Z triad colours, readable on `viewport_bg`.
    pub axis_x: Color32,
    pub axis_y: Color32,
    pub axis_z: Color32,
    /// True for the two dark palettes (used for effect choices).
    pub is_dark: bool,
}

impl Palette {
    /// Viewport clear colour in wgpu's 0..1 **linear** space.
    ///
    /// The surface is an sRGB target, and wgpu interprets `LoadOp::Clear`
    /// values as linear, so the palette's sRGB bytes must be linearised or the
    /// background renders several stops too bright.
    pub fn viewport_rgb(&self) -> [f64; 3] {
        fn linear(channel: u8) -> f64 {
            let value = f64::from(channel) / 255.0;
            if value <= 0.040_45 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        }
        [
            linear(self.viewport_bg.r()),
            linear(self.viewport_bg.g()),
            linear(self.viewport_bg.b()),
        ]
    }

    /// Build the egui [`Visuals`] for this palette.
    fn visuals(&self) -> Visuals {
        let mut visuals = if self.is_dark {
            Visuals::dark()
        } else {
            Visuals::light()
        };
        visuals.panel_fill = self.panel_bg;
        visuals.window_fill = self.panel_bg;
        visuals.faint_bg_color = self.app_bg;
        visuals.extreme_bg_color = self.field_bg;
        visuals.code_bg_color = self.field_bg;
        visuals.window_stroke = Stroke::new(1.0, self.border);
        visuals.window_corner_radius = CornerRadius::same(RADIUS_DIALOG);
        visuals.menu_corner_radius = CornerRadius::same(RADIUS_POPOVER);

        let widgets = &mut visuals.widgets;
        widgets.noninteractive.bg_fill = self.panel_bg;
        widgets.noninteractive.weak_bg_fill = self.panel_bg;
        widgets.noninteractive.bg_stroke = Stroke::new(1.0, self.border);
        widgets.noninteractive.fg_stroke = Stroke::new(1.0, self.text);

        widgets.inactive.bg_fill = self.panel_raised;
        widgets.inactive.weak_bg_fill = self.panel_raised;
        widgets.inactive.bg_stroke = Stroke::new(1.0, self.border);
        widgets.inactive.fg_stroke = Stroke::new(1.0, self.text);

        widgets.hovered.bg_fill = self.hover;
        widgets.hovered.weak_bg_fill = self.hover;
        widgets.hovered.bg_stroke = Stroke::new(1.0, self.border_strong);
        widgets.hovered.fg_stroke = Stroke::new(1.0, self.text);

        widgets.active.bg_fill = self.selection;
        widgets.active.weak_bg_fill = self.selection;
        widgets.active.bg_stroke = Stroke::new(1.0, self.accent);
        widgets.active.fg_stroke = Stroke::new(1.0, self.text);

        widgets.open.bg_fill = self.panel_raised;
        widgets.open.weak_bg_fill = self.panel_raised;
        widgets.open.bg_stroke = Stroke::new(1.0, self.border);
        widgets.open.fg_stroke = Stroke::new(1.0, self.text);

        for widget in [
            &mut widgets.noninteractive,
            &mut widgets.inactive,
            &mut widgets.hovered,
            &mut widgets.active,
            &mut widgets.open,
        ] {
            widget.corner_radius = CornerRadius::same(RADIUS_BUTTON);
        }

        visuals.selection.bg_fill = self.selection;
        visuals.selection.stroke = Stroke::new(1.0, self.text);
        visuals.hyperlink_color = self.accent;
        visuals.warn_fg_color = self.warning;
        visuals.error_fg_color = self.danger;
        visuals.striped = false;
        visuals
    }
}

/// Dark palette — DESIGN.md §3.1.
pub static DARK: Palette = Palette {
    app_bg: Color32::from_rgb(0x10, 0x15, 0x1C),
    chrome_bg: Color32::from_rgb(0x16, 0x1D, 0x26),
    panel_bg: Color32::from_rgb(0x1B, 0x25, 0x30),
    panel_raised: Color32::from_rgb(0x24, 0x30, 0x3E),
    field_bg: Color32::from_rgb(0x14, 0x20, 0x2B),
    viewport_bg: Color32::from_rgb(0x0E, 0x14, 0x1D),
    border: Color32::from_rgb(0x39, 0x46, 0x56),
    border_strong: Color32::from_rgb(0x61, 0x72, 0x87),
    text: Color32::from_rgb(0xE9, 0xEE, 0xF5),
    text_secondary: Color32::from_rgb(0xB8, 0xC4, 0xD2),
    text_tertiary: Color32::from_rgb(0x93, 0xA5, 0xB8),
    accent: Color32::from_rgb(0x70, 0xB9, 0xF5),
    accent_soft: Color32::from_rgb(0x1E, 0x35, 0x4A),
    selection: Color32::from_rgb(0x25, 0x44, 0x5F),
    hover: Color32::from_rgb(0x22, 0x2D, 0x3A),
    focus: Color32::from_rgb(0x98, 0xD4, 0xFF),
    success: Color32::from_rgb(0x77, 0xD6, 0xAE),
    warning: Color32::from_rgb(0xF0, 0xC6, 0x74),
    danger: Color32::from_rgb(0xF2, 0x9A, 0x9A),
    info: Color32::from_rgb(0x70, 0xB9, 0xF5),
    axis_x: Color32::from_rgb(0xE0, 0x6C, 0x75),
    axis_y: Color32::from_rgb(0x7E, 0xC6, 0x99),
    axis_z: Color32::from_rgb(0x7A, 0xA2, 0xF7),
    is_dark: true,
};

/// Light palette — DESIGN.md §3.1.
pub static LIGHT: Palette = Palette {
    app_bg: Color32::from_rgb(0xEE, 0xF2, 0xF6),
    chrome_bg: Color32::from_rgb(0xF8, 0xFA, 0xFC),
    panel_bg: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    panel_raised: Color32::from_rgb(0xE5, 0xEB, 0xF2),
    field_bg: Color32::from_rgb(0xF5, 0xF7, 0xFA),
    viewport_bg: Color32::from_rgb(0xE9, 0xEE, 0xF4),
    border: Color32::from_rgb(0xCC, 0xD6, 0xE0),
    border_strong: Color32::from_rgb(0x86, 0x98, 0xAB),
    text: Color32::from_rgb(0x19, 0x27, 0x35),
    text_secondary: Color32::from_rgb(0x52, 0x64, 0x77),
    text_tertiary: Color32::from_rgb(0x52, 0x64, 0x77),
    accent: Color32::from_rgb(0x17, 0x69, 0xB2),
    accent_soft: Color32::from_rgb(0xE6, 0xF1, 0xFB),
    selection: Color32::from_rgb(0xD6, 0xEA, 0xFC),
    hover: Color32::from_rgb(0xF0, 0xF4, 0xF8),
    focus: Color32::from_rgb(0x17, 0x69, 0xB2),
    success: Color32::from_rgb(0x17, 0x74, 0x51),
    warning: Color32::from_rgb(0x8A, 0x5B, 0x00),
    danger: Color32::from_rgb(0xB4, 0x23, 0x18),
    info: Color32::from_rgb(0x17, 0x69, 0xB2),
    axis_x: Color32::from_rgb(0xB0, 0x3A, 0x3A),
    axis_y: Color32::from_rgb(0x2E, 0x7D, 0x32),
    axis_z: Color32::from_rgb(0x1F, 0x4F, 0xA8),
    is_dark: false,
};

/// High-contrast fallback: black surfaces, white text, yellow cues.
pub static HIGH_CONTRAST: Palette = Palette {
    app_bg: Color32::from_rgb(0x00, 0x00, 0x00),
    chrome_bg: Color32::from_rgb(0x00, 0x00, 0x00),
    panel_bg: Color32::from_rgb(0x00, 0x00, 0x00),
    panel_raised: Color32::from_rgb(0x1A, 0x1A, 0x1A),
    field_bg: Color32::from_rgb(0x00, 0x00, 0x00),
    viewport_bg: Color32::from_rgb(0x00, 0x00, 0x00),
    border: Color32::from_rgb(0x8A, 0x8A, 0x8A),
    border_strong: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    text: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    text_secondary: Color32::from_rgb(0xE6, 0xE6, 0xE6),
    text_tertiary: Color32::from_rgb(0xD0, 0xD0, 0xD0),
    accent: Color32::from_rgb(0xFF, 0xD1, 0x3D),
    accent_soft: Color32::from_rgb(0x3A, 0x30, 0x00),
    selection: Color32::from_rgb(0x4A, 0x3B, 0x00),
    hover: Color32::from_rgb(0x1F, 0x1F, 0x1F),
    focus: Color32::from_rgb(0xFF, 0xE8, 0x7A),
    success: Color32::from_rgb(0x6B, 0xFF, 0xB0),
    warning: Color32::from_rgb(0xFF, 0xD1, 0x3D),
    danger: Color32::from_rgb(0xFF, 0x8A, 0x8A),
    info: Color32::from_rgb(0x7C, 0xC7, 0xFF),
    axis_x: Color32::from_rgb(0xFF, 0x7B, 0x7B),
    axis_y: Color32::from_rgb(0x8B, 0xFF, 0x9C),
    axis_z: Color32::from_rgb(0x8A, 0xB4, 0xFF),
    is_dark: true,
};

/// Control height (compact density, DESIGN.md §3.4).
pub const CONTROL_HEIGHT: f32 = 28.0;
/// Structure-tree row height.
pub const ROW_HEIGHT: f32 = 26.0;
/// Base spacing rhythm.
pub const SPACE: f32 = 8.0;
/// Small spacing step.
pub const SPACE_SMALL: f32 = 4.0;
/// Panel inner padding.
pub const PANEL_PADDING: i8 = 12;
/// Corner radius for inputs.
pub const RADIUS_INPUT: u8 = 4;
/// Corner radius for buttons.
pub const RADIUS_BUTTON: u8 = 5;
/// Corner radius for popovers.
pub const RADIUS_POPOVER: u8 = 7;
/// Corner radius for dialogs.
pub const RADIUS_DIALOG: u8 = 8;

fn theme_id() -> Id {
    Id::new("qcsxcad.theme")
}

/// The theme in use for this egui context (dark until changed).
pub fn current(ctx: &egui::Context) -> ThemeMode {
    ctx.data(|data| data.get_temp::<ThemeMode>(theme_id()))
        .unwrap_or_default()
}

/// The active palette for this egui context.
pub fn palette(ctx: &egui::Context) -> &'static Palette {
    current(ctx).palette()
}

/// Change the active theme and apply it immediately.
pub fn set(ctx: &egui::Context, mode: ThemeMode) {
    apply(ctx, mode);
}

/// Apply `mode` to the context. Called once per frame by the shell.
pub fn apply(ctx: &egui::Context, mode: ThemeMode) {
    ctx.set_visuals(mode.palette().visuals());
    ctx.data_mut(|data| data.insert_temp(theme_id(), mode));

    let palette = mode.palette();
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(SPACE, SPACE_SMALL);
        style.spacing.button_padding = egui::vec2(8.0, 3.0);
        style.spacing.menu_margin = Margin::same(PANEL_PADDING);
        style.spacing.indent = 16.0;
        style.spacing.interact_size.y = CONTROL_HEIGHT;
        style.spacing.slider_width = 140.0;
        style.visuals.widgets.noninteractive.corner_radius = CornerRadius::same(RADIUS_INPUT);
        style.visuals.widgets.inactive.corner_radius = CornerRadius::same(RADIUS_BUTTON);
        style.visuals.widgets.hovered.corner_radius = CornerRadius::same(RADIUS_BUTTON);
        style.visuals.widgets.active.corner_radius = CornerRadius::same(RADIUS_BUTTON);
        style.visuals.window_corner_radius = CornerRadius::same(RADIUS_DIALOG);
        style.visuals.menu_corner_radius = CornerRadius::same(RADIUS_POPOVER);
        style.visuals.window_shadow.color = Color32::from_black_alpha(if palette.is_dark {
            120
        } else {
            40
        });
        style.text_styles.insert(
            TextStyle::Body,
            FontId::proportional(13.5),
        );
        style.text_styles.insert(
            TextStyle::Button,
            FontId::proportional(13.5),
        );
        style
            .text_styles
            .insert(TextStyle::Small, FontId::proportional(12.0));
        style
            .text_styles
            .insert(TextStyle::Heading, FontId::proportional(15.0));
        style
            .text_styles
            .insert(TextStyle::Monospace, FontId::monospace(12.5));
    });
}

/// A dock section: small caption over a hairline, then the content.
pub fn section(ui: &mut egui::Ui, title: &str, content: impl FnOnce(&mut egui::Ui)) {
    let palette = palette(ui.ctx());
    ui.add_space(SPACE_SMALL);
    ui.label(
        egui::RichText::new(title.to_uppercase())
            .color(palette.text_tertiary)
            .size(11.0)
            .strong(),
    );
    ui.add_space(2.0);
    content(ui);
    ui.add_space(SPACE_SMALL);
    ui.separator();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luminance(color: Color32) -> f32 {
        let channel = |value: u8| {
            let value = f32::from(value) / 255.0;
            if value <= 0.039_28 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
    }

    fn contrast(foreground: Color32, background: Color32) -> f32 {
        let (a, b) = (luminance(foreground), luminance(background));
        let (lighter, darker) = if a > b { (a, b) } else { (b, a) };
        (lighter + 0.05) / (darker + 0.05)
    }

    #[test]
    fn every_palette_meets_wcag_aa_for_body_text() {
        for mode in ThemeMode::ALL {
            let palette = mode.palette();
            // AA for normal text is 4.5:1; allow the tertiary tone the same bar.
            for (name, foreground) in [
                ("primary", palette.text),
                ("secondary", palette.text_secondary),
                ("tertiary", palette.text_tertiary),
            ] {
                for (surface_name, surface) in [
                    ("panel", palette.panel_bg),
                    ("field", palette.field_bg),
                    ("chrome", palette.chrome_bg),
                ] {
                    let ratio = contrast(foreground, surface);
                    assert!(
                        ratio >= 4.5,
                        "{} {name} on {surface_name}: {ratio:.2}:1",
                        mode.label()
                    );
                }
            }
        }
    }

    #[test]
    fn selected_rows_keep_text_readable() {
        for mode in ThemeMode::ALL {
            let palette = mode.palette();
            let ratio = contrast(palette.text, palette.selection);
            assert!(
                ratio >= 4.5,
                "{} selection text: {ratio:.2}:1",
                mode.label()
            );
        }
    }

    #[test]
    fn dark_is_the_default_and_the_light_theme_is_dark_on_light() {
        assert_eq!(ThemeMode::default(), ThemeMode::Dark);
        assert!(DARK.is_dark);
        assert!(!LIGHT.is_dark);
        assert!(luminance(LIGHT.text) < luminance(LIGHT.panel_bg));
        assert!(luminance(DARK.text) > luminance(DARK.panel_bg));
    }

    #[test]
    fn the_viewport_background_maps_to_unit_linear_rgb() {
        for mode in ThemeMode::ALL {
            let rgb = mode.palette().viewport_rgb();
            for channel in rgb {
                assert!((0.0..=1.0).contains(&channel));
            }
        }
        // #0E141D in sRGB is roughly 0.0044 in linear space, not 0.055: a
        // regression here is what makes the dark viewport look grey.
        let dark = DARK.viewport_rgb();
        assert!((dark[0] - 0.004_44).abs() < 1.0e-4, "{dark:?}");
    }

    #[test]
    fn the_rhythm_constants_are_consistent() {
        assert_eq!(SPACE, 8.0);
        assert_eq!(SPACE_SMALL, 4.0);
        assert!(ROW_HEIGHT >= CONTROL_HEIGHT - 4.0);
        assert!(RADIUS_INPUT < RADIUS_BUTTON);
    }
}
