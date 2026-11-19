// Terrain: atlas texture × vertex colour × vanilla's lightmap curve, with distance fog.

struct Uniforms {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    fog_color: vec4<f32>,
    sky_color: vec4<f32>,
    // fog start, fog end, sky brightness (1 at noon), brightness option (vanilla default 0.5)
    params: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var atlas: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;

struct VertexIn {
    @location(0) pos: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) light: vec4<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) distance: f32,
};

// LightTexture: a light level's brightness before the sky and block colours are mixed.
fn brightness(level: f32) -> f32 {
    let f = level / 15.0;
    return f / (4.0 - 3.0 * f);
}

fn not_gamma(c: vec3<f32>) -> vec3<f32> {
    let inv = vec3<f32>(1.0) - c;
    return vec3<f32>(1.0) - inv * inv * inv * inv;
}

// Vanilla's lightmap for a sky and block light level, at noon in the overworld.
fn lightmap(sky_level: f32, block_level: f32) -> vec3<f32> {
    let b = brightness(floor(block_level));
    let s = brightness(floor(sky_level)) * u.params.z;
    var color = vec3<f32>(b, b * ((b * 0.6 + 0.4) * 0.6 + 0.4), b * (b * b * 0.6 + 0.4));
    color = color + vec3<f32>(s);
    color = mix(color, vec3<f32>(0.75), 0.04);
    color = clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));
    color = mix(color, not_gamma(color), u.params.w);
    color = mix(color, vec3<f32>(0.75), 0.04);
    return clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));
}

@vertex
fn vs(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip = u.view_proj * vec4<f32>(in.pos, 1.0);
    out.uv = in.uv;
    // Light bytes hold level × 16; the vertex format normalises them to 0..1.
    let light = lightmap(in.light.x * 255.0 / 16.0, in.light.y * 255.0 / 16.0);
    out.color = vec4<f32>(in.color.rgb * light, 1.0);
    // Vanilla fogs by cylindrical distance: horizontal, and vertical separately.
    let d = in.pos - u.eye.xyz;
    out.distance = max(length(d.xz), abs(d.y));
    return out;
}

fn shade(in: VertexOut) -> vec4<f32> {
    let texel = textureSample(atlas, atlas_sampler, in.uv);
    let color = texel * in.color;
    let fog = smoothstep(u.params.x, u.params.y, in.distance);
    return vec4<f32>(mix(color.rgb, u.fog_color.rgb, fog), color.a);
}

@fragment
fn fs_opaque(in: VertexOut) -> @location(0) vec4<f32> {
    return vec4<f32>(shade(in).rgb, 1.0);
}

@fragment
fn fs_cutout(in: VertexOut) -> @location(0) vec4<f32> {
    let c = shade(in);
    if c.a < 0.5 {
        discard;
    }
    return vec4<f32>(c.rgb, 1.0);
}

@fragment
fn fs_translucent(in: VertexOut) -> @location(0) vec4<f32> {
    return shade(in);
}

// Sky: one full-screen triangle, pale fog colour at the horizon fading to the sky colour above.
struct SkyOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_sky(@builtin(vertex_index) i: u32) -> SkyOut {
    let xy = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
    var out: SkyOut;
    out.clip = vec4<f32>(xy, 0.0, 1.0);
    out.ndc = xy;
    return out;
}

@fragment
fn fs_sky(in: SkyOut) -> @location(0) vec4<f32> {
    let far = u.inv_view_proj * vec4<f32>(in.ndc, 1.0, 1.0);
    let dir = normalize(far.xyz / far.w - u.eye.xyz);
    let t = smoothstep(0.0, 0.35, dir.y);
    return vec4<f32>(mix(u.fog_color.rgb, u.sky_color.rgb, t), 1.0);
}
