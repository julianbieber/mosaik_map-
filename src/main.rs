mod mosaic;
mod osm;

use bevy::prelude::*;
use mosaic::CellType;
use std::collections::{HashMap, HashSet};
use std::sync::{mpsc, Mutex};

// Mainz Römisches Theater
const CENTER_LAT: f64 = 50.000;
const CENTER_LON: f64 = 8.270;

// Chunk geography: each chunk covers this many degrees
const CHUNK_LAT: f64 = 0.008;
const CHUNK_LON: f64 = 0.012;

// Grid resolution per chunk
const TILES_PER_CHUNK: usize = 150;
const TILE_SIZE: f32 = 4.0;

// How many chunks around the camera to keep loaded
const LOAD_RADIUS: i32 = 2;
// Despawn chunks further than this
const UNLOAD_RADIUS: i32 = 4;

// Derived: world units per degree
const SCALE_X: f64 = (TILES_PER_CHUNK as f64 * TILE_SIZE as f64) / CHUNK_LON;
const SCALE_Y: f64 = (TILES_PER_CHUNK as f64 * TILE_SIZE as f64) / CHUNK_LAT;

fn chunk_coord(lat: f64, lon: f64) -> (i32, i32) {
    (
        (lon / CHUNK_LON).floor() as i32,
        (lat / CHUNK_LAT).floor() as i32,
    )
}

fn camera_to_latlon(world_x: f32, world_y: f32) -> (f64, f64) {
    let lon = CENTER_LON + world_x as f64 / SCALE_X;
    let lat = CENTER_LAT + world_y as f64 / SCALE_Y;
    (lat, lon)
}

fn chunk_world_origin(cx: i32, cy: i32) -> (f32, f32) {
    let wx = (cx as f64 * CHUNK_LON - CENTER_LON) * SCALE_X;
    let wy = (cy as f64 * CHUNK_LAT - CENTER_LAT) * SCALE_Y;
    (wx as f32, wy as f32)
}

#[derive(Component)]
struct ChunkTile {
    chunk: (i32, i32),
}

#[derive(Resource)]
struct ChunkManager {
    loaded: HashMap<(i32, i32), Vec<Entity>>,
    pending: HashSet<(i32, i32)>,
    receiver: Mutex<mpsc::Receiver<ChunkResult>>,
    sender: mpsc::Sender<ChunkRequest>,
}

struct ChunkRequest {
    cx: i32,
    cy: i32,
    min_lat: f64,
    min_lon: f64,
    max_lat: f64,
    max_lon: f64,
}

struct ChunkResult {
    cx: i32,
    cy: i32,
    cells: Vec<CellType>,
}

#[derive(Resource)]
struct TileMaterials {
    residential: Handle<ColorMaterial>,
    park: Handle<ColorMaterial>,
    water: Handle<ColorMaterial>,
    street: Handle<ColorMaterial>,
    railway: Handle<ColorMaterial>,
    building: Handle<ColorMaterial>,
    empty: Handle<ColorMaterial>,
}

#[derive(Resource)]
struct TileMesh(Handle<Mesh>);

fn main() {
    let (req_tx, req_rx) = mpsc::channel::<ChunkRequest>();
    let (res_tx, res_rx) = mpsc::channel::<ChunkResult>();

    // Background loader thread
    std::thread::spawn(move || {
        for req in req_rx {
            match osm::download_osm_data(req.min_lat, req.min_lon, req.max_lat, req.max_lon) {
                Ok((osm_data, was_cached)) => {
                    let grid =
                        mosaic::build_grid(&osm_data, TILES_PER_CHUNK, TILES_PER_CHUNK);
                    let _ = res_tx.send(ChunkResult {
                        cx: req.cx,
                        cy: req.cy,
                        cells: grid.cells,
                    });
                    // Only delay after actual network requests
                    if !was_cached {
                        std::thread::sleep(std::time::Duration::from_secs(2));
                    }
                }
                Err(e) => {
                    eprintln!("Failed to load chunk ({},{}): {}", req.cx, req.cy, e);
                    let _ = res_tx.send(ChunkResult {
                        cx: req.cx,
                        cy: req.cy,
                        cells: vec![CellType::Empty; TILES_PER_CHUNK * TILES_PER_CHUNK],
                    });
                    // Also delay after failures to back off
                    std::thread::sleep(std::time::Duration::from_secs(3));
                }
            }
        }
    });

    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Mosaik Map".into(),
                resolution: (1200u32, 900u32).into(),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(ChunkManager {
            loaded: HashMap::new(),
            pending: HashSet::new(),
            receiver: Mutex::new(res_rx),
            sender: req_tx,
        })
        .add_systems(Startup, setup)
        .add_systems(Update, camera_controls)
        .add_systems(Update, (request_chunks, receive_chunks, unload_distant_chunks))
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    commands.spawn(Camera2d);

    commands.insert_resource(TileMesh(meshes.add(Rectangle::new(TILE_SIZE, TILE_SIZE))));

    commands.insert_resource(TileMaterials {
        residential: materials.add(Color::srgb(0.85, 0.82, 0.75)),
        park: materials.add(Color::srgb(0.45, 0.72, 0.40)),
        water: materials.add(Color::srgb(0.35, 0.55, 0.85)),
        street: materials.add(Color::srgb(0.55, 0.55, 0.55)),
        railway: materials.add(Color::srgb(0.30, 0.20, 0.20)),
        building: materials.add(Color::srgb(0.75, 0.70, 0.65)),
        empty: materials.add(Color::srgb(0.93, 0.91, 0.88)),
    });

    commands.spawn((
        Text::new(
            "WASD: pan | QE: zoom | Mosaik Map\n\
             Gray=Street  Brown=Railway  Blue=Water\n\
             Green=Park  Beige=Residential  Tan=Building",
        ),
        Node {
            position_type: PositionType::Absolute,
            top: px(12),
            left: px(12),
            ..default()
        },
    ));
}

