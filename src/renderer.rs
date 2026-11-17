use crate::messages::{
    RendererWorkerMessage, SimulationDate, SimulationDayType, SimulationRendererMessage,
    SimulationSpeed, SimulationTime, TimeOfDayPeriod, UiInput,
};
use crate::room::{RoomId, RoomType};
use crate::tower::{FLOORS_ABOVE_GROUND, FLOORS_BELOW_GROUND, MAX_FLOOR_WIDTH_COLUMNS};
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::transform::TransformSystems;
use bevy::ui::Pressed;
use bevy::ui_widgets::{Activate, Button};
use bevy::window::PrimaryWindow;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};

// Tower positions are converted from columns/floors into these Bevy world-space units.
const COLUMN_WIDTH: f32 = 8.0;
const FLOOR_HEIGHT: f32 = 24.0;
const FLOOR_THICKNESS: f32 = 6.0;
const CAMERA_SPEED: f32 = 700.0;
const CAMERA_MIN_SCALE: f32 = 0.35;
const CAMERA_MAX_SCALE: f32 = 3.0;
const CAMERA_ZOOM_RATE: f32 = 1.2;
const HUD_HEIGHT: f32 = 78.0;
const ROOM_MENU_LEFT: f32 = 12.0;
const ROOM_MENU_TOP: f32 = 90.0;
const ROOM_MENU_WIDTH: f32 = 190.0;
const ROOM_MENU_HEIGHT: f32 = 220.0;
const ERROR_TOOLTIP_WIDTH: f32 = 300.0;
const ERROR_TOOLTIP_HEIGHT: f32 = 72.0;

// std channels are not Sync; the mutex wrappers let Bevy store their receivers as a Resource.
#[derive(Resource)]
struct RendererChannels {
    to_worker: Sender<RendererWorkerMessage>,
    from_worker: Mutex<Receiver<RendererWorkerMessage>>,
    from_simulation: Mutex<Receiver<SimulationRendererMessage>>,
}

#[derive(Resource)]
struct SimulationClock {
    tick: u64,
    date: SimulationDate,
    time: SimulationTime,
    cash: u64,
}

// This reflects the last speed request successfully queued for the simulation worker.
#[derive(Resource)]
struct ActiveSpeed {
    current: SimulationSpeed,
    last_running: SimulationSpeed,
}

impl ActiveSpeed {
    fn select(&mut self, speed: SimulationSpeed) {
        self.current = speed;
        if speed != SimulationSpeed::Paused {
            self.last_running = speed;
        }
    }

    fn pause_toggle_target(&self) -> SimulationSpeed {
        if self.current == SimulationSpeed::Paused {
            self.last_running
        } else {
            SimulationSpeed::Paused
        }
    }
}

#[derive(Resource, Default)]
struct PlacementState {
    selected_room: Option<RoomType>,
    request_pending: bool,
    last_click_position: Vec2,
    error: Option<String>,
}

impl Default for SimulationClock {
    fn default() -> Self {
        Self {
            tick: 0,
            date: SimulationDate {
                day_counter: 0,
                day_type: SimulationDayType::Weekday1,
                quarter: 1,
                year: 1,
            },
            time: SimulationTime {
                hour: 7,
                minute: 0,
                period: TimeOfDayPeriod::Day,
            },
            cash: 2_000_000,
        }
    }
}

#[derive(Component)]
struct TowerCamera;

#[derive(Component)]
struct HudText;

#[derive(Component)]
struct RoomVisual(RoomId);

#[derive(Component)]
struct SpeedControlButton(SimulationSpeed);

#[derive(Component)]
struct RoomTypeButton(RoomType);

#[derive(Component)]
struct PlacementPreview;

#[derive(Component)]
struct PlacementErrorTooltip;

