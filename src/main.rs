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

// Terrain settings
const GROUND_DAMAGE_RESISTANCE: f32 = 2.0;

// Building settings
const BUILD_RADIUS: f32 = 200.0;

// AA Missile settings
const AA_DETECTION_RANGE: f32 = 800.0; // Range at which AA detects incoming projectiles
const AA_MISSILE_ACCELERATION: f32 = 800.0;
const AA_MISSILE_MAX_SPEED: f32 = 600.0;
const AA_MISSILE_TURN_RATE: f32 = 4.0; // Radians per second
const AA_MISSILE_MAX_RANGE: f32 = 500.0;
const AA_MISSILE_SIZE: f32 = 6.0;
const AA_MISSILE_EXPLOSION_RADIUS: f32 = 30.0;

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
                handle_buildable_selection,
                update_weapon_tooltip,
                handle_aiming,
                handle_building,
                draw_buildable_area,
                update_aim_line,
                update_charge_indicator,
            ),
        )
        .add_systems(
            Update,
            (
                update_projectiles,
                aa_fire_missiles,
                update_aa_missiles,
                check_projectile_phase_end,
                update_explosions,
                rebuild_terrain_mesh,
                update_falling_bases,
                update_health_bars,
                check_base_destruction,
                check_aa_destruction,
                reset_aa_launchers,
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
        damage: f32,
        blast_radius: f32,
    ) {
        if self.heights.is_empty() {
            return;
        }

        // Effective crater radius is reduced by ground damage resistance
        let crater_radius = blast_radius * damage / (damage + GROUND_DAMAGE_RESISTANCE);

        let half_width = WORLD_WIDTH / 2.0;
        let half_height = WORLD_HEIGHT / 2.0;
        let segment_width = WORLD_WIDTH / TERRAIN_SEGMENTS as f32;

        // Convert impact position to terrain coordinates (heights are stored in terrain space)
        let impact_terrain_y = impact_world_y + half_height;

        // Calculate which segments are affected by the blast
        let left_world_x = impact_world_x - crater_radius;
        let right_world_x = impact_world_x + crater_radius;

        let left_segment = ((left_world_x + half_width) / segment_width)
            .floor()
            .max(0.0) as usize;
        let right_segment = ((right_world_x + half_width) / segment_width)
            .ceil()
            .min(TERRAIN_SEGMENTS as f32) as usize;

        // Carve a circular crater - any terrain within the blast circle is destroyed
        // The crater is centered at the impact point with the given crater_radius
        for i in left_segment..=right_segment.min(TERRAIN_SEGMENTS) {
            let segment_world_x = i as f32 * segment_width - half_width;
            let dx = segment_world_x - impact_world_x;

            // Calculate the crater depth at this x position (circular crater)
            // For a circle: x² + y² = r², so y = sqrt(r² - x²)
            let dx_squared = dx * dx;
            let radius_squared = crater_radius * crater_radius;

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
    ClusterGrenade,
    ClusterSubmunition,
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
                blast_radius: 100.0,
            },
            Weapon::ClusterGrenade => WeaponStats {
                damage: 0.0, // Main grenade doesn't explode
                blast_radius: 0.0,
            },
            Weapon::ClusterSubmunition => WeaponStats {
                damage: 4.0,
                blast_radius: 50.0,
            },
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Weapon::Artillery => "Artillery",
            Weapon::ClusterGrenade => "Cluster",
            Weapon::ClusterSubmunition => "Submunition",
        }
    }

    fn is_selectable(&self) -> bool {
        match self {
            Weapon::Artillery | Weapon::ClusterGrenade => true,
            Weapon::ClusterSubmunition => false,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Buildable {
    AALauncher,
}

impl Buildable {
    fn name(&self) -> &'static str {
        match self {
            Buildable::AALauncher => "AA Launcher",
        }
    }

    fn health(&self) -> f32 {
        match self {
            Buildable::AALauncher => 3.0,
        }
    }

    fn size(&self) -> f32 {
        match self {
            Buildable::AALauncher => 30.0,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum TurnPhase {
    #[default]
    Aiming,
    Building,
    ProjectileInFlight,
    TurnEnding,
    GameOver,
}

#[derive(Resource)]
struct GameState {
    current_player: Player,
    selected_weapon: Option<Weapon>,
    selected_buildable: Option<Buildable>,
    phase: TurnPhase,
    turn_end_timer: f32,
    winner: Option<Player>,
    projectiles_seen: bool, // Track if we've seen projectiles this phase (for deferred spawn handling)
}

impl Default for GameState {
    fn default() -> Self {
        Self {
            current_player: Player::Blue,
            selected_weapon: None,
            selected_buildable: None,
            phase: TurnPhase::Aiming,
            turn_end_timer: 0.0,
            winner: None,
            projectiles_seen: false,
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
struct BuildableButton {
    buildable: Buildable,
}

#[derive(Component)]
struct BuildPreview;

#[derive(Component)]
struct BuildableAreaOverlay;

#[derive(Component)]
struct AALauncher {
    player: Player,
    fired_this_turn: bool,
}

#[derive(Component)]
struct AAMissile {
    velocity: Vec2,
    target: Entity,
    distance_traveled: f32,
}

#[derive(Component)]
struct ChargeIndicatorWorld;

#[derive(Component)]
struct Projectile {
    velocity: Vec2,
    weapon: Weapon,
    prev_velocity_y: f32, // For apoapsis detection
}

#[derive(Component)]
struct Explosion {
    timer: f32,
    max_time: f32,
    max_radius: f32,
}

#[derive(Component)]
struct HealthBarBackground {
    owner: Entity,
}

const EXPLOSION_DURATION: f32 = 0.4;

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

            // Cluster grenade button
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
                        weapon: Weapon::ClusterGrenade,
                    },
                ))
                .with_child((
                    Text::new("Cluster"),
                    TextFont {
                        font_size: 16.0,
                        ..default()
                    },
                    TextColor(Color::WHITE),
                ));

            // AA Launcher button
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
                    BuildableButton {
                        buildable: Buildable::AALauncher,
                    },
                ))
                .with_child((
                    Text::new("AA"),
                    TextFont {
                        font_size: 16.0,
                        ..default()
                    },
                    TextColor(Color::WHITE),
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

    // Build preview (follows cursor when buildable is selected)
    commands.spawn((
        Sprite {
            color: Color::srgba(0.5, 0.5, 0.5, 0.5),
            custom_size: Some(Vec2::splat(30.0)),
            ..default()
        },
        Transform::from_xyz(0.0, 0.0, 3.0),
        Visibility::Hidden,
        BuildPreview,
    ));

    // Buildable area overlay (shown when buildable is selected)
    commands.spawn((
        Sprite {
            color: Color::srgba(0.0, 0.0, 0.0, 0.0), // Will be drawn via gizmos instead
            custom_size: Some(Vec2::ZERO),
            ..default()
        },
        Transform::from_xyz(0.0, 0.0, 0.5),
        Visibility::Hidden,
        BuildableAreaOverlay,
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
    mut button_query: Query<(
        &Interaction,
        &WeaponButton,
        &mut BorderColor,
        &mut BackgroundColor,
    )>,
) {
    let is_disabled = game_state.phase != TurnPhase::Aiming;

    // Handle clicks - toggle selection (only when enabled)
    if !is_disabled {
        for (interaction, weapon_button) in &interaction_query {
            if *interaction == Interaction::Pressed {
                if game_state.selected_weapon == Some(weapon_button.weapon) {
                    game_state.selected_weapon = None;
                } else {
                    game_state.selected_weapon = Some(weapon_button.weapon);
                    game_state.selected_buildable = None; // Deselect buildable
                }
            }
        }
    }

    // Update button visuals
    for (interaction, weapon_button, mut border_color, mut bg_color) in &mut button_query {
        if is_disabled {
            // Grayed out when disabled
            *border_color = BorderColor::all(Color::srgb(0.4, 0.4, 0.4));
            *bg_color = BackgroundColor(Color::srgb(0.2, 0.2, 0.2));
        } else {
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
}

fn handle_buildable_selection(
    mut game_state: ResMut<GameState>,
    interaction_query: Query<(&Interaction, &BuildableButton), Changed<Interaction>>,
    mut button_query: Query<(
        &Interaction,
        &BuildableButton,
        &mut BorderColor,
        &mut BackgroundColor,
    )>,
) {
    let is_disabled = game_state.phase != TurnPhase::Aiming;

    // Handle clicks - toggle selection (only when enabled)
    if !is_disabled {
        for (interaction, buildable_button) in &interaction_query {
            if *interaction == Interaction::Pressed {
                if game_state.selected_buildable == Some(buildable_button.buildable) {
                    game_state.selected_buildable = None;
                } else {
                    game_state.selected_buildable = Some(buildable_button.buildable);
                    game_state.selected_weapon = None; // Deselect weapon
                }
            }
        }
    }

    // Update button visuals
    for (interaction, buildable_button, mut border_color, mut bg_color) in &mut button_query {
        if is_disabled {
            // Grayed out when disabled
            *border_color = BorderColor::all(Color::srgb(0.4, 0.4, 0.4));
            *bg_color = BackgroundColor(Color::srgb(0.2, 0.2, 0.2));
        } else {
            let is_selected = game_state.selected_buildable == Some(buildable_button.buildable);
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
}

fn update_weapon_tooltip(
    weapon_buttons: Query<(&Interaction, &WeaponButton)>,
    buildable_buttons: Query<(&Interaction, &BuildableButton)>,
    mut tooltip_query: Query<(&mut Node, &Children), With<WeaponTooltip>>,
    mut text_query: Query<&mut Text>,
    windows: Query<&Window>,
) {
    let Ok((mut node, children)) = tooltip_query.single_mut() else {
        return;
    };

    // Find hovered weapon button
    let hovered_weapon = weapon_buttons
        .iter()
        .find(|(interaction, _)| **interaction == Interaction::Hovered);

    // Find hovered buildable button
    let hovered_buildable = buildable_buttons
        .iter()
        .find(|(interaction, _)| **interaction == Interaction::Hovered);

    let tooltip_text = if let Some((_, weapon_button)) = hovered_weapon {
        // Special handling for cluster grenade - show submunition stats
        if weapon_button.weapon == Weapon::ClusterGrenade {
            let sub_stats = Weapon::ClusterSubmunition.stats();
            Some(format!(
                "{}\n5x Submunitions\nDamage: {} each\nBlast Radius: {}",
                weapon_button.weapon.name(),
                sub_stats.damage,
                sub_stats.blast_radius
            ))
        } else {
            let stats = weapon_button.weapon.stats();
            Some(format!(
                "{}\nDamage: {}\nBlast Radius: {}",
                weapon_button.weapon.name(),
                stats.damage,
                stats.blast_radius
            ))
        }
    } else if let Some((_, buildable_button)) = hovered_buildable {
        match buildable_button.buildable {
            Buildable::AALauncher => Some(format!(
                "AA Launcher\nHealth: {}\nDetection Range: {}\nFires tracking missiles at\nenemy projectiles (1/turn)",
                Buildable::AALauncher.health() as i32,
                AA_DETECTION_RANGE as i32
            )),
        }
    } else {
        None
    };

    if let Some(text_content) = tooltip_text {
        // Update text in child
        if let Some(child) = children.iter().next() {
            if let Ok(mut text) = text_query.get_mut(child) {
                **text = text_content;
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
            Transform::from_xyz(start.x, start.y, 3.0),
            Projectile {
                velocity,
                weapon,
                prev_velocity_y: velocity.y,
            },
        ));

        aiming_state.charging = false;
        aiming_state.charge_time = 0.0;
        game_state.selected_weapon = None;
        game_state.phase = TurnPhase::ProjectileInFlight;
    }
}

fn handle_building(
    mut commands: Commands,
    mut gizmos: Gizmos,
    mut game_state: ResMut<GameState>,
    mouse_button: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    camera_query: Query<&Transform, With<MainCamera>>,
    terrain_data: Res<TerrainData>,
    interaction_query: Query<&Interaction, With<Button>>,
    mut preview_query: Query<
        (&mut Transform, &mut Visibility, &mut Sprite),
        (
            With<BuildPreview>,
            Without<MainCamera>,
            Without<PlayerBase>,
            Without<AALauncher>,
        ),
    >,
    player_bases: Query<
        (&Transform, &PlayerBase),
        (
            Without<BuildPreview>,
            Without<MainCamera>,
            Without<AALauncher>,
        ),
    >,
    aa_launchers: Query<
        (&Transform, &AALauncher),
        (
            Without<BuildPreview>,
            Without<MainCamera>,
            Without<PlayerBase>,
        ),
    >,
) {
    let Ok((mut preview_transform, mut preview_visibility, mut preview_sprite)) =
        preview_query.single_mut()
    else {
        return;
    };

    // Hide preview if not in aiming phase or no buildable selected
    if game_state.phase != TurnPhase::Aiming || game_state.selected_buildable.is_none() {
        *preview_visibility = Visibility::Hidden;
        return;
    }

    let buildable = game_state.selected_buildable.unwrap();

    let Ok(window) = windows.single() else {
        *preview_visibility = Visibility::Hidden;
        return;
    };
    let Ok(camera_transform) = camera_query.single() else {
        *preview_visibility = Visibility::Hidden;
        return;
    };

    let Some(cursor_position) = window.cursor_position() else {
        *preview_visibility = Visibility::Hidden;
        return;
    };

    // Convert cursor to world position
    let window_size = Vec2::new(window.width(), window.height());
    let cursor_ndc = (cursor_position - window_size / 2.0) * Vec2::new(1.0, -1.0);
    let cursor_world =
        camera_transform.translation.truncate() + cursor_ndc * camera_transform.scale.x;

    // Snap to terrain height
    let terrain_y = terrain_data
        .get_height_at(cursor_world.x)
        .unwrap_or(cursor_world.y);
    let placement_pos = Vec2::new(cursor_world.x, terrain_y + buildable.size() / 2.0);

    // Collect friendly structure positions for build radius check
    let friendly_positions: Vec<Vec2> = player_bases
        .iter()
        .filter(|(_, base)| base.player == game_state.current_player)
        .map(|(t, _)| t.translation.truncate())
        .chain(
            aa_launchers
                .iter()
                .filter(|(_, aa)| aa.player == game_state.current_player)
                .map(|(t, _)| t.translation.truncate()),
        )
        .collect();

    // Check if placement is within build radius of any friendly structure
    let is_valid_placement = friendly_positions
        .iter()
        .any(|pos| pos.distance(placement_pos) <= BUILD_RADIUS);

    // Update preview
    preview_transform.translation.x = placement_pos.x;
    preview_transform.translation.y = placement_pos.y;
    preview_sprite.custom_size = Some(Vec2::splat(buildable.size()));

    // Color based on valid/invalid placement
    if is_valid_placement {
        // Green tint for valid
        preview_sprite.color = Color::srgba(0.2, 0.8, 0.2, 0.6);
    } else {
        // Red tint for invalid
        preview_sprite.color = Color::srgba(0.8, 0.2, 0.2, 0.6);
    }
    *preview_visibility = Visibility::Visible;

    // Draw AA detection range preview
    if matches!(buildable, Buildable::AALauncher) {
        gizmos.circle_2d(
            placement_pos,
            AA_DETECTION_RANGE,
            Color::srgba(1.0, 1.0, 1.0, 0.5),
        );
    }

    // Don't place if clicking on UI
    let clicking_ui = interaction_query.iter().any(|i| *i != Interaction::None);

    // Handle placement (only if valid)
    if mouse_button.just_pressed(MouseButton::Left) && !clicking_ui && is_valid_placement {
        let health_bar_y = placement_pos.y - buildable.size() / 2.0 - 20.0;

        // Spawn the actual structure
        let aa_entity = commands
            .spawn((
                Sprite {
                    color: game_state.current_player.color(),
                    custom_size: Some(Vec2::splat(buildable.size())),
                    ..default()
                },
                Transform::from_xyz(placement_pos.x, placement_pos.y, 1.0),
                AALauncher {
                    player: game_state.current_player,
                    fired_this_turn: false,
                },
                Health::new(buildable.health()),
            ))
            .id();

        // Health bar background
        commands.spawn((
            Sprite {
                color: Color::srgba(0.0, 0.0, 0.0, 0.5),
                custom_size: Some(Vec2::new(40.0, 28.0)),
                ..default()
            },
            Transform::from_xyz(placement_pos.x, health_bar_y, 4.0),
            HealthBarBackground { owner: aa_entity },
        ));

        // Health bar text
        commands.spawn((
            Text2d::new(format!("{}", buildable.health() as i32)),
            TextFont {
                font_size: 32.0,
                ..default()
            },
            TextColor(game_state.current_player.color()),
            Transform::from_xyz(placement_pos.x, health_bar_y, 5.0),
            HealthBar { owner: aa_entity },
        ));

        // End turn
        game_state.selected_buildable = None;
        game_state.phase = TurnPhase::TurnEnding;
        game_state.turn_end_timer = TURN_END_DELAY;
        *preview_visibility = Visibility::Hidden;
    }
}

fn draw_buildable_area(
    mut gizmos: Gizmos,
    game_state: Res<GameState>,
    player_bases: Query<(&Transform, &PlayerBase), Without<AALauncher>>,
    aa_launchers: Query<(&Transform, &AALauncher), Without<PlayerBase>>,
) {
    // Only show when a buildable is selected
    if game_state.phase != TurnPhase::Aiming || game_state.selected_buildable.is_none() {
        return;
    }

    // Collect friendly structure positions
    let friendly_positions: Vec<Vec2> = player_bases
        .iter()
        .filter(|(_, base)| base.player == game_state.current_player)
        .map(|(t, _)| t.translation.truncate())
        .chain(
            aa_launchers
                .iter()
                .filter(|(_, aa)| aa.player == game_state.current_player)
                .map(|(t, _)| t.translation.truncate()),
        )
        .collect();

    // Draw filled circles for each friendly structure's build radius
    // Using multiple concentric circles to create a filled effect
    let player_color = game_state.current_player.color().to_srgba();
    let fill_color = Color::srgba(
        player_color.red,
        player_color.green,
        player_color.blue,
        0.15,
    );

    for pos in &friendly_positions {
        // Draw filled area using concentric circles
        let num_rings = 20;
        for i in 0..num_rings {
            let radius = BUILD_RADIUS * (i as f32 + 1.0) / num_rings as f32;
            gizmos.circle_2d(*pos, radius, fill_color);
        }

        // Draw outer edge
        gizmos.circle_2d(
            *pos,
            BUILD_RADIUS,
            Color::srgba(player_color.red, player_color.green, player_color.blue, 0.5),
        );
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
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    time: Res<Time>,
    mut projectiles: Query<(Entity, &mut Transform, &mut Projectile, &Sprite)>,
    mut player_bases: Query<
        (Entity, &Transform, &PlayerBase, &mut Health),
        (Without<Projectile>, Without<AALauncher>),
    >,
    mut aa_launchers: Query<
        (Entity, &Transform, &AALauncher, &mut Health),
        (Without<Projectile>, Without<PlayerBase>),
    >,
) {
    if game_state.phase != TurnPhase::ProjectileInFlight {
        return;
    }

    let dt = time.delta_secs();
    let half_width = WORLD_WIDTH / 2.0;
    let mut rng = rand::rng();

    // Collect entities to despawn and submunitions to spawn (to avoid borrow issues)
    let mut to_despawn = Vec::new();
    let mut submunitions_to_spawn = Vec::new();

    for (entity, mut transform, mut projectile, sprite) in &mut projectiles {
        let prev_vel_y = projectile.prev_velocity_y;

        // Apply gravity
        projectile.velocity.y -= GRAVITY * dt;

        // Update position
        transform.translation.x += projectile.velocity.x * dt;
        transform.translation.y += projectile.velocity.y * dt;

        let pos = transform.translation.truncate();

        // Check for apoapsis (velocity.y crosses from positive to negative)
        if projectile.weapon == Weapon::ClusterGrenade
            && prev_vel_y >= 0.0
            && projectile.velocity.y < 0.0
        {
            // Split into submunitions
            let base_velocity = projectile.velocity;
            let color = sprite.color;

            for i in 0..5 {
                // Spread angle: -20 to +20 degrees from current direction
                let angle_offset = ((i as f32 - 2.0) / 2.0) * 0.35; // ~20 degrees
                let random_offset = rng.random_range(-0.1..0.1);
                let total_offset = angle_offset + random_offset;

                let speed_variation = rng.random_range(0.8..1.2);
                let rotated_velocity = Vec2::new(
                    base_velocity.x * total_offset.cos() - base_velocity.y * total_offset.sin(),
                    base_velocity.x * total_offset.sin() + base_velocity.y * total_offset.cos(),
                ) * speed_variation;

                submunitions_to_spawn.push((pos, rotated_velocity, color));
            }

            to_despawn.push(entity);
            continue;
        }

        // Store current velocity for next frame's apoapsis check
        projectile.prev_velocity_y = projectile.velocity.y;

        // Check world bounds
        if pos.x < -half_width || pos.x > half_width {
            to_despawn.push(entity);
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
            if stats.blast_radius > 0.0 {
                terrain_data.apply_damage(pos.x, pos.y, stats.damage, stats.blast_radius);

                // Apply blast damage to all bases within blast radius
                for (_, base_transform, _, mut health) in &mut player_bases {
                    let base_pos = base_transform.translation.truncate();
                    let half_size = PLAYER_BASE_SIZE / 2.0;

                    // Calculate distance to nearest point on the base (not center)
                    let nearest_x = pos.x.clamp(base_pos.x - half_size, base_pos.x + half_size);
                    let nearest_y = pos.y.clamp(base_pos.y - half_size, base_pos.y + half_size);
                    let nearest_point = Vec2::new(nearest_x, nearest_y);
                    let distance = pos.distance(nearest_point);

                    if distance <= 0.0 {
                        // Direct hit - impact is inside the base
                        health.take_damage(stats.damage);
                    } else if distance < stats.blast_radius {
                        // Damage falls off linearly with distance from edge of base
                        let damage_factor = 1.0 - (distance / stats.blast_radius);
                        let damage = stats.damage * damage_factor;
                        health.take_damage(damage);
                    }
                }

                // Apply blast damage to all AA launchers within blast radius
                for (_, aa_transform, _, mut health) in &mut aa_launchers {
                    let aa_pos = aa_transform.translation.truncate();
                    let half_size = Buildable::AALauncher.size() / 2.0;

                    // Calculate distance to nearest point on the AA launcher
                    let nearest_x = pos.x.clamp(aa_pos.x - half_size, aa_pos.x + half_size);
                    let nearest_y = pos.y.clamp(aa_pos.y - half_size, aa_pos.y + half_size);
                    let nearest_point = Vec2::new(nearest_x, nearest_y);
                    let distance = pos.distance(nearest_point);

                    if distance <= 0.0 {
                        health.take_damage(stats.damage);
                    } else if distance < stats.blast_radius {
                        let damage_factor = 1.0 - (distance / stats.blast_radius);
                        let damage = stats.damage * damage_factor;
                        health.take_damage(damage);
                    }
                }

                // Spawn explosion
                commands.spawn((
                    Mesh2d(meshes.add(Circle::new(1.0))),
                    MeshMaterial2d(
                        materials.add(ColorMaterial::from_color(Color::srgba(1.0, 0.6, 0.0, 1.0))),
                    ),
                    Transform::from_xyz(pos.x, pos.y, 2.0), // Behind health bars
                    Explosion {
                        timer: 0.0,
                        max_time: EXPLOSION_DURATION,
                        max_radius: stats.blast_radius,
                    },
                ));
            }

            to_despawn.push(entity);
        }
    }

    // Calculate final projectile count before consuming vectors
    let remaining_projectiles = projectiles.iter().count();
    let despawn_count = to_despawn.len();
    let spawn_count = submunitions_to_spawn.len();

    // Despawn projectiles
    for entity in to_despawn {
        commands.entity(entity).despawn();
    }

    // Spawn submunitions
    for (pos, velocity, color) in submunitions_to_spawn {
        commands.spawn((
            Sprite {
                color,
                custom_size: Some(Vec2::splat(PROJECTILE_RADIUS * 1.5)), // Slightly smaller
                ..default()
            },
            Transform::from_xyz(pos.x, pos.y, 3.0),
            Projectile {
                velocity,
                weapon: Weapon::ClusterSubmunition,
                prev_velocity_y: velocity.y,
            },
        ));
    }

    // Only end turn when no projectiles remain (but not if we never had any -
    // commands are deferred so projectile might not exist on spawn frame)
    let final_count = remaining_projectiles - despawn_count + spawn_count;
    if final_count == 0 && remaining_projectiles > 0 {
        // Note: AA missiles are checked separately in check_projectile_phase_end
    }
}

fn check_projectile_phase_end(
    mut game_state: ResMut<GameState>,
    projectiles: Query<Entity, With<Projectile>>,
    aa_missiles: Query<Entity, With<AAMissile>>,
) {
    if game_state.phase != TurnPhase::ProjectileInFlight {
        return;
    }

    // Track if we've seen any projectiles (handles deferred spawn)
    if !projectiles.is_empty() {
        game_state.projectiles_seen = true;
    }

    // End turn when both projectiles and AA missiles are gone
    // but only if we've actually seen projectiles (to handle deferred spawn)
    if projectiles.is_empty() && aa_missiles.is_empty() && game_state.projectiles_seen {
        game_state.phase = TurnPhase::TurnEnding;
        game_state.turn_end_timer = TURN_END_DELAY;
        game_state.projectiles_seen = false;
    }
}

fn update_explosions(
    mut commands: Commands,
    time: Res<Time>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut explosions: Query<(
        Entity,
        &mut Explosion,
        &mut Transform,
        &MeshMaterial2d<ColorMaterial>,
    )>,
) {
    for (entity, mut explosion, mut transform, material_handle) in &mut explosions {
        explosion.timer += time.delta_secs();

        let progress = explosion.timer / explosion.max_time;

        if progress >= 1.0 {
            commands.entity(entity).despawn();
            continue;
        }

        // Expand quickly at first, then slow down
        let size_progress = 1.0 - (1.0 - progress).powi(2);
        // Circle mesh has radius 1.0, so scale equals final radius
        let scale = explosion.max_radius * size_progress;
        transform.scale = Vec3::splat(scale.max(0.1));

        // Fade from orange to red to transparent
        if let Some(material) = materials.get_mut(&material_handle.0) {
            let alpha = 1.0 - progress;
            let green = 0.6 * (1.0 - progress);
            material.color = Color::srgba(1.0, green, 0.0, alpha);
        }
    }
}

fn aa_fire_missiles(
    mut commands: Commands,
    game_state: Res<GameState>,
    mut aa_launchers: Query<(&Transform, &mut AALauncher)>,
    projectiles: Query<(Entity, &Transform), With<Projectile>>,
) {
    // Only fire during projectile flight phase
    if game_state.phase != TurnPhase::ProjectileInFlight {
        return;
    }

    for (aa_transform, mut aa_launcher) in &mut aa_launchers {
        // Only fire at enemy projectiles (AA belongs to player who isn't current)
        if aa_launcher.player == game_state.current_player {
            continue;
        }

        // Only fire once per turn
        if aa_launcher.fired_this_turn {
            continue;
        }

        let aa_pos = aa_transform.translation.truncate();

        // Find closest enemy projectile in range
        let mut closest: Option<(Entity, f32)> = None;
        for (proj_entity, proj_transform) in &projectiles {
            let proj_pos = proj_transform.translation.truncate();
            let distance = aa_pos.distance(proj_pos);

            if distance <= AA_DETECTION_RANGE {
                if closest.is_none() || distance < closest.unwrap().1 {
                    closest = Some((proj_entity, distance));
                }
            }
        }

        // Fire at closest target
        if let Some((target_entity, _)) = closest {
            let target_pos = projectiles
                .get(target_entity)
                .map(|(_, t)| t.translation.truncate())
                .unwrap();

            // Initial velocity pointing toward target
            let direction = (target_pos - aa_pos).normalize_or_zero();
            let initial_speed = 100.0;

            commands.spawn((
                Sprite {
                    color: aa_launcher.player.color(),
                    custom_size: Some(Vec2::new(AA_MISSILE_SIZE, AA_MISSILE_SIZE * 2.0)),
                    ..default()
                },
                Transform::from_xyz(aa_pos.x, aa_pos.y + Buildable::AALauncher.size() / 2.0, 3.0)
                    .with_rotation(Quat::from_rotation_z(
                        direction.y.atan2(direction.x) - std::f32::consts::FRAC_PI_2,
                    )),
                AAMissile {
                    velocity: direction * initial_speed,
                    target: target_entity,
                    distance_traveled: 0.0,
                },
            ));

            aa_launcher.fired_this_turn = true;
        }
    }
}

fn update_aa_missiles(
    mut commands: Commands,
    game_state: Res<GameState>,
    time: Res<Time>,
    mut missiles: Query<(Entity, &mut Transform, &mut AAMissile, &mut Sprite)>,
    projectiles: Query<&Transform, (With<Projectile>, Without<AAMissile>)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    if game_state.phase != TurnPhase::ProjectileInFlight {
        return;
    }

    let dt = time.delta_secs();

    for (entity, mut transform, mut missile, _sprite) in &mut missiles {
        // Check if target still exists
        let target_pos = if let Ok(target_transform) = projectiles.get(missile.target) {
            target_transform.translation.truncate()
        } else {
            // Target destroyed, despawn missile
            commands.entity(entity).despawn();
            continue;
        };

        let pos = transform.translation.truncate();

        // Calculate desired direction to target
        let to_target = target_pos - pos;
        let desired_direction = to_target.normalize_or_zero();

        // Current direction from velocity
        let current_speed = missile.velocity.length();
        let current_direction = if current_speed > 0.1 {
            missile.velocity / current_speed
        } else {
            desired_direction
        };

        // Calculate angle difference and apply turn rate limit
        let current_angle = current_direction.y.atan2(current_direction.x);
        let desired_angle = desired_direction.y.atan2(desired_direction.x);
        let mut angle_diff = desired_angle - current_angle;

        // Normalize angle difference to [-PI, PI]
        while angle_diff > std::f32::consts::PI {
            angle_diff -= 2.0 * std::f32::consts::PI;
        }
        while angle_diff < -std::f32::consts::PI {
            angle_diff += 2.0 * std::f32::consts::PI;
        }

        // Apply turn rate limit
        let max_turn = AA_MISSILE_TURN_RATE * dt;
        let actual_turn = angle_diff.clamp(-max_turn, max_turn);
        let new_angle = current_angle + actual_turn;

        let new_direction = Vec2::new(new_angle.cos(), new_angle.sin());

        // Accelerate
        let new_speed = (current_speed + AA_MISSILE_ACCELERATION * dt).min(AA_MISSILE_MAX_SPEED);
        missile.velocity = new_direction * new_speed;

        // Update position
        let movement = missile.velocity * dt;
        transform.translation.x += movement.x;
        transform.translation.y += movement.y;
        missile.distance_traveled += movement.length();

        // Update rotation to face direction of travel
        transform.rotation = Quat::from_rotation_z(new_angle - std::f32::consts::FRAC_PI_2);

        // Check if exceeded max range
        if missile.distance_traveled > AA_MISSILE_MAX_RANGE {
            // Explode harmlessly
            commands.spawn((
                Mesh2d(meshes.add(Circle::new(1.0))),
                MeshMaterial2d(
                    materials.add(ColorMaterial::from_color(Color::srgba(0.8, 0.8, 0.2, 1.0))),
                ),
                Transform::from_xyz(transform.translation.x, transform.translation.y, 2.0),
                Explosion {
                    timer: 0.0,
                    max_time: EXPLOSION_DURATION * 0.5,
                    max_radius: AA_MISSILE_EXPLOSION_RADIUS * 0.5,
                },
            ));
            commands.entity(entity).despawn();
            continue;
        }

        // Check collision with target
        let distance_to_target = to_target.length();
        if distance_to_target < AA_MISSILE_EXPLOSION_RADIUS {
            // Hit! Spawn explosion and destroy both missile and projectile
            commands.spawn((
                Mesh2d(meshes.add(Circle::new(1.0))),
                MeshMaterial2d(
                    materials.add(ColorMaterial::from_color(Color::srgba(0.8, 0.8, 0.2, 1.0))),
                ),
                Transform::from_xyz(transform.translation.x, transform.translation.y, 2.0),
                Explosion {
                    timer: 0.0,
                    max_time: EXPLOSION_DURATION,
                    max_radius: AA_MISSILE_EXPLOSION_RADIUS,
                },
            ));

            // Destroy the projectile
            commands.entity(missile.target).despawn();
            commands.entity(entity).despawn();
        }
    }
}

fn reset_aa_launchers(game_state: Res<GameState>, mut aa_launchers: Query<&mut AALauncher>) {
    // Reset fired_this_turn when turn ends
    if game_state.phase != TurnPhase::Aiming {
        return;
    }

    for mut aa_launcher in &mut aa_launchers {
        aa_launcher.fired_this_turn = false;
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
    bases: Query<(Entity, &Transform, &Health), (With<PlayerBase>, Without<AALauncher>)>,
    aa_launchers: Query<(Entity, &Transform, &Health), (With<AALauncher>, Without<PlayerBase>)>,
    mut health_bars: Query<
        (&mut Text2d, &mut Transform, &HealthBar),
        (
            Without<PlayerBase>,
            Without<HealthBarBackground>,
            Without<AALauncher>,
        ),
    >,
    mut health_bar_backgrounds: Query<
        (&mut Transform, &HealthBarBackground),
        (Without<PlayerBase>, Without<HealthBar>, Without<AALauncher>),
    >,
) {
    for (mut text, mut bar_transform, health_bar) in &mut health_bars {
        // Find the owner (base or AA launcher)
        if let Some((_, owner_transform, health, size)) = bases
            .iter()
            .find(|(e, _, _)| *e == health_bar.owner)
            .map(|(e, t, h)| (e, t, h, PLAYER_BASE_SIZE))
            .or_else(|| {
                aa_launchers
                    .iter()
                    .find(|(e, _, _)| *e == health_bar.owner)
                    .map(|(e, t, h)| (e, t, h, Buildable::AALauncher.size()))
            })
        {
            // Update text
            **text = format!("{}", health.current.ceil() as i32);

            // Position below the structure
            let health_bar_y = owner_transform.translation.y - size / 2.0 - 20.0;
            bar_transform.translation.x = owner_transform.translation.x;
            bar_transform.translation.y = health_bar_y;
        }
    }

    // Update background positions
    for (mut bg_transform, bg) in &mut health_bar_backgrounds {
        if let Some((_, owner_transform, size)) = bases
            .iter()
            .find(|(e, _, _)| *e == bg.owner)
            .map(|(_, t, _)| ((), t, PLAYER_BASE_SIZE))
            .or_else(|| {
                aa_launchers
                    .iter()
                    .find(|(e, _, _)| *e == bg.owner)
                    .map(|(_, t, _)| ((), t, Buildable::AALauncher.size()))
            })
        {
            let health_bar_y = owner_transform.translation.y - size / 2.0 - 20.0;
            bg_transform.translation.x = owner_transform.translation.x;
            bg_transform.translation.y = health_bar_y;
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

fn check_aa_destruction(
    mut commands: Commands,
    aa_launchers: Query<(Entity, &Health), With<AALauncher>>,
    health_bars: Query<(Entity, &HealthBar)>,
    health_bar_backgrounds: Query<(Entity, &HealthBarBackground)>,
) {
    for (entity, health) in &aa_launchers {
        if health.is_dead() {
            // Despawn the AA launcher
            commands.entity(entity).despawn();

            // Despawn associated health bar
            for (bar_entity, bar) in &health_bars {
                if bar.owner == entity {
                    commands.entity(bar_entity).despawn();
                }
            }

            // Despawn associated health bar background
            for (bg_entity, bg) in &health_bar_backgrounds {
                if bg.owner == entity {
                    commands.entity(bg_entity).despawn();
                }
            }
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
    health_bar_bg_query: Query<Entity, With<HealthBarBackground>>,
    explosion_query: Query<Entity, With<Explosion>>,
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
            for entity in health_bar_bg_query.iter() {
                commands.entity(entity).despawn();
            }
            for entity in explosion_query.iter() {
                commands.entity(entity).despawn();
            }

            // Generate new world
            spawn_world(
                &mut commands,
                &mut meshes,
                &mut materials,
                &mut terrain_data,
            );
        }
    }
}

fn generate_terrain(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut terrain_data: ResMut<TerrainData>,
) {
    spawn_world(
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut terrain_data,
    );
}

fn spawn_world(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<ColorMaterial>>,
    terrain_data: &mut ResMut<TerrainData>,
) {
    let heights = generate_terrain_heights();
    terrain_data.heights = heights.clone();

    spawn_terrain_mesh(commands, meshes, materials, &heights);
    spawn_player_bases(commands, meshes, materials, &heights);
}

fn generate_terrain_heights() -> Vec<f32> {
    let mut rng = rand::rng();
    let mut heights = vec![0.0f32; TERRAIN_SEGMENTS + 1];
    heights[0] = rng.random_range(600.0..900.0);
    heights[TERRAIN_SEGMENTS] = rng.random_range(600.0..900.0);
    midpoint_displacement(&mut heights, 0, TERRAIN_SEGMENTS, 400.0, &mut rng);
    heights
}

fn spawn_terrain_mesh(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<ColorMaterial>>,
    heights: &[f32],
) {
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
}

fn spawn_player_bases(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<ColorMaterial>>,
    heights: &[f32],
) {
    let segment_width = WORLD_WIDTH / TERRAIN_SEGMENTS as f32;
    let half_width = WORLD_WIDTH / 2.0;
    let half_height = WORLD_HEIGHT / 2.0;

    for (player, segment_percent) in [(Player::Blue, 15), (Player::Red, 85)] {
        let segment = TERRAIN_SEGMENTS * segment_percent / 100;
        let x = segment as f32 * segment_width - half_width;
        let y = heights[segment] - half_height + PLAYER_BASE_SIZE / 2.0;
        let health_bar_y = y - PLAYER_BASE_SIZE / 2.0 - 25.0;

        let base_entity = commands
            .spawn((
                Sprite {
                    color: player.color(),
                    custom_size: Some(Vec2::splat(PLAYER_BASE_SIZE)),
                    ..default()
                },
                Transform::from_xyz(x, y, 1.0),
                PlayerBase { player },
                Health::new(PLAYER_BASE_HEALTH),
            ))
            .id();

        // Health bar background
        commands.spawn((
            Mesh2d(meshes.add(Rectangle::new(60.0, 40.0))),
            MeshMaterial2d(
                materials.add(ColorMaterial::from_color(Color::srgba(0.0, 0.0, 0.0, 0.5))),
            ),
            Transform::from_xyz(x, health_bar_y, 4.0),
            HealthBarBackground { owner: base_entity },
        ));

        // Health bar text
        commands.spawn((
            Text2d::new(format!("{}", PLAYER_BASE_HEALTH as i32)),
            TextFont {
                font_size: 48.0,
                ..default()
            },
            TextColor(player.color()),
            Transform::from_xyz(x, health_bar_y, 5.0),
            HealthBar { owner: base_entity },
        ));
    }
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
