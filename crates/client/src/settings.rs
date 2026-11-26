//! ⚙️ Ajustes (`#pSet` no original): tamanho da interface, brilho, tema (escuro ou claro), alto contraste e bots na
//! sala (só o anfitrião muda). Ficam em `skyke3d/online/ajustes.json` (preferências locais, como o `localStorage`).
//! Volumes e opções de microfone entram com o áudio e a voz.

use crate::state::{ChatLog, Screen, SendMsg, Session};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use sky_protocol::ClientMsg;
use sky_rendering::ToonMaterial;
use sky_ui::{Activate, Theme, ThemeKind, UiFonts, button, panel};

#[derive(Debug)]
pub struct SettingsPlugin;

impl Plugin for SettingsPlugin {
    fn build(&self, app: &mut App) {
        let s = Settings::load();
        app.insert_resource(Theme::new(s.theme_kind()))
            .insert_resource(UiScale(s.ui))
            .insert_resource(s)
            .init_resource::<Bots>()
            .add_systems(OnEnter(Screen::Playing), spawn_panel)
            .add_systems(OnExit(Screen::Playing), crate::menu::despawn::<SettingsPanel>)
            .add_systems(Update, (apply, brightness, refresh_labels));
    }
}

/// Preferências salvas.
#[derive(Resource, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Tamanho da interface (`--ui`): 0,8 a 1,5.
    pub ui: f32,
    /// Brilho da cena (`BRI`): 0,6 a 1,6.
    pub br: f32,
    /// Tema claro (senão escuro).
    pub light: bool,
    /// Alto contraste.
    pub hc: bool,
    /// Volumes de efeitos, ambiente e música (0 a 1) e se a música toca.
    pub sfx: f32,
    pub amb: f32,
    pub mus: f32,
    pub music: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { ui: 1.0, br: 1.0, light: false, hc: false, sfx: 0.8, amb: 0.7, mus: 0.6, music: true }
    }
}

impl Settings {
    fn path() -> std::path::PathBuf {
        crate::state::data_dir().join("online/ajustes.json")
    }

    fn load() -> Self {
        let s: Self = sky_persistence::load_json(&Self::path(), 4096).ok().flatten().unwrap_or_default();
        s.clamped()
    }

    /// Valores fora da faixa (arquivo editado à mão) voltam para dentro dela.
    fn clamped(mut self) -> Self {
        let fix = |v: f32, lo: f32, hi: f32| if v.is_finite() { v.clamp(lo, hi) } else { 1.0 };
        self.ui = fix(self.ui, 0.8, 1.5);
        self.br = fix(self.br, 0.6, 1.6);
        let vol = |v: f32, d: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { d };
        self.sfx = vol(self.sfx, 0.8);
        self.amb = vol(self.amb, 0.7);
        self.mus = vol(self.mus, 0.6);
        self
    }

    fn theme_kind(&self) -> ThemeKind {
        match (self.hc, self.light) {
            (true, _) => ThemeKind::HighContrast,
            (false, true) => ThemeKind::Light,
            (false, false) => ThemeKind::Dark,
        }
    }
}

/// Bots pedidos para a sala (o servidor só aceita do anfitrião).
#[derive(Resource, Debug, Default)]
struct Bots(u32);

#[derive(Component, Debug)]
pub struct SettingsPanel;
/// Texto de valor de uma linha (atualizado quando os ajustes mudam).
#[derive(Component, Debug, Clone, Copy)]
enum Value {
    Ui,
    Br,
    Sfx,
    Amb,
    Mus,
    Music,
    Theme,
    Hc,
    Bots,
}

pub fn toggle(q: &mut Query<&mut Node, With<SettingsPanel>>) {
    for mut n in q.iter_mut() {
        n.display = if n.display == Display::None { Display::Flex } else { Display::None };
    }
}

fn step(v: f32, d: f32, lo: f32, hi: f32) -> f32 {
    ((v + d) * 20.0).round().clamp(lo * 20.0, hi * 20.0) / 20.0
}

