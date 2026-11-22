// Terrain: atlas texture × vertex colour × vanilla's lightmap curve, with distance fog.

struct Uniforms {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    fog_color: vec4<f32>,
    // w: the sky's fog end (`FogSkyEnd`), in blocks.
    sky_color: vec4<f32>,
    // fog start, fog end, unused, brightness option (vanilla default 0.5)
    params: vec4<f32>,
    // x: vanilla's `GameTime`, the day's progress, (ticks % 24000) / 24000.
    time: vec4<f32>,
    // Width and height in pixels, outline width in pixels, GUI scale.
    screen: vec4<f32>,
    // The environment's fog (`FogEnvironmentalStart` and `End`), by spherical distance:
    // water's and lava's; out of them it never reaches anything.
    fog_env: vec4<f32>,
    // The underwater overlay: its texture's offsets, brightness, and the tangent of half
    // the hand's field of view.
    water: vec4<f32>,
    // The lightmap's inputs: the sky light's colour and factor, the ambient colour and the
    // block light factor, and the block light's tint, with the vignette's darkness in w.
    light_sky: vec4<f32>,
    light_ambient: vec4<f32>,
    light_tint: vec4<f32>,
    // `CloudInfo`: the clouds' colour, and the offset of the cell under the eye with the
    // clouds' fog end (`FogCloudsEnd`) in w.
    cloud_color: vec4<f32>,
    cloud_offset: vec4<f32>,
    // x: draw the dark disc; y: the sky is the overworld's.
    sky_flags: vec4<f32>,
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
    // The lightmap's colour, and 1 where an entity flashes red.
    @location(3) light: vec3<f32>,
    @location(4) red: f32,
    @location(5) spherical: f32,
};

// Vertex flags, in the light's fourth byte: an entity's red hurt flash (`OverlayTexture`'s
// red row), and quads drawn without the lightmap.
const FLAG_RED: u32 = 1u;
const FLAG_UNLIT: u32 = 2u;

// `lightmap.fsh`: a light level's brightness, 0 to 1.
fn brightness(level: f32) -> f32 {
    return level / (4.0 - 3.0 * level);
}

fn not_gamma(color: vec3<f32>) -> vec3<f32> {
    let m = max(max(color.x, color.y), color.z);
    let inv = 1.0 - m;
    return color * ((1.0 - inv * inv * inv * inv) / m);
}

// `lightmap.fsh`'s colour for a sky and block light level, without night vision, the
// boss overlay's darkening or the darkness effect.
fn lightmap(sky_level: f32, block_level: f32) -> vec3<f32> {
    let block = floor(block_level) / 15.0;
    let sky = floor(sky_level) / 15.0;
    var color = u.light_ambient.rgb + u.light_sky.rgb * brightness(sky) * u.light_sky.w;
    let parabolic = (2.0 * block - 1.0) * (2.0 * block - 1.0);
    let block_color = mix(u.light_tint.rgb, vec3<f32>(1.0), 0.9 * parabolic);
    color += block_color * brightness(block) * u.light_ambient.w;
    color = clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));
    if max(max(color.x, color.y), color.z) > 0.0 {
        color = mix(color, not_gamma(color), u.params.w);
    }
    return color;
}

@vertex
fn vs(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip = u.view_proj * vec4<f32>(in.pos, 1.0);
    out.uv = in.uv;
    // Light bytes hold level × 16; the vertex format normalises them to 0..1.
    let flags = u32(round(in.light.w * 255.0));
    out.light = lightmap(in.light.x * 255.0 / 16.0, in.light.y * 255.0 / 16.0);
    if (flags & FLAG_UNLIT) != 0u {
        out.light = vec3<f32>(1.0);
    }
    out.red = select(0.0, 1.0, (flags & FLAG_RED) != 0u);
    out.color = in.color;
    // Vanilla fogs by cylindrical distance: horizontal, and vertical separately.
    let d = in.pos - u.eye.xyz;
    out.distance = max(length(d.xz), abs(d.y));
    out.spherical = length(d);
    return out;
}

// `total_fog_value`: the environment's linear fog or the render distance's, whichever is
// thicker. Vanilla's render distance fog is linear too; this keeps its smoother curve.
fn fog_value(cylindrical: f32, spherical: f32) -> f32 {
    let env = clamp((spherical - u.fog_env.x) / (u.fog_env.y - u.fog_env.x), 0.0, 1.0);
    return max(env, smoothstep(u.params.x, u.params.y, cylindrical));
}

