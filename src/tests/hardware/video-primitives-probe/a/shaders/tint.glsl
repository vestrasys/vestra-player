//!HOOK MAIN
//!BIND HOOKED
//!DESC video-primitives-probe tint
vec4 hook() {
    vec4 c = HOOKED_tex(HOOKED_pos);
    return vec4(min(c.r + 0.25, 1.0), c.g * 0.8, c.b * 0.8, c.a);
}
