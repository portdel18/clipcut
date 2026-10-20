// The resolved output (sRGB-encoded RGBA8, opaque over black) as NV12: BT.709 limited range,
// 4:2:0 with each chroma sample the average of its 2×2 pixels, the same arithmetic as
// `filmcraft_export::rgba_to_yuv420_8` on the CPU. One invocation writes a 4×2 block: four luma
// bytes on each of two rows and one row of interleaved Cb Cr for those four columns, one u32 each.

struct Dims { w: u32, h: u32, _a: u32, _b: u32 };

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var<storage, read_write> out: array<u32>;
@group(0) @binding(2) var<uniform> dims: Dims;

fn byte(v: f32) -> u32 {
    return u32(clamp(floor(v + 0.5), 1.0, 254.0));
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let x0 = id.x * 4u;
    let y0 = id.y * 2u;
    if (x0 >= dims.w || y0 >= dims.h) {
        return;
    }
    var cb = vec2<f32>(0.0, 0.0);
    var cr = vec2<f32>(0.0, 0.0);
    for (var dy = 0u; dy < 2u; dy++) {
        var word = 0u;
        for (var dx = 0u; dx < 4u; dx++) {
            let c = textureLoad(src, vec2<i32>(i32(x0 + dx), i32(y0 + dy)), 0).rgb;
            let yy = 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
            word = word | (byte(16.0 + 219.0 * yy) << (8u * dx));
            let u = (c.b - yy) / 1.8556;
            let v = (c.r - yy) / 1.5748;
            if (dx < 2u) { cb.x += u; cr.x += v; } else { cb.y += u; cr.y += v; }
        }
        out[((y0 + dy) * dims.w + x0) / 4u] = word;
    }
    let uv = byte(128.0 + 224.0 * cb.x / 4.0) | (byte(128.0 + 224.0 * cr.x / 4.0) << 8u)
        | (byte(128.0 + 224.0 * cb.y / 4.0) << 16u) | (byte(128.0 + 224.0 * cr.y / 4.0) << 24u);
    out[(dims.w * dims.h + (y0 / 2u) * dims.w + x0) / 4u] = uv;
}