fn shade(in: VertexOut) -> vec4<f32> {
    let texel = textureSample(atlas, atlas_sampler, in.uv);
    var color = texel * in.color;
    // The `entity` shader's overlay: the red row's 0xB2 alpha keeps that much of the
    // colour and makes the rest red, before the lightmap.
    let flashed = mix(vec3<f32>(1.0, 0.0, 0.0), color.rgb, 178.0 / 255.0);
    color = vec4<f32>(mix(color.rgb, flashed, in.red) * in.light, color.a);
    let fog = fog_value(in.distance, in.spherical);
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
    if dir.y > 0.0 && u.sky_flags.y > 0.5 {
        let r = 16.0 * length(dir.xz) / dir.y;
        if r < 512.0 {
            let distance = mix(16.0, length(vec2<f32>(512.0, 16.0)), r / 512.0);
            color = mix(u.sky_color.rgb, color, clamp(distance / u.sky_color.w, 0.0, 1.0));
        }
    }
    return vec4<f32>(color, 1.0);
}

// `SkyRenderer.renderDarkDisc`: under the horizon, a black disc 4 blocks below the eye,
// its fog by the distance its vertices interpolate as the sky disc's do.
@fragment
fn fs_dark_disc(in: SkyOut) -> @location(0) vec4<f32> {
    let far = u.inv_view_proj * vec4<f32>(in.ndc, 1.0, 1.0);
    let dir = normalize(far.xyz / far.w - u.eye.xyz);
    if dir.y >= 0.0 {
        discard;
    }
    let r = 4.0 * length(dir.xz) / -dir.y;
    if r >= 512.0 {
        discard;
    }
    let distance = mix(16.0, length(vec2<f32>(512.0, 16.0)), r / 512.0);
    let fog = clamp(distance / u.sky_color.w, 0.0, 1.0);
    return vec4<f32>(u.fog_color.rgb * fog, 1.0);
}

// The sunrise fan (`position_color`), the sun and moon (`position_tex`), and the stars,
// with no light or fog.
@fragment
fn fs_sunrise(in: VertexOut) -> @location(0) vec4<f32> {
    return in.color;
}

@fragment
fn fs_celestial(in: VertexOut) -> @location(0) vec4<f32> {
    return textureSample(atlas, atlas_sampler, in.uv) * in.color;
}

@fragment
fn fs_stars(in: VertexOut) -> @location(0) vec4<f32> {
    return in.color;
}

// Clouds: `clouds.vsh`, each instance a face packed as x and z (cells from the eye's, plus
// 512), the `Direction` and whether it faces in, drawn as two triangles.
struct CloudOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) distance: f32,
};

const CLOUD_X: u32 = 0xF03CC3u;
const CLOUD_Y: u32 = 0x6666F0u;
const CLOUD_Z: u32 = 0xC3F066u;
const CLOUD_CELL: vec3<f32> = vec3<f32>(12.0, 4.0, 12.0);
const CLOUD_SHADE = array<f32, 6>(0.7, 1.0, 0.8, 0.8, 0.9, 0.9);
const QUAD = array<u32, 6>(0u, 1u, 2u, 0u, 2u, 3u);

@vertex
fn vs_cloud(@builtin(vertex_index) i: u32, @location(0) face: u32) -> CloudOut {
    let x = f32(i32(face & 1023u) - 512);
    let z = f32(i32((face >> 10u) & 1023u) - 512);
    let direction = (face >> 20u) & 7u;
    let inside = ((face >> 23u) & 1u) == 1u;
    var corner = QUAD[i];
    if inside {
        corner = 3u - corner;
    }
    let index = direction * 4u + corner;
    let vertex = vec3<f32>(
        f32((CLOUD_X >> index) & 1u),
        f32((CLOUD_Y >> index) & 1u),
        f32((CLOUD_Z >> index) & 1u),
    );
    let pos = vertex * CLOUD_CELL + vec3<f32>(x, 0.0, z) * CLOUD_CELL + u.cloud_offset.xyz;
    var out: CloudOut;
    out.clip = u.view_proj * vec4<f32>(pos + u.eye.xyz, 1.0);
    out.color = vec4<f32>(vec3<f32>(CLOUD_SHADE[direction]), 1.0) * u.cloud_color;
    out.distance = length(pos);
    return out;
}

