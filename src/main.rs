use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use rand::RngExt;

const WINDOW_WIDTH: u32 = 1280;
const WINDOW_HEIGHT: u32 = 720;

// World size (larger than window for zooming)
const WORLD_WIDTH: f32 = 3840.0;
const WORLD_HEIGHT: f32 = 2160.0;
const TERRAIN_SEGMENTS: usize = 384;

// Camera settings
const MAX_ZOOM: f32 = 4.0;
const ZOOM_SPEED: f32 = 0.1;

// Player settings
const PLAYER_BASE_SIZE: f32 = 40.0;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Slugs".into(),
                resolution: (WINDOW_WIDTH, WINDOW_HEIGHT).into(),
                ..default()
            }),
            ..default()
        }))
        .init_resource::<GameState>()
        .init_resource::<TerrainData>()
        .add_systems(Startup, (setup_camera, generate_terrain, setup_ui))
        .add_systems(Update, (camera_zoom, camera_pan, update_turn_indicator))
        .run();
}

// Resources
#[derive(Resource, Default)]
struct TerrainData {
    heights: Vec<f32>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Player {
    Blue,
    Red,
}

impl Player {
    fn color(&self) -> Color {
        match self {
            Player::Blue => Color::srgb(0.2, 0.4, 0.8),
            Player::Red => Color::srgb(0.8, 0.2, 0.2),
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Player::Blue => "Blue",
            Player::Red => "Red",
        }
    }
}

#[derive(Resource)]
struct GameState {
    current_player: Player,
}

impl Default for GameState {
    fn default() -> Self {
        Self {
            current_player: Player::Blue,
        }
    }
}

// Components
#[derive(Component)]
struct MainCamera;

#[derive(Component)]
struct Terrain;

#[derive(Component)]
struct PlayerBase {
    player: Player,
}

#[derive(Component)]
struct TurnIndicator;

fn setup_camera(mut commands: Commands) {
    // Start zoomed out to see the whole world
    let initial_scale = WORLD_WIDTH / WINDOW_WIDTH as f32;
    commands.spawn((
        Camera2d,
        Transform::from_scale(Vec3::splat(initial_scale)),
        MainCamera,
    ));
}

fn setup_ui(mut commands: Commands) {
    commands.spawn((
        Text::new("Blue's Turn"),
        TextFont {
            font_size: 32.0,
            ..default()
        },
        TextColor(Player::Blue.color()),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(10.0),
            left: Val::Px(10.0),
            ..default()
        },
        TurnIndicator,
    ));
}

fn update_turn_indicator(
    game_state: Res<GameState>,
    mut query: Query<(&mut Text, &mut TextColor), With<TurnIndicator>>,
) {
    if !game_state.is_changed() {
        return;
    }

    for (mut text, mut color) in &mut query {
        **text = format!("{}'s Turn", game_state.current_player.name());
        *color = TextColor(game_state.current_player.color());
    }
}

fn generate_terrain(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut terrain_data: ResMut<TerrainData>,
) {
    let mut rng = rand::rng();

    // Generate terrain heights using midpoint displacement
    let mut heights = vec![0.0f32; TERRAIN_SEGMENTS + 1];
    heights[0] = rng.random_range(200.0..600.0);
    heights[TERRAIN_SEGMENTS] = rng.random_range(200.0..600.0);

    midpoint_displacement(&mut heights, 0, TERRAIN_SEGMENTS, 400.0, &mut rng);

    // Store heights for later use
    terrain_data.heights = heights.clone();

    // Build the terrain mesh
    let segment_width = WORLD_WIDTH / TERRAIN_SEGMENTS as f32;
    let half_width = WORLD_WIDTH / 2.0;
    let half_height = WORLD_HEIGHT / 2.0;

    let mut vertices = Vec::new();
    let mut indices = Vec::new();

    for i in 0..TERRAIN_SEGMENTS {
        let x0 = i as f32 * segment_width - half_width;
        let x1 = (i + 1) as f32 * segment_width - half_width;
        let y0 = heights[i] - half_height;
        let y1 = heights[i + 1] - half_height;
        let bottom = -half_height;

        let base = vertices.len() as u32;
        vertices.push([x0, bottom, 0.0]);
        vertices.push([x1, bottom, 0.0]);
        vertices.push([x1, y1, 0.0]);
        vertices.push([x0, y0, 0.0]);

        indices.push(base);
        indices.push(base + 1);
        indices.push(base + 2);
        indices.push(base);
        indices.push(base + 2);
        indices.push(base + 3);
    }

    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vertices);
    mesh.insert_indices(Indices::U32(indices));

    commands.spawn((
        Mesh2d(meshes.add(mesh)),
        MeshMaterial2d(materials.add(ColorMaterial::from_color(Color::srgb(0.2, 0.5, 0.2)))),
        Terrain,
    ));

    // Spawn player bases
    let half_width = WORLD_WIDTH / 2.0;
    let half_height = WORLD_HEIGHT / 2.0;

    // Blue player on the left (around 15% from left edge)
    let blue_segment = TERRAIN_SEGMENTS * 15 / 100;
    let blue_x = blue_segment as f32 * segment_width - half_width;
    let blue_y = heights[blue_segment] - half_height + PLAYER_BASE_SIZE / 2.0;

    commands.spawn((
        Sprite {
            color: Player::Blue.color(),
            custom_size: Some(Vec2::splat(PLAYER_BASE_SIZE)),
            ..default()
        },
        Transform::from_xyz(blue_x, blue_y, 1.0),
        PlayerBase {
            player: Player::Blue,
        },
    ));

    // Red player on the right (around 85% from left edge)
    let red_segment = TERRAIN_SEGMENTS * 85 / 100;
    let red_x = red_segment as f32 * segment_width - half_width;
    let red_y = heights[red_segment] - half_height + PLAYER_BASE_SIZE / 2.0;

    commands.spawn((
        Sprite {
            color: Player::Red.color(),
            custom_size: Some(Vec2::splat(PLAYER_BASE_SIZE)),
            ..default()
        },
        Transform::from_xyz(red_x, red_y, 1.0),
        PlayerBase {
            player: Player::Red,
        },
    ));
}

