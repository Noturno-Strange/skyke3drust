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
        if !app.is_plugin_added::<bevy::input_focus::tab_navigation::TabNavigationPlugin>() {
            app.add_plugins(bevy::input_focus::tab_navigation::TabNavigationPlugin);
        }
        app.init_resource::<Theme>()
            .init_resource::<UiFonts>()
            .add_systems(PostUpdate, retheme)
            .add_systems(Update, (scroll_areas, placeholders, focus_ring, line_floor))
            .add_observer(shift_space)
            .init_resource::<LastPress>()
            .add_observer(remember_press)
            .add_observer(release_elsewhere);
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

/// Tema trocado (Ajustes): cada cor do tema anterior vira a mesma cor do novo, em fundos, bordas e textos.
fn retheme(
    theme: Res<Theme>,
    mut last: Local<Option<Theme>>,
    mut bg: Query<&mut BackgroundColor>,
    mut bc: Query<&mut BorderColor>,
    mut tc: Query<&mut TextColor>,
) {
    let new = *theme;
    let Some(old) = last.replace(new) else { return };
    if old.kind == new.kind {
        return;
    }
    let pairs = [
        (old.panel, new.panel),
        (old.fg, new.fg),
        (old.accent, new.accent),
        (old.border, new.border),
        (old.strong, new.strong),
        (old.on_accent, new.on_accent),
        (old.bg, new.bg),
        (old.start_panel, new.start_panel),
        (old.start_border, new.start_border),
        (old.start_accent, new.start_accent),
    ];
    let map = |c: Color| pairs.iter().find(|(o, _)| *o == c).map_or(c, |(_, n)| *n);
    for mut b in &mut bg {
        let c = map(b.0);
        if c != b.0 {
            b.0 = c;
        }
    }
    for mut b in &mut bc {
        let n = BorderColor { top: map(b.top), right: map(b.right), bottom: map(b.bottom), left: map(b.left) };
        if n != *b {
            *b = n;
        }
    }
    for mut t in &mut tc {
        let c = map(t.0);
        if c != t.0 {
            t.0 = c;
        }
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

    /// Setas e formas (◀ ▶ ▲ ▼) em estilo de texto, como no navegador: a Manrope não tem esses sinais e a fonte de emoji
    /// os desenha como botões laranja. Usa a fonte de símbolos de cada sistema.
    pub fn symbol(&self, size: f32, weight: u16) -> TextFont {
        let font = FontSource::list([
            FontSource::from(self.manrope.clone()),
            FontSource::family("DejaVu Sans"),
            FontSource::family("Segoe UI Symbol"),
            FontSource::family("Apple Symbols"),
            FontSource::family("Noto Sans Symbols 2"),
            FontSource::family("Noto Sans"),
            FontSource::family("Noto Color Emoji"),
            FontSource::family("Segoe UI Emoji"),
        ]);
        TextFont { font, font_size: size.into(), weight: FontWeight(weight), ..default() }
    }

    /// Fonte certa para um rótulo: a de símbolos se ele for só setas e formas geométricas.
    pub fn for_label(&self, text: &str, size: f32, weight: u16) -> TextFont {
        if is_shape_label(text) { self.symbol(size, weight) } else { self.font(size, weight) }
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
            b.spawn((Text::new(text), f.for_label(text, 16.0, weight), TextColor(fg), ButtonLabel));
        })
        .id()
}

/// Rótulo feito só de setas e formas geométricas (bloco U+25A0–U+25FF), sem emoji de verdade.
pub fn is_shape_label(s: &str) -> bool {
    !s.trim().is_empty()
        && s.chars().all(|c| c.is_whitespace() || ('\u{25A0}'..='\u{25FF}').contains(&c) || c == '\u{FE0E}')
}

/// Campo de texto de uma linha com o estilo dos `input` do original.
pub fn text_input(t: &Theme, f: &UiFonts, initial: &str, width: Val) -> impl Bundle {
    text_input_with(t, f, initial, width, "", None)
}