@fragment
fn fs_cloud(in: CloudOut) -> @location(0) vec4<f32> {
    let fog = clamp(in.distance / u.cloud_offset.w, 0.0, 1.0);
    return vec4<f32>(in.color.rgb, in.color.a * (1.0 - fog));
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

// `rendertype_lightning.fsh`: the vertex colour, fading out into the fog.
@fragment
fn fs_lightning(in: VertexOut) -> @location(0) vec4<f32> {
    return in.color * (1.0 - fog_value(in.distance, in.spherical));
}

// The vignette (`Hud.extractVignette`): `misc/vignette.png` stretched over the screen, its
// colour times the darkness, which the blend takes away from what's behind.
@fragment
fn fs_vignette(in: SkyOut) -> @location(0) vec4<f32> {
    let uv = vec2<f32>(in.ndc.x * 0.5 + 0.5, 0.5 - in.ndc.y * 0.5);
    let texel = textureSample(portal_sky, portal_sampler, uv);
    return vec4<f32>(texel.rgb * u.light_tint.w, texel.a);
}

// Rain and snow (`particle.fsh`): the texture in the portal's sky slot, repeating, times
// the vertex colour and the lightmap, nearly clear texels dropped, then fogged.
@fragment
fn fs_weather(in: VertexOut) -> @location(0) vec4<f32> {
    let color = textureSample(portal_sky, portal_sampler, in.uv) * in.color
        * vec4<f32>(in.light, 1.0);
    if color.a < 0.1 {
        discard;
    }
    let fog = fog_value(in.distance, in.spherical);
    return vec4<f32>(mix(color.rgb, u.fog_color.rgb, fog), color.a);
}

// The underwater overlay (`ScreenEffectRenderer.submitWater`): `misc/underwater.png`
// four times across a quad from -1 to 1 half a block in front of the eye, seen through the
// hand's projection, shifted as the view turns, and 10% opaque. It's bound in the portal's
// sky slot, tiled the same way.
@fragment
fn fs_underwater(in: SkyOut) -> @location(0) vec4<f32> {
    let half = 0.5 * u.water.w;
    let xy = in.ndc * vec2<f32>(half * u.screen.x / u.screen.y, half);
    let uv = u.water.xy + 2.0 * (vec2<f32>(1.0) - xy);
    return textureSample(portal_sky, portal_sampler, uv) * vec4<f32>(vec3<f32>(u.water.z), 0.1);
}

// The block outline: vanilla's `rendertype_lines`. Each corner of an edge's quad is placed
// on screen from the edge's end and direction, then pushed sideways by half the line width
// in pixels, so lines are equally thick at any distance. `VIEW_SCALE` pulls them a hair
// toward the camera so they draw over the faces they lie on.
const VIEW_SCALE: f32 = 1.0 - 1.0 / 256.0;

struct LineIn {
    @location(0) pos: vec3<f32>,
    @location(1) dir: vec3<f32>,
    @location(2) side: f32,
};

@vertex
fn vs_line(in: LineIn) -> @builtin(position) vec4<f32> {
    let eye = u.eye.xyz;
    let start = u.view_proj * vec4<f32>(eye + (in.pos - eye) * VIEW_SCALE, 1.0);
    let end = u.view_proj * vec4<f32>(eye + (in.pos + in.dir - eye) * VIEW_SCALE, 1.0);
    let ndc1 = start.xyz / start.w;
    let ndc2 = end.xyz / end.w;
    let screen = u.screen.xy;
    let dir = normalize((ndc2.xy - ndc1.xy) * screen);
    var offset = vec2<f32>(-dir.y, dir.x) * u.screen.z / screen;
    if offset.x < 0.0 {
        offset = -offset;
    }
    return vec4<f32>((ndc1 + vec3<f32>(offset * in.side, 0.0)) * start.w, start.w);
}

@fragment
fn fs_line() -> @location(0) vec4<f32> {
    // `ARGB.black(102)`.
    return vec4<f32>(0.0, 0.0, 0.0, 102.0 / 255.0);
}

// The crosshair: `hud/crosshair.png`, a 15 × 15 sprite whose plus sign runs 3 to 11 along
// row and column 7, blitted at the GUI's centre, scaled by the GUI scale.
@fragment
fn fs_crosshair(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let scale = u.screen.w;
    let gui = ceil(u.screen.xy / scale);
    let origin = floor((gui - 15.0) / 2.0) * scale;
    let t = floor((p.xy - origin) / scale);
    let across = t.y == 7.0 && t.x >= 3.0 && t.x <= 11.0;
    let down = t.x == 7.0 && t.y >= 3.0 && t.y <= 11.0;
    if !(across || down) {
        discard;
    }
    return vec4<f32>(1.0);
}

// The HUD: positions are in GUI pixels, which the GUI scale turns into screen pixels, and
// sprites are sampled at full size.
struct GuiOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_gui(in: VertexIn) -> GuiOut {
    var out: GuiOut;
    let screen = in.pos.xy * u.screen.w / u.screen.xy;
    // Nearer (larger z) is smaller depth.
    out.clip = vec4<f32>(screen.x * 2.0 - 1.0, 1.0 - screen.y * 2.0, 0.5 - in.pos.z / 1000.0, 1.0);
    out.uv = in.uv;
    out.color = in.color;
    return out;
}

@fragment
fn fs_gui(in: GuiOut) -> @location(0) vec4<f32> {
    let c = textureSampleLevel(atlas, atlas_sampler, in.uv, 0.0) * in.color;
    if c.a < 0.1 {
        discard;
    }
    return c;
}

// Cracks: `rendertype_crumbling`, whose blend doubles the crack texture's colour times
// what's beneath, so mid grey leaves it as it was.
@fragment
fn fs_crack(in: VertexOut) -> @location(0) vec4<f32> {
    let c = textureSample(atlas, atlas_sampler, in.uv);
    if c.a < 0.1 {
        discard;
    }
    return vec4<f32>(c.rgb, 1.0);
}
