//! Interface do Skyke3D em Bevy UI.
//!
//! As cores vêm das variáveis de `assets/app.css` do jogo original (`--bg`, `--panel`, `--fg`, `--ac`, `--bd`, `--st`,
//! `--onac`), nos três temas: escuro, claro e alto contraste. A tela inicial usa a paleta roxa própria (`#start`) no tema
//! escuro, como no original. Fonte: Manrope (OFL), com emoji pela fonte do sistema.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use bevy::prelude::*;
use bevy::text::{FontSource, FontWeight};
pub use bevy::ui_widgets::Activate;
use bevy::ui_widgets::Button;

#[derive(Debug)]
pub struct SkyUiPlugin;

impl Plugin for SkyUiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Theme>().init_resource::<UiFonts>();
    }
}

/// Tema escolhido (Ajustes → Tema e Alto contraste no original).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeKind {
    #[default]
    Dark,
    Light,
    HighContrast,
}

/// Paleta ativa.
#[derive(Resource, Debug, Clone, Copy)]
pub struct Theme {
    pub kind: ThemeKind,
    pub bg: Color,
    pub panel: Color,
    pub fg: Color,
    pub accent: Color,
    pub border: Color,
    pub strong: Color,
    pub on_accent: Color,
    /// Paleta da tela inicial (`#start`).
    pub start_panel: Color,
    pub start_border: Color,
    pub start_accent: Color,
}

fn hex(s: &str) -> Color {
    Color::Srgba(Srgba::hex(s).unwrap_or(Srgba::WHITE))
}

impl Theme {
    pub fn new(kind: ThemeKind) -> Self {
        match kind {
            ThemeKind::Dark => Self {
                kind,
                bg: hex("#1b1713"),
                panel: hex("#2a231cf2"),
                fg: hex("#f5ede0"),
                accent: hex("#f0a53a"),
                border: hex("#5a4a3a"),
                strong: hex("#ffcf8a"),
                on_accent: hex("#111111"),
                start_panel: hex("#2a2236f2"),
                start_border: hex("#5b4b73"),
                start_accent: hex("#ffb23e"),
            },
            ThemeKind::Light => Self {
                kind,
                bg: hex("#efe6d8"),
                panel: hex("#fffaf0f2"),
                fg: hex("#2b2218"),
                accent: hex("#a85a00"),
                border: hex("#c9b8a0"),
                strong: hex("#7a4300"),
                on_accent: hex("#ffffff"),
                start_panel: hex("#fffaf0f7"),
                start_border: hex("#c9b8a0"),
                start_accent: hex("#a85a00"),
            },
            ThemeKind::HighContrast => Self {
                kind,
                bg: hex("#000000"),
                panel: hex("#000000"),
                fg: hex("#ffffff"),
                accent: hex("#ffe600"),
                border: hex("#ffffff"),
                strong: hex("#ffe600"),
                on_accent: hex("#000000"),
                start_panel: hex("#000000"),
                start_border: hex("#ffffff"),
                start_accent: hex("#ffe600"),
            },
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::new(ThemeKind::Dark)
    }
}

/// Fontes da interface.
#[derive(Resource, Debug, Clone)]
pub struct UiFonts {
    pub manrope: Handle<Font>,
}

impl UiFonts {
    /// Manrope com emoji colorido da fonte do sistema como alternativa (Noto Color Emoji / Segoe UI Emoji).
    pub fn source(&self) -> FontSource {
        FontSource::list([
            FontSource::from(self.manrope.clone()),
            FontSource::family("Noto Color Emoji"),
            FontSource::family("Segoe UI Emoji"),
        ])
    }

    pub fn font(&self, size: f32, weight: u16) -> TextFont {
        TextFont { font: self.source(), font_size: size.into(), weight: FontWeight(weight), ..default() }
    }
}

impl FromWorld for UiFonts {
    fn from_world(world: &mut World) -> Self {
        let assets = world.resource::<AssetServer>();
        Self { manrope: assets.load("fonts/Manrope.ttf") }
    }
}

/// Painel `.p` do original: fundo do painel, borda de 1 px, cantos de 12 px e respiro de 10 px.
pub fn panel(t: &Theme) -> impl Bundle {
    (
        Node {
            padding: UiRect::all(Val::Px(10.0)),
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::all(Val::Px(12.0)),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(8.0),
            ..default()
        },
        BackgroundColor(t.panel),
        BorderColor::all(t.border),
        BoxShadow::new(Color::srgba(0.0, 0.0, 0.0, 0.4), Val::Px(0.0), Val::Px(4.0), Val::Px(0.0), Val::Px(18.0)),
    )
}

/// Marca o texto de um botão (para trocar o rótulo depois).
#[derive(Component, Debug)]
pub struct ButtonLabel;

/// Botão do original: borda de 1 px, cantos de 8 px, respiro 6×10; `primary` = `button.pri` (fundo de destaque, negrito).
pub fn button(commands: &mut Commands, t: &Theme, f: &UiFonts, text: &str, primary: bool) -> Entity {
    let (bg, fg, weight) = if primary { (t.accent, t.on_accent, 700) } else { (t.panel, t.fg, 500) };
    commands
        .spawn((
            Button,
            Node {
                padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(8.0)),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(bg),
            BorderColor::all(t.border),
        ))
        .with_children(|b| {
            b.spawn((Text::new(text), f.font(16.0, weight), TextColor(fg), ButtonLabel));
        })
        .id()
}

/// Campo de texto de uma linha com o estilo dos `input` do original.
pub fn text_input(t: &Theme, f: &UiFonts, initial: &str, width: Val) -> impl Bundle {
    (
        bevy::ui_widgets::TextInput,
        bevy::text::EditableText::new(initial),
        Node {
            width,
            padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::all(Val::Px(8.0)),
            ..default()
        },
        f.font(16.0, 500),
        TextColor(t.fg),
        BorderColor::all(t.border),
        BackgroundColor(Color::NONE),
    )
}

/// Texto do campo, sem quebras de linha nem controles.
pub fn input_value(e: &bevy::text::EditableText) -> String {
    e.value().to_string().chars().filter(|c| !c.is_control()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn themes_match_original_css() {
        let d = Theme::new(ThemeKind::Dark);
        assert_eq!(d.accent.to_srgba().to_hex(), "#F0A53A");
        let l = Theme::new(ThemeKind::Light);
        assert_eq!(l.fg.to_srgba().to_hex(), "#2B2218");
        let h = Theme::new(ThemeKind::HighContrast);
        assert_eq!(h.accent.to_srgba().to_hex(), "#FFE600");
    }
}