/// Campo de texto com `placeholder` (mostrado enquanto está vazio) e `maxlength`.
pub fn text_input_with(
    t: &Theme,
    f: &UiFonts,
    initial: &str,
    width: Val,
    placeholder: &str,
    max: Option<usize>,
) -> impl Bundle {
    let mut e = bevy::text::EditableText::new(initial);
    e.max_characters = max;
    (
        bevy::ui_widgets::TextInput,
        e,
        // sem TabIndex o clique não dá foco ao campo (PointerFocusPlugin) e nada se digita
        bevy::input_focus::tab_navigation::TabIndex(0),
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
        // vazio, o campo do Bevy não tem altura de linha: garante uma linha
        LineHeightFloor,
        Placeholder(placeholder.to_owned()),
        FocusRing { idle: t.border, focus: t.accent },
    )
}

/// Marca campos que precisam ter ao menos a altura de uma linha de texto (`min_height` vindo do tamanho da fonte).
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct LineHeightFloor;

#[allow(clippy::type_complexity)]
fn line_floor(mut q: Query<(&mut Node, &TextFont), (With<LineHeightFloor>, Or<(Changed<Node>, Changed<TextFont>)>)>) {
    for (mut n, f) in &mut q {
        let pad = |v: Val| if let Val::Px(x) = v { x } else { 0.0 };
        let bw = |v: Val| if let Val::Px(x) = v { x } else { 0.0 };
        let fs = if let bevy::text::FontSize::Px(x) = f.font_size { x } else { 16.0 };
        let want =
            Val::Px(fs * 1.3 + pad(n.padding.top) + pad(n.padding.bottom) + bw(n.border.top) + bw(n.border.bottom));
        if n.min_height != want {
            n.min_height = want;
        }
    }
}

/// Texto cinza mostrado num campo vazio (o `placeholder` do HTML; o `EditableText` do Bevy ainda não tem).
#[derive(Component, Debug, Clone, Default)]
pub struct Placeholder(pub String);

#[derive(Component, Debug)]
struct PlaceholderText;

/// Borda do campo: a de sempre e a de quando ele tem o foco (`:focus` do original).
#[derive(Component, Debug, Clone, Copy)]
pub struct FocusRing {
    pub idle: Color,
    pub focus: Color,
}

/// Botão apertado pelo ponteiro, para completar um clique que o Bevy perdeu (ver [`release_elsewhere`]).
#[derive(Resource, Debug, Default)]
struct LastPress(Option<Entity>);

fn remember_press(
    press: On<bevy::picking::events::PointerPress>,
    buttons: Query<(), With<Button>>,
    mut last: ResMut<LastPress>,
) {
    if press.button == PointerButton::Primary && buttons.contains(press.entity) {
        last.0 = Some(press.entity);
    }
}

/// Clique rápido num quadro lento: quando o aperto e a soltura caem no mesmo quadro (PC lento, painel pesado aberto),
/// o Bevy entrega o aperto ao botão mas a soltura à janela. Sem `Click`, o botão não dispara [`Activate`] e ainda fica
/// preso em `Pressed`, o que faz o clique seguinte falhar também (o "fechar painel" que não fechava). Aqui, se a soltura
/// não foi no botão apertado, mas o ponteiro ainda está dentro dele, o clique é completado; fora dele, só destrava.
#[allow(clippy::type_complexity)]
fn release_elsewhere(
    release: On<bevy::picking::events::PointerRelease>,
    mut last: ResMut<LastPress>,
    parents: Query<&ChildOf>,
    nodes: Query<(&ComputedNode, &bevy::ui::UiGlobalTransform, Has<bevy::ui::InteractionDisabled>), With<Button>>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut commands: Commands,
) {
    if release.button != PointerButton::Primary || release.entity != release.original_event_target() {
        return;
    }
    let Some(b) = last.0.take() else { return };
    // soltou no próprio botão (ou num filho dele): o Bevy cuida
    let mut e = release.entity;
    loop {
        if e == b {
            return;
        }
        match parents.get(e) {
            Ok(p) => e = p.parent(),
            Err(_) => break,
        }
    }
    let Ok((node, tf, disabled)) = nodes.get(b) else { return };
    // tudo em pixels físicos: o UiScale e a escala da tela não entram na conta
    let center = tf.translation;
    let half = node.size() / 2.0;
    let inside = windows
        .single()
        .ok()
        .and_then(Window::physical_cursor_position)
        .is_some_and(|p| (p - center).abs().cmple(half).all());
    commands.entity(b).remove::<bevy::ui::Pressed>();
    if !disabled && inside {
        commands.trigger(Activate { entity: b });
    }
}

