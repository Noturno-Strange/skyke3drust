// Skyke3D — material toon (WGSL com os imports do WESL, formato de shader do Bevy 0.20).
// SPDX-License-Identifier: GPL-3.0-only
//
// Reproduz o MeshToonMaterial do three.js r186 como o cliente web usa (assets/app-base.js):
//  * luz direta (principal, preenchimento, fogueira): irradiância = rampa[dot(N,L)·0,5+0,5] × cor×intensidade, numa rampa de
//    3 faixas com filtro "nearest" (a rampa é tingida pela cor de preenchimento de cada mundo: `toonTint`);
//  * luz de hemisfério (céu/chão) sem faixas: mix(chão, céu, dot(N,cima)·0,5+0,5);
//  * difuso de Lambert (cor/π), emissivo somado, neblina linear smoothstep(perto, longe, profundidade);
//  * saída linear (o alvo sRGB converte), sem tone mapping, como `renderer.outputColorSpace = SRGB` sem toneMapping.
// Modos: toon, básico (sem luz: MeshBasicMaterial) e contorno (casco invertido: básico com faces da frente descartadas).

import bevy_pbr::render::forward_io::VertexOutput;
import bevy_pbr::render::view_transformations::position_world_to_view;
import bevy_pbr::render::mesh_view_bindings::view;

fn view_position() -> vec3<f32> {
    return view.world_position.xyz;
}

