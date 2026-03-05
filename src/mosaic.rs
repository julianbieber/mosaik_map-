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
    pub width: usize,
    pub height: usize,
    pub cells: Vec<CellType>,
}

pub fn build_grid(osm: &OsmData, width: usize, height: usize) -> MosaicGrid {
    let [min_lat, min_lon, max_lat, max_lon] = osm.bbox;
    let lat_range = max_lat - min_lat;
    let lon_range = max_lon - min_lon;

    // Priority grid: higher number = higher priority when overwriting
    let mut priority: Vec<u8> = vec![0; width * height];
    let mut cells = vec![CellType::Empty; width * height];

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

        let coords: Vec<(usize, usize)> = way
            .node_ids
            .iter()
            .filter_map(|id| osm.nodes.get(id))
            .map(|node| {
                let gx =
                    ((node.lon - min_lon) / lon_range * (width as f64 - 1.0)).round() as usize;
                let gy =
                    ((node.lat - min_lat) / lat_range * (height as f64 - 1.0)).round() as usize;
                (gx.min(width - 1), gy.min(height - 1))
            })
            .collect();

        let is_area = matches!(
            way.feature,
            FeatureType::Water
                | FeatureType::Park
                | FeatureType::Residential
                | FeatureType::Building
        );

        if is_area && coords.len() >= 3 {
            fill_polygon(&coords, width, height, cell_type, prio, &mut cells, &mut priority);
        } else {
            // Draw as a line with thickness
            let thickness = match way.feature {
                FeatureType::MajorStreet => 3,
                FeatureType::Railway => 2,
                _ => 1,
            };
            for window in coords.windows(2) {
                draw_thick_line(
                    window[0].0 as i32,
                    window[0].1 as i32,
                    window[1].0 as i32,
                    window[1].1 as i32,
                    thickness,
                    width,
                    height,
                    cell_type,
                    prio,
                    &mut cells,
                    &mut priority,
                );
            }
        }
    }

    MosaicGrid {
        width,
        height,
        cells,
    }
}

fn set_cell(
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    cell_type: CellType,
    prio: u8,
    cells: &mut [CellType],
    priority: &mut [u8],
) {
    if x < w && y < h {
        let idx = y * w + x;
        if prio >= priority[idx] {
            cells[idx] = cell_type;
            priority[idx] = prio;
        }
    }
}

fn draw_thick_line(
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    thickness: i32,
    w: usize,
    h: usize,
    cell_type: CellType,
    prio: u8,
    cells: &mut [CellType],
    priority: &mut [u8],
) {
    // Bresenham's line with thickness
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
                let px = cx + tx;
                let py = cy + ty;
                if px >= 0 && py >= 0 {
                    set_cell(px as usize, py as usize, w, h, cell_type, prio, cells, priority);
                }
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
    coords: &[(usize, usize)],
    w: usize,
    h: usize,
    cell_type: CellType,
    prio: u8,
    cells: &mut [CellType],
    priority: &mut [u8],
) {
    // Find bounding box
    let min_y = coords.iter().map(|c| c.1).min().unwrap_or(0);
    let max_y = coords.iter().map(|c| c.1).max().unwrap_or(0);

    // Scanline fill
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
                let start = (pair[0].ceil() as usize).min(w - 1);
                let end = (pair[1].floor() as usize).min(w - 1);
                for x in start..=end {
                    set_cell(x, y, w, h, cell_type, prio, cells, priority);
                }
            }
        }
    }

    // Also draw outline
    for window in coords.windows(2) {
        draw_thick_line(
            window[0].0 as i32,
            window[0].1 as i32,
            window[1].0 as i32,
            window[1].1 as i32,
            1,
            w,
            h,
            cell_type,
            prio,
            cells,
            priority,
        );
    }
}