/// Starts the Bevy app and forwards simulation and worker updates to the scene.
pub fn run(
    to_worker: Sender<RendererWorkerMessage>,
    from_worker: Receiver<RendererWorkerMessage>,
    from_simulation: Receiver<SimulationRendererMessage>,
) {
    let channels = RendererChannels {
        to_worker: to_worker.clone(),
        from_worker: Mutex::new(from_worker),
        from_simulation: Mutex::new(from_simulation),
    };

    App::new()
        .insert_resource(ClearColor(Color::srgb(0.055, 0.07, 0.10)))
        .insert_resource(channels)
        .init_resource::<SimulationClock>()
        .insert_resource(ActiveSpeed {
            current: SimulationSpeed::OneX,
            last_running: SimulationSpeed::OneX,
        })
        .init_resource::<PlacementState>()
        .add_plugins(DefaultPlugins)
        .add_systems(Startup, (setup_scene, setup_hud))
        .add_systems(
            Update,
            (
                move_camera,
                toggle_pause,
                receive_updates,
                update_hud,
                update_button_appearance,
                update_room_button_appearance,
                update_error_tooltip,
            ),
        )
        .add_systems(
            PostUpdate,
            update_placement_preview.after(TransformSystems::Propagate),
        )
        .run();

    // Let the worker exit cleanly once the Bevy window has closed.
    if let Err(error) = to_worker.send(RendererWorkerMessage::Shutdown) {
        eprintln!("could not stop worker after renderer exit: {error}");
    }
}

fn setup_scene(mut commands: Commands, window: Single<&Window, With<PrimaryWindow>>) {
    let (map_min, map_max) = map_bounds();
    let tower_width = map_max.x;
    let camera_start_y = map_min.y + window.height() * CAMERA_MIN_SCALE * 0.5;

    // Floor zero starts at the world origin; positive floors extend upward.
    commands.spawn((
        Camera2d,
        Projection::Orthographic(OrthographicProjection {
            scale: CAMERA_MIN_SCALE,
            ..OrthographicProjection::default_2d()
        }),
        Transform::from_xyz(tower_width * 0.5, camera_start_y, 1_000.0),
        TowerCamera,
    ));

    let underground_height = map_min.y.abs();
    commands.spawn((
        Sprite::from_color(
            Color::srgb(0.18, 0.10, 0.055),
            Vec2::new(tower_width, underground_height),
        ),
        Transform::from_xyz(tower_width * 0.5, -underground_height * 0.5, -1.0),
    ));

    // Draw the full tower footprint so the camera can scroll through empty floors too.
    for floor in -FLOORS_BELOW_GROUND..FLOORS_ABOVE_GROUND {
        commands.spawn((
            Sprite::from_color(
                Color::srgb(0.18, 0.22, 0.28),
                Vec2::new(tower_width, FLOOR_THICKNESS),
            ),
            Transform::from_xyz(
                tower_width * 0.5,
                floor as f32 * FLOOR_HEIGHT - FLOOR_THICKNESS * 0.5,
                0.0,
            ),
        ));
    }

    commands.spawn((
        Sprite::from_color(Color::srgba(0.35, 0.9, 0.67, 0.36), Vec2::ZERO),
        Transform::from_xyz(0.0, 0.0, 3.0),
        Visibility::Hidden,
        PlacementPreview,
    ));
}

