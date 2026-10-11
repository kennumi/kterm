// screen_cell: screen w/h, cell w/h. pad_center: padding x/y, scale center x/y.
// misc: scale, opacity, baseline, unused.
struct U { screen_cell: vec4f, pad_center: vec4f, misc: vec4f }

@group(0) @binding(0) var<uniform> u: U;
// Two vec4 per glyph: (atlas x, y, w, h), (left, top, -, -).
@group(0) @binding(1) var<storage, read> glyphs: array<vec4f>;
@group(0) @binding(2) var atlas: texture_2d<f32>;

const CORNERS = array(vec2f(0, 0), vec2f(1, 0), vec2f(0, 1), vec2f(1, 0), vec2f(1, 1), vec2f(0, 1));

fn clip(p: vec2f) -> vec4f {
    let q = u.pad_center.zw + (p - u.pad_center.zw) * u.misc.x;
    return vec4f(q.x / u.screen_cell.x * 2.0 - 1.0, 1.0 - q.y / u.screen_cell.y * 2.0, 0.0, 1.0);
}

fn cell_origin(pos: u32) -> vec2f {
    return u.pad_center.xy + vec2f(f32(pos & 0xffffu), f32(pos >> 16u)) * u.screen_cell.zw;
}

struct CellIn {
    @location(0) pos: u32,
    @location(1) glyph: u32,
    @location(2) fg: u32,
    @location(3) bg: u32,
}

struct VOut {
    @builtin(position) pos: vec4f,
    @location(0) color: vec4f,
    @location(1) uv: vec2f,
}

const CULLED = vec4f(0, 0, 2, 1); // outside the clip volume

@vertex
fn vs_bg(@builtin(vertex_index) vi: u32, c: CellIn) -> VOut {
    var o: VOut;
    let color = unpack4x8unorm(c.bg);
    if color.a == 0.0 { o.pos = CULLED; return o; } // default background: let the clear show
    o.pos = clip(cell_origin(c.pos) + CORNERS[vi] * u.screen_cell.zw);
    o.color = color;
    return o;
}

@vertex
fn vs_fg(@builtin(vertex_index) vi: u32, c: CellIn) -> VOut {
    var o: VOut;
    if c.glyph == 0u { o.pos = CULLED; return o; }
    let rect = glyphs[c.glyph * 2u];
    let off = glyphs[c.glyph * 2u + 1u];
    let origin = cell_origin(c.pos) + vec2f(off.x, u.misc.z - off.y);
    o.pos = clip(origin + CORNERS[vi] * rect.zw);
    o.uv = rect.xy + CORNERS[vi] * rect.zw;
    o.color = unpack4x8unorm(c.fg);
    return o;
}

@fragment
fn fs_bg(i: VOut) -> @location(0) vec4f {
    let a = i.color.a * u.misc.y;
    return vec4f(i.color.rgb * a, a);
}

@fragment
fn fs_fg(i: VOut) -> @location(0) vec4f {
    let a = textureLoad(atlas, vec2i(floor(i.uv)), 0).r * i.color.a * u.misc.y;
    return vec4f(i.color.rgb * a, a);
}

// Free-form quads: cursor, cursor trail, underlines. Corners tl, tr, br, bl.
struct QuadIn {
    @location(0) p01: vec4f,
    @location(1) p23: vec4f,
    @location(2) color: u32,
}

@vertex
fn vs_quad(@builtin(vertex_index) vi: u32, q: QuadIn) -> VOut {
    let pts = array(q.p01.xy, q.p01.zw, q.p23.xy, q.p23.zw);
    let idx = array(0u, 1u, 2u, 0u, 2u, 3u);
    var o: VOut;
    o.pos = clip(pts[idx[vi]]);
    o.color = unpack4x8unorm(q.color);
    return o;
}
