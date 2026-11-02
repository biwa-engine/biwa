// 矩形とグリフを、どちらもインスタンスの矩形として描く。
// mode 0: 色で塗る。mode 1: グリフのアトラスのアルファに色を掛ける。

struct Globals {
    // 描画先の大きさ (物理ピクセル)。
    size: vec2<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var atlas: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;

struct Instance {
    // x, y, width, height (物理ピクセル、左上が原点)。
    @location(0) rect: vec4<f32>,
    // アトラスの u0, v0, u1, v1。
    @location(1) uv: vec4<f32>,
    // 線形の RGBA (アルファは乗算していない)。
    @location(2) color: vec4<f32>,
    @location(3) mode: u32,
};

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) mode: u32,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32, instance: Instance) -> VertexOut {
    // 2 つの三角形で矩形を作る。
    var corners = array<vec2<f32>, 6>(
        vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
        vec2(0.0, 1.0), vec2(1.0, 0.0), vec2(1.0, 1.0),
    );
    let corner = corners[index];
    let pixel = instance.rect.xy + corner * instance.rect.zw;
    let ndc = vec2(pixel.x / globals.size.x * 2.0 - 1.0, 1.0 - pixel.y / globals.size.y * 2.0);

    var out: VertexOut;
    out.position = vec4(ndc, 0.0, 1.0);
    out.uv = mix(instance.uv.xy, instance.uv.zw, corner);
    out.color = instance.color;
    out.mode = instance.mode;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    var coverage = 1.0;
    if in.mode == 1u {
        coverage = textureSample(atlas, atlas_sampler, in.uv).r;
    }
    let a = in.color.a * coverage;
    // 乗算済みアルファで出す (ブレンドもそれに合わせてある)。
    return vec4(in.color.rgb * a, a);
}