fn setup_hud(mut commands: Commands, channels: Res<RendererChannels>) {
    // The absolute-positioned translucent node stays fixed while the world camera moves.
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: px(0),
                left: px(0),
                width: percent(100),
                height: px(78),
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::SpaceBetween,
                column_gap: px(10),
                padding: UiRect::axes(px(18), px(10)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.035, 0.05, 0.075, 0.86)),
        ))
        .with_children(|bar| {
            bar.spawn((
                Text::new(format_clock(&SimulationClock::default())),
                TextFont::from_font_size(16.0),
                TextColor(Color::srgb(0.91, 0.94, 0.98)),
                Node {
                    flex_grow: 1.0,
                    min_width: px(250),
                    ..default()
                },
                HudText,
            ));

            for (label, speed) in [
                ("Pause", SimulationSpeed::Paused),
                ("1x", SimulationSpeed::OneX),
                ("3x", SimulationSpeed::ThreeX),
                ("5x", SimulationSpeed::FiveX),
            ] {
                let to_worker = channels.to_worker.clone();
                bar.spawn((
                    Button,
                    Hovered::default(),
                    SpeedControlButton(speed),
                    Node {
                        width: px(78),
                        height: px(42),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        border: UiRect::all(px(1)),
                        border_radius: BorderRadius::all(px(6)),
                        ..default()
                    },
                    BorderColor::all(Color::srgb(0.30, 0.38, 0.48)),
                    BackgroundColor(Color::srgba(0.15, 0.20, 0.28, 0.92)),
                    children![(
                        Text::new(label),
                        TextFont::from_font_size(15.0),
                        TextColor(Color::srgb(0.94, 0.96, 1.0)),
                    )],
                ))
                .observe(
                    move |_activate: On<Activate>, mut active_speed: ResMut<ActiveSpeed>| {
                        // Only highlight a speed after its request has been accepted by the channel.
                        let message =
                            RendererWorkerMessage::UiInput(UiInput::SetSimulationSpeed(speed));
                        match to_worker.send(message) {
                            Ok(()) => active_speed.select(speed),
                            Err(error) => {
                                eprintln!("could not send simulation speed request: {error}");
                            }
                        }
                    },
                );
            }
        });

    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: px(ROOM_MENU_TOP),
                left: px(ROOM_MENU_LEFT),
                width: px(ROOM_MENU_WIDTH),
                height: px(ROOM_MENU_HEIGHT),
                flex_direction: FlexDirection::Column,
                row_gap: px(6),
                padding: UiRect::all(px(8)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.035, 0.05, 0.075, 0.92)),
        ))
        .with_children(|menu| {
            menu.spawn((
                Text::new("BUILD ROOM"),
                TextFont::from_font_size(13.0),
                TextColor(Color::srgb(0.63, 0.72, 0.83)),
            ));

            for (label, room_type) in [
                ("Condo", RoomType::Condo),
                ("Party Hall", RoomType::PartyHall),
                ("Lobby", RoomType::Lobby),
                ("Office", RoomType::Office),
            ] {
                menu.spawn((
                    Button,
                    Hovered::default(),
                    RoomTypeButton(room_type),
                    Node {
                        width: percent(100),
                        height: px(34),
                        justify_content: JustifyContent::FlexStart,
                        align_items: AlignItems::Center,
                        padding: UiRect::horizontal(px(10)),
                        border: UiRect::all(px(1)),
                        border_radius: BorderRadius::all(px(5)),
                        ..default()
                    },
                    BorderColor::all(Color::srgb(0.30, 0.38, 0.48)),
                    BackgroundColor(Color::srgba(0.15, 0.20, 0.28, 0.92)),
                    children![(
                        Text::new(label),
                        TextFont::from_font_size(14.0),
                        TextColor(Color::srgb(0.94, 0.96, 1.0)),
                    )],
                ))
                .observe(
                    move |_activate: On<Activate>, mut placement: ResMut<PlacementState>| {
                        if !placement.request_pending {
                            placement.selected_room = Some(room_type);
                            placement.error = None;
                        }
                    },
                );
            }
        });

    commands.spawn((
        Text::new(""),
        TextFont::from_font_size(14.0),
        TextColor(Color::srgb(1.0, 0.92, 0.90)),
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            top: px(0),
            width: px(ERROR_TOOLTIP_WIDTH),
            height: px(ERROR_TOOLTIP_HEIGHT),
            padding: UiRect::all(px(8)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.35, 0.08, 0.09, 0.96)),
        Visibility::Hidden,
        PlacementErrorTooltip,
    ));
}

