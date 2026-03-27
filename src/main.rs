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
const PLAYER_BASE_HEALTH: f32 = 10.0;

// Projectile settings
const GRAVITY: f32 = 400.0;
const MAX_CHARGE_TIME: f32 = 2.0;
const MAX_LAUNCH_SPEED: f32 = 1200.0;
const PROJECTILE_RADIUS: f32 = 8.0;
const TURN_END_DELAY: f32 = 1.5;

// Aiming settings
const AIM_LINE_LENGTH: f32 = 500.0;
const TICK_MARK_SIZE: f32 = 15.0;

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
        .init_resource::<AimingState>()
        .add_systems(Startup, (setup_camera, generate_terrain, setup_ui))
        .add_systems(
            Update,
            (
                camera_zoom,
                camera_pan,
                update_turn_indicator,
                handle_weapon_selection,
                update_weapon_tooltip,
                handle_debug_win_button,
                handle_aiming,
                update_aim_line,
                update_charge_indicator,
            ),
        )
        .add_systems(
            Update,
            (
                update_projectiles,
                rebuild_terrain_mesh,
                update_falling_bases,
                update_health_bars,
                check_base_destruction,
                check_turn_end,
                update_game_over_overlay,
                handle_game_over_buttons,
            ),
        )
        .run();
}

// Resources
#[derive(Resource, Default)]
struct TerrainData {
    heights: Vec<f32>,
    needs_rebuild: bool,
}

impl TerrainData {
    fn get_height_at(&self, world_x: f32) -> Option<f32> {
        if self.heights.is_empty() {
            return None;
        }

        let half_width = WORLD_WIDTH / 2.0;
        let half_height = WORLD_HEIGHT / 2.0;

        // Convert world x to terrain segment
        let terrain_x = world_x + half_width;
        let segment_width = WORLD_WIDTH / TERRAIN_SEGMENTS as f32;
        let segment_f = terrain_x / segment_width;
        let segment = segment_f as usize;

        if segment >= TERRAIN_SEGMENTS {
            return None;
        }

        // Interpolate between segment heights
        let t = segment_f.fract();
        let h0 = self.heights[segment];
        let h1 = self.heights[segment + 1];
        let height = h0 + (h1 - h0) * t;

        Some(height - half_height)
    }

    fn apply_damage(
        &mut self,
        impact_world_x: f32,
        impact_world_y: f32,
        _damage: f32,
        blast_radius: f32,
    ) {
        if self.heights.is_empty() {
            return;
        }

        let half_width = WORLD_WIDTH / 2.0;
        let half_height = WORLD_HEIGHT / 2.0;
        let segment_width = WORLD_WIDTH / TERRAIN_SEGMENTS as f32;

        // Convert impact position to terrain coordinates (heights are stored in terrain space)
        let impact_terrain_y = impact_world_y + half_height;

        // Calculate which segments are affected by the blast
        let left_world_x = impact_world_x - blast_radius;
        let right_world_x = impact_world_x + blast_radius;

        let left_segment = ((left_world_x + half_width) / segment_width)
            .floor()
            .max(0.0) as usize;
        let right_segment = ((right_world_x + half_width) / segment_width)
            .ceil()
            .min(TERRAIN_SEGMENTS as f32) as usize;

        // Carve a circular crater - any terrain within the blast circle is destroyed
        // The crater is centered at the impact point with the given blast_radius
        for i in left_segment..=right_segment.min(TERRAIN_SEGMENTS) {
            let segment_world_x = i as f32 * segment_width - half_width;
            let dx = segment_world_x - impact_world_x;

            // Calculate the crater depth at this x position (circular crater)
            // For a circle: x² + y² = r², so y = sqrt(r² - x²)
            let dx_squared = dx * dx;
            let radius_squared = blast_radius * blast_radius;

            if dx_squared < radius_squared {
                // This segment is within the horizontal extent of the blast
                let crater_depth = (radius_squared - dx_squared).sqrt();
                let crater_floor = impact_terrain_y - crater_depth;

                // If terrain is above the crater floor, carve it down
                if self.heights[i] > crater_floor {
                    self.heights[i] = crater_floor.max(0.0);
                }
            }
        }

        self.needs_rebuild = true;
    }
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

