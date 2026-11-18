//! Bevy scene, HUD, and input handling for the tower renderer.
use bevy::input::gestures::PinchGesture;
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::transform::TransformSystems;
use bevy::ui::Pressed;
use bevy::ui_widgets::{Activate, Button};
use bevy::window::PrimaryWindow;
use shiny_tower::messages::{
    RendererWorkerMessage, SimulationDate, SimulationDayType, SimulationRendererMessage,
    SimulationSpeed, SimulationTime, TimeOfDayPeriod, UiInput,
};
use shiny_tower::room::{RoomId, RoomType};
use shiny_tower::steps::StepsId;
use shiny_tower::tower::{FLOORS_ABOVE_GROUND, FLOORS_BELOW_GROUND, MAX_FLOOR_WIDTH_COLUMNS};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};

// Tower positions are converted from columns and floors into these world-space units.
const COLUMN_WIDTH: f32 = 8.0;
const FLOOR_HEIGHT: f32 = 32.0; // 24px room height plus the 8px ceiling/floor segment.
const FLOOR_THICKNESS: f32 = 8.0;

// Camera movement, depth, and zoom limits.
const CAMERA_Z: f32 = 100.0;
const CAMERA_SPEED: f32 = 700.0;
const CAMERA_START_SCALE: f32 = 0.35;
const CAMERA_MIN_SCALE: f32 = 0.2;
const CAMERA_MAX_SCALE: f32 = 3.0;
const CAMERA_ZOOM_RATE: f32 = 1.2;
const CAMERA_PAN_MIN_SPEED_FACTOR: f32 = 0.75;
// Keep the initial view anchored near the uppermost underground floors.
const INITIAL_UNDERGROUND_FLOORS: i32 = 2;

// Fixed-screen HUD and tooltip layout.
const HUD_HEIGHT: f32 = 78.0;
const ROOM_MENU_LEFT: f32 = 12.0;
const ROOM_MENU_TOP: f32 = 90.0;
const ROOM_MENU_WIDTH: f32 = 190.0;
const ROOM_MENU_HEIGHT: f32 = 390.0;
const ERROR_TOOLTIP_WIDTH: f32 = 300.0;
const ERROR_TOOLTIP_HEIGHT: f32 = 72.0;
const CONDO_SHEET_SIZE: UVec2 = UVec2::new(688, 128);
const CONDO_FRAME_SIZE: UVec2 = UVec2::new(128, 24);
const CONDO_FRAME_Y: u32 = 8;
const CONDO_FRAME_X: [u32; 3] = [8, 144, 280];

/// Message endpoints shared by renderer systems.
///
/// The mutexes make standard-library receivers usable as Bevy resources.
#[derive(Resource)]
struct RendererChannels {
    to_worker: Sender<RendererWorkerMessage>,
    from_worker: Mutex<Receiver<RendererWorkerMessage>>,
    from_simulation: Mutex<Receiver<SimulationRendererMessage>>,
}

/// Most recent simulation values displayed by the HUD.
#[derive(Resource)]
struct SimulationClock {
    tick: u64,
    date: SimulationDate,
    time: SimulationTime,
    cash: u64,
}

/// Speed shown as active in the HUD, including the last queued speed before pausing.
#[derive(Resource)]
struct ActiveSpeed {
    current: SimulationSpeed,
    last_running: SimulationSpeed,
}

impl ActiveSpeed {
    /// Records a speed request after it was accepted by the worker channel.
    fn select(&mut self, speed: SimulationSpeed) {
        self.current = speed;
        if speed != SimulationSpeed::Paused {
            self.last_running = speed;
        }
    }

    /// Returns the speed that should be selected by the pause toggle.
    fn pause_toggle_target(&self) -> SimulationSpeed {
        if self.current == SimulationSpeed::Paused {
            self.last_running
        } else {
            SimulationSpeed::Paused
        }
    }
}

/// Current room selection and status of the in-flight placement request.
#[derive(Resource, Default)]
struct PlacementState {
    selected_build: Option<BuildSelection>,
    request_pending: bool,
    last_click_position: Vec2,
    error: Option<String>,
    room_drag_origin: Option<(i32, u32, RoomType)>,
    room_drag_last_target: Option<(i32, u32)>,
    visited_room_targets: HashSet<(i32, u32)>,
    queued_room_builds: VecDeque<RoomBuildRequest>,
}