fn move_camera(
    time: Res<Time>,
    keyboard: Res<ButtonInput<KeyCode>>,
    mut mouse_wheel: MessageReader<MouseWheel>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut cameras: Query<(&mut Transform, &mut Projection), With<TowerCamera>>,
) {
    let mut direction = Vec3::ZERO;
    if keyboard.pressed(KeyCode::KeyW) {
        direction.y += 1.0;
    }
    if keyboard.pressed(KeyCode::KeyS) {
        direction.y -= 1.0;
    }
    if keyboard.pressed(KeyCode::KeyA) {
        direction.x -= 1.0;
    }
    if keyboard.pressed(KeyCode::KeyD) {
        direction.x += 1.0;
    }
    let mut zoom_direction = 0.0;
    if keyboard.pressed(KeyCode::KeyQ) {
        zoom_direction += 1.0;
    }
    if keyboard.pressed(KeyCode::KeyE) {
        zoom_direction -= 1.0;
    }
    let mut scroll_delta = Vec2::ZERO;
    for event in mouse_wheel.read() {
        // Trackpads send pixel deltas; ordinary wheels send line deltas.
        let unit_scale = match event.unit {
            MouseScrollUnit::Line => FLOOR_HEIGHT,
            MouseScrollUnit::Pixel => 1.0,
        };
        scroll_delta += Vec2::new(event.x, event.y) * unit_scale;
    }

    let (map_min, map_max) = map_bounds();
    for (mut transform, mut projection) in &mut cameras {
        if direction != Vec3::ZERO {
            // Normalize so diagonal movement is not faster; delta time keeps motion frame-rate independent.
            transform.translation += direction.normalize() * CAMERA_SPEED * time.delta_secs();
        }

        if let Projection::Orthographic(orthographic) = &mut *projection {
            if zoom_direction != 0.0 {
                let zoom_factor = (CAMERA_ZOOM_RATE * time.delta_secs() * zoom_direction).exp();
                orthographic.scale =
                    (orthographic.scale * zoom_factor).clamp(CAMERA_MIN_SCALE, CAMERA_MAX_SCALE);
            }
            if scroll_delta != Vec2::ZERO {
                let world_delta = scroll_delta * orthographic.scale;
                transform.translation.x += world_delta.x;
                transform.translation.y += world_delta.y;
            }
            clamp_camera_to_map(
                &mut transform,
                orthographic,
                window.size(),
                map_min,
                map_max,
            );
        }
    }
}

fn toggle_pause(
    keyboard: Res<ButtonInput<KeyCode>>,
    channels: Res<RendererChannels>,
    mut active_speed: ResMut<ActiveSpeed>,
) {
    if !keyboard.just_pressed(KeyCode::Space) {
        return;
    }

    let speed = active_speed.pause_toggle_target();
    let message = RendererWorkerMessage::UiInput(UiInput::SetSimulationSpeed(speed));
    match channels.to_worker.send(message) {
        Ok(()) => active_speed.select(speed),
        Err(error) => eprintln!("could not send simulation speed request: {error}"),
    }
}

fn map_bounds() -> (Vec2, Vec2) {
    (
        Vec2::new(
            0.0,
            -(FLOORS_BELOW_GROUND as f32) * FLOOR_HEIGHT - FLOOR_THICKNESS,
        ),
        Vec2::new(
            MAX_FLOOR_WIDTH_COLUMNS as f32 * COLUMN_WIDTH,
            FLOORS_ABOVE_GROUND as f32 * FLOOR_HEIGHT,
        ),
    )
}

fn clamp_camera_to_map(
    transform: &mut Transform,
    projection: &mut OrthographicProjection,
    viewport_size: Vec2,
    map_min: Vec2,
    map_max: Vec2,
) {
    if viewport_size.x <= 0.0 || viewport_size.y <= 0.0 {
        return;
    }

    let map_size = map_max - map_min;
    let max_fit_scale = (map_size.x / viewport_size.x)
        .min(map_size.y / viewport_size.y)
        .min(CAMERA_MAX_SCALE);
    let min_scale = CAMERA_MIN_SCALE.min(max_fit_scale);
    projection.scale = projection.scale.clamp(min_scale, max_fit_scale);

    let half_view = viewport_size * projection.scale * 0.5;
    transform.translation.x = transform
        .translation
        .x
        .clamp(map_min.x + half_view.x, map_max.x - half_view.x);
    transform.translation.y = transform
        .translation
        .y
        .clamp(map_min.y + half_view.y, map_max.y - half_view.y);
}

