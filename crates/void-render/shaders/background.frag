#version 450
layout(location=0) in vec2 uv;
layout(location=0) out vec4 color;
layout(push_constant) uniform Params { vec4 data; } pc;
float hash(vec2 p) { return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
void main() {
    float aspect = pc.data.x;
    vec2 p = (uv - vec2(0.68, 0.47)) * vec2(aspect, 1.0);
    float r = length(p);
    float angle = atan(p.y, p.x);
    vec3 base = vec3(0.008, 0.011, 0.019);
    vec2 cell = floor(uv * vec2(900.0, 600.0));
    float star = step(0.9988, hash(cell));
    base += star * pow(hash(cell + 2.0), 3.0) * 0.16;
    float ellipse = length(vec2(p.x, p.y * 4.7));
    float disk = exp(-abs(ellipse - 0.235) * 29.0);
    float turbulence = 0.62 + 0.22*sin(ellipse*280.0+angle*6.0) + 0.16*sin(ellipse*610.0-angle*3.0);
    float lens = exp(-abs(r - 0.128) * 180.0);
    float halo = exp(-abs(r - 0.137) * 31.0) * 0.22;
    float backArc = exp(-abs(length(vec2(p.x, p.y*1.18)) - 0.151)*120.0) * smoothstep(0.01,0.06,-p.y);
    vec3 amber = mix(vec3(0.60,0.12,0.025), vec3(1.0,0.69,0.32), clamp(turbulence,0.0,1.0));
    base += amber * (disk*turbulence*1.55 + halo + backArc*0.65);
    base *= smoothstep(0.118,0.124,r);
    base += vec3(1.0,0.74,0.43)*lens*1.7;
    // Keep the navigation side quiet and readable.
    base *= mix(0.10,1.0,smoothstep(0.23,0.60,uv.x));
    base *= 1.0 - 0.35*length(uv-0.5);
    color = vec4(base,1.0);
}