/// Shift+Espaço digita um espaço, como no navegador (o `TextInput` do Bevy 0.20 só aceita o espaço sem modificador, e
/// quem escreve "Ana Clara" ainda segurando o Shift do "C" perdia o espaço).
fn shift_space(
    ev: On<bevy::input_focus::FocusedInput<bevy::input::keyboard::KeyboardInput>>,
    keys: Res<ButtonInput<bevy::input::keyboard::Key>>,
    mut q: Query<&mut bevy::text::EditableText, With<bevy::ui_widgets::TextInput>>,
) {
    use bevy::input::keyboard::Key;
    let k = &ev.input;
    if k.logical_key != Key::Space || !k.state.is_pressed() {
        return;
    }
    let others = keys.pressed(Key::Control) || keys.pressed(Key::Alt) || keys.pressed(Key::Super);
    if !keys.pressed(Key::Shift) || others {
        return;
    }
    if let Ok(mut e) = q.get_mut(ev.focused_entity) {
        e.queue_edit(bevy::text::TextEdit::Insert(k.text.clone().unwrap_or_else(|| " ".into())));
    }
}

/// Qualquer nó com rolagem vertical rola com a roda do mouse (`ScrollArea`), como as `div` do original.
#[allow(clippy::type_complexity)]
fn scroll_areas(
    mut commands: Commands,
    q: Query<(Entity, &Node), (Changed<Node>, Without<bevy::ui_widgets::ScrollArea>)>,
) {
    for (e, n) in &q {
        if n.overflow.y == OverflowAxis::Scroll || n.overflow.x == OverflowAxis::Scroll {
            commands.entity(e).insert(bevy::ui_widgets::ScrollArea);
        }
    }
}

#[allow(clippy::type_complexity)]
fn placeholders(
    mut commands: Commands,
    added: Query<(Entity, &Placeholder, &Node, &TextFont, &TextColor), Added<Placeholder>>,
    inputs: Query<(&bevy::text::EditableText, &Children), With<Placeholder>>,
    mut texts: Query<&mut Visibility, With<PlaceholderText>>,
) {
    for (e, p, n, font, color) in &added {
        if p.0.is_empty() {
            continue;
        }
        let child = commands
            .spawn((
                PlaceholderText,
                Text::new(p.0.clone()),
                font.clone(),
                TextColor(color.0.with_alpha(0.45)),
                Node { position_type: PositionType::Absolute, left: n.padding.left, top: n.padding.top, ..default() },
                Pickable::IGNORE,
                Visibility::Hidden,
            ))
            .id();
        commands.entity(e).add_child(child);
    }
    for (ed, kids) in &inputs {
        let empty = ed.value().to_string().is_empty();
        for k in kids {
            if let Ok(mut v) = texts.get_mut(*k) {
                let want = if empty { Visibility::Inherited } else { Visibility::Hidden };
                if *v != want {
                    *v = want;
                }
            }
        }
    }
}

fn focus_ring(focus: Res<bevy::input_focus::InputFocus>, mut q: Query<(Entity, &FocusRing, &mut BorderColor)>) {
    for (e, r, mut b) in &mut q {
        let c = if focus.get() == Some(e) { r.focus } else { r.idle };
        if b.top != c {
            *b = BorderColor::all(c);
        }
    }
}

/// Texto do campo, sem quebras de linha nem controles.
pub fn input_value(e: &bevy::text::EditableText) -> String {
    e.value().to_string().chars().filter(|c| !c.is_control()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shape_labels_use_the_symbol_font() {
        assert!(is_shape_label("◀") && is_shape_label("▶") && is_shape_label(" ▼ "));
        assert!(!is_shape_label("🎲 Aleatório") && !is_shape_label("") && !is_shape_label("◀ voltar"));
    }

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