fn camera_zoom(
    mut scroll_events: MessageReader<MouseWheel>,
    mut camera_query: Query<&mut Transform, With<MainCamera>>,
    windows: Query<&Window>,
) {
    let mut scroll_delta = 0.0;

    for event in scroll_events.read() {
        scroll_delta += match event.unit {
            MouseScrollUnit::Line => event.y * ZOOM_SPEED,
            MouseScrollUnit::Pixel => event.y * ZOOM_SPEED * 0.01,
        };
    }

    if scroll_delta == 0.0 {
        return;
    }

    let Ok(mut camera_transform) = camera_query.single_mut() else {
        return;
    };
    let Ok(window) = windows.single() else {
        return;
    };

    let Some(cursor_position) = window.cursor_position() else {
        return;
    };

    // Convert cursor position to world coordinates before zoom
    let window_size = Vec2::new(window.width(), window.height());
    let cursor_ndc = (cursor_position - window_size / 2.0) * Vec2::new(1.0, -1.0);
    let cursor_world_before =
        camera_transform.translation.truncate() + cursor_ndc * camera_transform.scale.x;

    // Apply zoom
    let old_scale = camera_transform.scale.x;
    let max_scale = WORLD_WIDTH / WINDOW_WIDTH as f32; // Zoomed out to see whole world
    let min_scale = max_scale / MAX_ZOOM;

    let new_scale = (old_scale * (1.0 - scroll_delta)).clamp(min_scale, max_scale);
    camera_transform.scale = Vec3::splat(new_scale);

    // Convert cursor position to world coordinates after zoom
    let cursor_world_after =
        camera_transform.translation.truncate() + cursor_ndc * camera_transform.scale.x;

    // Adjust camera position so cursor stays over the same world point
    let delta = cursor_world_before - cursor_world_after;
    camera_transform.translation.x += delta.x;
    camera_transform.translation.y += delta.y;

    // Clamp camera to world bounds
    clamp_camera_to_world(&mut camera_transform, window_size);
}

fn camera_pan(
    mouse_button: Res<ButtonInput<MouseButton>>,
    mut camera_query: Query<&mut Transform, With<MainCamera>>,
    windows: Query<&Window>,
    mut last_cursor_pos: Local<Option<Vec2>>,
) {
    let Ok(mut camera_transform) = camera_query.single_mut() else {
        return;
    };
    let Ok(window) = windows.single() else {
        return;
    };

    let Some(cursor_position) = window.cursor_position() else {
        *last_cursor_pos = None;
        return;
    };

    if mouse_button.pressed(MouseButton::Middle) || mouse_button.pressed(MouseButton::Right) {
        if let Some(last_pos) = *last_cursor_pos {
            let delta = (cursor_position - last_pos) * Vec2::new(-1.0, 1.0);
            camera_transform.translation.x += delta.x * camera_transform.scale.x;
            camera_transform.translation.y += delta.y * camera_transform.scale.x;

            // Clamp camera to world bounds
            let window_size = Vec2::new(window.width(), window.height());
            clamp_camera_to_world(&mut camera_transform, window_size);
        }
        *last_cursor_pos = Some(cursor_position);
    } else {
        *last_cursor_pos = None;
    }
}

fn clamp_camera_to_world(camera_transform: &mut Transform, window_size: Vec2) {
    let half_view_width = window_size.x * camera_transform.scale.x / 2.0;
    let half_view_height = window_size.y * camera_transform.scale.x / 2.0;

    let half_world_width = WORLD_WIDTH / 2.0;
    let half_world_height = WORLD_HEIGHT / 2.0;

    // Only clamp if the view is smaller than the world
    if half_view_width < half_world_width {
        camera_transform.translation.x = camera_transform.translation.x.clamp(
            -half_world_width + half_view_width,
            half_world_width - half_view_width,
        );
    } else {
        camera_transform.translation.x = 0.0;
    }

    if half_view_height < half_world_height {
        camera_transform.translation.y = camera_transform.translation.y.clamp(
            -half_world_height + half_view_height,
            half_world_height - half_view_height,
        );
    } else {
        camera_transform.translation.y = 0.0;
    }
}

fn midpoint_displacement(
    heights: &mut [f32],
    left: usize,
    right: usize,
    roughness: f32,
    rng: &mut impl rand::Rng,
) {
    if right - left <= 1 {
        return;
    }

    let mid = (left + right) / 2;
    let avg = (heights[left] + heights[right]) / 2.0;
    heights[mid] = avg + rng.random_range(-roughness..roughness);

    let new_roughness = roughness * 0.6;
    midpoint_displacement(heights, left, mid, new_roughness, rng);
    midpoint_displacement(heights, mid, right, new_roughness, rng);
}