    fn next(&self) -> Player {
        match self {
            Player::Blue => Player::Red,
            Player::Red => Player::Blue,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Weapon {
    Artillery,
}

struct WeaponStats {
    damage: f32,
    blast_radius: f32,
}

impl Weapon {
    fn stats(&self) -> WeaponStats {
        match self {
            Weapon::Artillery => WeaponStats {
                damage: 10.0,
                blast_radius: 50.0,
            },
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Weapon::Artillery => "Artillery",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum TurnPhase {
    #[default]
    Aiming,
    ProjectileInFlight,
    TurnEnding,
    GameOver,
}

#[derive(Resource)]
struct GameState {
    current_player: Player,
    selected_weapon: Option<Weapon>,
    phase: TurnPhase,
    turn_end_timer: f32,
    winner: Option<Player>,
}

impl Default for GameState {
    fn default() -> Self {
        Self {
            current_player: Player::Blue,
            selected_weapon: None,
            phase: TurnPhase::Aiming,
            turn_end_timer: 0.0,
            winner: None,
        }
    }
}

#[derive(Resource, Default)]
struct AimingState {
    charging: bool,
    charge_time: f32,
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
struct Health {
    current: f32,
    max: f32,
}

impl Health {
    fn new(max: f32) -> Self {
        Self { current: max, max }
    }

    fn take_damage(&mut self, amount: f32) {
        self.current = (self.current - amount).max(0.0);
    }

    fn is_dead(&self) -> bool {
        self.current <= 0.0
    }
}

#[derive(Component)]
struct HealthBar {
    owner: Entity,
}

#[derive(Component)]
struct TurnIndicator;

#[derive(Component)]
struct WeaponButton {
    weapon: Weapon,
}

#[derive(Component)]
struct ChargeIndicatorWorld;

#[derive(Component)]
struct Projectile {
    velocity: Vec2,
    weapon: Weapon,
}

#[derive(Component)]
struct DebugWinButton;

#[derive(Component)]
struct GameOverOverlay;

#[derive(Component)]
struct NewGameButton;

#[derive(Component)]
struct ExitButton;

#[derive(Component)]
struct WeaponTooltip;

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
    // Turn indicator
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

    // Weapon toolbar at bottom
    commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(10.0),
            left: Val::Px(10.0),
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(10.0),
            ..default()
        })
        .with_children(|parent| {
            // Artillery button
            parent
                .spawn((
                    Button,
                    Interaction::None,
                    Node {
                        width: Val::Px(100.0),
                        height: Val::Px(40.0),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        border: UiRect::all(Val::Px(2.0)),
                        ..default()
                    },
                    BorderColor::all(Color::WHITE),
                    BackgroundColor(Color::srgb(0.3, 0.3, 0.3)),
                    WeaponButton {
                        weapon: Weapon::Artillery,
                    },
                ))
                .with_child((
                    Text::new("Artillery"),
                    TextFont {
                        font_size: 16.0,
                        ..default()
                    },
                    TextColor(Color::WHITE),
                ));

            // Debug win button
            parent
                .spawn((
                    Button,
                    Node {
                        width: Val::Px(60.0),
                        height: Val::Px(40.0),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        border: UiRect::all(Val::Px(2.0)),
                        ..default()
                    },
                    BorderColor::all(Color::srgb(0.5, 0.5, 0.0)),
                    BackgroundColor(Color::srgb(0.2, 0.2, 0.1)),
                    DebugWinButton,
                ))
                .with_child((
                    Text::new("Win"),
                    TextFont {
                        font_size: 16.0,
                        ..default()
                    },
                    TextColor(Color::srgb(1.0, 1.0, 0.5)),
                ));
        });

    // Weapon tooltip (hidden by default, positioned near cursor)
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                padding: UiRect::all(Val::Px(8.0)),
                left: Val::Px(100.0),
                top: Val::Px(100.0),
                display: Display::None,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.9)),
            GlobalZIndex(100),
            WeaponTooltip,
        ))
        .with_child((
            Text::new("Artillery\nDamage: 10\nBlast: 50"),
            TextFont {
                font_size: 14.0,
                ..default()
            },
            TextColor(Color::WHITE),
        ));

    // World-space charge indicator (will be positioned near base during aiming)
    commands.spawn((
        Text2d::new(""),
        TextFont {
            font_size: 36.0,
            ..default()
        },
        TextColor(Color::srgb(1.0, 0.7, 0.0)),
        Transform::from_xyz(0.0, 0.0, 10.0),
        Visibility::Hidden,
        ChargeIndicatorWorld,
    ));

