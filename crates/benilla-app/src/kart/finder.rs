//! The Race Finder window (`I`): queue for a race from anywhere, as the dungeon finder does for a
//! dungeon. Tick courses or "Any course" and Find race; the server's ready check, its answer
//! (F5/F6) and the queue's heartbeat are `race.rs`'s, which this only starts and stops.

use bevy::prelude::*;
use bevy::ui::FocusPolicy;

use super::race::{finder_banner, Races, COURSES, RACING_NOTICE};

const GOLD: Color = Color::srgb(1.0, 0.82, 0.0);
const ROW: Color = Color::srgba(1.0, 1.0, 1.0, 0.06);
const ROW_HOVER: Color = Color::srgba(1.0, 0.82, 0.0, 0.25);

#[derive(Resource)]
pub(crate) struct FinderUi {
    open: bool,
    any: bool,
    /// Per course of `COURSES`, ticked.
    picked: Vec<bool>,
}

impl Default for FinderUi {
    fn default() -> Self {
        Self {
            open: false,
            any: true,
            picked: vec![false; COURSES.len()],
        }
    }
}

impl FinderUi {
    /// The queue line's course list: `any`, or the ticked keys.
    fn keys(&self) -> String {
        if self.any {
            return "any".into();
        }
        let keys: Vec<&str> = COURSES
            .iter()
            .zip(&self.picked)
            .filter(|(_, on)| **on)
            .map(|(c, _)| c.key.as_str())
            .collect();
        keys.join(",")
    }
}

#[derive(Component)]
pub(crate) struct FinderRoot;

#[derive(Component)]
pub(crate) struct FinderStatus;

#[derive(Component, Clone, Copy, PartialEq)]
pub(crate) enum FinderButton {
    Any,
    Course(usize),
    Queue,
    Close,
}

pub(crate) fn spawn(mut commands: Commands) {
    let text = |size: f32, color: Color| {
        (
            Text::new(String::new()),
            TextFont {
                font_size: size,
                ..default()
            },
            TextColor(color),
            TextShadow::default(),
        )
    };
    let button = |b: FinderButton| {
        (
            b,
            Interaction::default(),
            FocusPolicy::Block,
            BackgroundColor(ROW),
            Node {
                padding: UiRect::axes(Val::Px(10.0), Val::Px(5.0)),
                margin: UiRect::top(Val::Px(4.0)),
                ..default()
            },
        )
    };
    commands
        .spawn((
            FinderRoot,
            Visibility::Hidden,
            FocusPolicy::Block,
            Interaction::default(),
            BackgroundColor(Color::srgba(0.02, 0.02, 0.05, 0.85)),
            BorderColor::all(GOLD),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(4.0),
                top: Val::Percent(22.0),
                width: Val::Px(300.0),
                padding: UiRect::all(Val::Px(12.0)),
                border: UiRect::all(Val::Px(1.0)),
                flex_direction: FlexDirection::Column,
                ..default()
            },
            GlobalZIndex(12),
            Name::new("race finder"),
        ))
        .with_children(|w| {
            w.spawn(text(24.0, GOLD)).insert(Text::new("Race Finder"));
            w.spawn(button(FinderButton::Any))
                .with_child(text(18.0, Color::WHITE));
            for i in 0..COURSES.len() {
                w.spawn(button(FinderButton::Course(i)))
                    .with_child(text(18.0, Color::WHITE));
            }
            w.spawn((FinderStatus, text(16.0, Color::srgb(0.8, 0.8, 0.8))))
                .insert(Node {
                    margin: UiRect::vertical(Val::Px(8.0)),
                    ..default()
                });
            w.spawn(button(FinderButton::Queue))
                .with_child(text(20.0, GOLD));
            w.spawn(button(FinderButton::Close))
                .with_child(text(16.0, Color::WHITE));
        });
}