struct ToonParams {
    color: vec4<f32>,
    emissive: vec4<f32>,
    // xy = escala, zw = deslocamento do UV (KHR_texture_transform)
    uv: vec4<f32>,
    ramp0: vec4<f32>,
    ramp1: vec4<f32>,
    ramp2: vec4<f32>,
    hemi_sky: vec4<f32>,
    hemi_ground: vec4<f32>,
    key_dir: vec4<f32>,
    key_color: vec4<f32>,
    fill_dir: vec4<f32>,
    fill_color: vec4<f32>,
    // xyz = posição, w = alcance (distance do three; 0 = infinito)
    fire_pos: vec4<f32>,
    // rgb = cor×intensidade, w = decaimento
    fire_color: vec4<f32>,
    fog_color: vec4<f32>,
    // x = perto, y = longe, z = corte de alfa (máscara), w livre
    fog_range: vec4<f32>,
    // cor média do chão (linear), usada com F_GROUND
    ground_avg: vec4<f32>,
    flags: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

const F_TOON: u32 = 1u;
const F_FLAT: u32 = 2u;
const F_TEXTURE: u32 = 4u;
const F_FOG: u32 = 8u;
const F_VERTEX_COLOR: u32 = 16u;
const F_MASK: u32 = 32u;
const F_DOUBLE: u32 = 64u;
// domo do céu: ramp0/1/2 = horizonte, meio e alto (sRGB, gravados direto como no ShaderMaterial original)
const F_SKY: u32 = 128u;
// chão (groundMat): cor média ao longe e ondulação de brilho
const F_GROUND: u32 = 256u;

const RECIPROCAL_PI: f32 = 0.3183098861837907;

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> p: ToonParams;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var base_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var base_sampler: sampler;

// gradientMap de 3 texels com NearestFilter: coord.x em [0,1] escolhe a faixa
fn ramp(dot_nl: f32) -> vec3<f32> {
    let x = clamp(dot_nl * 0.5 + 0.5, 0.0, 1.0);
    let i = min(u32(floor(x * 3.0)), 2u);
    if i == 0u { return p.ramp0.rgb; }
    if i == 1u { return p.ramp1.rgb; }
    return p.ramp2.rgb;
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

// Atributos opcionais da malha: só existem em VertexOutput quando a malha os tem.
@if(VERTEX_UVS_A)
fn mesh_uv(in: VertexOutput) -> vec2<f32> { return in.uv; }
@if(!VERTEX_UVS_A)
fn mesh_uv(in: VertexOutput) -> vec2<f32> { return vec2<f32>(0.0); }
@if(VERTEX_COLORS)
fn mesh_color(in: VertexOutput) -> vec4<f32> { return in.color; }
@if(!VERTEX_COLORS)
fn mesh_color(in: VertexOutput) -> vec4<f32> { return vec4<f32>(1.0); }

// getDistanceAttenuation do three.js
fn distance_attenuation(d: f32, cutoff: f32, decay: f32) -> f32 {
    var f = 1.0 / max(pow(d, decay), 0.01);
    if cutoff > 0.0 {
        let r = clamp(1.0 - pow(d / cutoff, 4.0), 0.0, 1.0);
        f = f * r * r;
    }
    return f;
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> @location(0) vec4<f32> {
    if (p.flags & F_SKY) != 0u {
        // vH = normalize(position).y do domo (centrado na origem do mundo)
        let h = normalize(in.world_position.xyz).y;
        let c = mix(mix(p.ramp0.rgb, p.ramp1.rgb, smoothstep(0.0, 0.3, h)), p.ramp2.rgb, smoothstep(0.12, 0.62, h));
        // o original grava a cor sRGB direto no framebuffer; aqui o alvo sRGB converte, então devolvemos linear
        return vec4<f32>(srgb_to_linear(c), 1.0);
    }
    var diffuse = p.color;
    if (p.flags & F_TEXTURE) != 0u {
        let uv = mesh_uv(in) * p.uv.xy + p.uv.zw;
        diffuse = diffuse * textureSample(base_texture, base_sampler, uv);
    }
    if (p.flags & F_VERTEX_COLOR) != 0u {
        diffuse = diffuse * mesh_color(in);
    }
    if (p.flags & F_MASK) != 0u && diffuse.a < p.fog_range.z {
        discard;
    }
    if (p.flags & F_GROUND) != 0u {
        // groundMat: a textura some na cor média entre 8 e 34 m da câmera, com uma ondulação larga de brilho
        let wp = in.world_position.xyz;
        let gf = smoothstep(8.0, 34.0, distance(wp, view_position()));
        let gv = 1.0 + 0.09 * (sin(wp.x * 0.07 + 1.3) * sin(wp.z * 0.06) + 0.6 * sin(wp.x * 0.021 - wp.z * 0.026 + 0.7));
        diffuse = vec4<f32>(mix(diffuse.rgb, p.ground_avg.rgb, gf * 0.92) * gv, diffuse.a);
    }

    var rgb = diffuse.rgb;
    if (p.flags & F_TOON) != 0u {
        var n = normalize(in.world_normal);
        if (p.flags & F_FLAT) != 0u {
            n = normalize(cross(dpdx(in.world_position.xyz), dpdy(in.world_position.xyz)));
        } else if (p.flags & F_DOUBLE) != 0u && !is_front {
            n = -n;
        }
        let lambert = diffuse.rgb * RECIPROCAL_PI;

        // indireta: hemisfério (céu em cima, chão embaixo)
        let w = 0.5 * n.y + 0.5;
        var indirect = mix(p.hemi_ground.rgb, p.hemi_sky.rgb, w);

        // direta: principal, preenchimento e fogueira, em faixas
        var direct = ramp(dot(n, p.key_dir.xyz)) * p.key_color.rgb;
        direct += ramp(dot(n, p.fill_dir.xyz)) * p.fill_color.rgb;
        let to_fire = p.fire_pos.xyz - in.world_position.xyz;
        let d = length(to_fire);
        if d > 0.0001 && any(p.fire_color.rgb > vec3<f32>(0.0)) {
            let att = distance_attenuation(d, p.fire_pos.w, p.fire_color.w);
            direct += ramp(dot(n, to_fire / d)) * p.fire_color.rgb * att;
        }
        rgb = (direct + indirect) * lambert + p.emissive.rgb;
    }

    if (p.flags & F_FOG) != 0u {
        // three.js aplica a neblina DEPOIS da conversão para sRGB (fog_fragment vem após colorspace_fragment),
        // com a cor da neblina já em sRGB; mistura em sRGB e volta para linear para o alvo sRGB do Bevy.
        let depth = -position_world_to_view(in.world_position.xyz).z;
        let f = smoothstep(p.fog_range.x, p.fog_range.y, depth);
        rgb = srgb_to_linear(mix(linear_to_srgb(max(rgb, vec3<f32>(0.0))), p.fog_color.rgb, f));
    }
    return vec4<f32>(rgb, diffuse.a);
}