    // Game over overlay (hidden by default)
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(30.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.7)),
            Visibility::Hidden,
            GameOverOverlay,
        ))
        .with_children(|parent| {
            // Winner text
            parent.spawn((
                Text::new(""),
                TextFont {
                    font_size: 72.0,
                    ..default()
                },
                TextColor(Color::WHITE),
            ));

            // Button container
            parent
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(20.0),
                    ..default()
                })
                .with_children(|button_parent| {
                    // New Game button
                    button_parent
                        .spawn((
                            Button,
                            Node {
                                width: Val::Px(150.0),
                                height: Val::Px(50.0),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                border: UiRect::all(Val::Px(2.0)),
                                ..default()
                            },
                            BorderColor::all(Color::WHITE),
                            BackgroundColor(Color::srgb(0.2, 0.5, 0.2)),
                            NewGameButton,
                        ))
                        .with_child((
                            Text::new("New Game"),
                            TextFont {
                                font_size: 20.0,
                                ..default()
                            },
                            TextColor(Color::WHITE),
                        ));

                    // Exit button
                    button_parent
                        .spawn((
                            Button,
                            Node {
                                width: Val::Px(150.0),
                                height: Val::Px(50.0),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                border: UiRect::all(Val::Px(2.0)),
                                ..default()
                            },
                            BorderColor::all(Color::WHITE),
                            BackgroundColor(Color::srgb(0.5, 0.2, 0.2)),
                            ExitButton,
                        ))
                        .with_child((
                            Text::new("Exit"),
                            TextFont {
                                font_size: 20.0,
                                ..default()
                            },
                            TextColor(Color::WHITE),
                        ));
                });
        });
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

fn handle_weapon_selection(
    mut game_state: ResMut<GameState>,
    interaction_query: Query<(&Interaction, &WeaponButton), Changed<Interaction>>,
    mut button_query: Query<(&Interaction, &WeaponButton, &mut BorderColor, &mut BackgroundColor)>,
) {
    // Handle clicks - toggle selection
    for (interaction, weapon_button) in &interaction_query {
        if *interaction == Interaction::Pressed {
            if game_state.selected_weapon == Some(weapon_button.weapon) {
                game_state.selected_weapon = None;
            } else {
                game_state.selected_weapon = Some(weapon_button.weapon);
            }
        }
    }

    // Update button visuals
    for (interaction, weapon_button, mut border_color, mut bg_color) in &mut button_query {
        let is_selected = game_state.selected_weapon == Some(weapon_button.weapon);
        let is_hovered = *interaction == Interaction::Hovered;

        // Border: yellow if selected, white otherwise
        if is_selected {
            *border_color = BorderColor::all(Color::srgb(1.0, 1.0, 0.0));
        } else {
            *border_color = BorderColor::all(Color::WHITE);
        }

        // Background: combine selected and hovered states
        let base = if is_selected { 0.4 } else { 0.3 };
        let brightness = if is_hovered { base + 0.15 } else { base };
        *bg_color = BackgroundColor(Color::srgb(brightness, brightness, brightness));
    }
}

fn update_weapon_tooltip(
    weapon_buttons: Query<(&Interaction, &WeaponButton)>,
    mut tooltip_query: Query<(&mut Node, &Children), With<WeaponTooltip>>,
    mut text_query: Query<&mut Text>,
    windows: Query<&Window>,
) {
    let Ok((mut node, children)) = tooltip_query.single_mut() else {
        return;
    };

    // Find hovered weapon button
    let hovered = weapon_buttons
        .iter()
        .find(|(interaction, _)| **interaction == Interaction::Hovered);

    if let Some((_, weapon_button)) = hovered {
        let stats = weapon_button.weapon.stats();

        // Update text in child
        if let Some(child) = children.iter().next() {
            if let Ok(mut text) = text_query.get_mut(child) {
                **text = format!(
                    "{}\nDamage: {}\nBlast Radius: {}",
                    weapon_button.weapon.name(),
                    stats.damage,
                    stats.blast_radius
                );
            }
        }

        // Position tooltip above cursor
        if let Ok(window) = windows.single() {
            if let Some(cursor) = window.cursor_position() {
                node.left = Val::Px(cursor.x + 15.0);
                node.top = Val::Auto;
                node.bottom = Val::Px(window.height() - cursor.y + 15.0);
            }
        }
        node.display = Display::Flex;
    } else {
        node.display = Display::None;
    }
}

fn handle_debug_win_button(
    mut game_state: ResMut<GameState>,
    interaction_query: Query<&Interaction, (Changed<Interaction>, With<DebugWinButton>)>,
) {
    for interaction in &interaction_query {
        if *interaction == Interaction::Pressed {
            game_state.winner = Some(game_state.current_player);
            game_state.phase = TurnPhase::GameOver;
        }
    }
}