fn update_placement_preview(
    window: Single<&Window, With<PrimaryWindow>>,
    camera_query: Single<(&Camera, &GlobalTransform), With<TowerCamera>>,
    mouse: Res<ButtonInput<MouseButton>>,
    channels: Res<RendererChannels>,
    mut placement: ResMut<PlacementState>,
    mut previews: Query<(&mut Sprite, &mut Transform, &mut Visibility), With<PlacementPreview>>,
) {
    let Some(room_type) = placement.selected_room else {
        for (_, _, mut visibility) in &mut previews {
            *visibility = Visibility::Hidden;
        }
        return;
    };
    let Some(cursor_position) = window.cursor_position() else {
        for (_, _, mut visibility) in &mut previews {
            *visibility = Visibility::Hidden;
        }
        return;
    };

    if pointer_over_ui(cursor_position, window.size(), &placement) {
        for (_, _, mut visibility) in &mut previews {
            *visibility = Visibility::Hidden;
        }
        return;
    };

    let (camera, camera_transform) = *camera_query;
    let world_position = match camera.viewport_to_world_2d(camera_transform, cursor_position) {
        Ok(position) => position,
        Err(_) => {
            for (_, _, mut visibility) in &mut previews {
                *visibility = Visibility::Hidden;
            }
            return;
        }
    };

    let Some((floor, column)) = world_to_grid(world_position) else {
        for (_, _, mut visibility) in &mut previews {
            *visibility = Visibility::Hidden;
        }
        return;
    };

    let properties = room_type.properties();
    let room_width = properties.width as f32 * COLUMN_WIDTH;
    let room_height = properties.height as f32 * FLOOR_HEIGHT;
    for (mut sprite, mut transform, mut visibility) in &mut previews {
        sprite.color = room_preview_color(room_type);
        sprite.custom_size = Some(Vec2::new(room_width - 4.0, room_height - 8.0));
        transform.translation = Vec3::new(
            column as f32 * COLUMN_WIDTH + room_width * 0.5,
            floor as f32 * FLOOR_HEIGHT + room_height * 0.5,
            3.0,
        );
        *visibility = Visibility::Visible;
    }

    if mouse.just_pressed(MouseButton::Left) && !placement.request_pending {
        let message = RendererWorkerMessage::UiInput(UiInput::BuildRoom {
            floor,
            x: column,
            room_type,
        });
        placement.last_click_position = cursor_position;
        placement.error = None;
        match channels.to_worker.send(message) {
            Ok(()) => placement.request_pending = true,
            Err(error) => {
                placement.error = Some(format!("Could not send room request: {error}"));
            }
        }
    }
}

fn world_to_grid(position: Vec2) -> Option<(i32, u32)> {
    if !position.is_finite() {
        return None;
    }

    let column = (position.x / COLUMN_WIDTH).floor();
    let floor = (position.y / FLOOR_HEIGHT).floor();
    if !(0.0..MAX_FLOOR_WIDTH_COLUMNS as f32).contains(&column)
        || floor < -FLOORS_BELOW_GROUND as f32
        || floor >= FLOORS_ABOVE_GROUND as f32
    {
        return None;
    }

    Some((floor as i32, column as u32))
}

fn pointer_over_ui(position: Vec2, window_size: Vec2, placement: &PlacementState) -> bool {
    let over_hud = position.y <= HUD_HEIGHT;
    let over_room_menu = position.x >= ROOM_MENU_LEFT
        && position.x <= ROOM_MENU_LEFT + ROOM_MENU_WIDTH
        && position.y >= ROOM_MENU_TOP
        && position.y <= ROOM_MENU_TOP + ROOM_MENU_HEIGHT;
    let tooltip_position = error_tooltip_position(placement.last_click_position, window_size);
    let over_error_tooltip = placement.error.is_some()
        && position.x >= tooltip_position.x
        && position.x <= tooltip_position.x + ERROR_TOOLTIP_WIDTH
        && position.y >= tooltip_position.y
        && position.y <= tooltip_position.y + ERROR_TOOLTIP_HEIGHT;
    over_hud || over_room_menu || over_error_tooltip
}

