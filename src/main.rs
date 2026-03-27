use bevy::prelude::*;
use bevy::mesh::{Indices, PrimitiveTopology};
use rand::RngExt;

const WINDOW_WIDTH: u32 = 1280;
const WINDOW_HEIGHT: u32 = 720;
const TERRAIN_SEGMENTS: usize = 128;

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
        .add_systems(Startup, (setup_camera, generate_terrain))
        .run();
}

fn setup_camera(mut commands: Commands) {
    commands.spawn(Camera2d);
}

#[derive(Component)]
struct Terrain;

fn generate_terrain(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    let mut rng = rand::rng();

    let width = WINDOW_WIDTH as f32;
    let height = WINDOW_HEIGHT as f32;

    // Generate terrain heights using midpoint displacement
    let mut heights = vec![0.0f32; TERRAIN_SEGMENTS + 1];
    heights[0] = rng.random_range(100.0..300.0);
    heights[TERRAIN_SEGMENTS] = rng.random_range(100.0..300.0);

    midpoint_displacement(&mut heights, 0, TERRAIN_SEGMENTS, 150.0, &mut rng);

    // Build the terrain mesh
    let segment_width = width / TERRAIN_SEGMENTS as f32;
    let half_width = width / 2.0;
    let half_height = height / 2.0;

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
