// 塗りの矩形・グリフ・画像を、どれも「中心・大きさ・回転を持つインスタンスの矩形」として描く。
// mode 0: 色で塗る。mode 1: グリフのアトラスのアルファに色を掛ける。mode 2: 画像に不透明度を掛ける。

struct Globals {
    // 描画先の大きさ (物理ピクセル)。
    size: vec2<f32>,
    // 中身 (レターボックスの内側) の左上の位置 (物理ピクセル)。インスタンスの座標はここが原点。
    origin: vec2<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var tex: texture_2d<f32>;
@group(0) @binding(2) var tex_sampler: sampler;

struct Instance {
    // 中心 (物理ピクセル、左上が原点)。
    @location(0) center: vec2<f32>,
    // 幅と高さ (物理ピクセル)。
    @location(1) size: vec2<f32>,
    // テクスチャの u0, v0, u1, v1。
    @location(2) uv: vec4<f32>,
    // 線形の RGBA (アルファは乗算していない)。画像では a だけを使う (不透明度)。
    @location(3) color: vec4<f32>,
    @location(4) mode: u32,
    // 回転 (ラジアン、画面の上で時計回りが正)。
    @location(5) angle: f32,
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
    // 中心のまわりに回す。y が下向きの座標なので、この行列で時計回りになる。
    let local = (corner - vec2(0.5, 0.5)) * instance.size;
    let c = cos(instance.angle);
    let s = sin(instance.angle);
    let pixel = globals.origin + instance.center + vec2(c * local.x - s * local.y, s * local.x + c * local.y);
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
    // サンプリングは分岐の外で行う (一様でない制御の中でのサンプリングは許されない)。
    let texel = textureSample(tex, tex_sampler, in.uv);
    if in.mode == 2u {
        let a = texel.a * in.color.a;
        return vec4(texel.rgb * a, a);
    }
    var coverage = 1.0;
    if in.mode == 1u {
        coverage = texel.r;
    }
    let a = in.color.a * coverage;
    // 乗算済みアルファで出す (ブレンドもそれに合わせてある)。
    return vec4(in.color.rgb * a, a);
}