fn spawn_panel(mut commands: Commands, t: Res<Theme>, f: Res<UiFonts>) {
    let root = commands
        .spawn(panel(&t))
        .insert((
            SettingsPanel,
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(12.0),
                bottom: Val::Px(64.0),
                width: Val::Px(360.0),
                padding: UiRect::all(Val::Px(12.0)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(12.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(8.0),
                display: Display::None,
                ..default()
            },
        ))
        .id();
    let title = commands.spawn((Text::new("⚙️ Ajustes"), f.font(15.0, 700), TextColor(t.strong))).id();
    commands.entity(root).add_child(title);
    let row = |commands: &mut Commands, label: &str, v: Value, dec: Entity, inc: Entity| {
        let r = commands.spawn(Node { align_items: AlignItems::Center, column_gap: Val::Px(6.0), ..default() }).id();
        let l = commands
            .spawn((Text::new(label), f.font(15.0, 600), TextColor(t.fg), Node { width: Val::Px(110.0), ..default() }))
            .id();
        let val = commands
            .spawn((
                Text::new(""),
                f.font(15.0, 500),
                TextColor(t.fg),
                v,
                Node { min_width: Val::Px(90.0), justify_content: JustifyContent::Center, ..default() },
            ))
            .id();
        commands.entity(r).add_children(&[l, dec, val, inc]);
        r
    };
    let mut rows = Vec::new();
    for (label, v, lo, hi) in [
        ("Interface", Value::Ui, 0.8, 1.5),
        ("Brilho", Value::Br, 0.6, 1.6),
        ("Efeitos", Value::Sfx, 0.0, 1.0),
        ("Ambiente", Value::Amb, 0.0, 1.0),
        ("Música", Value::Mus, 0.0, 1.0),
    ] {
        let dec = button(&mut commands, &t, &f, "−", false);
        let inc = button(&mut commands, &t, &f, "+", false);
        for (b, d) in [(dec, -0.05f32), (inc, 0.05)] {
            let d = if matches!(v, Value::Sfx | Value::Amb | Value::Mus) { d * 2.0 } else { d };
            commands.entity(b).observe(move |_: On<Activate>, mut s: ResMut<Settings>| match v {
                Value::Ui => s.ui = step(s.ui, d, lo, hi),
                Value::Sfx => s.sfx = step(s.sfx, d, lo, hi),
                Value::Amb => s.amb = step(s.amb, d, lo, hi),
                Value::Mus => s.mus = step(s.mus, d, lo, hi),
                _ => s.br = step(s.br, d, lo, hi),
            });
        }
        rows.push(row(&mut commands, label, v, dec, inc));
    }
    let dark = button(&mut commands, &t, &f, "Escuro", false);
    let light = button(&mut commands, &t, &f, "Claro", false);
    for (b, l) in [(dark, false), (light, true)] {
        commands.entity(b).observe(move |_: On<Activate>, mut s: ResMut<Settings>| s.light = l);
    }
    rows.push(row(&mut commands, "Tema", Value::Theme, dark, light));
    let hc = button(&mut commands, &t, &f, "Alto contraste", false);
    commands.entity(hc).observe(|_: On<Activate>, mut s: ResMut<Settings>| s.hc = !s.hc);
    let hc_val = commands.spawn((Text::new(""), f.font(15.0, 500), TextColor(t.fg), Value::Hc)).id();
    let hc_row = commands.spawn(Node { align_items: AlignItems::Center, column_gap: Val::Px(8.0), ..default() }).id();
    commands.entity(hc_row).add_children(&[hc, hc_val]);
    rows.push(hc_row);
    let mu = button(&mut commands, &t, &f, "🎵 Música", false);
    commands.entity(mu).observe(|_: On<Activate>, mut s: ResMut<Settings>| s.music = !s.music);
    let mu_val = commands.spawn((Text::new(""), f.font(15.0, 500), TextColor(t.fg), Value::Music)).id();
    let mu_row = commands.spawn(Node { align_items: AlignItems::Center, column_gap: Val::Px(8.0), ..default() }).id();
    commands.entity(mu_row).add_children(&[mu, mu_val]);
    rows.push(mu_row);
    let dec = button(&mut commands, &t, &f, "−", false);
    let inc = button(&mut commands, &t, &f, "+", false);
    for (b, d) in [(dec, -1i32), (inc, 1)] {
        commands.entity(b).observe(
            move |_: On<Activate>,
                  mut bots: ResMut<Bots>,
                  s: Res<Session>,
                  mut send: MessageWriter<SendMsg>,
                  mut log: ResMut<ChatLog>| {
                if !s.online {
                    log.push("", "Sem conexão: os bots são da sala do servidor.", true);
                    return;
                }
                bots.0 = (bots.0 as i32 + d).clamp(0, sky_shared::limits::BOTS as i32) as u32;
                send.write(SendMsg(ClientMsg::Bots { n: bots.0 }));
            },
        );
    }
    rows.push(row(&mut commands, "Bots na sala", Value::Bots, dec, inc));
    let note = commands.spawn((Text::new("Só o anfitrião muda os bots."), f.font(13.0, 500), TextColor(t.fg))).id();
    rows.push(note);
    commands.entity(root).add_children(&rows);
}

/// Ajustes mudaram: aplica e grava.
fn apply(s: Res<Settings>, mut theme: ResMut<Theme>, mut scale: ResMut<UiScale>) {
    if !s.is_changed() || s.is_added() {
        return;
    }
    if scale.0 != s.ui {
        scale.0 = s.ui;
    }
    let kind = s.theme_kind();
    if theme.kind != kind {
        *theme = Theme::new(kind);
    }
    if let Err(e) = sky_persistence::save_json(&Settings::path(), &*s, false) {
        warn!("ajustes: {e}");
    }
}

/// Brilho em todos os materiais (valor absoluto: os criados depois, como os de um mundo novo, recebem o mesmo).
fn brightness(
    s: Res<Settings>,
    mut mats: ResMut<Assets<ToonMaterial>>,
    mut events: MessageReader<AssetEvent<ToonMaterial>>,
) {
    let added: Vec<_> =
        events.read().filter_map(|e| if let AssetEvent::Added { id } = e { Some(*id) } else { None }).collect();
    if s.is_changed() {
        let ids: Vec<_> = mats.ids().collect();
        for id in ids {
            if mats.get(id).is_some_and(|m| m.brightness() != s.br)
                && let Some(mut m) = mats.get_mut(id)
            {
                m.set_brightness(s.br);
            }
        }
    } else {
        for id in added {
            if mats.get(id).is_some_and(|m| m.brightness() != s.br)
                && let Some(mut m) = mats.get_mut(id)
            {
                m.set_brightness(s.br);
            }
        }
    }
}

fn refresh_labels(s: Res<Settings>, bots: Res<Bots>, mut q: Query<(Ref<Value>, &mut Text)>) {
    let all = s.is_changed() || bots.is_changed();
    for (v, mut t) in &mut q {
        if !all && !v.is_added() {
            continue;
        }
        t.0 = match *v {
            Value::Ui => format!("{:.0}%", s.ui * 100.0),
            Value::Br => format!("{:.0}%", s.br * 100.0),
            Value::Theme => if s.light { "Claro" } else { "Escuro" }.into(),
            Value::Hc => if s.hc { "ligado" } else { "desligado" }.into(),
            Value::Sfx => format!("{:.0}%", s.sfx * 100.0),
            Value::Amb => format!("{:.0}%", s.amb * 100.0),
            Value::Mus => format!("{:.0}%", s.mus * 100.0),
            Value::Music => if s.music { "ligada" } else { "desligada" }.into(),
            Value::Bots => bots.0.to_string(),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_and_limits_like_the_sliders() {
        assert_eq!(step(1.0, 0.05, 0.8, 1.5), 1.05);
        assert_eq!(step(1.5, 0.05, 0.8, 1.5), 1.5);
        assert_eq!(step(0.8, -0.05, 0.8, 1.5), 0.8);
        let s = Settings { ui: 9.0, br: f32::NAN, light: true, sfx: 7.0, ..Settings::default() }.clamped();
        assert_eq!(s.sfx, 1.0);
        assert_eq!((s.ui, s.br), (1.5, 1.0));
        assert_eq!(s.theme_kind(), ThemeKind::Light);
    }
}