fn error_tooltip_position(anchor: Vec2, window_size: Vec2) -> Vec2 {
    let min_y = HUD_HEIGHT + 8.0;
    Vec2::new(
        (anchor.x + 14.0).clamp(8.0, (window_size.x - ERROR_TOOLTIP_WIDTH - 8.0).max(8.0)),
        (anchor.y + 14.0).clamp(
            min_y,
            (window_size.y - ERROR_TOOLTIP_HEIGHT - 8.0).max(min_y),
        ),
    )
}

fn update_error_tooltip(
    placement: Res<PlacementState>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut tooltip: Query<(&mut Text, &mut Node, &mut Visibility), With<PlacementErrorTooltip>>,
) {
    let position = error_tooltip_position(placement.last_click_position, window.size());
    for (mut text, mut node, mut visibility) in &mut tooltip {
        if let Some(error) = &placement.error {
            text.0 = error.clone();
            node.left = px(position.x);
            node.top = px(position.y);
            *visibility = Visibility::Visible;
        } else {
            *visibility = Visibility::Hidden;
        }
    }
}

fn receive_updates(
    mut commands: Commands,
    channels: Res<RendererChannels>,
    mut clock: ResMut<SimulationClock>,
    mut placement: ResMut<PlacementState>,
    mut app_exit: MessageWriter<AppExit>,
    rooms: Query<(Entity, &RoomVisual)>,
) {
    // Drain both queues without blocking the render loop, so one producer cannot stall the other.
    let worker_messages = match channels.from_worker.lock() {
        Ok(receiver) => {
            let mut messages = Vec::new();
            loop {
                match receiver.try_recv() {
                    Ok(message) => messages.push(message),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        messages.push(RendererWorkerMessage::Shutdown);
                        break;
                    }
                }
            }
            messages
        }
        Err(error) => {
            eprintln!("worker receiver lock poisoned: {error}");
            vec![RendererWorkerMessage::Shutdown]
        }
    };

    for message in worker_messages {
        match message {
            RendererWorkerMessage::RoomBuilt {
                id,
                floor,
                x,
                room_type,
            } => {
                spawn_room(&mut commands, id, floor, x, room_type);
                placement.request_pending = false;
                placement.error = None;
            }
            RendererWorkerMessage::RoomBuildFailed { error } => {
                placement.request_pending = false;
                placement.error = Some(error);
            }
            RendererWorkerMessage::FloorSegmentBuilt { floor, column } => {
                spawn_floor_segment(&mut commands, floor, column);
            }
            RendererWorkerMessage::RoomRemoved(room_id) => {
                // Removing a room leaves its floor structure in place.
                for (entity, visual) in &rooms {
                    if visual.0 == room_id {
                        commands.entity(entity).despawn();
                    }
                }
                placement.request_pending = false;
                placement.error = None;
            }
            RendererWorkerMessage::Shutdown => {}
            RendererWorkerMessage::UiInput(_) => {
                eprintln!("unexpected UI input received by renderer");
            }
        }
    }

    let simulation_messages = match channels.from_simulation.lock() {
        Ok(receiver) => {
            let mut messages = Vec::new();
            loop {
                match receiver.try_recv() {
                    Ok(message) => messages.push(message),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        messages.push(SimulationRendererMessage::Shutdown);
                        break;
                    }
                }
            }
            messages
        }
        Err(error) => {
            eprintln!("simulation receiver lock poisoned: {error}");
            vec![SimulationRendererMessage::Shutdown]
        }
    };

    for message in simulation_messages {
        match message {
            SimulationRendererMessage::Tick {
                tick,
                date,
                time,
                cash,
            } => {
                // The HUD only needs the latest snapshot; intermediate ticks can be coalesced.
                clock.tick = tick;
                clock.date = date;
                clock.time = time;
                clock.cash = cash;
            }
            SimulationRendererMessage::Shutdown => {
                app_exit.write(AppExit::Success);
            }
        }
    }
}

