#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

@group(0) @binding(0) var screen_texture: texture_2d<f32>;
@group(0) @binding(1) var texture_sampler: sampler;

struct MosaicSettings {
    blur_radius: f32,
    tone_intensity: f32,
    tone_scale: f32,
    camera_scale: f32,
    camera_pos: vec2<f32>,
    voronoi_scale: f32,
    _padding: f32,
}
@group(0) @binding(2) var<uniform> settings: MosaicSettings;

// Hash for Voronoi cell point jitter
fn hash2(p: vec2<f32>) -> vec2<f32> {
    let q = vec2<f32>(dot(p, vec2<f32>(127.1, 311.7)), dot(p, vec2<f32>(269.5, 183.3)));
    return fract(sin(q) * 43758.5453);
}

// Voronoi — returns (distance_to_nearest, distance_to_second, cell_offset_to_nearest)
fn voronoi(p: vec2<f32>) -> vec4<f32> {
    let cell = floor(p);
    let frac = fract(p);

    var min_d = 8.0;
    var second_d = 8.0;
    var nearest_offset = vec2<f32>(0.0);

    for (var dy = -1i; dy <= 1i; dy = dy + 1i) {
        for (var dx = -1i; dx <= 1i; dx = dx + 1i) {
            let neighbor = vec2<f32>(f32(dx), f32(dy));
            let point = hash2(cell + neighbor) * 0.8 + 0.1; // jitter within cell
            let diff = neighbor + point - frac;
            let d = dot(diff, diff);
            if d < min_d {
                second_d = min_d;
                min_d = d;
                nearest_offset = diff;
            } else if d < second_d {
                second_d = d;
            }
        }
    }
    return vec4<f32>(sqrt(min_d), sqrt(second_d), nearest_offset);
}

// R2 quasi-random sequence — blue-noise-like low-discrepancy distribution
fn r2_noise(cell: vec2<f32>) -> f32 {
    return fract(dot(cell, vec2<f32>(0.7548776662, 0.5698402910)));
}

fn r2_noise2(cell: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(
        r2_noise(cell),
        r2_noise(cell + vec2<f32>(0.5, 0.5)),
    );
}

// FBM using R2 noise at multiple octaves for organic variation
fn fbm(p: vec2<f32>) -> f32 {
    var val = 0.0;
    var amp = 0.5;
    var pos = p;
    for (var i = 0; i < 4; i = i + 1) {
        val = val + amp * r2_noise(floor(pos));
        pos = pos * 2.0 + vec2<f32>(1.7, 3.1);
        amp = amp * 0.5;
    }
    return val;
}

fn fbm2(p: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(
        fbm(p),
        fbm(p + vec2<f32>(42.0, 17.0)),
    );
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let tex_size = vec2<f32>(textureDimensions(screen_texture));
    let texel = 1.0 / tex_size;
    let uv = in.uv;

    // Convert screen UV to world position for stable effects
    let centered_uv = uv - vec2<f32>(0.5);
    let world_pos = settings.camera_pos + centered_uv * tex_size * settings.camera_scale;

    // Voronoi distortion in world space — warp UVs toward Voronoi cell centers
    let voronoi_scale = settings.voronoi_scale;
    let vor = voronoi(world_pos / voronoi_scale);
    let warp_strength = 0.008; // UV displacement amount
    // Push pixels toward their Voronoi cell center
    let warp_offset = vor.zw * warp_strength;
    // Edge darkening: where two cells meet (min_d ≈ second_d)
    let edge_factor = smoothstep(0.0, 0.15, vor.y - vor.x);

    let warped_uv = clamp(uv + warp_offset, vec2<f32>(0.0), vec2<f32>(1.0));

    // Fixed screen-pixel blur — zooming out blurs a larger world area
    let r = settings.blur_radius;
    var color = vec3<f32>(0.0);
    var total_weight = 0.0;

    // 9x9 weighted blur kernel on warped UVs
    for (var dy = -4i; dy <= 4i; dy = dy + 1i) {
        for (var dx = -4i; dx <= 4i; dx = dx + 1i) {
            let offset = vec2<f32>(f32(dx), f32(dy)) * texel * r;
            let dist = length(vec2<f32>(f32(dx), f32(dy)));
            let weight = exp(-dist * dist * 0.5);
            let sample_uv = clamp(warped_uv + offset, vec2<f32>(0.0), vec2<f32>(1.0));
            color = color + textureSample(screen_texture, texture_sampler, sample_uv).rgb * weight;
            total_weight = total_weight + weight;
        }
    }
    color = color / total_weight;

    // Subtle darkening at Voronoi cell edges (mosaic grout lines)
    color = color * mix(0.85, 1.0, edge_factor);

    // FBM tone variation in world space for organic feel
    let scaled_pos = world_pos / settings.tone_scale;
    let noise = fbm(scaled_pos);
    let noise2 = fbm2(scaled_pos + vec2<f32>(42.0, 17.0));

    // Warm/cool shift
    let warm = vec3<f32>(1.02, 1.0, 0.97);
    let cool = vec3<f32>(0.97, 1.0, 1.03);
    let tint = mix(warm, cool, noise);
    color = color * mix(vec3<f32>(1.0), tint, settings.tone_intensity);

    // Subtle brightness variation
    let brightness = 1.0 + (noise2.x - 0.5) * settings.tone_intensity * 0.3;
    color = color * brightness;

    // Very subtle saturation variation
    let luma = dot(color, vec3<f32>(0.299, 0.587, 0.114));
    let sat_factor = 1.0 + (noise2.y - 0.5) * settings.tone_intensity * 0.2;
    color = mix(vec3<f32>(luma), color, sat_factor);

    return vec4<f32>(color, 1.0);
}