fn handle_aiming(
    mut commands: Commands,
    mut game_state: ResMut<GameState>,
    mut aiming_state: ResMut<AimingState>,
    mouse_button: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    camera_query: Query<&Transform, With<MainCamera>>,
    player_bases: Query<(&Transform, &PlayerBase), Without<MainCamera>>,
    time: Res<Time>,
    interaction_query: Query<&Interaction, With<Button>>,
) {
    // Only allow aiming if a weapon is selected
    if game_state.phase != TurnPhase::Aiming || game_state.selected_weapon.is_none() {
        aiming_state.charging = false;
        aiming_state.charge_time = 0.0;
        return;
    }

    // Don't start charging if clicking on a UI button
    let clicking_ui = interaction_query.iter().any(|i| *i != Interaction::None);

    let Ok(window) = windows.single() else {
        return;
    };
    let Ok(camera_transform) = camera_query.single() else {
        return;
    };

    // Find current player's base position
    let Some(base_pos) = player_bases.iter().find_map(|(transform, base)| {
        if base.player == game_state.current_player {
            Some(transform.translation.truncate())
        } else {
            None
        }
    }) else {
        return;
    };

    // Get cursor world position
    let Some(cursor_position) = window.cursor_position() else {
        return;
    };
    let window_size = Vec2::new(window.width(), window.height());
    let cursor_ndc = (cursor_position - window_size / 2.0) * Vec2::new(1.0, -1.0);
    let cursor_world =
        camera_transform.translation.truncate() + cursor_ndc * camera_transform.scale.x;

    // Calculate aim direction
    let aim_direction = (cursor_world - base_pos).normalize_or_zero();

    if mouse_button.just_pressed(MouseButton::Left) && !clicking_ui {
        aiming_state.charging = true;
        aiming_state.charge_time = 0.0;
    }

    if aiming_state.charging {
        aiming_state.charge_time =
            (aiming_state.charge_time + time.delta_secs()).min(MAX_CHARGE_TIME);
    }

    if mouse_button.just_released(MouseButton::Left) && aiming_state.charging {
        // Fire projectile
        let speed = (aiming_state.charge_time / MAX_CHARGE_TIME) * MAX_LAUNCH_SPEED;
        let velocity = aim_direction * speed;
        let weapon = game_state.selected_weapon.unwrap();

        let start = base_pos + Vec2::Y * (PLAYER_BASE_SIZE / 2.0);
        commands.spawn((
            Sprite {
                color: game_state.current_player.color(),
                custom_size: Some(Vec2::splat(PROJECTILE_RADIUS * 2.0)),
                ..default()
            },
            Transform::from_xyz(start.x, start.y, 2.0),
            Projectile { velocity, weapon },
        ));

        aiming_state.charging = false;
        aiming_state.charge_time = 0.0;
        game_state.phase = TurnPhase::ProjectileInFlight;
    }
}