fn camera_controls(
    camera: Single<(&mut Transform, &mut Projection), With<Camera2d>>,
    time: Res<Time>,
    kb: Res<ButtonInput<KeyCode>>,
) {
    let (ref mut transform, ref mut projection) = camera.into_inner();

    let scale = match projection.as_ref() {
        Projection::Orthographic(ortho) => ortho.scale,
        _ => 1.0,
    };
    let speed = 400.0 * scale * time.delta_secs();

    if kb.pressed(KeyCode::KeyW) || kb.pressed(KeyCode::ArrowUp) {
        transform.translation.y += speed;
    }
    if kb.pressed(KeyCode::KeyS) || kb.pressed(KeyCode::ArrowDown) {
        transform.translation.y -= speed;
    }
    if kb.pressed(KeyCode::KeyA) || kb.pressed(KeyCode::ArrowLeft) {
        transform.translation.x -= speed;
    }
    if kb.pressed(KeyCode::KeyD) || kb.pressed(KeyCode::ArrowRight) {
        transform.translation.x += speed;
    }
    if let Projection::Orthographic(ortho) = projection.as_mut() {
        if kb.pressed(KeyCode::KeyQ) {
            ortho.scale *= 1.0 + 1.5 * time.delta_secs();
        }
        if kb.pressed(KeyCode::KeyE) {
            ortho.scale *= 1.0 - 1.5 * time.delta_secs();
        }
    }
}

fn request_chunks(
    camera: Single<&Transform, With<Camera2d>>,
    mut manager: ResMut<ChunkManager>,
) {
    let (lat, lon) = camera_to_latlon(camera.translation.x, camera.translation.y);
    let (ccx, ccy) = chunk_coord(lat, lon);

    for dy in -LOAD_RADIUS..=LOAD_RADIUS {
        for dx in -LOAD_RADIUS..=LOAD_RADIUS {
            let cx = ccx + dx;
            let cy = ccy + dy;
            let key = (cx, cy);

            if manager.loaded.contains_key(&key) || manager.pending.contains(&key) {
                continue;
            }

            let min_lon = cx as f64 * CHUNK_LON;
            let min_lat = cy as f64 * CHUNK_LAT;
            let max_lon = min_lon + CHUNK_LON;
            let max_lat = min_lat + CHUNK_LAT;

            println!("Requesting chunk ({cx},{cy}) [{min_lat:.4},{min_lon:.4} - {max_lat:.4},{max_lon:.4}]");
            manager.pending.insert(key);
            let _ = manager.sender.send(ChunkRequest {
                cx,
                cy,
                min_lat,
                min_lon,
                max_lat,
                max_lon,
            });
        }
    }
}

fn receive_chunks(
    mut commands: Commands,
    mut manager: ResMut<ChunkManager>,
    tile_mesh: Res<TileMesh>,
    mats: Res<TileMaterials>,
) {
    // Collect results first to release the lock before mutating manager
    let results: Vec<ChunkResult> = {
        let receiver = manager.receiver.lock().unwrap();
        let mut results = Vec::new();
        while let Ok(result) = receiver.try_recv() {
            results.push(result);
        }
        results
    };

    for result in results {
        let key = (result.cx, result.cy);
        manager.pending.remove(&key);

        let (origin_x, origin_y) = chunk_world_origin(result.cx, result.cy);
        let mut entities = Vec::new();

        for gy in 0..TILES_PER_CHUNK {
            for gx in 0..TILES_PER_CHUNK {
                let cell = result.cells[gy * TILES_PER_CHUNK + gx];
                let material = match cell {
                    CellType::Empty => mats.empty.clone(),
                    CellType::Residential => mats.residential.clone(),
                    CellType::Park => mats.park.clone(),
                    CellType::Water => mats.water.clone(),
                    CellType::Street => mats.street.clone(),
                    CellType::Railway => mats.railway.clone(),
                    CellType::Building => mats.building.clone(),
                };

                let entity = commands
                    .spawn((
                        Mesh2d(tile_mesh.0.clone()),
                        MeshMaterial2d(material),
                        Transform::from_xyz(
                            origin_x + gx as f32 * TILE_SIZE,
                            origin_y + gy as f32 * TILE_SIZE,
                            0.0,
                        ),
                        ChunkTile { chunk: key },
                    ))
                    .id();
                entities.push(entity);
            }
        }

        println!(
            "Loaded chunk ({},{}) - {} tiles",
            result.cx,
            result.cy,
            entities.len()
        );
        manager.loaded.insert(key, entities);
    }
}

fn unload_distant_chunks(
    mut commands: Commands,
    camera: Single<&Transform, With<Camera2d>>,
    mut manager: ResMut<ChunkManager>,
) {
    let (lat, lon) = camera_to_latlon(camera.translation.x, camera.translation.y);
    let (ccx, ccy) = chunk_coord(lat, lon);

    let to_unload: Vec<(i32, i32)> = manager
        .loaded
        .keys()
        .filter(|(cx, cy)| {
            (cx - ccx).abs() > UNLOAD_RADIUS || (cy - ccy).abs() > UNLOAD_RADIUS
        })
        .copied()
        .collect();

    for key in to_unload {
        if let Some(entities) = manager.loaded.remove(&key) {
            for entity in entities {
                commands.entity(entity).despawn();
            }
            println!("Unloaded chunk ({},{})", key.0, key.1);
        }
    }
}
