//! Swift-style host chrome over the Bevy 3D view (parity with SpecChumMac ContentView).
//!
//! Toolbar buttons + glass status footer — **no bare letter hotkeys**. Spectrum matrix
//! keeps O/P/I/M/etc. Host actions use clicks or ⌘ shortcuts (like the macOS shell).

use bevy::prelude::*;

use crate::audio::{AudioMuted, AudioOut};
use crate::camera::CameraIntro;
use crate::file_dialog::OpenMediaDialog;
use crate::host::{model_label, EmulatorHost};
use spec_chum_host::ModelId;

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
enum ChromeAction {
    Open,
    Instant,
    Play,
    Mute,
    Pause,
    Reset,
    Model16K,
    Model48,
    Model128,
    ModelPlus2,
    ModelPlus2A,
    ModelPlus3,
    ModelPentagon,
}

#[derive(Component, Debug)]
struct StatusText;

#[derive(Component, Debug)]
struct IntroHintText;

#[derive(Component, Debug)]
struct MuteLabel;

#[derive(Component, Debug)]
struct PauseLabel;

/// Frosted bar — approximates `glassBarBackground()` when true vibrancy isn't available.
const GLASS_BG: Color = Color::srgba(0.14, 0.14, 0.16, 0.78);
const GLASS_BORDER: Color = Color::srgba(1.0, 1.0, 1.0, 0.10);
const TOOLBAR_BG: Color = Color::srgba(0.11, 0.11, 0.13, 0.92);
const BTN_BG: Color = Color::srgba(0.22, 0.22, 0.26, 0.95);
const BTN_HOVER: Color = Color::srgba(0.32, 0.32, 0.38, 0.98);
const BTN_PRESS: Color = Color::srgba(0.18, 0.18, 0.22, 1.0);
const FG: Color = Color::srgb(0.94, 0.94, 0.90);
const FG_DIM: Color = Color::srgb(0.72, 0.72, 0.68);

#[derive(Debug, Default)]
pub struct UiOverlayPlugin;

impl Plugin for UiOverlayPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_ui)
            .add_systems(Update, (chrome_buttons, update_labels, host_cmd_shortcuts));
    }
}

fn setup_ui(mut commands: Commands) {
    commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::SpaceBetween,
                padding: UiRect::axes(Val::Px(12.0), Val::Px(10.0)),
                ..default()
            },
            BackgroundColor(Color::NONE),
            GlobalZIndex(100),
            Name::new("ui_chrome_root"),
        ))
        .with_children(|root| {
            // Top toolbar — primary actions (Swift `.toolbar` analogue).
            root.spawn((
                Node {
                    width: Val::Percent(100.0),
                    min_height: Val::Px(44.0),
                    padding: UiRect::axes(Val::Px(10.0), Val::Px(8.0)),
                    column_gap: Val::Px(8.0),
                    align_items: AlignItems::Center,
                    flex_wrap: FlexWrap::Wrap,
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::all(Val::Px(10.0)),
                    ..default()
                },
                BackgroundColor(TOOLBAR_BG),
                BorderColor::all(GLASS_BORDER),
            ))
            .with_children(|bar| {
                chrome_button(bar, "Open", ChromeAction::Open);
                chrome_button(bar, "Instant", ChromeAction::Instant);
                chrome_button(bar, "Play", ChromeAction::Play);
                chrome_button(bar, "Mute", ChromeAction::Mute);
                chrome_button(bar, "Pause", ChromeAction::Pause);
                chrome_button(bar, "Reset", ChromeAction::Reset);
                spacer(bar);
                chrome_button(bar, "16K", ChromeAction::Model16K);
                chrome_button(bar, "48K", ChromeAction::Model48);
                chrome_button(bar, "128K", ChromeAction::Model128);
                chrome_button(bar, "+2", ChromeAction::ModelPlus2);
                chrome_button(bar, "+2A", ChromeAction::ModelPlus2A);
                chrome_button(bar, "+3", ChromeAction::ModelPlus3);
                chrome_button(bar, "Pent", ChromeAction::ModelPentagon);
            });

            // Middle: pure 3D (intro hint only when dollying).
            root.spawn((
                Node {
                    width: Val::Percent(100.0),
                    flex_grow: 1.0,
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::FlexStart,
                    padding: UiRect::top(Val::Px(8.0)),
                    ..default()
                },
                BackgroundColor(Color::NONE),
            ))
            .with_children(|mid| {
                mid.spawn((
                    Text::new(""),
                    TextFont::from_font_size(14.0),
                    TextColor(FG_DIM),
                    IntroHintText,
                ));
            });

            // Bottom glass status footer (Swift `statusFooter` / `glassBarBackground`).
            root.spawn((
                Node {
                    width: Val::Percent(100.0),
                    min_height: Val::Px(36.0),
                    padding: UiRect::axes(Val::Px(14.0), Val::Px(8.0)),
                    align_items: AlignItems::Center,
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::all(Val::Px(10.0)),
                    ..default()
                },
                BackgroundColor(GLASS_BG),
                BorderColor::all(GLASS_BORDER),
            ))
            .with_children(|foot| {
                foot.spawn((
                    Text::new(""),
                    TextFont::from_font_size(13.0),
                    TextColor(FG_DIM),
                    StatusText,
                ));
            });
        });
}