fn update_aim_line(
    mut gizmos: Gizmos,
    game_state: Res<GameState>,
    aiming_state: Res<AimingState>,
    windows: Query<&Window>,
    camera_query: Query<&Transform, With<MainCamera>>,
    player_bases: Query<(&Transform, &PlayerBase), Without<MainCamera>>,
) {
    // Only show aim line if weapon is selected and in aiming phase
    if game_state.phase != TurnPhase::Aiming || game_state.selected_weapon.is_none() {
        return;
    }

    let Ok(window) = windows.single() else {
        return;
    };
    let Ok(camera_transform) = camera_query.single() else {
        return;
    };

    // Find current player's base position
    let Some(base_pos) = player_bases.iter().find_map(|(transform, base)| {
        if base.player == game_state.current_player {
            Some(transform.translation.truncate())
        } else {
            None
        }
    }) else {
        return;
    };

    // Get cursor world position
    let Some(cursor_position) = window.cursor_position() else {
        return;
    };
    let window_size = Vec2::new(window.width(), window.height());
    let cursor_ndc = (cursor_position - window_size / 2.0) * Vec2::new(1.0, -1.0);
    let cursor_world =
        camera_transform.translation.truncate() + cursor_ndc * camera_transform.scale.x;

    // Calculate aim direction (fixed length line)
    let start = base_pos + Vec2::Y * (PLAYER_BASE_SIZE / 2.0);
    let aim_direction = (cursor_world - start).normalize_or_zero();
    let end = start + aim_direction * AIM_LINE_LENGTH;

    // Calculate perpendicular direction for tick marks
    let perp = Vec2::new(-aim_direction.y, aim_direction.x);

    // Draw the main aim line (white/gray)
    gizmos.line_2d(start, end, Color::srgb(0.7, 0.7, 0.7));

    // Draw tick marks at 25%, 50%, 75%, 100%
    for i in 1..=4 {
        let t = i as f32 * 0.25;
        let tick_pos = start + aim_direction * (AIM_LINE_LENGTH * t);
        let tick_half = perp * (TICK_MARK_SIZE / 2.0);
        gizmos.line_2d(
            tick_pos - tick_half,
            tick_pos + tick_half,
            Color::srgb(0.7, 0.7, 0.7),
        );
    }

    // Draw charge line (orange/yellow gradient effect via segments)
    if aiming_state.charging {
        let charge_fraction = aiming_state.charge_time / MAX_CHARGE_TIME;
        let charge_length = AIM_LINE_LENGTH * charge_fraction;

        // Draw gradient segments
        let num_segments = 10;
        for i in 0..num_segments {
            let seg_start_t = i as f32 / num_segments as f32;
            let seg_end_t = (i + 1) as f32 / num_segments as f32;

            // Only draw if this segment is within the charge
            if seg_start_t >= charge_fraction {
                break;
            }

            let actual_end_t = seg_end_t.min(charge_fraction);
            let seg_start = start + aim_direction * (AIM_LINE_LENGTH * seg_start_t);
            let seg_end = start + aim_direction * (AIM_LINE_LENGTH * actual_end_t);

            // Interpolate color from yellow to orange/red based on position
            let color_t = (seg_start_t + actual_end_t) / 2.0;
            let r = 1.0;
            let g = 1.0 - color_t * 0.6; // Goes from 1.0 (yellow) to 0.4 (orange-red)
            let b = 0.0;

            gizmos.line_2d(seg_start, seg_end, Color::srgb(r, g, b));
        }

        // Draw small circle at charge end
        let charge_end = start + aim_direction * charge_length;
        gizmos.circle_2d(charge_end, 5.0, Color::srgb(1.0, 0.5, 0.0));
    }
}

fn update_charge_indicator(
    game_state: Res<GameState>,
    aiming_state: Res<AimingState>,
    windows: Query<&Window>,
    camera_query: Query<
        &Transform,
        (
            With<MainCamera>,
            Without<ChargeIndicatorWorld>,
            Without<PlayerBase>,
        ),
    >,
    player_bases: Query<
        (&Transform, &PlayerBase),
        (Without<MainCamera>, Without<ChargeIndicatorWorld>),
    >,
    mut indicator_query: Query<
        (&mut Text2d, &mut Transform, &mut Visibility),
        With<ChargeIndicatorWorld>,
    >,
) {
    let Ok((mut text, mut transform, mut visibility)) = indicator_query.single_mut() else {
        return;
    };

    // Hide if not aiming or not charging
    if game_state.phase != TurnPhase::Aiming
        || game_state.selected_weapon.is_none()
        || !aiming_state.charging
    {
        *visibility = Visibility::Hidden;
        return;
    }

    let Ok(window) = windows.single() else {
        return;
    };
    let Ok(camera_transform) = camera_query.single() else {
        return;
    };

    // Find current player's base position
    let Some(base_pos) = player_bases.iter().find_map(|(t, base)| {
        if base.player == game_state.current_player {
            Some(t.translation.truncate())
        } else {
            None
        }
    }) else {
        return;
    };

    // Get cursor world position for aim direction
    let Some(cursor_position) = window.cursor_position() else {
        return;
    };
    let window_size = Vec2::new(window.width(), window.height());
    let cursor_ndc = (cursor_position - window_size / 2.0) * Vec2::new(1.0, -1.0);
    let cursor_world =
        camera_transform.translation.truncate() + cursor_ndc * camera_transform.scale.x;

    let start = base_pos + Vec2::Y * (PLAYER_BASE_SIZE / 2.0);
    let aim_direction = (cursor_world - start).normalize_or_zero();

    // Position text offset from the base, perpendicular to aim direction
    let perp = Vec2::new(-aim_direction.y, aim_direction.x);
    let text_offset = perp * 50.0 - aim_direction * 20.0; // Offset to the side and slightly back
    let text_pos = start + text_offset;

    // Update text and position
    let charge_percent = (aiming_state.charge_time / MAX_CHARGE_TIME * 100.0) as u32;
    **text = format!("{}%", charge_percent);
    transform.translation.x = text_pos.x;
    transform.translation.y = text_pos.y;
    *visibility = Visibility::Visible;
}

