//!HOOK MAIN
//!BIND HOOKED
//!DESC video-primitives-probe broken (does not compile on purpose)
vec4 hook() {
    return HOOKED_tex(HOOKED_pos) * undefined_name;
}
