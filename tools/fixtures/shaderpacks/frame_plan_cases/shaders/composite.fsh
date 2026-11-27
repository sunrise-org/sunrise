#version 120
/* RENDERTARGETS: 3 */
uniform sampler2D colortex0;
uniform sampler2D colortex1;
uniform sampler2D colortex2;
uniform sampler2D shadowtex0;
varying vec2 uv;
void main() {
    vec4 world = texture2D(colortex0, uv);
    float shade = texture2D(shadowtex0, uv).r;
    gl_FragData[0] = vec4(world.rgb * (0.5 + 0.5 * shade) + texture2D(colortex1, uv).rgb * 0.125
        + texture2D(colortex2, uv).rgb * 0.125, 1.0);
}