fn update_projectiles(
    mut commands: Commands,
    mut game_state: ResMut<GameState>,
    mut terrain_data: ResMut<TerrainData>,
    time: Res<Time>,
    mut projectiles: Query<(Entity, &mut Transform, &mut Projectile)>,
    mut player_bases: Query<(Entity, &Transform, &PlayerBase, &mut Health), Without<Projectile>>,
) {
    if game_state.phase != TurnPhase::ProjectileInFlight {
        return;
    }

    let dt = time.delta_secs();
    let half_width = WORLD_WIDTH / 2.0;

    for (entity, mut transform, mut projectile) in &mut projectiles {
        // Apply gravity
        projectile.velocity.y -= GRAVITY * dt;

        // Update position
        transform.translation.x += projectile.velocity.x * dt;
        transform.translation.y += projectile.velocity.y * dt;

        let pos = transform.translation.truncate();

        // Check world bounds
        if pos.x < -half_width || pos.x > half_width {
            commands.entity(entity).despawn();
            game_state.phase = TurnPhase::TurnEnding;
            game_state.turn_end_timer = TURN_END_DELAY;
            continue;
        }

        // Check direct base collision (projectile hits base directly)
        let mut hit_base_directly = false;
        for (_, base_transform, _, _) in &player_bases {
            let base_pos = base_transform.translation.truncate();
            let half_size = PLAYER_BASE_SIZE / 2.0;

            if pos.x >= base_pos.x - half_size
                && pos.x <= base_pos.x + half_size
                && pos.y >= base_pos.y - half_size
                && pos.y <= base_pos.y + half_size
            {
                hit_base_directly = true;
                break;
            }
        }

        // Check terrain collision
        let terrain_hit = terrain_data
            .get_height_at(pos.x)
            .map(|h| pos.y <= h)
            .unwrap_or(false);

        if hit_base_directly || terrain_hit {
            // Apply blast damage to terrain
            let stats = projectile.weapon.stats();
            terrain_data.apply_damage(pos.x, pos.y, stats.damage, stats.blast_radius);

            // Apply blast damage to all bases within blast radius
            for (_, base_transform, _, mut health) in &mut player_bases {
                let base_pos = base_transform.translation.truncate();
                let distance = pos.distance(base_pos);

                // Direct hit if impact touches the base (within half the base size)
                let direct_hit_radius = PLAYER_BASE_SIZE / 2.0;
                if distance <= direct_hit_radius {
                    // Full damage for direct hit
                    health.take_damage(stats.damage);
                } else if distance < stats.blast_radius {
                    // Damage falls off linearly with distance from edge of base
                    let effective_distance = distance - direct_hit_radius;
                    let effective_radius = stats.blast_radius - direct_hit_radius;
                    let damage_factor = 1.0 - (effective_distance / effective_radius);
                    let damage = stats.damage * damage_factor;
                    health.take_damage(damage);
                }
            }

            commands.entity(entity).despawn();
            game_state.phase = TurnPhase::TurnEnding;
            game_state.turn_end_timer = TURN_END_DELAY;
        }
    }
}

fn rebuild_terrain_mesh(
    mut terrain_data: ResMut<TerrainData>,
    mut meshes: ResMut<Assets<Mesh>>,
    terrain_query: Query<&Mesh2d, With<Terrain>>,
) {
    if !terrain_data.needs_rebuild {
        return;
    }

    terrain_data.needs_rebuild = false;

    let Ok(mesh_handle) = terrain_query.single() else {
        return;
    };

    let Some(mesh) = meshes.get_mut(&mesh_handle.0) else {
        return;
    };

    // Rebuild vertices
    let segment_width = WORLD_WIDTH / TERRAIN_SEGMENTS as f32;
    let half_width = WORLD_WIDTH / 2.0;
    let half_height = WORLD_HEIGHT / 2.0;

    let mut vertices = Vec::new();
    let mut indices = Vec::new();

    for i in 0..TERRAIN_SEGMENTS {
        let x0 = i as f32 * segment_width - half_width;
        let x1 = (i + 1) as f32 * segment_width - half_width;
        let y0 = terrain_data.heights[i] - half_height;
        let y1 = terrain_data.heights[i + 1] - half_height;
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

    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vertices);
    mesh.insert_indices(Indices::U32(indices));
}

fn update_falling_bases(
    terrain_data: Res<TerrainData>,
    mut bases: Query<&mut Transform, With<PlayerBase>>,
    time: Res<Time>,
) {
    for mut transform in &mut bases {
        let base_x = transform.translation.x;
        let base_bottom = transform.translation.y - PLAYER_BASE_SIZE / 2.0;

        // Get terrain height at base position
        if let Some(terrain_height) = terrain_data.get_height_at(base_x) {
            // If base is above terrain, make it fall
            if base_bottom > terrain_height + 1.0 {
                // Apply gravity
                let fall_speed = GRAVITY * time.delta_secs();
                transform.translation.y -= fall_speed;

                // Don't fall below terrain
                let min_y = terrain_height + PLAYER_BASE_SIZE / 2.0;
                if transform.translation.y < min_y {
                    transform.translation.y = min_y;
                }
            } else {
                // Snap to terrain if close
                transform.translation.y = terrain_height + PLAYER_BASE_SIZE / 2.0;
            }
        }
    }
}

