use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct OsmData {
    pub nodes: HashMap<i64, Node>,
    pub ways: Vec<Way>,
    pub bbox: [f64; 4], // min_lat, min_lon, max_lat, max_lon
}

#[derive(Debug, Clone, Copy)]
pub struct Node {
    pub lat: f64,
    pub lon: f64,
}

#[derive(Debug, Clone)]
pub struct Way {
    pub node_ids: Vec<i64>,
    pub feature: FeatureType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeatureType {
    MajorStreet,
    MinorStreet,
    Railway,
    Water,
    Park,
    Residential,
    Building,
    Unknown,
}

/// Returns (osm_data, was_cached)
pub fn download_osm_data(
    min_lat: f64,
    min_lon: f64,
    max_lat: f64,
    max_lon: f64,
) -> Result<(OsmData, bool), Box<dyn std::error::Error>> {
    let bbox_str = format!("{},{},{},{}", min_lat, min_lon, max_lat, max_lon);
    let query = format!(
        r#"[out:xml][timeout:60];
(
  way["highway"~"^(motorway|trunk|primary|secondary|tertiary|residential|unclassified)$"]({bbox});
  way["railway"~"^(rail|light_rail|subway|tram)$"]({bbox});
  way["waterway"]({bbox});
  way["natural"="water"]({bbox});
  way["water"]({bbox});
  way["leisure"="park"]({bbox});
  way["landuse"="grass"]({bbox});
  way["landuse"="residential"]({bbox});
  way["building"]({bbox});
  relation["natural"="water"]({bbox});
  relation["leisure"="park"]({bbox});
  relation["landuse"="grass"]({bbox});
);
(._;>;);
out body;"#,
        bbox = bbox_str
    );

    let cache_file = format!("osm_cache_{:.4}_{:.4}_{:.4}_{:.4}.xml", min_lat, min_lon, max_lat, max_lon);
    let cache_path = Path::new(&cache_file);

    let (resp, was_cached) = if cache_path.exists() {
        let data = std::fs::read_to_string(cache_path)?;
        // Don't use cached error responses
        if data.contains("<osm") {
            println!("Using cached OSM data from {cache_file}");
            (data, true)
        } else {
            std::fs::remove_file(cache_path)?;
            return download_osm_data(min_lat, min_lon, max_lat, max_lon);
        }
    } else {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()?;
        let data = client
            .post("https://overpass-api.de/api/interpreter")
            .body(query)
            .send()?
            .text()?;
        if data.contains("<osm") {
            std::fs::write(cache_path, &data)?;
            println!("Cached OSM data to {cache_file}");
            (data, false)
        } else {
            return Err("Overpass API error response (rate limit?)".into());
        }
    };

    Ok((parse_osm_xml(&resp, [min_lat, min_lon, max_lat, max_lon])?, was_cached))
}

fn parse_osm_xml(
    xml: &str,
    bbox: [f64; 4],
) -> Result<OsmData, Box<dyn std::error::Error>> {
    let doc = roxmltree::Document::parse(xml)?;
    let mut nodes = HashMap::new();
    let mut ways = Vec::new();
    let mut way_nodes: HashMap<i64, Vec<i64>> = HashMap::new();

    for node_elem in doc.descendants().filter(|n| n.has_tag_name("node")) {
        if let (Some(id), Some(lat), Some(lon)) = (
            node_elem.attribute("id"),
            node_elem.attribute("lat"),
            node_elem.attribute("lon"),
        ) {
            let id: i64 = id.parse()?;
            let lat: f64 = lat.parse()?;
            let lon: f64 = lon.parse()?;
            nodes.insert(id, Node { lat, lon });
        }
    }

    for way_elem in doc.descendants().filter(|n| n.has_tag_name("way")) {
        let node_ids: Vec<i64> = way_elem
            .children()
            .filter(|c| c.has_tag_name("nd"))
            .filter_map(|c| c.attribute("ref")?.parse().ok())
            .collect();

        if let Some(id) = way_elem.attribute("id") {
            if let Ok(way_id) = id.parse::<i64>() {
                way_nodes.insert(way_id, node_ids.clone());
            }
        }

        let tags: HashMap<&str, &str> = way_elem
            .children()
            .filter(|c| c.has_tag_name("tag"))
            .filter_map(|c| Some((c.attribute("k")?, c.attribute("v")?)))
            .collect();

        let feature = classify_way(&tags);
        if feature != FeatureType::Unknown {
            ways.push(Way { node_ids, feature });
        }
    }

    // Parse relations to assemble multipolygons (rivers, large parks, etc.)
    for rel_elem in doc.descendants().filter(|n| n.has_tag_name("relation")) {
        let tags: HashMap<&str, &str> = rel_elem
            .children()
            .filter(|c| c.has_tag_name("tag"))
            .filter_map(|c| Some((c.attribute("k")?, c.attribute("v")?)))
            .collect();

        let feature = classify_way(&tags);
        if feature == FeatureType::Unknown {
            continue;
        }

        // Collect outer member way IDs
        let outer_way_ids: Vec<i64> = rel_elem
            .children()
            .filter(|c| c.has_tag_name("member"))
            .filter(|c| c.attribute("type") == Some("way"))
            .filter(|c| {
                let role = c.attribute("role").unwrap_or("outer");
                role == "outer" || role == ""
            })
            .filter_map(|c| c.attribute("ref")?.parse().ok())
            .collect();

        // Assemble outer ways into closed rings
        let rings = assemble_rings(&outer_way_ids, &way_nodes);
        for ring in rings {
            ways.push(Way {
                node_ids: ring,
                feature,
            });
        }
    }

    Ok(OsmData { nodes, ways, bbox })
}

/// Chain way segments together into closed rings.
/// Each way is a sequence of node IDs. Ways connect where the last node of one
/// equals the first node of another.
fn assemble_rings(way_ids: &[i64], way_nodes: &HashMap<i64, Vec<i64>>) -> Vec<Vec<i64>> {
    let mut remaining: Vec<Vec<i64>> = way_ids
        .iter()
        .filter_map(|id| way_nodes.get(id).cloned())
        .filter(|ns| ns.len() >= 2)
        .collect();

    let mut rings = Vec::new();

    while !remaining.is_empty() {
        let mut ring = remaining.remove(0);

        // Keep trying to extend the ring
        let mut changed = true;
        while changed {
            changed = false;
            let front = *ring.first().unwrap();
            let back = *ring.last().unwrap();

            // Ring is closed
            if front == back && ring.len() >= 4 {
                break;
            }

            for i in 0..remaining.len() {
                let seg = &remaining[i];
                let seg_front = *seg.first().unwrap();
                let seg_back = *seg.last().unwrap();

                if seg_front == back {
                    // Append segment (skip first node, it's the junction)
                    let mut seg = remaining.remove(i);
                    seg.remove(0);
                    ring.append(&mut seg);
                    changed = true;
                    break;
                } else if seg_back == back {
                    // Append reversed segment
                    let mut seg = remaining.remove(i);
                    seg.reverse();
                    seg.remove(0);
                    ring.append(&mut seg);
                    changed = true;
                    break;
                } else if seg_back == front {
                    // Prepend segment
                    let mut seg = remaining.remove(i);
                    seg.pop();
                    seg.append(&mut ring);
                    ring = seg;
                    changed = true;
                    break;
                } else if seg_front == front {
                    // Prepend reversed segment
                    let mut seg = remaining.remove(i);
                    seg.reverse();
                    seg.pop();
                    seg.append(&mut ring);
                    ring = seg;
                    changed = true;
                    break;
                }
            }
        }

        rings.push(ring);
    }

    rings
}

fn classify_way(tags: &HashMap<&str, &str>) -> FeatureType {
    if let Some(highway) = tags.get("highway") {
        return match *highway {
            "motorway" | "trunk" | "primary" | "secondary" => FeatureType::MajorStreet,
            "tertiary" | "residential" | "unclassified" => FeatureType::MinorStreet,
            _ => FeatureType::Unknown,
        };
    }

    if tags.contains_key("railway") {
        return FeatureType::Railway;
    }

    if tags.contains_key("waterway") || tags.get("natural") == Some(&"water") || tags.contains_key("water") {
        return FeatureType::Water;
    }

    if tags.get("leisure") == Some(&"park") || tags.get("landuse") == Some(&"grass") {
        return FeatureType::Park;
    }

    if tags.get("landuse") == Some(&"residential") {
        return FeatureType::Residential;
    }

    if tags.contains_key("building") {
        return FeatureType::Building;
    }

    FeatureType::Unknown
}
