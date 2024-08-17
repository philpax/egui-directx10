void vs_main(
    in const float2 i_pos  : POSITION,
    in const float2 i_uv   : TEXCOORD,
    in const float4 i_color: COLOR,
    out      float4 o_pos  : SV_POSITION,
    out      float2 o_uv   : TEXCOORD,
    out      float4 o_color: COLOR) {
    o_pos   = float4(i_pos, 0.0, 1.0);
    o_uv    = i_uv;
    o_color = i_color;
}

Texture2D<float4> g_tex    : register(t0);
SamplerState      g_sampler: register(s0);

// 0-1 sRGB gamma  from  0-1 linear
// <https://github.com/emilk/egui/blob/1f6ae49a5f6bf43a869c215dea0d3028be8d742a/crates/egui-wgpu/src/egui.wgsl#L49>
float3 gamma_from_linear_rgb(float3 rgb) {
    float3 cutoff = step(rgb, float3(0.0031308, 0.0031308, 0.0031308));
    float3 lower = rgb * float3(12.92, 12.92, 12.92);
    float3 higher = float3(1.055, 1.055, 1.055) * pow(abs(rgb), float3(1.0 / 2.4, 1.0 / 2.4, 1.0 / 2.4)) - float3(0.055, 0.055, 0.055);
    return lerp(higher, lower, cutoff);
}

float4 ps_main_gamma(
    in const float4 i_pos  : SV_POSITION,
    in const float2 i_uv   : TEXCOORD,
    in const float4 i_color: COLOR): SV_TARGET {
    float4 linearColor = i_color * g_tex.Sample(g_sampler, i_uv);
    return float4(gamma_from_linear_rgb(linearColor.rgb), linearColor.a);
}

float4 ps_main_linear(
    in const float4 i_pos  : SV_POSITION,
    in const float2 i_uv   : TEXCOORD,
    in const float4 i_color: COLOR): SV_TARGET {
    return i_color * g_tex.Sample(g_sampler, i_uv);
}