fn spawn_room(commands: &mut Commands, id: RoomId, floor: i32, x: u32, room_type: RoomType) {
    let properties = room_type.properties();
    let width = properties.width as f32 * COLUMN_WIDTH;
    let height = properties.height as f32 * FLOOR_HEIGHT;
    // Center the room sprite over its column/floor footprint; z=2 places it above floor segments.
    commands.spawn((
        Sprite::from_color(room_color(room_type), Vec2::new(width - 4.0, height - 8.0)),
        Transform::from_xyz(
            x as f32 * COLUMN_WIDTH + width * 0.5,
            floor as f32 * FLOOR_HEIGHT + height * 0.5,
            2.0,
        ),
        RoomVisual(id),
    ));
}

fn spawn_floor_segment(commands: &mut Commands, floor: i32, column: u32) {
    // A segment occupies one column and sits directly on its floor line.
    commands.spawn((
        Sprite::from_color(
            Color::srgb(0.49, 0.58, 0.68),
            Vec2::new(COLUMN_WIDTH - 1.0, FLOOR_THICKNESS),
        ),
        Transform::from_xyz(
            column as f32 * COLUMN_WIDTH + COLUMN_WIDTH * 0.5,
            floor as f32 * FLOOR_HEIGHT - FLOOR_THICKNESS * 0.5,
            1.0,
        ),
    ));
}

fn room_color(room_type: RoomType) -> Color {
    match room_type {
        RoomType::Condo => Color::srgb(0.28, 0.65, 0.84),
        RoomType::PartyHall => Color::srgb(0.69, 0.42, 0.82),
        RoomType::Lobby => Color::srgb(0.91, 0.68, 0.29),
        RoomType::Office => Color::srgb(0.36, 0.73, 0.55),
    }
}

fn room_preview_color(room_type: RoomType) -> Color {
    let (red, green, blue) = match room_type {
        RoomType::Condo => (0.28, 0.65, 0.84),
        RoomType::PartyHall => (0.69, 0.42, 0.82),
        RoomType::Lobby => (0.91, 0.68, 0.29),
        RoomType::Office => (0.36, 0.73, 0.55),
    };
    Color::srgba(red, green, blue, 0.36)
}

fn update_room_button_appearance(
    placement: Res<PlacementState>,
    mut buttons: Query<(
        &Hovered,
        Has<Pressed>,
        &RoomTypeButton,
        &mut BackgroundColor,
        &mut BorderColor,
    )>,
) {
    for (hovered, pressed, room_button, mut background, mut border) in &mut buttons {
        let is_selected = placement.selected_room == Some(room_button.0);
        background.0 = if pressed {
            Color::srgb(0.25, 0.39, 0.56)
        } else if is_selected {
            Color::srgb(0.15, 0.39, 0.31)
        } else if hovered.get() {
            Color::srgb(0.21, 0.29, 0.41)
        } else {
            Color::srgba(0.15, 0.20, 0.28, 0.92)
        };
        border.set_all(if is_selected {
            Color::srgb(0.42, 0.85, 0.66)
        } else {
            Color::srgb(0.30, 0.38, 0.48)
        });
    }
}

fn update_hud(clock: Res<SimulationClock>, mut hud: Query<&mut Text, With<HudText>>) {
    if !clock.is_changed() {
        return;
    }
    for mut text in &mut hud {
        text.0 = format_clock(&clock);
    }
}

fn format_clock(clock: &SimulationClock) -> String {
    let day_type = match clock.date.day_type {
        SimulationDayType::Weekday1 => "Weekday 1",
        SimulationDayType::Weekday2 => "Weekday 2",
        SimulationDayType::Weekend => "Weekend",
    };
    format!(
        "Tick {}    {:02}:{:02}    Day {} ({})    Q{}, Year {}    {}",
        clock.tick,
        clock.time.hour,
        clock.time.minute,
        clock.date.day_counter + 1,
        day_type,
        clock.date.quarter,
        clock.date.year,
        format_cash(clock.cash),
    )
}

