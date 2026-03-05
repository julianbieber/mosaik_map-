#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

@group(0) @binding(0) var screen_texture: texture_2d<f32>;
@group(0) @binding(1) var texture_sampler: sampler;

struct MosaicSettings {
    blur_radius: f32,
    tone_intensity: f32,
    tone_scale: f32,
    camera_scale: f32,
    camera_pos: vec2<f32>,
    _padding: vec2<f32>,
}
@group(0) @binding(2) var<uniform> settings: MosaicSettings;

// Hash-based pseudo-random for tone variation
fn hash21(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

fn hash22(p: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(hash21(p), hash21(p + vec2<f32>(127.1, 311.7)));
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let tex_size = vec2<f32>(textureDimensions(screen_texture));
    let texel = 1.0 / tex_size;
    let uv = in.uv;

    // Gaussian-ish blur by sampling neighbors
    let r = settings.blur_radius;
    var color = vec3<f32>(0.0);
    var total_weight = 0.0;

    // 5x5 weighted blur kernel
    for (var dy = -2i; dy <= 2i; dy = dy + 1i) {
        for (var dx = -2i; dx <= 2i; dx = dx + 1i) {
            let offset = vec2<f32>(f32(dx), f32(dy)) * texel * r;
            let dist = length(vec2<f32>(f32(dx), f32(dy)));
            let weight = exp(-dist * dist * 0.5);
            let sample_uv = clamp(uv + offset, vec2<f32>(0.0), vec2<f32>(1.0));
            color = color + textureSample(screen_texture, texture_sampler, sample_uv).rgb * weight;
            total_weight = total_weight + weight;
        }
    }
    color = color / total_weight;

    // Convert screen UV to world position for stable noise
    // UV (0,0) is top-left, (1,1) is bottom-right
    // World pos = camera_pos + (uv - 0.5) * viewport_size * camera_scale
    let centered_uv = uv - vec2<f32>(0.5);
    let world_pos = settings.camera_pos + centered_uv * tex_size * settings.camera_scale;

    // Tone variation grid in world space
    let grid = floor(world_pos / settings.tone_scale);
    let noise = hash21(grid);
    let noise2 = hash22(grid + vec2<f32>(42.0, 17.0));

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
