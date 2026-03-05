use bevy::prelude::Resource;

use crate::osm::{FeatureType, OsmData};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellType {
    Empty,
    Residential,
    Park,
    Water,
    Street,
    Railway,
    Building,
}

#[derive(Resource)]
pub struct MosaicGrid {
    pub cells: Vec<CellType>,
}

pub fn build_grid(osm: &OsmData, width: usize, height: usize, render_bbox: [f64; 4]) -> MosaicGrid {
    let [min_lat, min_lon, max_lat, max_lon] = render_bbox;
    let lat_range = max_lat - min_lat;
    let lon_range = max_lon - min_lon;

    let mut priority: Vec<u8> = vec![0; width * height];
    let mut cells = vec![CellType::Empty; width * height];

    let w = width as i32;
    let h = height as i32;

    for way in &osm.ways {
        let cell_type = match way.feature {
            FeatureType::MajorStreet => CellType::Street,
            FeatureType::MinorStreet => CellType::Street,
            FeatureType::Railway => CellType::Railway,
            FeatureType::Water => CellType::Water,
            FeatureType::Park => CellType::Park,
            FeatureType::Residential => CellType::Residential,
            FeatureType::Building => CellType::Building,
            FeatureType::Unknown => continue,
        };

        let prio = match cell_type {
            CellType::Empty => 0,
            CellType::Residential => 1,
            CellType::Building => 2,
            CellType::Park => 3,
            CellType::Water => 4,
            CellType::Street => 5,
            CellType::Railway => 6,
        };

        // Use i32 coords so nodes outside the grid aren't clamped
        let coords: Vec<(i32, i32)> = way
            .node_ids
            .iter()
            .filter_map(|id| osm.nodes.get(id))
            .map(|node| {
                let gx = ((node.lon - min_lon) / lon_range * (width as f64 - 1.0)).round() as i32;
                let gy = ((node.lat - min_lat) / lat_range * (height as f64 - 1.0)).round() as i32;
                (gx, gy)
            })
            .collect();

        let is_area = matches!(
            way.feature,
            FeatureType::Water
                | FeatureType::Park
                | FeatureType::Residential
                | FeatureType::Building
        );

        let is_closed = way.node_ids.len() >= 4
            && way.node_ids.first() == way.node_ids.last();

        if is_area && is_closed && coords.len() >= 3 {
            fill_polygon(&coords, w, h, cell_type, prio, &mut cells, &mut priority);
        } else {
            let thickness = match way.feature {
                FeatureType::MajorStreet => 3,
                FeatureType::Railway => 2,
                FeatureType::Water => 3,
                _ => 1,
            };
            for window in coords.windows(2) {
                draw_thick_line(
                    window[0].0, window[0].1,
                    window[1].0, window[1].1,
                    thickness, w, h,
                    cell_type, prio, &mut cells, &mut priority,
                );
            }
        }
    }

    MosaicGrid { cells }
}

fn set_cell(
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    cell_type: CellType,
    prio: u8,
    cells: &mut [CellType],
    priority: &mut [u8],
) {
    if x >= 0 && y >= 0 && x < w && y < h {
        let idx = y as usize * w as usize + x as usize;
        if prio >= priority[idx] {
            cells[idx] = cell_type;
            priority[idx] = prio;
        }
    }
}

fn draw_thick_line(
    x0: i32, y0: i32, x1: i32, y1: i32,
    thickness: i32, w: i32, h: i32,
    cell_type: CellType, prio: u8,
    cells: &mut [CellType], priority: &mut [u8],
) {
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let mut cx = x0;
    let mut cy = y0;
    let half = thickness / 2;

    loop {
        for tx in -half..=half {
            for ty in -half..=half {
                set_cell(cx + tx, cy + ty, w, h, cell_type, prio, cells, priority);
            }
        }

        if cx == x1 && cy == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            cx += sx;
        }
        if e2 <= dx {
            err += dx;
            cy += sy;
        }
    }
}

fn fill_polygon(
    coords: &[(i32, i32)],
    w: i32, h: i32,
    cell_type: CellType, prio: u8,
    cells: &mut [CellType], priority: &mut [u8],
) {
    let min_y = coords.iter().map(|c| c.1).min().unwrap_or(0).max(0);
    let max_y = coords.iter().map(|c| c.1).max().unwrap_or(0).min(h - 1);

    for y in min_y..=max_y {
        let mut intersections = Vec::new();
        let n = coords.len();
        for i in 0..n {
            let (x0, y0) = (coords[i].0 as f64, coords[i].1 as f64);
            let (x1, y1) = (coords[(i + 1) % n].0 as f64, coords[(i + 1) % n].1 as f64);
            let yf = y as f64;

            if (y0 <= yf && y1 > yf) || (y1 <= yf && y0 > yf) {
                let x = x0 + (yf - y0) / (y1 - y0) * (x1 - x0);
                intersections.push(x);
            }
        }
        intersections.sort_by(|a, b| a.partial_cmp(b).unwrap());

        for pair in intersections.chunks(2) {
            if pair.len() == 2 {
                let start = (pair[0].ceil() as i32).max(0);
                let end = (pair[1].floor() as i32).min(w - 1);
                for x in start..=end {
                    set_cell(x, y, w, h, cell_type, prio, cells, priority);
                }
            }
        }
    }

    // Draw outline
    for window in coords.windows(2) {
        draw_thick_line(
            window[0].0, window[0].1,
            window[1].0, window[1].1,
            1, w, h,
            cell_type, prio, cells, priority,
        );
    }
}
