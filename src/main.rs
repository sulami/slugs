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
                process_pending_burst_shots,
                aa_fire_missiles,
                update_aa_missiles,
                check_projectile_phase_end,
                process_pending_explosions,
                process_pending_emps,
                update_explosions,
                update_particles,
                rebuild_terrain_mesh,
                update_falling_entities,
                update_health_bars,
                check_base_destruction,
                check_structure_destruction,
                reset_aa_launchers,
                decrement_aa_disabled,
                update_shield_domes.after(decrement_aa_disabled),
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
    EMP,
    Burst,
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
            Weapon::EMP => WeaponStats {
                damage: 0.0,
                blast_radius: 300.0,
            },
            Weapon::Burst => WeaponStats {
                damage: 6.0,
                blast_radius: 60.0,
            },
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Weapon::Artillery => "Artillery",
            Weapon::ClusterGrenade => "Cluster",
            Weapon::ClusterSubmunition => "Submunition",
            Weapon::EMP => "EMP",
            Weapon::Burst => "Burst",
        }
    }

    fn is_selectable(&self) -> bool {
        match self {
            Weapon::Artillery | Weapon::ClusterGrenade | Weapon::EMP | Weapon::Burst => true,
            Weapon::ClusterSubmunition => false,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Buildable {
    AALauncher,
    Wall,
    ShieldGenerator,
}

impl Buildable {
    fn name(&self) -> &'static str {
        match self {
            Buildable::AALauncher => "AA Launcher",
            Buildable::Wall => "Wall",
            Buildable::ShieldGenerator => "Shield Generator",
        }
    }

    fn health(&self) -> f32 {
        match self {
            Buildable::AALauncher => 3.0,
            Buildable::Wall => 8.0,
            Buildable::ShieldGenerator => 4.0,
        }
    }

    fn size(&self) -> f32 {
        match self {
            Buildable::AALauncher => 30.0,
            Buildable::Wall => 40.0,
            Buildable::ShieldGenerator => 30.0,
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
    turn_start_processed: bool, // Track if turn-start logic has run for current turn
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
            turn_start_processed: true, // First turn starts already processed
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
struct ShieldPreview;

#[derive(Component)]
struct BuildableAreaOverlay;

#[derive(Component)]
struct AALauncher {
    player: Player,
    fired_this_turn: bool,
    disabled_turns: u32,
}

/// Marker for entities that should fall due to gravity and rest on terrain
#[derive(Component)]
struct FallsWithGravity {
    size: f32, // Used to calculate bottom of entity
}

#[derive(Component)]
struct Wall;

const SHIELD_RADIUS: f32 = 200.0;
const SHIELD_MAX_HEALTH: f32 = 10.0;
const SHIELD_RECHARGE_PER_TURN: f32 = 3.0;
const SHIELD_MIN_THICKNESS: f32 = 2.0;
const SHIELD_MAX_THICKNESS: f32 = 10.0;
// Shield arc coverage: 135° total (from horizontal front to 45° past vertical on back)
// Half angle is 67.5°, so shield extends 67.5° above and below the facing direction
const SHIELD_ARC_HALF_ANGLE: f32 = std::f32::consts::FRAC_PI_4 * 1.5; // 67.5 degrees

#[derive(Component)]
struct ShieldGenerator {
    player: Player,
    shield_health: f32,
    disabled_turns: u32,
}

/// Creates a shield arc mesh with the given thickness, facing direction based on player
/// Shield starts at horizontal (front) and extends 135° toward the back (upward)
fn create_shield_mesh(thickness: f32, player: Player) -> Mesh {
    let segments = 32;
    let inner_radius = SHIELD_RADIUS - thickness;
    let outer_radius = SHIELD_RADIUS;

    // Blue faces right: front is 0° (right), arc goes from 0° to 135° (toward top-left)
    // Red faces left: front is 180° (left), arc goes from 180° down to 45° (toward top-right)
    let (start_angle, end_angle) = match player {
        Player::Blue => (0.0, SHIELD_ARC_HALF_ANGLE * 2.0), // 0° to 135°
        Player::Red => (
            std::f32::consts::PI - SHIELD_ARC_HALF_ANGLE * 2.0,
            std::f32::consts::PI,
        ), // 45° to 180°
    };
    let angle_range = end_angle - start_angle;

    let mut vertices = Vec::new();
    for i in 0..=segments {
        let angle = start_angle + angle_range * (i as f32 / segments as f32);
        let cos_a = angle.cos();
        let sin_a = angle.sin();
        vertices.push([cos_a * inner_radius, sin_a * inner_radius, 0.0]);
        vertices.push([cos_a * outer_radius, sin_a * outer_radius, 0.0]);
    }

    let mut indices = Vec::new();
    for i in 0..segments {
        let base = (i * 2) as u32;
        indices.push(base);
        indices.push(base + 1);
        indices.push(base + 2);
        indices.push(base + 1);
        indices.push(base + 3);
        indices.push(base + 2);
    }

    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vertices);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// Checks if a point is within the shield arc for a given player
/// Shield starts at horizontal (front) and extends 135° toward the back (upward)
fn point_in_shield(rel_pos: Vec2, player: Player) -> bool {
    let distance = rel_pos.length();
    if distance > SHIELD_RADIUS {
        return false;
    }

    // Calculate angle of the point (atan2 gives angle from -PI to PI)
    let angle = rel_pos.y.atan2(rel_pos.x);

    // Blue faces right: arc from 0° to 135°
    // Red faces left: arc from 45° to 180°
    let (min_angle, max_angle) = match player {
        Player::Blue => (0.0, SHIELD_ARC_HALF_ANGLE * 2.0),
        Player::Red => (
            std::f32::consts::PI - SHIELD_ARC_HALF_ANGLE * 2.0,
            std::f32::consts::PI,
        ),
    };

    angle >= min_angle && angle <= max_angle
}

/// Visual entity for the shield dome
#[derive(Component)]
struct ShieldDome {
    owner: Entity,
    last_health: f32, // Track health to rebuild mesh when thickness changes
}

/// Marker for structures that extend the buildable area for a player
#[derive(Component)]
struct ExtendsBuildArea {
    player: Player,
}

#[derive(Component)]
struct AAMissile {
    velocity: Vec2,
    distance_traveled: f32,
}

#[derive(Component)]
struct PendingEMP {
    blast_radius: f32,
}

#[derive(Component)]
struct ChargeIndicatorWorld;

#[derive(Component)]
struct Projectile {
    velocity: Vec2,
    weapon: Weapon,
    prev_velocity_y: f32, // For apoapsis detection
    player: Player,
}

#[derive(Component)]
struct Explosion {
    timer: f32,
    max_time: f32,
    max_radius: f32,
    is_emp: bool,
}

/// Simple particle for visual effects
#[derive(Component)]
struct Particle {
    velocity: Vec2,
    lifetime: f32,
    max_lifetime: f32,
    gravity: bool,
    fade: bool,
}

/// Marker for pending explosions that need to apply damage
#[derive(Component)]
struct PendingExplosion {
    damage: f32,
    blast_radius: f32,
}

/// Pending burst shots that fire with a delay
#[derive(Component)]
struct PendingBurstShot {
    delay: f32,
    velocity: Vec2,
    start_pos: Vec2,
    player: Player,
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

fn setup_ui(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
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

            // EMP button
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
                        weapon: Weapon::EMP,
                    },
                ))
                .with_child((
                    Text::new("EMP"),
                    TextFont {
                        font_size: 16.0,
                        ..default()
                    },
                    TextColor(Color::WHITE),
                ));

            // Burst button
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
                        weapon: Weapon::Burst,
                    },
                ))
                .with_child((
                    Text::new("Burst"),
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

            // Wall button
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
                        buildable: Buildable::Wall,
                    },
                ))
                .with_child((
                    Text::new("Wall"),
                    TextFont {
                        font_size: 16.0,
                        ..default()
                    },
                    TextColor(Color::WHITE),
                ));

            // Shield button
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
                        buildable: Buildable::ShieldGenerator,
                    },
                ))
                .with_child((
                    Text::new("Shield"),
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

    // Shield preview arc (shown when placing shield generator)
    {
        // Use Blue as default, will be updated when shown
        let mesh = create_shield_mesh(SHIELD_MAX_THICKNESS, Player::Blue);
        commands.spawn((
            Mesh2d(meshes.add(mesh)),
            MeshMaterial2d(
                materials.add(ColorMaterial::from_color(Color::srgba(0.5, 0.5, 0.5, 0.4))),
            ),
            Transform::from_xyz(0.0, 0.0, 2.9),
            Visibility::Hidden,
            ShieldPreview,
        ));
    }

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
        } else if weapon_button.weapon == Weapon::EMP {
            let stats = weapon_button.weapon.stats();
            Some(format!(
                "{}\nDisables AA for {} turns\nBlast Radius: {}",
                weapon_button.weapon.name(),
                EMP_DISABLE_TURNS,
                stats.blast_radius
            ))
        } else if weapon_button.weapon == Weapon::Burst {
            let stats = weapon_button.weapon.stats();
            Some(format!(
                "{}\n3x Projectiles\nDamage: {} each\nBlast Radius: {}",
                weapon_button.weapon.name(),
                stats.damage,
                stats.blast_radius
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
            Buildable::Wall => Some(format!(
                "Wall\nHealth: {}\nDefensive barrier that\nblocks projectiles",
                Buildable::Wall.health() as i32
            )),
            Buildable::ShieldGenerator => Some(format!(
                "Shield Generator\nHealth: {}\nShield: {} (recharges {}/ turn)\nRadius: {}\nProjects protective dome\nthat absorbs damage",
                Buildable::ShieldGenerator.health() as i32,
                SHIELD_MAX_HEALTH as i32,
                SHIELD_RECHARGE_PER_TURN as i32,
                SHIELD_RADIUS as i32
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

    // Fire on mouse release OR when charge reaches 100%
    let should_fire = aiming_state.charging
        && (mouse_button.just_released(MouseButton::Left)
            || aiming_state.charge_time >= MAX_CHARGE_TIME);

    if should_fire {
        // Fire projectile
        let speed = (aiming_state.charge_time / MAX_CHARGE_TIME) * MAX_LAUNCH_SPEED;
        let velocity = aim_direction * speed;
        let weapon = game_state.selected_weapon.unwrap();

        let start = base_pos + Vec2::Y * (PLAYER_BASE_SIZE / 2.0);

        if weapon == Weapon::Burst {
            // Burst fires 3 projectiles sequentially with small random spread
            let mut rng = rand::rng();
            for i in 0..3 {
                let angle_offset: f32 = rng.random_range(-0.02..0.02); // Small random spread
                let rotated_velocity = Vec2::new(
                    velocity.x * angle_offset.cos() - velocity.y * angle_offset.sin(),
                    velocity.x * angle_offset.sin() + velocity.y * angle_offset.cos(),
                );

                if i == 0 {
                    // First shot fires immediately
                    commands.spawn((
                        Sprite {
                            color: game_state.current_player.color(),
                            custom_size: Some(Vec2::splat(PROJECTILE_RADIUS * 2.0)),
                            ..default()
                        },
                        Transform::from_xyz(start.x, start.y, 3.0),
                        Projectile {
                            velocity: rotated_velocity,
                            weapon,
                            prev_velocity_y: rotated_velocity.y,
                            player: game_state.current_player,
                        },
                    ));
                } else {
                    // Subsequent shots are delayed
                    commands.spawn(PendingBurstShot {
                        delay: i as f32 * 0.3, // 300ms between shots
                        velocity: rotated_velocity,
                        start_pos: start,
                        player: game_state.current_player,
                    });
                }
            }
        } else {
            // Normal single projectile
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
                    player: game_state.current_player,
                },
            ));
        }

        // Muzzle flash particles
        spawn_particles(
            &mut commands,
            start,
            12,
            Color::srgb(1.0, 0.8, 0.3),
            (50.0, 150.0),
            0.15,
            4.0,
            false,
            true,
            Some(aim_direction),
            0.5,
        );

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
    interaction_query: Query<&Interaction, With<Button>>,
    terrain_data: Res<TerrainData>,
    mut preview_query: Query<
        (&mut Transform, &mut Visibility, &mut Sprite),
        (
            With<BuildPreview>,
            Without<MainCamera>,
            Without<PlayerBase>,
            Without<AALauncher>,
            Without<ShieldPreview>,
            Without<ExtendsBuildArea>,
            Without<FallsWithGravity>,
        ),
    >,
    mut shield_preview_query: Query<
        (&mut Transform, &mut Visibility, &mut Mesh2d),
        (
            With<ShieldPreview>,
            Without<BuildPreview>,
            Without<MainCamera>,
            Without<PlayerBase>,
            Without<AALauncher>,
            Without<ExtendsBuildArea>,
            Without<FallsWithGravity>,
        ),
    >,
    mut meshes: ResMut<Assets<Mesh>>,
    build_extenders: Query<
        (&Transform, &ExtendsBuildArea),
        (Without<BuildPreview>, Without<ShieldPreview>),
    >,
    structures: Query<
        (&Transform, &Sprite),
        (
            With<FallsWithGravity>,
            Without<BuildPreview>,
            Without<ShieldPreview>,
        ),
    >,
) {
    let Ok((mut preview_transform, mut preview_visibility, mut preview_sprite)) =
        preview_query.single_mut()
    else {
        return;
    };

    let Ok((mut shield_preview_transform, mut shield_preview_visibility, mut shield_preview_mesh)) =
        shield_preview_query.single_mut()
    else {
        return;
    };

    // Hide preview if not in aiming phase or no buildable selected
    if game_state.phase != TurnPhase::Aiming || game_state.selected_buildable.is_none() {
        *shield_preview_visibility = Visibility::Hidden;
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

    // Free placement at cursor position - gravity will handle falling
    let placement_pos = cursor_world;
    let preview_size = match buildable {
        Buildable::Wall => Vec2::new(buildable.size(), buildable.size() * 1.5),
        _ => Vec2::splat(buildable.size()),
    };

    // Collect friendly structure positions for build radius check
    let friendly_positions: Vec<Vec2> = build_extenders
        .iter()
        .filter(|(_, ext)| ext.player == game_state.current_player)
        .map(|(t, _)| t.translation.truncate())
        .collect();

    // Check if placement is within build radius of any friendly structure
    let in_build_radius = friendly_positions
        .iter()
        .any(|pos| pos.distance(placement_pos) <= BUILD_RADIUS);

    // Check for overlap with existing structures
    let overlaps_structure = structures.iter().any(|(t, s)| {
        let struct_pos = t.translation.truncate();
        let struct_size = s.custom_size.unwrap_or(Vec2::splat(30.0));

        // AABB collision check
        let half_preview = preview_size / 2.0;
        let half_struct = struct_size / 2.0;

        (placement_pos.x - struct_pos.x).abs() < (half_preview.x + half_struct.x)
            && (placement_pos.y - struct_pos.y).abs() < (half_preview.y + half_struct.y)
    });

    // Check if placement is in the ground
    let terrain_height = terrain_data.get_height_at(placement_pos.x).unwrap_or(0.0);
    let bottom_of_preview = placement_pos.y - preview_size.y / 2.0;
    let in_ground = bottom_of_preview < terrain_height;

    let is_valid_placement = in_build_radius && !overlaps_structure && !in_ground;

    // Update preview
    preview_transform.translation.x = placement_pos.x;
    preview_transform.translation.y = placement_pos.y;
    preview_sprite.custom_size = Some(preview_size);

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

    // Show shield dome preview when placing shield generator
    if matches!(buildable, Buildable::ShieldGenerator) {
        shield_preview_transform.translation.x = placement_pos.x;
        shield_preview_transform.translation.y = placement_pos.y;
        // Update mesh to face correct direction for current player
        let new_mesh = create_shield_mesh(SHIELD_MAX_THICKNESS, game_state.current_player);
        shield_preview_mesh.0 = meshes.add(new_mesh);
        *shield_preview_visibility = Visibility::Visible;
    } else {
        *shield_preview_visibility = Visibility::Hidden;
    }

    // Don't place if clicking on UI
    let clicking_ui = interaction_query.iter().any(|i| *i != Interaction::None);

    // Handle placement (only if valid)
    if mouse_button.just_pressed(MouseButton::Left) && !clicking_ui && is_valid_placement {
        let health_bar_y = placement_pos.y - buildable.size() / 2.0 - 20.0;

        // Spawn the structure based on type
        let structure_entity = match buildable {
            Buildable::AALauncher => commands
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
                        disabled_turns: 0,
                    },
                    Health::new(buildable.health()),
                    FallsWithGravity {
                        size: buildable.size(),
                    },
                    ExtendsBuildArea {
                        player: game_state.current_player,
                    },
                ))
                .id(),
            Buildable::Wall => commands
                .spawn((
                    Sprite {
                        color: game_state.current_player.color(),
                        custom_size: Some(Vec2::new(buildable.size(), buildable.size() * 1.5)),
                        ..default()
                    },
                    Transform::from_xyz(placement_pos.x, placement_pos.y, 1.0),
                    Wall,
                    Health::new(buildable.health()),
                    FallsWithGravity {
                        size: buildable.size() * 1.5,
                    },
                    ExtendsBuildArea {
                        player: game_state.current_player,
                    },
                ))
                .id(),
            Buildable::ShieldGenerator => {
                let generator_entity = commands
                    .spawn((
                        Sprite {
                            color: game_state.current_player.color(),
                            custom_size: Some(Vec2::splat(buildable.size())),
                            ..default()
                        },
                        Transform::from_xyz(placement_pos.x, placement_pos.y, 1.0),
                        ShieldGenerator {
                            player: game_state.current_player,
                            shield_health: SHIELD_MAX_HEALTH,
                            disabled_turns: 0,
                        },
                        Health::new(buildable.health()),
                        FallsWithGravity {
                            size: buildable.size(),
                        },
                        ExtendsBuildArea {
                            player: game_state.current_player,
                        },
                    ))
                    .id();
                generator_entity
            }
        };

        // Health bar background
        commands.spawn((
            Sprite {
                color: Color::srgba(0.0, 0.0, 0.0, 0.5),
                custom_size: Some(Vec2::new(40.0, 28.0)),
                ..default()
            },
            Transform::from_xyz(placement_pos.x, health_bar_y, 4.0),
            HealthBarBackground {
                owner: structure_entity,
            },
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
            HealthBar {
                owner: structure_entity,
            },
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
    build_extenders: Query<(&Transform, &ExtendsBuildArea)>,
) {
    // Only show when a buildable is selected
    if game_state.phase != TurnPhase::Aiming || game_state.selected_buildable.is_none() {
        return;
    }

    // Collect friendly structure positions
    let friendly_positions: Vec<Vec2> = build_extenders
        .iter()
        .filter(|(_, ext)| ext.player == game_state.current_player)
        .map(|(t, _)| t.translation.truncate())
        .collect();

    // Draw subtle outer ring for each friendly structure's build radius
    let player_color = game_state.current_player.color().to_srgba();

    for pos in &friendly_positions {
        gizmos.circle_2d(
            *pos,
            BUILD_RADIUS,
            Color::srgba(player_color.red, player_color.green, player_color.blue, 0.3),
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
    terrain_data: Res<TerrainData>,
    time: Res<Time>,
    mut projectiles: Query<(Entity, &mut Transform, &mut Projectile, &Sprite)>,
    player_bases: Query<
        &Transform,
        (
            With<PlayerBase>,
            Without<Projectile>,
            Without<Wall>,
            Without<ShieldGenerator>,
        ),
    >,
    walls: Query<
        (&Transform, &Sprite),
        (
            With<Wall>,
            Without<Projectile>,
            Without<PlayerBase>,
            Without<ShieldGenerator>,
        ),
    >,
    mut shield_generators: Query<(Entity, &Transform, &mut ShieldGenerator), Without<Projectile>>,
) {
    if game_state.phase != TurnPhase::ProjectileInFlight {
        return;
    }

    let dt = time.delta_secs();
    let half_width = WORLD_WIDTH / 2.0;
    let mut rng = rand::rng();

    // Collect entities to despawn and submunitions to spawn (to avoid borrow issues)
    let mut to_despawn = Vec::new();
    let mut submunitions_to_spawn: Vec<(Vec2, Vec2, Color, Player)> = Vec::new();
    let mut explosions_to_spawn = Vec::new();
    let mut emps_to_spawn = Vec::new();
    let mut smoke_particles = Vec::new();
    let mut cluster_split_positions = Vec::new();
    let mut shield_hits: Vec<(Entity, f32, Vec2)> = Vec::new(); // (generator_entity, damage, hit_pos)

    for (entity, mut transform, mut projectile, sprite) in &mut projectiles {
        let prev_vel_y = projectile.prev_velocity_y;

        // Apply gravity
        projectile.velocity.y -= GRAVITY * dt;

        // Update position
        transform.translation.x += projectile.velocity.x * dt;
        transform.translation.y += projectile.velocity.y * dt;

        let pos = transform.translation.truncate();

        // Spawn smoke trail (randomly, ~30% chance per frame)
        if rng.random_range(0.0..1.0) < 0.3 {
            smoke_particles.push(pos);
        }

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

                submunitions_to_spawn.push((pos, rotated_velocity, color, projectile.player));
            }

            // Record split position for particle effect
            cluster_split_positions.push(pos);

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

        // Check shield collision first (shields protect structures behind them)
        let mut hit_shield = false;
        for (gen_entity, gen_transform, generator) in &shield_generators {
            // Skip friendly shields (projectiles pass through own team's shields)
            if generator.player == projectile.player {
                continue;
            }
            // Skip disabled or depleted shields
            if generator.disabled_turns > 0 || generator.shield_health <= 0.0 {
                continue;
            }

            let gen_pos = gen_transform.translation.truncate();
            let rel_pos = pos - gen_pos;

            // Check if projectile is within the shield arc
            if point_in_shield(rel_pos, generator.player) {
                // Hit the shield!
                let stats = projectile.weapon.stats();
                shield_hits.push((gen_entity, stats.damage.max(1.0), pos));
                hit_shield = true;

                // Spawn impact particles
                spawn_particles(
                    &mut commands,
                    pos,
                    8,
                    generator.player.color(),
                    (30.0, 80.0),
                    0.3,
                    3.0,
                    false,
                    true,
                    None,
                    1.0,
                );

                // EMP still triggers its effect when hitting shield
                if projectile.weapon == Weapon::EMP {
                    emps_to_spawn.push((pos, stats.blast_radius));
                }

                to_despawn.push(entity);
                break;
            }
        }

        if hit_shield {
            continue;
        }

        // Check direct base collision (projectile hits base directly)
        let mut hit_structure = false;
        for base_transform in &player_bases {
            let base_pos = base_transform.translation.truncate();
            let half_size = PLAYER_BASE_SIZE / 2.0;

            if pos.x >= base_pos.x - half_size
                && pos.x <= base_pos.x + half_size
                && pos.y >= base_pos.y - half_size
                && pos.y <= base_pos.y + half_size
            {
                hit_structure = true;
                break;
            }
        }

        // Check wall collision
        if !hit_structure {
            for (wall_transform, wall_sprite) in &walls {
                let wall_pos = wall_transform.translation.truncate();
                let wall_size = wall_sprite.custom_size.unwrap_or(Vec2::splat(40.0));
                let half_w = wall_size.x / 2.0;
                let half_h = wall_size.y / 2.0;

                if pos.x >= wall_pos.x - half_w
                    && pos.x <= wall_pos.x + half_w
                    && pos.y >= wall_pos.y - half_h
                    && pos.y <= wall_pos.y + half_h
                {
                    hit_structure = true;
                    break;
                }
            }
        }

        // Check terrain collision
        let terrain_hit = terrain_data
            .get_height_at(pos.x)
            .map(|h| pos.y <= h)
            .unwrap_or(false);

        if hit_structure || terrain_hit {
            let stats = projectile.weapon.stats();
            if projectile.weapon == Weapon::EMP {
                // EMP has special handling - no damage, just disable effect
                emps_to_spawn.push((pos, stats.blast_radius));
            } else if stats.blast_radius > 0.0 {
                explosions_to_spawn.push((pos, stats.damage, stats.blast_radius));
            }
            to_despawn.push(entity);
        }
    }

    // Apply shield damage
    for (gen_entity, damage, _hit_pos) in shield_hits {
        if let Ok((_, _, mut generator)) = shield_generators.get_mut(gen_entity) {
            generator.shield_health = (generator.shield_health - damage).max(0.0);
        }
    }

    // Despawn projectiles
    for entity in to_despawn {
        commands.entity(entity).despawn();
    }

    // Spawn submunitions
    for (pos, velocity, color, player) in submunitions_to_spawn {
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
                player,
            },
        ));
    }

    // Spawn explosions
    for (pos, damage, blast_radius) in explosions_to_spawn {
        spawn_explosion(&mut commands, pos, damage, blast_radius);
    }

    // Spawn EMP effects
    for (pos, blast_radius) in emps_to_spawn {
        commands.spawn((
            Transform::from_xyz(pos.x, pos.y, 2.0),
            PendingEMP { blast_radius },
        ));
    }

    // Spawn smoke trail particles
    for pos in smoke_particles {
        commands.spawn((
            Sprite {
                color: Color::srgba(0.5, 0.5, 0.5, 0.6),
                custom_size: Some(Vec2::splat(3.0)),
                ..default()
            },
            Transform::from_xyz(pos.x, pos.y, 2.5),
            Particle {
                velocity: Vec2::new(rng.random_range(-10.0..10.0), rng.random_range(5.0..15.0)),
                lifetime: 0.0,
                max_lifetime: 0.4,
                gravity: false,
                fade: true,
            },
        ));
    }

    // Spawn cluster split effect - burst of sparks radiating outward
    for pos in cluster_split_positions {
        spawn_particles(
            &mut commands,
            pos,
            20,
            Color::srgb(1.0, 0.9, 0.4), // Bright yellow-white sparks
            (80.0, 200.0),
            0.3,
            3.0,
            true,
            true,
            None, // Radial burst in all directions
            1.0,
        );
    }
}

fn check_projectile_phase_end(
    mut game_state: ResMut<GameState>,
    projectiles: Query<Entity, With<Projectile>>,
    aa_missiles: Query<Entity, With<AAMissile>>,
    pending_burst_shots: Query<Entity, With<PendingBurstShot>>,
) {
    if game_state.phase != TurnPhase::ProjectileInFlight {
        return;
    }

    // Track if we've seen any projectiles (handles deferred spawn)
    if !projectiles.is_empty() {
        game_state.projectiles_seen = true;
    }

    // End turn when projectiles, AA missiles, and pending burst shots are all gone
    // but only if we've actually seen projectiles (to handle deferred spawn)
    if projectiles.is_empty()
        && aa_missiles.is_empty()
        && pending_burst_shots.is_empty()
        && game_state.projectiles_seen
    {
        game_state.phase = TurnPhase::TurnEnding;
        game_state.turn_end_timer = TURN_END_DELAY;
        game_state.projectiles_seen = false;
    }
}

/// Spawns a pending explosion entity that will apply damage and show animation
fn spawn_explosion(commands: &mut Commands, pos: Vec2, damage: f32, blast_radius: f32) {
    commands.spawn((
        Transform::from_xyz(pos.x, pos.y, 2.0),
        PendingExplosion {
            damage,
            blast_radius,
        },
    ));
}

fn process_pending_burst_shots(
    mut commands: Commands,
    time: Res<Time>,
    mut pending_shots: Query<(Entity, &mut PendingBurstShot)>,
) {
    let dt = time.delta_secs();

    for (entity, mut shot) in &mut pending_shots {
        shot.delay -= dt;

        if shot.delay <= 0.0 {
            // Spawn the projectile
            commands.spawn((
                Sprite {
                    color: shot.player.color(),
                    custom_size: Some(Vec2::splat(PROJECTILE_RADIUS * 2.0)),
                    ..default()
                },
                Transform::from_xyz(shot.start_pos.x, shot.start_pos.y, 3.0),
                Projectile {
                    velocity: shot.velocity,
                    weapon: Weapon::Burst,
                    prev_velocity_y: shot.velocity.y,
                    player: shot.player,
                },
            ));

            // Spawn muzzle flash for this shot
            spawn_particles(
                &mut commands,
                shot.start_pos,
                8,
                Color::srgb(1.0, 0.8, 0.3),
                (40.0, 100.0),
                0.1,
                3.0,
                false,
                true,
                Some(shot.velocity.normalize_or_zero()),
                0.5,
            );

            commands.entity(entity).despawn();
        }
    }
}

fn process_pending_explosions(
    mut commands: Commands,
    mut terrain_data: ResMut<TerrainData>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    pending: Query<(Entity, &Transform, &PendingExplosion)>,
    mut player_bases: Query<
        (&Transform, &mut Health),
        (
            With<PlayerBase>,
            Without<PendingExplosion>,
            Without<AALauncher>,
            Without<Wall>,
        ),
    >,
    mut aa_launchers: Query<
        (&Transform, &mut Health),
        (
            With<AALauncher>,
            Without<PendingExplosion>,
            Without<PlayerBase>,
            Without<Wall>,
        ),
    >,
    mut walls: Query<
        (&Transform, &Sprite, &mut Health),
        (
            With<Wall>,
            Without<PendingExplosion>,
            Without<PlayerBase>,
            Without<AALauncher>,
        ),
    >,
    projectiles: Query<(Entity, &Transform, &Projectile), Without<PendingExplosion>>,
    aa_missiles: Query<(Entity, &Transform), (With<AAMissile>, Without<PendingExplosion>)>,
) {
    for (entity, transform, explosion) in &pending {
        let pos = transform.translation.truncate();

        if explosion.blast_radius > 0.0 {
            // Apply terrain damage
            terrain_data.apply_damage(pos.x, pos.y, explosion.damage, explosion.blast_radius);

            // Apply blast damage to bases
            for (base_transform, mut health) in &mut player_bases {
                let base_pos = base_transform.translation.truncate();
                let half_size = PLAYER_BASE_SIZE / 2.0;

                let nearest_x = pos.x.clamp(base_pos.x - half_size, base_pos.x + half_size);
                let nearest_y = pos.y.clamp(base_pos.y - half_size, base_pos.y + half_size);
                let nearest_point = Vec2::new(nearest_x, nearest_y);
                let distance = pos.distance(nearest_point);

                if distance <= 0.0 {
                    health.take_damage(explosion.damage);
                } else if distance < explosion.blast_radius {
                    let damage_factor = 1.0 - (distance / explosion.blast_radius);
                    health.take_damage(explosion.damage * damage_factor);
                }
            }

            // Apply blast damage to AA launchers
            for (aa_transform, mut health) in &mut aa_launchers {
                let aa_pos = aa_transform.translation.truncate();
                let half_size = Buildable::AALauncher.size() / 2.0;

                let nearest_x = pos.x.clamp(aa_pos.x - half_size, aa_pos.x + half_size);
                let nearest_y = pos.y.clamp(aa_pos.y - half_size, aa_pos.y + half_size);
                let nearest_point = Vec2::new(nearest_x, nearest_y);
                let distance = pos.distance(nearest_point);

                if distance <= 0.0 {
                    health.take_damage(explosion.damage);
                } else if distance < explosion.blast_radius {
                    let damage_factor = 1.0 - (distance / explosion.blast_radius);
                    health.take_damage(explosion.damage * damage_factor);
                }
            }

            // Apply blast damage to walls
            for (wall_transform, wall_sprite, mut health) in &mut walls {
                let wall_pos = wall_transform.translation.truncate();
                let wall_size = wall_sprite.custom_size.unwrap_or(Vec2::splat(40.0));
                let half_w = wall_size.x / 2.0;
                let half_h = wall_size.y / 2.0;

                let nearest_x = pos.x.clamp(wall_pos.x - half_w, wall_pos.x + half_w);
                let nearest_y = pos.y.clamp(wall_pos.y - half_h, wall_pos.y + half_h);
                let nearest_point = Vec2::new(nearest_x, nearest_y);
                let distance = pos.distance(nearest_point);

                if distance <= 0.0 {
                    health.take_damage(explosion.damage);
                } else if distance < explosion.blast_radius {
                    let damage_factor = 1.0 - (distance / explosion.blast_radius);
                    health.take_damage(explosion.damage * damage_factor);
                }
            }

            // Destroy projectiles caught in blast (triggers chain reaction)
            for (proj_entity, proj_transform, projectile) in &projectiles {
                let proj_pos = proj_transform.translation.truncate();
                let distance = pos.distance(proj_pos);

                if distance < explosion.blast_radius {
                    // Spawn explosion/EMP for this projectile
                    let stats = projectile.weapon.stats();
                    if projectile.weapon == Weapon::EMP {
                        commands.spawn((
                            Transform::from_xyz(proj_pos.x, proj_pos.y, 2.0),
                            PendingEMP {
                                blast_radius: stats.blast_radius,
                            },
                        ));
                    } else if stats.blast_radius > 0.0 {
                        spawn_explosion(&mut commands, proj_pos, stats.damage, stats.blast_radius);
                    }
                    commands.entity(proj_entity).despawn();
                }
            }

            // Destroy AA missiles caught in blast (triggers chain reaction)
            for (missile_entity, missile_transform) in &aa_missiles {
                let missile_pos = missile_transform.translation.truncate();
                let distance = pos.distance(missile_pos);

                if distance < explosion.blast_radius {
                    // AA missiles have a small explosion
                    spawn_explosion(&mut commands, missile_pos, 1.0, 20.0);
                    commands.entity(missile_entity).despawn();
                }
            }

            // Spawn explosion animation
            commands.spawn((
                Mesh2d(meshes.add(Circle::new(1.0))),
                MeshMaterial2d(
                    materials.add(ColorMaterial::from_color(Color::srgba(1.0, 0.6, 0.0, 1.0))),
                ),
                Transform::from_xyz(pos.x, pos.y, 2.0),
                Explosion {
                    timer: 0.0,
                    max_time: EXPLOSION_DURATION,
                    max_radius: explosion.blast_radius,
                    is_emp: false,
                },
            ));

            // Spawn initial explosion particles - fiery burst
            spawn_particles(
                &mut commands,
                pos,
                25,
                Color::srgb(1.0, 0.8, 0.2), // Bright yellow-orange
                (100.0, 250.0),
                0.4,
                5.0,
                true,
                true,
                None, // Radial burst
                1.0,
            );
            // Add some darker debris particles
            spawn_particles(
                &mut commands,
                pos,
                15,
                Color::srgb(0.4, 0.3, 0.2), // Brown debris
                (50.0, 150.0),
                0.6,
                4.0,
                true,
                true,
                None,
                1.0,
            );
        }

        // Remove the pending explosion
        commands.entity(entity).despawn();
    }
}

// One player action is one turn.
const EMP_DISABLE_TURNS: u32 = 5;

fn process_pending_emps(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    pending: Query<(Entity, &Transform, &PendingEMP)>,
    mut aa_launchers: Query<(&Transform, &mut AALauncher, &mut Sprite), Without<ShieldGenerator>>,
    mut shield_generators: Query<
        (&Transform, &mut ShieldGenerator, &mut Sprite),
        Without<AALauncher>,
    >,
) {
    for (entity, transform, emp) in &pending {
        let pos = transform.translation.truncate();

        // Disable AA launchers in blast radius
        for (aa_transform, mut aa_launcher, mut aa_sprite) in &mut aa_launchers {
            let aa_pos = aa_transform.translation.truncate();
            let distance = pos.distance(aa_pos);

            if distance < emp.blast_radius {
                aa_launcher.disabled_turns = EMP_DISABLE_TURNS;
                // Visual indicator - darken the sprite
                aa_sprite.color = Color::srgb(0.3, 0.3, 0.3);
            }
        }

        // Disable shield generators in blast radius
        for (gen_transform, mut generator, mut gen_sprite) in &mut shield_generators {
            let gen_pos = gen_transform.translation.truncate();
            let distance = pos.distance(gen_pos);

            if distance < emp.blast_radius {
                generator.disabled_turns = EMP_DISABLE_TURNS;
                // Visual indicator - darken the sprite
                gen_sprite.color = Color::srgb(0.3, 0.3, 0.3);
            }
        }

        // Spawn EMP visual effect (blue/electric colored expanding ring)
        commands.spawn((
            Mesh2d(meshes.add(Circle::new(1.0))),
            MeshMaterial2d(
                materials.add(ColorMaterial::from_color(Color::srgba(0.2, 0.5, 1.0, 0.8))),
            ),
            Transform::from_xyz(pos.x, pos.y, 2.0),
            Explosion {
                timer: 0.0,
                max_time: EXPLOSION_DURATION,
                max_radius: emp.blast_radius,
                is_emp: true,
            },
        ));

        // Spawn EMP electric particles - blue/cyan sparks
        spawn_particles(
            &mut commands,
            pos,
            30,
            Color::srgb(0.3, 0.7, 1.0), // Electric blue
            (80.0, 200.0),
            0.5,
            4.0,
            false, // No gravity for electric effect
            true,
            None,
            1.0,
        );

        // Remove the pending EMP
        commands.entity(entity).despawn();
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
    let mut rng = rand::rng();

    for (entity, mut explosion, mut transform, material_handle) in &mut explosions {
        explosion.timer += time.delta_secs();

        let progress = explosion.timer / explosion.max_time;

        if progress >= 1.0 {
            commands.entity(entity).despawn();
            continue;
        }

        let pos = transform.translation.truncate();

        // Expand quickly at first, then slow down
        let size_progress = 1.0 - (1.0 - progress).powi(2);
        // Circle mesh has radius 1.0, so scale equals final radius
        let scale = explosion.max_radius * size_progress;
        transform.scale = Vec3::splat(scale.max(0.1));

        // Spawn rising smoke particles during explosion (more at start, less at end)
        if !explosion.is_emp && rng.random_range(0.0..1.0) < 0.4 * (1.0 - progress) {
            let offset = Vec2::new(
                rng.random_range(-scale..scale) * 0.5,
                rng.random_range(-scale..scale) * 0.5,
            );
            commands.spawn((
                Sprite {
                    color: Color::srgba(0.3, 0.3, 0.3, 0.5),
                    custom_size: Some(Vec2::splat(rng.random_range(4.0..8.0))),
                    ..default()
                },
                Transform::from_xyz(pos.x + offset.x, pos.y + offset.y, 2.6),
                Particle {
                    velocity: Vec2::new(
                        rng.random_range(-20.0..20.0),
                        rng.random_range(30.0..60.0),
                    ),
                    lifetime: 0.0,
                    max_lifetime: rng.random_range(0.5..1.0),
                    gravity: false,
                    fade: true,
                },
            ));
        }

        // Update color based on explosion type
        if let Some(material) = materials.get_mut(&material_handle.0) {
            let alpha = 1.0 - progress;
            if explosion.is_emp {
                // EMP: fade from blue to cyan to transparent
                let blue = 0.5 + 0.5 * (1.0 - progress);
                material.color = Color::srgba(0.2, 0.5, blue, alpha);
            } else {
                // Normal: fade from orange to red to transparent
                let green = 0.6 * (1.0 - progress);
                material.color = Color::srgba(1.0, green, 0.0, alpha);
            }
        }
    }
}

fn update_particles(
    mut commands: Commands,
    time: Res<Time>,
    mut particles: Query<(Entity, &mut Transform, &mut Particle, &mut Sprite)>,
) {
    let dt = time.delta_secs();

    for (entity, mut transform, mut particle, mut sprite) in &mut particles {
        particle.lifetime += dt;

        if particle.lifetime >= particle.max_lifetime {
            commands.entity(entity).despawn();
            continue;
        }

        // Apply velocity
        transform.translation.x += particle.velocity.x * dt;
        transform.translation.y += particle.velocity.y * dt;

        // Apply gravity if enabled
        if particle.gravity {
            particle.velocity.y -= GRAVITY * dt * 0.5;
        }

        // Fade out if enabled
        if particle.fade {
            let progress = particle.lifetime / particle.max_lifetime;
            let alpha = 1.0 - progress;
            sprite.color = sprite.color.with_alpha(alpha);
        }
    }
}

fn spawn_particles(
    commands: &mut Commands,
    pos: Vec2,
    count: u32,
    color: Color,
    speed_range: (f32, f32),
    lifetime: f32,
    size: f32,
    gravity: bool,
    fade: bool,
    direction: Option<Vec2>,
    spread: f32,
) {
    let mut rng = rand::rng();

    for _ in 0..count {
        let angle = if let Some(dir) = direction {
            let base_angle = dir.y.atan2(dir.x);
            base_angle + rng.random_range(-spread..spread)
        } else {
            rng.random_range(0.0..std::f32::consts::TAU)
        };

        let speed = rng.random_range(speed_range.0..speed_range.1);
        let velocity = Vec2::new(angle.cos(), angle.sin()) * speed;

        commands.spawn((
            Sprite {
                color,
                custom_size: Some(Vec2::splat(size)),
                ..default()
            },
            Transform::from_xyz(pos.x, pos.y, 3.5),
            Particle {
                velocity,
                lifetime: 0.0,
                max_lifetime: lifetime * rng.random_range(0.8..1.2),
                gravity,
                fade,
            },
        ));
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

        // Can't fire if disabled by EMP
        if aa_launcher.disabled_turns > 0 {
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
    terrain_data: Res<TerrainData>,
    mut missiles: Query<(Entity, &mut Transform, &mut AAMissile)>,
    projectiles: Query<(Entity, &Transform, &Projectile), Without<AAMissile>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    if game_state.phase != TurnPhase::ProjectileInFlight {
        return;
    }

    let dt = time.delta_secs();

    for (entity, mut transform, mut missile) in &mut missiles {
        let pos = transform.translation.truncate();

        // Find closest projectile to target
        let closest_target = projectiles
            .iter()
            .map(|(e, t, p)| {
                let proj_pos = t.translation.truncate();
                let distance = pos.distance(proj_pos);
                (e, proj_pos, p.weapon, distance)
            })
            .min_by(|a, b| a.3.partial_cmp(&b.3).unwrap());

        // Current direction from velocity
        let current_speed = missile.velocity.length();
        let current_direction = missile.velocity.normalize_or_zero();
        let current_angle = current_direction.y.atan2(current_direction.x);

        // If we have a target, steer towards it
        let new_angle = if let Some((_, target_pos, _, _)) = closest_target {
            let to_target = target_pos - pos;
            let desired_direction = to_target.normalize_or_zero();
            let desired_angle = desired_direction.y.atan2(desired_direction.x);

            // Calculate angle difference
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
            current_angle + actual_turn
        } else {
            // No target, keep flying straight
            current_angle
        };

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

        // Spawn flame trail particle behind the missile
        let flame_offset = -new_direction * 8.0;
        let flame_pos = pos + flame_offset;
        commands.spawn((
            Sprite {
                color: Color::srgb(1.0, 0.6, 0.1),
                custom_size: Some(Vec2::splat(5.0)),
                ..default()
            },
            Transform::from_xyz(flame_pos.x, flame_pos.y, 2.8),
            Particle {
                velocity: -new_direction * 30.0 + Vec2::new(0.0, 10.0),
                lifetime: 0.0,
                max_lifetime: 0.2,
                gravity: false,
                fade: true,
            },
        ));

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
                    is_emp: false,
                },
            ));
            commands.entity(entity).despawn();
            continue;
        }

        // Check terrain collision
        if let Some(terrain_height) = terrain_data.get_height_at(pos.x) {
            if pos.y < terrain_height {
                // Hit ground - small explosion
                spawn_explosion(&mut commands, pos, 1.0, 20.0);
                commands.entity(entity).despawn();
                continue;
            }
        }

        // Check collision with closest target
        if let Some((target_entity, target_pos, target_weapon, distance)) = closest_target {
            if distance < AA_MISSILE_EXPLOSION_RADIUS {
                // Hit! Trigger the projectile's effect at the intercept point
                let stats = target_weapon.stats();
                if target_weapon == Weapon::EMP {
                    // EMP still triggers when intercepted
                    commands.spawn((
                        Transform::from_xyz(target_pos.x, target_pos.y, 2.0),
                        PendingEMP {
                            blast_radius: stats.blast_radius,
                        },
                    ));
                } else if stats.blast_radius > 0.0 {
                    spawn_explosion(&mut commands, target_pos, stats.damage, stats.blast_radius);
                }

                // Destroy the projectile and missile
                commands.entity(target_entity).despawn();
                commands.entity(entity).despawn();
            }
        }
    }
}

fn reset_aa_launchers(mut game_state: ResMut<GameState>, mut aa_launchers: Query<&mut AALauncher>) {
    // Only run once at the start of each turn
    if game_state.phase != TurnPhase::Aiming || game_state.turn_start_processed {
        return;
    }

    for mut aa_launcher in &mut aa_launchers {
        aa_launcher.fired_this_turn = false;
    }
}

fn decrement_aa_disabled(
    mut game_state: ResMut<GameState>,
    mut aa_launchers: Query<(&mut AALauncher, &mut Sprite), Without<ShieldGenerator>>,
    mut shield_generators: Query<(&mut ShieldGenerator, &mut Sprite), Without<AALauncher>>,
) {
    // Only run once at the start of each turn
    if game_state.phase != TurnPhase::Aiming || game_state.turn_start_processed {
        return;
    }

    for (mut aa_launcher, mut sprite) in &mut aa_launchers {
        if aa_launcher.disabled_turns > 0 {
            aa_launcher.disabled_turns -= 1;

            // Restore color when no longer disabled
            if aa_launcher.disabled_turns == 0 {
                sprite.color = aa_launcher.player.color();
            }
        }
    }

    // Handle shield generator disabled countdown and recharge
    for (mut generator, mut sprite) in &mut shield_generators {
        if generator.disabled_turns > 0 {
            generator.disabled_turns -= 1;

            // Restore color when no longer disabled
            if generator.disabled_turns == 0 {
                sprite.color = generator.player.color();
            }
        }

        // Recharge shield when not disabled
        if generator.disabled_turns == 0 && generator.shield_health < SHIELD_MAX_HEALTH {
            generator.shield_health =
                (generator.shield_health + SHIELD_RECHARGE_PER_TURN).min(SHIELD_MAX_HEALTH);
        }
    }

    // Mark turn start as processed
    game_state.turn_start_processed = true;
}

fn update_shield_domes(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    generators: Query<(Entity, &Transform, &ShieldGenerator)>,
    mut domes: Query<
        (
            Entity,
            &ShieldDome,
            &mut Transform,
            &MeshMaterial2d<ColorMaterial>,
        ),
        Without<ShieldGenerator>,
    >,
) {
    // Track which generators have domes
    let mut generators_with_domes: std::collections::HashSet<Entity> =
        std::collections::HashSet::new();

    // Update existing domes or despawn if generator gone/disabled/depleted/health changed
    for (dome_entity, dome, mut dome_transform, material_handle) in &mut domes {
        if let Ok((_, gen_transform, generator)) = generators.get(dome.owner) {
            if generator.disabled_turns > 0 || generator.shield_health <= 0.0 {
                // Shield disabled or depleted - despawn dome
                commands.entity(dome_entity).despawn();
            } else if (generator.shield_health - dome.last_health).abs() > 0.01 {
                // Health changed - despawn and let it respawn with new thickness
                commands.entity(dome_entity).despawn();
            } else {
                // Update dome position to follow generator
                dome_transform.translation.x = gen_transform.translation.x;
                dome_transform.translation.y = gen_transform.translation.y;

                // Update dome color
                let health_ratio = generator.shield_health / SHIELD_MAX_HEALTH;
                if let Some(material) = materials.get_mut(&material_handle.0) {
                    let base_color = generator.player.color();
                    material.color = base_color.with_alpha(0.5 + 0.4 * health_ratio);
                }

                generators_with_domes.insert(dome.owner);
            }
        } else {
            // Generator no longer exists
            commands.entity(dome_entity).despawn();
        }
    }

    // Spawn domes for generators that don't have them
    for (entity, transform, generator) in &generators {
        if generators_with_domes.contains(&entity) {
            continue;
        }
        if generator.disabled_turns > 0 || generator.shield_health <= 0.0 {
            continue;
        }

        // Calculate thickness based on health
        let health_ratio = generator.shield_health / SHIELD_MAX_HEALTH;
        let thickness =
            SHIELD_MIN_THICKNESS + (SHIELD_MAX_THICKNESS - SHIELD_MIN_THICKNESS) * health_ratio;

        let mesh = create_shield_mesh(thickness, generator.player);
        let base_color = generator.player.color();

        commands.spawn((
            Mesh2d(meshes.add(mesh)),
            MeshMaterial2d(materials.add(ColorMaterial::from_color(
                base_color.with_alpha(0.5 + 0.4 * health_ratio),
            ))),
            Transform::from_xyz(
                transform.translation.x,
                transform.translation.y,
                0.9, // Slightly behind structures
            ),
            ShieldDome {
                owner: entity,
                last_health: generator.shield_health,
            },
        ));
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

fn update_falling_entities(
    terrain_data: Res<TerrainData>,
    mut entities: Query<(Entity, &mut Transform, &FallsWithGravity, &Sprite)>,
    time: Res<Time>,
) {
    // Collect all entity positions and sizes first (for collision checking)
    let entity_data: Vec<(Entity, Vec2, Vec2)> = entities
        .iter()
        .map(|(e, t, _, s)| {
            let size = s.custom_size.unwrap_or(Vec2::splat(30.0));
            (e, t.translation.truncate(), size)
        })
        .collect();

    for (entity, mut transform, falls, sprite) in &mut entities {
        let x = transform.translation.x;
        let bottom = transform.translation.y - falls.size / 2.0;
        let width = sprite.custom_size.map(|s| s.x).unwrap_or(30.0);

        // Get terrain height at entity position
        let terrain_height = terrain_data.get_height_at(x).unwrap_or(0.0);

        // Find the highest surface below this entity (terrain or another structure)
        let mut rest_height = terrain_height;

        for (other_entity, other_pos, other_size) in &entity_data {
            // Skip self
            if *other_entity == entity {
                continue;
            }

            // Check horizontal overlap
            let half_width = width / 2.0;
            let other_half_width = other_size.x / 2.0;
            let horizontal_overlap =
                (x - other_pos.x).abs() < (half_width + other_half_width - 5.0);

            if horizontal_overlap {
                // Top of the other entity
                let other_top = other_pos.y + other_size.y / 2.0;

                // Only consider structures that are below us
                if other_top < transform.translation.y && other_top > rest_height {
                    rest_height = other_top;
                }
            }
        }

        // If entity is above rest height, make it fall
        if bottom > rest_height + 1.0 {
            // Apply gravity
            let fall_speed = GRAVITY * time.delta_secs();
            transform.translation.y -= fall_speed;

            // Don't fall below rest height
            let min_y = rest_height + falls.size / 2.0;
            if transform.translation.y < min_y {
                transform.translation.y = min_y;
            }
        } else {
            // Snap to rest height if close
            transform.translation.y = rest_height + falls.size / 2.0;
        }
    }
}

fn update_health_bars(
    bases: Query<
        (Entity, &Transform, &Health),
        (
            With<PlayerBase>,
            Without<AALauncher>,
            Without<Wall>,
            Without<ShieldGenerator>,
        ),
    >,
    aa_launchers: Query<
        (Entity, &Transform, &Health),
        (
            With<AALauncher>,
            Without<PlayerBase>,
            Without<Wall>,
            Without<ShieldGenerator>,
        ),
    >,
    walls: Query<
        (Entity, &Transform, &Sprite, &Health),
        (
            With<Wall>,
            Without<PlayerBase>,
            Without<AALauncher>,
            Without<ShieldGenerator>,
        ),
    >,
    shield_generators: Query<
        (Entity, &Transform, &Health),
        (
            With<ShieldGenerator>,
            Without<PlayerBase>,
            Without<AALauncher>,
            Without<Wall>,
        ),
    >,
    mut health_bars: Query<
        (&mut Text2d, &mut Transform, &HealthBar),
        (
            Without<PlayerBase>,
            Without<HealthBarBackground>,
            Without<AALauncher>,
            Without<Wall>,
            Without<ShieldGenerator>,
        ),
    >,
    mut health_bar_backgrounds: Query<
        (&mut Transform, &HealthBarBackground),
        (
            Without<PlayerBase>,
            Without<HealthBar>,
            Without<AALauncher>,
            Without<Wall>,
            Without<ShieldGenerator>,
        ),
    >,
) {
    for (mut text, mut bar_transform, health_bar) in &mut health_bars {
        // Find the owner (base, AA launcher, wall, or shield generator)
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
            .or_else(|| {
                walls
                    .iter()
                    .find(|(e, _, _, _)| *e == health_bar.owner)
                    .map(|(e, t, sprite, h)| {
                        let size = sprite.custom_size.unwrap_or(Vec2::splat(40.0)).y;
                        (e, t, h, size)
                    })
            })
            .or_else(|| {
                shield_generators
                    .iter()
                    .find(|(e, _, _)| *e == health_bar.owner)
                    .map(|(e, t, h)| (e, t, h, Buildable::ShieldGenerator.size()))
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
            .or_else(|| {
                walls
                    .iter()
                    .find(|(e, _, _, _)| *e == bg.owner)
                    .map(|(_, t, sprite, _)| {
                        let size = sprite.custom_size.unwrap_or(Vec2::splat(40.0)).y;
                        ((), t, size)
                    })
            })
            .or_else(|| {
                shield_generators
                    .iter()
                    .find(|(e, _, _)| *e == bg.owner)
                    .map(|(_, t, _)| ((), t, Buildable::ShieldGenerator.size()))
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

fn check_structure_destruction(
    mut commands: Commands,
    aa_launchers: Query<
        (Entity, &Transform, &Health),
        (With<AALauncher>, Without<Wall>, Without<ShieldGenerator>),
    >,
    walls: Query<
        (Entity, &Transform, &Sprite, &Health),
        (With<Wall>, Without<AALauncher>, Without<ShieldGenerator>),
    >,
    shield_generators: Query<
        (Entity, &Transform, &Health),
        (With<ShieldGenerator>, Without<AALauncher>, Without<Wall>),
    >,
    health_bars: Query<(Entity, &HealthBar)>,
    health_bar_backgrounds: Query<(Entity, &HealthBarBackground)>,
) {
    // Check AA launchers
    for (entity, transform, health) in &aa_launchers {
        if health.is_dead() {
            let pos = transform.translation.truncate();
            // Spawn destruction debris
            spawn_particles(
                &mut commands,
                pos,
                20,
                Color::srgb(0.5, 0.5, 0.5), // Gray metal debris
                (60.0, 150.0),
                0.8,
                5.0,
                true,
                true,
                None,
                1.0,
            );
            spawn_particles(
                &mut commands,
                pos,
                10,
                Color::srgb(1.0, 0.6, 0.2), // Sparks
                (80.0, 180.0),
                0.4,
                3.0,
                true,
                true,
                None,
                1.0,
            );
            commands.entity(entity).despawn();
            despawn_health_bar(&mut commands, entity, &health_bars, &health_bar_backgrounds);
        }
    }

    // Check walls
    for (entity, transform, sprite, health) in &walls {
        if health.is_dead() {
            let pos = transform.translation.truncate();
            let size = sprite.custom_size.unwrap_or(Vec2::splat(40.0));
            // Spawn destruction debris - more particles for larger structures
            let particle_count = ((size.x * size.y) / 200.0) as u32;
            spawn_particles(
                &mut commands,
                pos,
                particle_count.max(15),
                Color::srgb(0.6, 0.5, 0.4), // Brown/tan debris
                (50.0, 120.0),
                1.0,
                6.0,
                true,
                true,
                None,
                1.0,
            );
            spawn_particles(
                &mut commands,
                pos,
                particle_count / 2,
                Color::srgb(0.4, 0.35, 0.3), // Darker debris
                (30.0, 80.0),
                1.2,
                4.0,
                true,
                true,
                None,
                1.0,
            );
            commands.entity(entity).despawn();
            despawn_health_bar(&mut commands, entity, &health_bars, &health_bar_backgrounds);
        }
    }

    // Check shield generators
    for (entity, transform, health) in &shield_generators {
        if health.is_dead() {
            let pos = transform.translation.truncate();
            // Spawn destruction debris - electric sparks and metal
            spawn_particles(
                &mut commands,
                pos,
                15,
                Color::srgb(0.5, 0.5, 0.6), // Gray-blue metal debris
                (60.0, 140.0),
                0.8,
                5.0,
                true,
                true,
                None,
                1.0,
            );
            spawn_particles(
                &mut commands,
                pos,
                12,
                Color::srgb(0.3, 0.7, 1.0), // Electric blue sparks
                (80.0, 160.0),
                0.5,
                3.0,
                false, // No gravity for electric sparks
                true,
                None,
                1.0,
            );
            commands.entity(entity).despawn();
            despawn_health_bar(&mut commands, entity, &health_bars, &health_bar_backgrounds);
        }
    }
}

fn despawn_health_bar(
    commands: &mut Commands,
    owner: Entity,
    health_bars: &Query<(Entity, &HealthBar)>,
    health_bar_backgrounds: &Query<(Entity, &HealthBarBackground)>,
) {
    for (bar_entity, bar) in health_bars {
        if bar.owner == owner {
            commands.entity(bar_entity).despawn();
        }
    }
    for (bg_entity, bg) in health_bar_backgrounds {
        if bg.owner == owner {
            commands.entity(bg_entity).despawn();
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
        game_state.turn_start_processed = false;
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
                FallsWithGravity {
                    size: PLAYER_BASE_SIZE,
                },
                ExtendsBuildArea { player },
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