fn spacer(parent: &mut ChildSpawnerCommands) {
    parent.spawn((
        Node {
            width: Val::Px(12.0),
            height: Val::Px(1.0),
            ..default()
        },
        BackgroundColor(Color::NONE),
    ));
}

fn chrome_button(parent: &mut ChildSpawnerCommands, label: &str, action: ChromeAction) {
    parent
        .spawn((
            Button,
            Node {
                padding: UiRect::axes(Val::Px(12.0), Val::Px(7.0)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(8.0)),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(BTN_BG),
            BorderColor::all(GLASS_BORDER),
            action,
        ))
        .with_children(|btn| {
            let mut text = btn.spawn((
                Text::new(label),
                TextFont::from_font_size(13.0),
                TextColor(FG),
            ));
            if action == ChromeAction::Mute {
                text.insert(MuteLabel);
            }
            if action == ChromeAction::Pause {
                text.insert(PauseLabel);
            }
        });
}

// Bevy Changed<Interaction> filter pack for chrome buttons (#171).
#[allow(clippy::type_complexity)]
fn chrome_buttons(
    mut interaction: Query<
        (&Interaction, &ChromeAction, &mut BackgroundColor),
        (Changed<Interaction>, With<Button>),
    >,
    mut host: ResMut<EmulatorHost>,
    mut muted: ResMut<AudioMuted>,
    mut open_dialog: ResMut<OpenMediaDialog>,
    audio: Option<Res<AudioOut>>,
) {
    for (interaction, action, mut bg) in &mut interaction {
        match *interaction {
            Interaction::Pressed => {
                *bg = BackgroundColor(BTN_PRESS);
                match *action {
                    ChromeAction::Open => {
                        // Never call sync rfd here — beachballs Bevy on macOS.
                        open_dialog.request = true;
                    }
                    ChromeAction::Instant => host.set_instant(),
                    ChromeAction::Play => host.set_play_ear(),
                    ChromeAction::Mute => {
                        muted.0 = !muted.0;
                        if muted.0 {
                            if let Some(a) = audio.as_ref() {
                                a.clear();
                            }
                        }
                        // Best-effort persist mute into the shared prefs file (#186).
                        let path = spec_chum_host::default_prefs_path();
                        let _ = spec_chum_host::update_prefs(&path, |prefs| {
                            prefs.muted = muted.0;
                        });
                    }
                    ChromeAction::Pause => toggle_pause(&mut host),
                    ChromeAction::Reset => host.reset(),
                    ChromeAction::Model16K => {
                        select_model_if_rom(&mut host, ModelId::Spectrum16K);
                    }
                    ChromeAction::Model48 => {
                        select_model_if_rom(&mut host, ModelId::Spectrum48);
                    }
                    ChromeAction::Model128 => {
                        select_model_if_rom(&mut host, ModelId::Spectrum128);
                    }
                    ChromeAction::ModelPlus2 => {
                        select_model_if_rom(&mut host, ModelId::SpectrumPlus2);
                    }
                    ChromeAction::ModelPlus2A => {
                        select_model_if_rom(&mut host, ModelId::SpectrumPlus2A);
                    }
                    ChromeAction::ModelPlus3 => {
                        select_model_if_rom(&mut host, ModelId::SpectrumPlus3);
                    }
                    ChromeAction::ModelPentagon => {
                        select_model_if_rom(&mut host, ModelId::Pentagon128);
                    }
                }
            }
            Interaction::Hovered => *bg = BackgroundColor(BTN_HOVER),
            Interaction::None => *bg = BackgroundColor(BTN_BG),
        }
    }
}

fn toggle_pause(host: &mut EmulatorHost) {
    host.paused_overlay = !host.paused_overlay;
    let paused = host.paused_overlay;
    host.session.set_paused(paused);
    let label = model_label(host.session.model());
    host.status = if paused {
        format!("{label} · Paused")
    } else {
        format!("{label} · Running")
    };
}

fn persist_living_room_model(model: ModelId) {
    let path = spec_chum_host::default_prefs_path();
    let _ = spec_chum_host::update_prefs(&path, |prefs| {
        prefs.set_model_from_id(model);
    });
}

fn select_model_if_rom(host: &mut EmulatorHost, model: ModelId) {
    if !model.rom_available() {
        host.status = format!("{} — {}", model_label(model), model.unavailable_reason());
        return;
    }
    host.select_model(model);
    persist_living_room_model(model);
}

/// ⌘ shortcuts mirroring SpecChumMac — never bare letters (those are Spectrum).
fn host_cmd_shortcuts(
    keys: Res<ButtonInput<KeyCode>>,
    mut host: ResMut<EmulatorHost>,
    mut open_dialog: ResMut<OpenMediaDialog>,
) {
    let cmd = keys.pressed(KeyCode::SuperLeft)
        || keys.pressed(KeyCode::SuperRight)
        || keys.pressed(KeyCode::ControlLeft)
        || keys.pressed(KeyCode::ControlRight);
    if !cmd {
        return;
    }
    let alt = keys.pressed(KeyCode::AltLeft) || keys.pressed(KeyCode::AltRight);

    if keys.just_pressed(KeyCode::KeyO) && !alt {
        open_dialog.request = true;
    }
    // Swift: Pause is ⌥⌘P
    if alt && keys.just_pressed(KeyCode::KeyP) {
        toggle_pause(&mut host);
    }
    if keys.just_pressed(KeyCode::KeyR) && !alt {
        host.reset();
    }
}

// Bevy multi-label Query filters for status chrome (#171).
#[allow(clippy::type_complexity)]
fn update_labels(
    host: Res<EmulatorHost>,
    muted: Res<AudioMuted>,
    intro: Option<Res<CameraIntro>>,
    mut status: Query<
        &mut Text,
        (
            With<StatusText>,
            Without<IntroHintText>,
            Without<MuteLabel>,
            Without<PauseLabel>,
        ),
    >,
    mut intro_hint: Query<
        &mut Text,
        (
            With<IntroHintText>,
            Without<StatusText>,
            Without<MuteLabel>,
            Without<PauseLabel>,
        ),
    >,
    mut mute_lbl: Query<
        &mut Text,
        (
            With<MuteLabel>,
            Without<StatusText>,
            Without<IntroHintText>,
            Without<PauseLabel>,
        ),
    >,
    mut pause_lbl: Query<
        &mut Text,
        (
            With<PauseLabel>,
            Without<StatusText>,
            Without<IntroHintText>,
            Without<MuteLabel>,
        ),
    >,
) {
    if let Ok(mut text) = status.single_mut() {
        *text = Text::new(host.status.clone());
    }
    if let Ok(mut text) = intro_hint.single_mut() {
        *text = if intro.is_some() {
            Text::new("Intro — click / Space to skip")
        } else {
            Text::new("")
        };
    }
    if let Ok(mut text) = mute_lbl.single_mut() {
        *text = Text::new(if muted.0 { "Unmute" } else { "Mute" });
    }
    if let Ok(mut text) = pause_lbl.single_mut() {
        *text = Text::new(if host.paused_overlay {
            "Continue"
        } else {
            "Pause"
        });
    }
}