impl PlacementState {
    fn is_busy(&self) -> bool {
        self.request_pending
            || !self.queued_room_builds.is_empty()
            || self.room_drag_origin.is_some()
    }
}

struct RoomBuildRequest {
    floor: i32,
    x: u32,
    room_type: RoomType,
    cursor_position: Vec2,
}

impl Default for SimulationClock {
    /// Starts the HUD with the simulation's initial date, time, and funds.
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

/// Marks the camera used to render the tower world.
#[derive(Component)]
struct TowerCamera;

/// Marks the text node that displays simulation status.
#[derive(Component)]
struct HudText;

#[derive(Resource, Default)]
struct RoomSpriteSheets(HashMap<RoomType, RoomSpriteSheet>);

struct RoomSpriteSheet {
    image: Handle<Image>,
    layout: Handle<TextureAtlasLayout>,
}

/// Associates a rendered room entity with its simulation data.
#[derive(Component)]
struct RoomVisual(RoomId);

#[derive(Clone, Copy, PartialEq, Eq)]
enum BuildSelection {
    Room(RoomType),
    Steps,
}

/// Stores the speed represented by a HUD speed button.
#[derive(Component)]
struct SpeedControlButton(SimulationSpeed);

/// Stores the structure represented by a build-menu button.
#[derive(Component)]
struct BuildOptionButton(BuildSelection);

/// Marks the sprite following the cursor during room placement.
#[derive(Component)]
struct PlacementPreview;

/// Marks the UI node used to display room-placement errors.
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
        .add_systems(Startup, (setup_scene, setup_hud, setup_room_assets))
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
            (
                update_placement_preview.after(TransformSystems::Propagate),
                dispatch_room_build_queue.after(update_placement_preview),
            ),
        )
        .run();

    // Let the worker exit cleanly once the Bevy window has closed.
    if let Err(error) = to_worker.send(RendererWorkerMessage::Shutdown) {
        eprintln!("could not stop worker after renderer exit: {error}");
    }
}

/// Creates the camera, tower backdrop, floor guides, and placement preview.
fn setup_scene(mut commands: Commands, window: Single<&Window, With<PrimaryWindow>>) {
    let (map_min, map_max) = map_bounds();
    let tower_width = map_max.x;
    let camera_start_y = initial_camera_y(window.height());

    // Floor zero starts at the world origin; positive floors extend upward.
    commands.spawn((
        Camera2d,
        Projection::Orthographic(OrthographicProjection {
            scale: CAMERA_START_SCALE,
            ..OrthographicProjection::default_2d()
        }),
        Transform::from_xyz(tower_width * 0.5, camera_start_y, CAMERA_Z),
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

fn setup_room_assets(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut atlas_layouts: ResMut<Assets<TextureAtlasLayout>>,
) {
    let image = asset_server.load("rooms/condos.png");
    let layout = atlas_layouts.add(condo_atlas_layout());
    commands.insert_resource(RoomSpriteSheets(HashMap::from([(
        RoomType::Condo,
        RoomSpriteSheet { image, layout },
    )])));
}

fn condo_atlas_layout() -> TextureAtlasLayout {
    let mut layout = TextureAtlasLayout::new_empty(CONDO_SHEET_SIZE);
    for frame_x in CONDO_FRAME_X {
        layout.add_texture(URect {
            min: UVec2::new(frame_x, CONDO_FRAME_Y),
            max: UVec2::new(
                frame_x + CONDO_FRAME_SIZE.x,
                CONDO_FRAME_Y + CONDO_FRAME_SIZE.y,
            ),
        });
    }
    layout
}

/// Builds the fixed HUD, speed controls, room menu, and error tooltip.
fn setup_hud(mut commands: Commands, channels: Res<RendererChannels>) {
    // The absolute-positioned translucent node stays fixed while the world camera moves.
    // Add room selection controls below the top status bar.
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
                ("Fast Food", RoomType::FastFood),
                ("Retail Shop", RoomType::RetailShop),
                ("Restaurant", RoomType::Restaurant),
            ] {
                menu.spawn((
                    Button,
                    Hovered::default(),
                    BuildOptionButton(BuildSelection::Room(room_type)),
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
                        if !placement.is_busy() {
                            placement.selected_build = Some(BuildSelection::Room(room_type));
                            placement.error = None;
                        }
                    },
                );
            }

            menu.spawn((
                Text::new("BUILD STEPS"),
                TextFont::from_font_size(13.0),
                TextColor(Color::srgb(0.63, 0.72, 0.83)),
            ));

            menu.spawn((
                Button,
                Hovered::default(),
                BuildOptionButton(BuildSelection::Steps),
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
                    Text::new("Steps ($5,000)"),
                    TextFont::from_font_size(14.0),
                    TextColor(Color::srgb(0.94, 0.96, 1.0)),
                )],
            ))
            .observe(
                move |_activate: On<Activate>, mut placement: ResMut<PlacementState>| {
                    if !placement.is_busy() {
                        placement.selected_build = Some(BuildSelection::Steps);
                        placement.error = None;
                    }
                },
            );
        });

    // Keep placement failures visible without blocking the tower view.
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