/// `I` opens and closes the window; its buttons tick courses and queue or leave.
pub(crate) fn update(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    ui: Res<crate::ui_script::UiKeyboardCapture>,
    mut state: ResMut<FinderUi>,
    mut races: ResMut<Races>,
    profession: super::profession::Profession,
    pressed: Query<(&Interaction, &FinderButton), Changed<Interaction>>,
) {
    if keys.just_pressed(KeyCode::KeyI) && !ui.typing {
        state.open = !state.open;
    }
    if !state.open {
        return;
    }
    for (interaction, button) in &pressed {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let picks_changed = match *button {
            FinderButton::Any => {
                state.any = true;
                state.picked.fill(false);
                true
            }
            FinderButton::Course(i) => {
                state.picked[i] = !state.picked[i];
                state.any = !state.picked.contains(&true);
                true
            }
            FinderButton::Queue => {
                if races.finder.queued.take().is_some() {
                    races.finder.status = None;
                    profession.send("KARTCRAFT\tR\tQ-".into());
                } else {
                    queue(&state, &mut races, &profession, time.elapsed_secs());
                }
                false
            }
            FinderButton::Close => {
                state.open = false;
                false
            }
        };
        // Changing the courses while queued keeps our place with the new choice.
        if picks_changed && races.finder.queued.is_some() {
            queue(&state, &mut races, &profession, time.elapsed_secs());
        }
    }
}

fn queue(
    state: &FinderUi,
    races: &mut Races,
    profession: &super::profession::Profession,
    now: f32,
) {
    // A race waiting to start is left for the queue; one under way keeps us.
    match races.take_lobby() {
        Err(()) => {
            races.finder.notice = Some((RACING_NOTICE, now + 6.0));
            return;
        }
        Ok(Some(id)) => profession.send(format!("KARTCRAFT\tR\tQ\t{id}")),
        Ok(None) => {}
    }
    let keys = state.keys();
    profession.send(format!("KARTCRAFT\tR\tQ+\t{keys}"));
    races.finder.queued = Some(keys);
}

#[allow(clippy::type_complexity)]
pub(crate) fn draw(
    time: Res<Time>,
    state: Res<FinderUi>,
    races: Res<Races>,
    mut root: Query<&mut Visibility, With<FinderRoot>>,
    mut buttons: Query<(&FinderButton, &Interaction, &Children, &mut BackgroundColor)>,
    mut status: Query<&mut Text, With<FinderStatus>>,
    mut texts: Query<&mut Text, Without<FinderStatus>>,
) {
    if let Ok(mut v) = root.single_mut() {
        let want = if state.open {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        v.set_if_neq(want);
    }
    if !state.open {
        return;
    }
    let tick = |on: bool| if on { "[x]" } else { "[  ]" };
    for (button, interaction, children, mut bg) in &mut buttons {
        let label = match *button {
            FinderButton::Any => format!("{} Any course", tick(state.any)),
            FinderButton::Course(i) => format!("{} {}", tick(state.picked[i]), COURSES[i].name),
            FinderButton::Queue if races.finder.queued.is_some() => "Leave queue".into(),
            FinderButton::Queue => "Find race".into(),
            FinderButton::Close => "Close (I)".into(),
        };
        if let Some(mut t) = children.first().and_then(|c| texts.get_mut(*c).ok()) {
            if t.0 != label {
                t.0 = label;
            }
        }
        bg.set_if_neq(BackgroundColor(if *interaction == Interaction::None {
            ROW
        } else {
            ROW_HOVER
        }));
    }
    if let Ok(mut t) = status.single_mut() {
        let line = match finder_banner(&races.finder, time.elapsed_secs()) {
            s if s.is_empty() => "Not queued. Pick courses and find a race.".to_string(),
            s => s,
        };
        if t.0 != line {
            t.0 = line;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_queue_names_any_or_the_ticked_courses() {
        let mut ui = FinderUi::default();
        assert_eq!(ui.keys(), "any");
        ui.any = false;
        ui.picked = vec![true, false, true];
        assert_eq!(ui.keys(), "sprint,stormwind");
    }
}