fn update_health_bars(
    bases: Query<(Entity, &Transform, &Health), With<PlayerBase>>,
    mut health_bars: Query<(&mut Text2d, &mut Transform, &HealthBar), Without<PlayerBase>>,
) {
    for (mut text, mut bar_transform, health_bar) in &mut health_bars {
        // Find the owner base
        if let Some((_, base_transform, health)) =
            bases.iter().find(|(e, _, _)| *e == health_bar.owner)
        {
            // Update text
            **text = format!("{}", health.current.ceil() as i32);

            // Position below the base
            bar_transform.translation.x = base_transform.translation.x;
            bar_transform.translation.y =
                base_transform.translation.y - PLAYER_BASE_SIZE / 2.0 - 25.0;
        }
    }
}

fn check_base_destruction(mut game_state: ResMut<GameState>, bases: Query<(&PlayerBase, &Health)>) {
    // Only check during turn ending phase to avoid premature game over
    if game_state.phase != TurnPhase::TurnEnding {
        return;
    }

    for (player_base, health) in &bases {
        if health.is_dead() {
            // This player's base was destroyed, the other player wins
            game_state.winner = Some(player_base.player.next());
            game_state.phase = TurnPhase::GameOver;
            return;
        }
    }
}

fn check_turn_end(mut game_state: ResMut<GameState>, time: Res<Time>) {
    if game_state.phase != TurnPhase::TurnEnding {
        return;
    }

    game_state.turn_end_timer -= time.delta_secs();

    if game_state.turn_end_timer <= 0.0 {
        game_state.current_player = game_state.current_player.next();
        game_state.selected_weapon = None;
        game_state.phase = TurnPhase::Aiming;
    }
}

fn update_game_over_overlay(
    game_state: Res<GameState>,
    mut overlay_query: Query<(&mut Visibility, &Children), With<GameOverOverlay>>,
    mut text_query: Query<(&mut Text, &mut TextColor)>,
) {
    let Ok((mut visibility, children)) = overlay_query.single_mut() else {
        return;
    };

    if game_state.phase == TurnPhase::GameOver {
        *visibility = Visibility::Visible;

        // Update winner text
        if let Some(winner) = game_state.winner {
            // The first child should be the winner text
            if let Some(first_child) = children.iter().next() {
                if let Ok((mut text, mut color)) = text_query.get_mut(first_child) {
                    **text = format!("{} Wins!", winner.name());
                    *color = TextColor(winner.color());
                }
            }
        }
    } else {
        *visibility = Visibility::Hidden;
    }
}