/// Applies keyboard and wheel movement, then keeps the view inside map bounds.
fn move_camera(
    time: Res<Time>,
    keyboard: Res<ButtonInput<KeyCode>>,
    mut mouse_wheel: MessageReader<MouseWheel>,
    mut pinch_gestures: MessageReader<PinchGesture>,
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
    let pinch_delta = pinch_gestures.read().map(|gesture| gesture.0).sum::<f32>();

    let (map_min, map_max) = map_bounds();
    let cursor_position = window.cursor_position();
    let viewport_size = window.size();
    for (mut transform, mut projection) in &mut cameras {
        if let Projection::Orthographic(orthographic) = &mut *projection {
            if direction != Vec3::ZERO {
                // Normalize so diagonal movement is not faster; delta time keeps motion frame-rate independent.
                let speed =
                    CAMERA_SPEED * time.delta_secs() * camera_pan_speed_factor(orthographic.scale);
                transform.translation += direction.normalize() * speed;
            }

            let zoom_factor = (CAMERA_ZOOM_RATE * time.delta_secs() * zoom_direction).exp()
                * pinch_zoom_factor(pinch_delta);
            if zoom_factor != 1.0 {
                zoom_camera(
                    &mut transform,
                    orthographic,
                    zoom_factor,
                    cursor_position,
                    viewport_size,
                );
            }
            if scroll_delta != Vec2::ZERO {
                let speed_factor = camera_pan_speed_factor(orthographic.scale);
                let world_delta = scroll_delta * orthographic.scale * speed_factor;
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

fn pinch_zoom_factor(delta: f32) -> f32 {
    (-CAMERA_ZOOM_RATE * delta).exp()
}

/// Changes zoom while keeping the world point under the cursor stationary.
fn zoom_camera(
    transform: &mut Transform,
    projection: &mut OrthographicProjection,
    zoom_factor: f32,
    cursor_position: Option<Vec2>,
    viewport_size: Vec2,
) {
    let old_scale = projection.scale;
    let new_scale = (old_scale * zoom_factor).clamp(CAMERA_MIN_SCALE, CAMERA_MAX_SCALE);

    if let Some(cursor) = cursor_position {
        let cursor_offset = Vec2::new(
            cursor.x - viewport_size.x * 0.5,
            viewport_size.y * 0.5 - cursor.y,
        );
        let world_anchor = transform.translation.truncate() + cursor_offset * old_scale;
        transform.translation.x = world_anchor.x - cursor_offset.x * new_scale;
        transform.translation.y = world_anchor.y - cursor_offset.y * new_scale;
    }

    projection.scale = new_scale;
}

/// Reduces keyboard and wheel panning speed slightly as the camera zooms in.
fn camera_pan_speed_factor(scale: f32) -> f32 {
    let zoom_progress =
        ((scale - CAMERA_MIN_SCALE) / (CAMERA_MAX_SCALE - CAMERA_MIN_SCALE)).clamp(0.0, 1.0);
    CAMERA_PAN_MIN_SPEED_FACTOR + (1.0 - CAMERA_PAN_MIN_SPEED_FACTOR) * zoom_progress
}

/// Toggles between paused simulation and the last running speed with Space.
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

/// Returns the world-space rectangle occupied by the tower map.
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

/// Places the default-scale camera so its lower edge starts at the selected basement floors.
fn initial_camera_y(window_height: f32) -> f32 {
    let view_min_y = -(INITIAL_UNDERGROUND_FLOORS as f32) * FLOOR_HEIGHT - FLOOR_THICKNESS;
    view_min_y + window_height * CAMERA_START_SCALE * 0.5
}

/// Limits zoom and camera position so the viewport remains within the tower map.
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

/// Positions the cursor preview and records room builds while dragging.
fn update_placement_preview(
    window: Single<&Window, With<PrimaryWindow>>,
    camera_query: Single<(&Camera, &GlobalTransform), With<TowerCamera>>,
    mouse: Res<ButtonInput<MouseButton>>,
    channels: Res<RendererChannels>,
    mut placement: ResMut<PlacementState>,
    mut previews: Query<(&mut Sprite, &mut Transform, &mut Visibility), With<PlacementPreview>>,
) {
    if !mouse.pressed(MouseButton::Left) {
        placement.room_drag_origin = None;
        placement.room_drag_last_target = None;
        placement.visited_room_targets.clear();
    }

    let Some(build_selection) = placement.selected_build else {
        for (_, _, mut visibility) in &mut previews {
            *visibility = Visibility::Hidden;
        }
        return;
    };
    let Some(cursor_position) = window.cursor_position() else {
        placement.room_drag_last_target = None;
        for (_, _, mut visibility) in &mut previews {
            *visibility = Visibility::Hidden;
        }
        return;
    };

    if pointer_over_ui(cursor_position, window.size(), &placement) {
        placement.room_drag_last_target = None;
        for (_, _, mut visibility) in &mut previews {
            *visibility = Visibility::Hidden;
        }
        return;
    };

    let (camera, camera_transform) = *camera_query;
    let world_position = match camera.viewport_to_world_2d(camera_transform, cursor_position) {
        Ok(position) => position,
        Err(_) => {
            placement.room_drag_last_target = None;
            for (_, _, mut visibility) in &mut previews {
                *visibility = Visibility::Hidden;
            }
            return;
        }
    };

    let Some((floor, column)) = world_to_grid(world_position) else {
        placement.room_drag_last_target = None;
        for (_, _, mut visibility) in &mut previews {
            *visibility = Visibility::Hidden;
        }
        return;
    };

    let drag_target = match build_selection {
        BuildSelection::Room(room_type) if mouse.just_pressed(MouseButton::Left) => {
            placement.room_drag_origin = Some((floor, column, room_type));
            placement.room_drag_last_target = None;
            placement.visited_room_targets.clear();
            Some((floor, column))
        }
        BuildSelection::Room(room_type) if mouse.pressed(MouseButton::Left) => {
            match placement.room_drag_origin {
                Some((origin_floor, origin_column, origin_type)) if origin_type == room_type => {
                    let target =
                        room_drag_target(origin_floor, origin_column, floor, column, room_type);
                    if target.is_none() {
                        placement.room_drag_last_target = None;
                    }
                    target
                }
                _ => None,
            }
        }
        _ => None,
    };

    let (preview_floor, preview_column) = drag_target.unwrap_or((floor, column));
    let (build_size, build_color, build_position) = match build_selection {
        BuildSelection::Room(room_type) => (
            room_dimensions(room_type),
            room_preview_color(room_type),
            room_position(preview_floor, preview_column, room_type),
        ),
        BuildSelection::Steps => (
            steps_dimensions(),
            Color::srgba(0.78, 0.65, 0.40, 0.5),
            steps_position(preview_floor, preview_column),
        ),
    };
    for (mut sprite, mut transform, mut visibility) in &mut previews {
        sprite.color = build_color;
        sprite.custom_size = Some(build_size);
        transform.translation = build_position.extend(3.0);
        *visibility = Visibility::Visible;
    }

    if let (BuildSelection::Room(room_type), Some(target)) = (build_selection, drag_target) {
        enqueue_room_drag_path(&mut placement, target, room_type, cursor_position);
    }

    if mouse.just_pressed(MouseButton::Left)
        && matches!(build_selection, BuildSelection::Steps)
        && !placement.is_busy()
    {
        let message = RendererWorkerMessage::UiInput(UiInput::BuildSteps { floor, x: column });
        placement.last_click_position = cursor_position;
        placement.error = None;
        match channels.to_worker.send(message) {
            Ok(()) => placement.request_pending = true,
            Err(error) => {
                placement.error = Some(format!("Could not send build request: {error}"));
            }
        }
    }
}

fn room_drag_target(
    origin_floor: i32,
    origin_column: u32,
    floor: i32,
    column: u32,
    room_type: RoomType,
) -> Option<(i32, u32)> {
    let room_width = i64::from(room_type.properties().width);
    let room_height = room_type.properties().height as i32;
    if room_width == 0 || room_height == 0 {
        return None;
    }
    let vertical_steps = (floor - origin_floor) / room_height;
    let target_floor = origin_floor + vertical_steps * room_height;
    let horizontal_steps = (i64::from(column) - i64::from(origin_column)) / room_width;
    let target_column = i64::from(origin_column) + horizontal_steps * room_width;
    u32::try_from(target_column)
        .ok()
        .map(|column| (target_floor, column))
}

fn enqueue_room_drag_path(
    placement: &mut PlacementState,
    target: (i32, u32),
    room_type: RoomType,
    cursor_position: Vec2,
) {
    let path = placement.room_drag_last_target.map_or_else(
        || vec![target],
        |last| room_drag_path(last, target, room_type),
    );
    for (floor, x) in path {
        if placement.visited_room_targets.insert((floor, x)) {
            placement.queued_room_builds.push_back(RoomBuildRequest {
                floor,
                x,
                room_type,
                cursor_position,
            });
        }
    }
    placement.room_drag_last_target = Some(target);
}

fn room_drag_path(from: (i32, u32), to: (i32, u32), room_type: RoomType) -> Vec<(i32, u32)> {
    let room_width = i64::from(room_type.properties().width);
    let room_height = room_type.properties().height as i32;
    if room_width == 0 || room_height == 0 {
        return Vec::new();
    }

    let vertical_steps = (to.0 - from.0) / room_height;
    let horizontal_steps = (i64::from(to.1) - i64::from(from.1)) / room_width;
    let step_count = vertical_steps
        .abs()
        .max(horizontal_steps.unsigned_abs() as i32);
    let mut path = Vec::new();
    for step in 1..=step_count {
        let progress = step as f32 / step_count as f32;
        let floor = from.0 + (vertical_steps as f32 * progress).round() as i32 * room_height;
        let x_steps = (horizontal_steps as f32 * progress).round() as i64;
        let x = i64::from(from.1) + x_steps * room_width;
        if let Ok(x) = u32::try_from(x) {
            path.push((floor, x));
        }
    }
    path
}

fn dispatch_room_build_queue(
    mut placement: ResMut<PlacementState>,
    channels: Res<RendererChannels>,
) {
    if placement.request_pending {
        return;
    }
    let Some(request) = placement.queued_room_builds.pop_front() else {
        return;
    };

    placement.last_click_position = request.cursor_position;
    placement.error = None;
    let message = RendererWorkerMessage::UiInput(UiInput::BuildRoom {
        floor: request.floor,
        x: request.x,
        room_type: request.room_type,
    });
    match channels.to_worker.send(message) {
        Ok(()) => placement.request_pending = true,
        Err(error) => {
            placement.error = Some(format!("Could not send build request: {error}"));
        }
    }
}

/// Converts a world position to a valid floor and leftmost column.
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

/// Computes the rendered room size, leaving the ceiling thickness above it.
fn room_dimensions(room_type: RoomType) -> Vec2 {
    let properties = room_type.properties();
    Vec2::new(
        properties.width as f32 * COLUMN_WIDTH,
        properties.height as f32 * FLOOR_HEIGHT - FLOOR_THICKNESS,
    )
}

/// Returns the center of a room's world-space footprint.
fn room_position(floor: i32, x: u32, room_type: RoomType) -> Vec2 {
    let size = room_dimensions(room_type);
    Vec2::new(
        x as f32 * COLUMN_WIDTH + size.x * 0.5,
        floor as f32 * FLOOR_HEIGHT + size.y * 0.5,
    )
}

fn steps_dimensions() -> Vec2 {
    Vec2::new(
        shiny_tower::steps::Steps::WIDTH as f32 * COLUMN_WIDTH,
        shiny_tower::steps::Steps::HEIGHT as f32 * FLOOR_HEIGHT - FLOOR_THICKNESS,
    )
}

fn steps_position(floor: i32, x: u32) -> Vec2 {
    let size = steps_dimensions();
    Vec2::new(
        x as f32 * COLUMN_WIDTH + size.x * 0.5,
        floor as f32 * FLOOR_HEIGHT + size.y * 0.5,
    )
}

/// Reports whether a cursor position overlaps UI that should block building.
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

/// Places the error tooltip near its click anchor while keeping it on screen.
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

/// Updates the error tooltip text, position, and visibility.
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

/// Drains worker and simulation channels and applies their updates to the scene.
fn receive_updates(
    mut commands: Commands,
    channels: Res<RendererChannels>,
    mut clock: ResMut<SimulationClock>,
    mut placement: ResMut<PlacementState>,
    mut app_exit: MessageWriter<AppExit>,
    room_sheets: Res<RoomSpriteSheets>,
    mut rooms: Query<(Entity, &RoomVisual, &mut Sprite)>,
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
                spawn_room(
                    &mut commands,
                    id,
                    floor,
                    x,
                    room_type,
                    clock.time.period,
                    &room_sheets,
                );
                placement.request_pending = false;
                placement.error = None;
            }
            RendererWorkerMessage::RoomBuildFailed { error } => {
                placement.request_pending = false;
                placement.error = Some(error);
            }
            RendererWorkerMessage::StepsBuilt { id, floor, x } => {
                spawn_steps(&mut commands, id, floor, x);
                placement.request_pending = false;
                placement.error = None;
            }
            RendererWorkerMessage::StepsBuildFailed { error } => {
                placement.request_pending = false;
                placement.error = Some(error);
            }
            RendererWorkerMessage::FloorSegmentBuilt { floor, column } => {
                spawn_floor_segment(&mut commands, floor, column);
            }
            RendererWorkerMessage::RoomRemoved(room_id) => {
                // Removing a room leaves its floor structure in place.
                for (entity, visual, _) in &mut rooms {
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
                let atlas_index = condo_atlas_index(time.period);
                for (_, _, mut sprite) in &mut rooms {
                    if let Some(atlas) = sprite.texture_atlas.as_mut() {
                        atlas.index = atlas_index;
                    }
                }
            }
            SimulationRendererMessage::Shutdown => {
                app_exit.write(AppExit::Success);
            }
        }
    }
}

/// Creates the room sprite corresponding to a successful build message.
fn spawn_room(
    commands: &mut Commands,
    id: RoomId,
    floor: i32,
    x: u32,
    room_type: RoomType,
    period: TimeOfDayPeriod,
    room_sheets: &RoomSpriteSheets,
) {
    let mut sprite = if let Some(sheet) = room_sheets.0.get(&room_type) {
        Sprite::from_atlas_image(
            sheet.image.clone(),
            TextureAtlas {
                layout: sheet.layout.clone(),
                index: condo_atlas_index(period),
            },
        )
    } else {
        Sprite::from_color(room_color(room_type), room_dimensions(room_type))
    };
    let mut position = room_position(floor, x, room_type);
    if sprite.texture_atlas.is_some() {
        sprite.custom_size = Some(Vec2::new(
            CONDO_FRAME_SIZE.x as f32,
            CONDO_FRAME_SIZE.y as f32,
        ));
        position.y = floor as f32 * FLOOR_HEIGHT + CONDO_FRAME_SIZE.y as f32 * 0.5;
    }

    commands.spawn((
        sprite,
        Transform::from_translation(position.extend(2.0)),
        RoomVisual(id),
    ));
}

fn condo_atlas_index(period: TimeOfDayPeriod) -> usize {
    match period {
        TimeOfDayPeriod::Day => 0,
        TimeOfDayPeriod::Dusk | TimeOfDayPeriod::Dawn => 1,
        TimeOfDayPeriod::Night => 2,
    }
}

#[derive(Component)]
struct StepsVisual {
    _id: StepsId,
}

fn spawn_steps(commands: &mut Commands, id: StepsId, floor: i32, x: u32) {
    commands.spawn((
        Sprite::from_color(Color::srgb(0.78, 0.65, 0.40), steps_dimensions()),
        Transform::from_translation(steps_position(floor, x).extend(2.5)),
        StepsVisual { _id: id },
    ));
}

/// Creates a single visible floor segment at its grid position.
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

/// Returns the opaque scene color associated with a room type.
fn room_color(room_type: RoomType) -> Color {
    match room_type {
        RoomType::Condo => Color::srgb(0.28, 0.65, 0.84),
        RoomType::PartyHall => Color::srgb(0.69, 0.42, 0.82),
        RoomType::Lobby => Color::srgb(0.91, 0.68, 0.29),
        RoomType::Office => Color::srgb(0.36, 0.73, 0.55),
        RoomType::FastFood => Color::srgb(0.91, 0.36, 0.24),
        RoomType::RetailShop => Color::srgb(0.88, 0.48, 0.72),
        RoomType::Restaurant => Color::srgb(0.78, 0.34, 0.22),
    }
}

/// Returns a translucent placement-preview color for a room type.
fn room_preview_color(room_type: RoomType) -> Color {
    let (red, green, blue) = match room_type {
        RoomType::Condo => (0.28, 0.65, 0.84),
        RoomType::PartyHall => (0.69, 0.42, 0.82),
        RoomType::Lobby => (0.91, 0.68, 0.29),
        RoomType::Office => (0.36, 0.73, 0.55),
        RoomType::FastFood => (0.91, 0.36, 0.24),
        RoomType::RetailShop => (0.88, 0.48, 0.72),
        RoomType::Restaurant => (0.78, 0.34, 0.22),
    };
    Color::srgba(red, green, blue, 0.36)
}

/// Reflects hover, press, and selection state in room-menu button colors.
fn update_room_button_appearance(
    placement: Res<PlacementState>,
    mut buttons: Query<(
        &Hovered,
        Has<Pressed>,
        &BuildOptionButton,
        &mut BackgroundColor,
        &mut BorderColor,
    )>,
) {
    for (hovered, pressed, room_button, mut background, mut border) in &mut buttons {
        let is_selected = placement.selected_build == Some(room_button.0);
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

/// Refreshes the status text when simulation values change.
fn update_hud(clock: Res<SimulationClock>, mut hud: Query<&mut Text, With<HudText>>) {
    if !clock.is_changed() {
        return;
    }
    for mut text in &mut hud {
        text.0 = format_clock(&clock);
    }
}

/// Formats the current simulation date, time, tick, and balance for display.
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

/// Formats a cash balance with a dollar sign and thousands separators.
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

/// Reflects hover, press, and active state in simulation-speed buttons.
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

/// Focused tests for renderer state, camera geometry, and formatting helpers.
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
    fn zoom_keeps_the_world_point_under_the_cursor_stationary() {
        let viewport = Vec2::new(1_280.0, 720.0);
        let cursor = Vec2::new(960.0, 180.0);
        let mut transform = Transform::from_xyz(400.0, 250.0, CAMERA_Z);
        let mut projection = OrthographicProjection {
            scale: 1.0,
            ..OrthographicProjection::default_2d()
        };
        let cursor_offset = Vec2::new(cursor.x - viewport.x * 0.5, viewport.y * 0.5 - cursor.y);
        let world_anchor_before =
            transform.translation.truncate() + cursor_offset * projection.scale;

        zoom_camera(&mut transform, &mut projection, 0.5, Some(cursor), viewport);

        let world_anchor_after =
            transform.translation.truncate() + cursor_offset * projection.scale;
        assert!(world_anchor_after.abs_diff_eq(world_anchor_before, 0.001));
        assert_eq!(projection.scale, 0.5);
    }

    #[test]
    fn zoom_without_cursor_keeps_the_camera_center_fixed() {
        let mut transform = Transform::from_xyz(400.0, 250.0, CAMERA_Z);
        let mut projection = OrthographicProjection {
            scale: 1.0,
            ..OrthographicProjection::default_2d()
        };
        let center_before = transform.translation;

        zoom_camera(
            &mut transform,
            &mut projection,
            0.5,
            None,
            Vec2::new(1_280.0, 720.0),
        );

        assert_eq!(transform.translation, center_before);
        assert_eq!(projection.scale, 0.5);
    }

    #[test]
    fn pinch_magnification_zooms_in_and_shrinking_zooms_out() {
        assert!(pinch_zoom_factor(0.1) < 1.0);
        assert!(pinch_zoom_factor(-0.1) > 1.0);
    }

    #[test]
    fn pan_speed_decreases_slightly_when_zoomed_in() {
        let zoomed_in = camera_pan_speed_factor(CAMERA_MIN_SCALE);
        let halfway = camera_pan_speed_factor((CAMERA_MIN_SCALE + CAMERA_MAX_SCALE) * 0.5);
        let zoomed_out = camera_pan_speed_factor(CAMERA_MAX_SCALE);

        assert!(zoomed_in < halfway);
        assert!(halfway < zoomed_out);
        assert!(zoomed_in >= zoomed_out * 0.75);
    }

    #[test]
    fn initial_camera_starts_just_above_the_first_two_underground_floors() {
        let window_height = 720.0;
        let center_y = initial_camera_y(window_height);
        let visible_bottom = center_y - window_height * CAMERA_START_SCALE * 0.5;
        let expected_bottom = -(INITIAL_UNDERGROUND_FLOORS as f32) * FLOOR_HEIGHT - FLOOR_THICKNESS;

        assert!((visible_bottom - expected_bottom).abs() < f32::EPSILON);
        assert!(CAMERA_MIN_SCALE < CAMERA_START_SCALE);
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

    #[test]
    fn room_drag_targets_advance_by_the_room_footprint() {
        assert_eq!(room_drag_target(2, 5, 2, 20, RoomType::Condo), Some((2, 5)));
        assert_eq!(
            room_drag_target(2, 5, 2, 21, RoomType::Condo),
            Some((2, 21))
        );
        assert_eq!(room_drag_target(2, 18, 2, 2, RoomType::Condo), Some((2, 2)));
        assert_eq!(
            room_drag_target(2, 5, 3, 5, RoomType::PartyHall),
            Some((2, 5))
        );
        assert_eq!(
            room_drag_target(2, 5, 4, 5, RoomType::PartyHall),
            Some((4, 5))
        );
    }

    #[test]
    fn room_drag_path_includes_each_intermediate_footprint() {
        assert_eq!(
            room_drag_path((2, 5), (2, 53), RoomType::Condo),
            vec![(2, 21), (2, 37), (2, 53)]
        );
        assert_eq!(
            room_drag_path((1, 0), (5, 48), RoomType::PartyHall),
            vec![(3, 24), (5, 48)]
        );
    }

    #[test]
    fn rooms_fill_their_columns_and_rest_on_the_floor() {
        let room_type = RoomType::Condo;
        let size = room_dimensions(room_type);
        let position = room_position(2, 0, room_type);

        assert_eq!(size.x, room_type.properties().width as f32 * COLUMN_WIDTH);
        assert_eq!(size.y, 24.0);
        assert_eq!(FLOOR_HEIGHT, size.y + FLOOR_THICKNESS);
        assert_eq!(position.y - size.y * 0.5, 2.0 * FLOOR_HEIGHT);
        assert_eq!(
            room_position(2, room_type.properties().width, room_type).x - size.x * 0.5,
            size.x
        );
    }

    #[test]
    fn condo_atlas_columns_follow_day_dusk_dawn_and_night() {
        assert_eq!(condo_atlas_index(TimeOfDayPeriod::Day), 0);
        assert_eq!(condo_atlas_index(TimeOfDayPeriod::Dusk), 1);
        assert_eq!(condo_atlas_index(TimeOfDayPeriod::Dawn), 1);
        assert_eq!(condo_atlas_index(TimeOfDayPeriod::Night), 2);
    }

    #[test]
    fn condo_atlas_frames_are_cropped_to_the_visible_sprite_size() {
        let layout = condo_atlas_layout();
        assert_eq!(layout.textures.len(), 3);
        for texture in layout.textures {
            assert_eq!(texture.max - texture.min, CONDO_FRAME_SIZE);
        }
    }
}
