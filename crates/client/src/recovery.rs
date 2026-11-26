//! Código de recuperação de uma conta criada agora (como o navegador faz logo depois de `enroll`): o código v2 leva o
//! id, a chave privada de recuperação, a chave de amigo, o nome e o visual, e abre também no navegador, que faz a
//! recuperação completa num aparelho novo.
//!
//! A chave de recuperação só existe na memória: o código aparece numa janela com 📋 Copiar e some quando o jogador
//! confirma que guardou (ou ao sair da sala). Nunca é gravado em disco.

use crate::menu::despawn;
use crate::state::Screen;
use bevy::clipboard::Clipboard;
use bevy::prelude::*;
use sky_identity::recovery::{RecoveryCode, encode_v2};
use sky_ui::{Activate, Theme, UiFonts, button, panel};

#[derive(Debug)]
pub struct RecoveryPlugin;

impl Plugin for RecoveryPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, show.run_if(in_state(Screen::Playing)))
            .add_systems(OnExit(Screen::Playing), (despawn::<RecoveryPanel>, forget));
    }
}

/// Código a mostrar (só na memória).
#[derive(Resource)]
pub struct PendingCode(String);

impl std::fmt::Debug for PendingCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PendingCode(…)")
    }
}

#[derive(Component, Debug)]
struct RecoveryPanel;
#[derive(Component, Debug)]
struct CopyNote;

/// Monta o código de uma conta nova com o nome e o visual do jogador; `None` se algo não serve (não deve acontecer).
pub fn code_for(acc: &sky_networking::NewAccount, name: &str, look: &sky_avatars::Cfg) -> Option<PendingCode> {
    let fields = sky_identity::recovery::FIELDS.iter().map(|k| ((*k).to_owned(), look.get(k))).collect();
    let gloss = ["tb", "eb", "rb", "hb", "sb", "pb", "cb"].iter().any(|k| look.flag(k));
    encode_v2(&RecoveryCode {
        id: acc.id.clone(),
        rec: acc.recovery_pkcs8.clone(),
        fk: acc.k.clone(),
        name: name.to_owned(),
        look: fields,
        gloss,
        friends: Vec::new(),
    })
    .map(PendingCode)
}

fn forget(mut commands: Commands) {
    commands.remove_resource::<PendingCode>();
}

fn show(
    mut commands: Commands,
    code: Option<Res<PendingCode>>,
    shown: Query<(), With<RecoveryPanel>>,
    t: Res<Theme>,
    f: Res<UiFonts>,
) {
    let Some(code) = code else { return };
    if !shown.is_empty() {
        return;
    }
    let root = commands
        .spawn(panel(&t))
        .insert((
            RecoveryPanel,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(50.0),
                top: Val::Px(60.0),
                margin: UiRect::left(Val::Px(-280.0)),
                width: Val::Px(560.0),
                padding: UiRect::all(Val::Px(16.0)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(12.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(10.0),
                ..default()
            },
            GlobalZIndex(10),
        ))
        .id();
    let title =
        commands.spawn((Text::new("🔑 Guarde seu código de recuperação"), f.font(18.0, 700), TextColor(t.strong))).id();
    let info = commands
        .spawn((
            Text::new(
                "Sua conta foi criada neste computador. Este código é a única forma de recuperá-la em outro aparelho \
                 (aqui ou no navegador) se este arquivo for perdido. Ele não fica guardado no jogo: copie e guarde em \
                 lugar seguro, e não mostre a ninguém.",
            ),
            f.font(15.0, 500),
            TextColor(t.fg),
        ))
        .id();
    let text = commands
        .spawn((
            Text::new(code.0.clone()),
            f.font(15.0, 700),
            TextColor(t.fg),
            Node {
                padding: UiRect::all(Val::Px(10.0)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(8.0)),
                ..default()
            },
            BorderColor::all(t.border),
        ))
        .id();
    let note = commands.spawn((Text::new(""), f.font(14.0, 600), TextColor(t.fg), CopyNote)).id();
    let row = commands.spawn(Node { column_gap: Val::Px(8.0), ..default() }).id();
    let copy = button(&mut commands, &t, &f, "📋 Copiar", false);
    commands.entity(copy).observe(
        |_: On<Activate>,
         code: Option<Res<PendingCode>>,
         mut cb: ResMut<Clipboard>,
         mut notes: Query<&mut Text, With<CopyNote>>| {
            let Some(code) = code else { return };
            let msg = match cb.set_text(code.0.clone()) {
                Ok(()) => "✔ Copiado. Cole num lugar seguro antes de fechar.",
                Err(_) => "⚠ Não deu para copiar: anote o código à mão.",
            };
            for mut n in &mut notes {
                n.0 = msg.into();
            }
        },
    );
    let done = button(&mut commands, &t, &f, "Já guardei", true);
    commands.entity(done).observe(|_: On<Activate>, mut commands: Commands, q: Query<Entity, With<RecoveryPanel>>| {
        commands.remove_resource::<PendingCode>();
        for e in &q {
            commands.entity(e).despawn();
        }
    });
    commands.entity(row).add_children(&[copy, done]);
    commands.entity(root).add_children(&[title, info, text, note, row]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_opens_with_account_and_look() {
        let mut look = sky_avatars::Cfg::default();
        look.0.insert("hs".into(), 7);
        look.0.insert("sb".into(), 1);
        let acc = sky_networking::NewAccount {
            id: "abcdefgh2345".into(),
            k: sky_identity::new_device_key(),
            recovery_pkcs8: sky_identity::KeyPair::generate().to_pkcs8().unwrap(),
        };
        let c = code_for(&acc, "Ana", &look).unwrap();
        let d = sky_identity::recovery::decode_v2(&c.0).unwrap();
        assert_eq!((d.id.as_str(), d.name.as_str(), d.look["hs"], d.gloss), ("abcdefgh2345", "Ana", 7, true));
        assert_eq!(d.fk, acc.k);
        assert!(sky_identity::KeyPair::from_pkcs8(&d.rec).is_ok());
    }
}