fn handle_game_over_buttons(
    new_game_query: Query<&Interaction, (Changed<Interaction>, With<NewGameButton>)>,
    exit_query: Query<&Interaction, (Changed<Interaction>, With<ExitButton>)>,
    mut app_exit: MessageWriter<AppExit>,
    mut game_state: ResMut<GameState>,
    mut terrain_data: ResMut<TerrainData>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    terrain_query: Query<Entity, With<Terrain>>,
    base_query: Query<Entity, With<PlayerBase>>,
    projectile_query: Query<Entity, With<Projectile>>,
    health_bar_query: Query<Entity, With<HealthBar>>,
) {
    // Handle exit button
    for interaction in &exit_query {
        if *interaction == Interaction::Pressed {
            app_exit.write(AppExit::Success);
        }
    }

    // Handle new game button
    for interaction in &new_game_query {
        if *interaction == Interaction::Pressed {
            // Reset game state
            *game_state = GameState::default();

            // Despawn old entities
            for entity in terrain_query.iter() {
                commands.entity(entity).despawn();
            }
            for entity in base_query.iter() {
                commands.entity(entity).despawn();
            }
            for entity in projectile_query.iter() {
                commands.entity(entity).despawn();
            }
            for entity in health_bar_query.iter() {
                commands.entity(entity).despawn();
            }

            // Generate new terrain (inline the logic here)
            let mut rng = rand::rng();

            let mut heights = vec![0.0f32; TERRAIN_SEGMENTS + 1];
            heights[0] = rng.random_range(200.0..600.0);
            heights[TERRAIN_SEGMENTS] = rng.random_range(200.0..600.0);

            midpoint_displacement(&mut heights, 0, TERRAIN_SEGMENTS, 400.0, &mut rng);

            terrain_data.heights = heights.clone();

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
                MeshMaterial2d(
                    materials.add(ColorMaterial::from_color(Color::srgb(0.2, 0.5, 0.2))),
                ),
                Terrain,
            ));

            // Spawn player bases
            let blue_segment = TERRAIN_SEGMENTS * 15 / 100;
            let blue_x = blue_segment as f32 * segment_width - half_width;
            let blue_y = heights[blue_segment] - half_height + PLAYER_BASE_SIZE / 2.0;

            let blue_base = commands
                .spawn((
                    Sprite {
                        color: Player::Blue.color(),
                        custom_size: Some(Vec2::splat(PLAYER_BASE_SIZE)),
                        ..default()
                    },
                    Transform::from_xyz(blue_x, blue_y, 1.0),
                    PlayerBase {
                        player: Player::Blue,
                    },
                    Health::new(PLAYER_BASE_HEALTH),
                ))
                .id();

            // Blue health bar
            commands.spawn((
                Text2d::new(format!("{}", PLAYER_BASE_HEALTH as i32)),
                TextFont {
                    font_size: 48.0,
                    ..default()
                },
                TextColor(Player::Blue.color()),
                Transform::from_xyz(blue_x, blue_y - PLAYER_BASE_SIZE / 2.0 - 25.0, 1.0),
                HealthBar { owner: blue_base },
            ));

            let red_segment = TERRAIN_SEGMENTS * 85 / 100;
            let red_x = red_segment as f32 * segment_width - half_width;
            let red_y = heights[red_segment] - half_height + PLAYER_BASE_SIZE / 2.0;

            let red_base = commands
                .spawn((
                    Sprite {
                        color: Player::Red.color(),
                        custom_size: Some(Vec2::splat(PLAYER_BASE_SIZE)),
                        ..default()
                    },
                    Transform::from_xyz(red_x, red_y, 1.0),
                    PlayerBase {
                        player: Player::Red,
                    },
                    Health::new(PLAYER_BASE_HEALTH),
                ))
                .id();

            // Red health bar
            commands.spawn((
                Text2d::new(format!("{}", PLAYER_BASE_HEALTH as i32)),
                TextFont {
                    font_size: 48.0,
                    ..default()
                },
                TextColor(Player::Red.color()),
                Transform::from_xyz(red_x, red_y - PLAYER_BASE_SIZE / 2.0 - 25.0, 1.0),
                HealthBar { owner: red_base },
            ));
        }
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

    let blue_base = commands
        .spawn((
            Sprite {
                color: Player::Blue.color(),
                custom_size: Some(Vec2::splat(PLAYER_BASE_SIZE)),
                ..default()
            },
            Transform::from_xyz(blue_x, blue_y, 1.0),
            PlayerBase {
                player: Player::Blue,
            },
            Health::new(PLAYER_BASE_HEALTH),
        ))
        .id();

    // Blue health bar
    commands.spawn((
        Text2d::new(format!("{}", PLAYER_BASE_HEALTH as i32)),
        TextFont {
            font_size: 48.0,
            ..default()
        },
        TextColor(Player::Blue.color()),
        Transform::from_xyz(blue_x, blue_y - PLAYER_BASE_SIZE / 2.0 - 25.0, 1.0),
        HealthBar { owner: blue_base },
    ));

    // Red player on the right (around 85% from left edge)
    let red_segment = TERRAIN_SEGMENTS * 85 / 100;
    let red_x = red_segment as f32 * segment_width - half_width;
    let red_y = heights[red_segment] - half_height + PLAYER_BASE_SIZE / 2.0;

    let red_base = commands
        .spawn((
            Sprite {
                color: Player::Red.color(),
                custom_size: Some(Vec2::splat(PLAYER_BASE_SIZE)),
                ..default()
            },
            Transform::from_xyz(red_x, red_y, 1.0),
            PlayerBase {
                player: Player::Red,
            },
            Health::new(PLAYER_BASE_HEALTH),
        ))
        .id();

    // Red health bar
    commands.spawn((
        Text2d::new(format!("{}", PLAYER_BASE_HEALTH as i32)),
        TextFont {
            font_size: 48.0,
            ..default()
        },
        TextColor(Player::Red.color()),
        Transform::from_xyz(red_x, red_y - PLAYER_BASE_SIZE / 2.0 - 25.0, 1.0),
        HealthBar { owner: red_base },
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
