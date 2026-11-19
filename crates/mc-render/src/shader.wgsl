// Terrain: atlas texture × vertex colour × vanilla's lightmap curve, with distance fog.

struct Uniforms {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    fog_color: vec4<f32>,
    // w: the sky's fog end (`FogSkyEnd`), in blocks.
    sky_color: vec4<f32>,
    // fog start, fog end, sky brightness (1 at noon), brightness option (vanilla default 0.5)
    params: vec4<f32>,
    // x: vanilla's `GameTime`, the day's progress, (ticks % 24000) / 24000.
    time: vec4<f32>,
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

// Sky: one full-screen triangle that draws vanilla's sky disc, a fan 16 blocks overhead with a
// 512-block radius, fogged by the distance its vertices interpolate (16 at the centre, 512 at
// the rim). Below the disc's edge only the fog colour shows, as vanilla's clear colour.
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
    var color = u.fog_color.rgb;
    if dir.y > 0.0 {
        let r = 16.0 * length(dir.xz) / dir.y;
        if r < 512.0 {
            let distance = mix(16.0, length(vec2<f32>(512.0, 16.0)), r / 512.0);
            color = mix(u.sky_color.rgb, color, clamp(distance / u.sky_color.w, 0.0, 1.0));
        }
    }
    return vec4<f32>(color, 1.0);
}

// End portals and gateways: vanilla's `rendertype_end_portal` shader. The sky texture and
// `PORTAL_LAYERS` layers of stars are sampled in screen space, each layer scaled, turned
// and drifting with the game time. WGSL, like GLSL, takes matrix constructor arguments by
// column and reads `v * m` as a row vector, so vanilla's matrices are copied as written.
@group(1) @binding(0) var portal_sky: texture_2d<f32>;
@group(1) @binding(1) var portal_stars: texture_2d<f32>;
@group(1) @binding(2) var portal_sampler: sampler;

const PORTAL_COLORS = array<vec3<f32>, 16>(
    vec3<f32>(0.022087, 0.098399, 0.110818),
    vec3<f32>(0.011892, 0.095924, 0.089485),
    vec3<f32>(0.027636, 0.101689, 0.100326),
    vec3<f32>(0.046564, 0.109883, 0.114838),
    vec3<f32>(0.064901, 0.117696, 0.097189),
    vec3<f32>(0.063761, 0.086895, 0.123646),
    vec3<f32>(0.084817, 0.111994, 0.166380),
    vec3<f32>(0.097489, 0.154120, 0.091064),
    vec3<f32>(0.106152, 0.131144, 0.195191),
    vec3<f32>(0.097721, 0.110188, 0.187229),
    vec3<f32>(0.133516, 0.138278, 0.148582),
    vec3<f32>(0.070006, 0.243332, 0.235792),
    vec3<f32>(0.196766, 0.142899, 0.214696),
    vec3<f32>(0.047281, 0.315338, 0.321970),
    vec3<f32>(0.204675, 0.390010, 0.302066),
    vec3<f32>(0.080955, 0.314821, 0.661491),
);

const SCALE_TRANSLATE = mat4x4<f32>(
    0.5, 0.0, 0.0, 0.25,
    0.0, 0.5, 0.0, 0.25,
    0.0, 0.0, 1.0, 0.0,
    0.0, 0.0, 0.0, 1.0,
);

struct PortalOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) tex_proj: vec4<f32>,
    @location(1) distance: f32,
    @location(2) @interpolate(flat) layers: u32,
};

@vertex
fn vs_portal(in: VertexIn) -> PortalOut {
    var out: PortalOut;
    out.clip = u.view_proj * vec4<f32>(in.pos, 1.0);
    // `projection_from_position`.
    let half = out.clip * 0.5;
    out.tex_proj = vec4<f32>(half.x + half.w, half.y + half.w, out.clip.z, out.clip.w);
    let d = in.pos - u.eye.xyz;
    out.distance = max(length(d.xz), abs(d.y));
    out.layers = u32(round(in.light.z * 255.0));
    return out;
}

fn end_portal_layer(layer: f32) -> mat4x4<f32> {
    let translate = mat4x4<f32>(
        1.0, 0.0, 0.0, 17.0 / layer,
        0.0, 1.0, 0.0, (2.0 + layer / 1.5) * (u.time.x * 1.5),
        0.0, 0.0, 1.0, 0.0,
        0.0, 0.0, 0.0, 1.0,
    );
    // `mat2_rotate_z`, then the uniform scale, widened to a mat4 as GLSL's `mat4(mat2)` is.
    let a = radians((layer * layer * 4321.0 + layer * 9.0) * 2.0);
    let s = (4.5 - layer / 4.0) * 2.0;
    let m = mat2x2<f32>(s, 0.0, 0.0, s) * mat2x2<f32>(cos(a), -sin(a), sin(a), cos(a));
    let wide = mat4x4<f32>(
        m[0].x, m[0].y, 0.0, 0.0,
        m[1].x, m[1].y, 0.0, 0.0,
        0.0, 0.0, 1.0, 0.0,
        0.0, 0.0, 0.0, 1.0,
    );
    return wide * translate * SCALE_TRANSLATE;
}

// `textureProj`: divide by w.
fn sample_proj(t: texture_2d<f32>, p: vec4<f32>) -> vec3<f32> {
    return textureSample(t, portal_sampler, p.xy / p.w).rgb;
}

@fragment
fn fs_portal(in: PortalOut) -> @location(0) vec4<f32> {
    var color = sample_proj(portal_sky, in.tex_proj) * PORTAL_COLORS[0];
    for (var i = 0u; i < 16u; i++) {
        // Uniform control flow for the sampling: skip a layer's colour, not its sample.
        let layer = sample_proj(portal_stars, in.tex_proj * end_portal_layer(f32(i + 1u)));
        if i < in.layers {
            color += layer * PORTAL_COLORS[i];
        }
    }
    let fog = smoothstep(u.params.x, u.params.y, in.distance);
    return vec4<f32>(mix(color, u.fog_color.rgb, fog), 1.0);
}