fn format_cash(cash: u64) -> String {
    let digits = cash.to_string();
    let first_group_len = match digits.len() % 3 {
        0 => 3,
        remainder => remainder,
    };
    let mut formatted = String::with_capacity(digits.len() + digits.len() / 3);
    formatted.push_str(&digits[..first_group_len]);
    // Group the remaining digits in threes for easier reading.
    for group in digits[first_group_len..].as_bytes().chunks(3) {
        formatted.push(',');
        formatted.push_str(std::str::from_utf8(group).expect("cash digits are ASCII"));
    }
    format!("${formatted}")
}

fn update_button_appearance(
    active_speed: Res<ActiveSpeed>,
    mut buttons: Query<(
        &Hovered,
        Has<Pressed>,
        &SpeedControlButton,
        &mut BackgroundColor,
        &mut BorderColor,
    )>,
) {
    for (hovered, pressed, speed, mut background, mut border) in &mut buttons {
        let is_active = speed.0 == active_speed.current;
        // Active speed has a persistent highlight; hover and press only affect the fill.
        background.0 = if pressed {
            Color::srgb(0.25, 0.39, 0.56)
        } else if is_active {
            Color::srgb(0.15, 0.39, 0.31)
        } else if hovered.get() {
            Color::srgb(0.21, 0.29, 0.41)
        } else {
            Color::srgba(0.15, 0.20, 0.28, 0.92)
        };
        border.set_all(if is_active {
            Color::srgb(0.42, 0.85, 0.66)
        } else {
            Color::srgb(0.30, 0.38, 0.48)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pause_toggle_resumes_the_last_running_speed() {
        let mut active_speed = ActiveSpeed {
            current: SimulationSpeed::ThreeX,
            last_running: SimulationSpeed::ThreeX,
        };

        assert_eq!(active_speed.pause_toggle_target(), SimulationSpeed::Paused);
        active_speed.select(SimulationSpeed::Paused);
        assert_eq!(active_speed.pause_toggle_target(), SimulationSpeed::ThreeX);
    }

    #[test]
    fn camera_is_clamped_inside_map_and_zoomed_to_fit() {
        let (map_min, map_max) = map_bounds();
        let viewport = Vec2::new(1_280.0, 720.0);
        let mut transform = Transform::from_xyz(-500.0, 5_000.0, 1_000.0);
        let mut projection = OrthographicProjection {
            scale: CAMERA_MAX_SCALE,
            ..OrthographicProjection::default_2d()
        };

        clamp_camera_to_map(&mut transform, &mut projection, viewport, map_min, map_max);

        assert!((projection.scale - 2.5).abs() < f32::EPSILON);
        assert!((transform.translation.x - 1_600.0).abs() < f32::EPSILON);
        assert!(
            (transform.translation.y - (map_max.y - viewport.y * projection.scale / 2.0)).abs()
                < f32::EPSILON
        );
    }

    #[test]
    fn cash_is_formatted_with_thousands_separators() {
        assert_eq!(format_cash(2_000_000), "$2,000,000");
        assert_eq!(format_cash(45), "$45");
    }

    #[test]
    fn clock_includes_simulation_calendar_and_time() {
        let clock = SimulationClock::default();
        assert_eq!(
            format_clock(&clock),
            "Tick 0    07:00    Day 1 (Weekday 1)    Q1, Year 1    $2,000,000"
        );
    }

    #[test]
    fn placement_snaps_cursor_to_the_leftmost_column_and_floor() {
        assert_eq!(
            world_to_grid(Vec2::new(
                COLUMN_WIDTH * 3.0 + 1.0,
                FLOOR_HEIGHT * 2.0 + 1.0
            )),
            Some((2, 3))
        );
        assert_eq!(world_to_grid(Vec2::new(-1.0, 0.0)), None);
    }
}
