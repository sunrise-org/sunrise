#version 120

uniform sampler2D texture;
uniform sampler2D lightmap;

varying vec2 texcoord;
varying vec2 lmcoord;
varying vec4 color;

void main() {
    // Terrain as vanilla colours it: its texture, its tint and the lightmap, in vanilla's gamma.
    vec4 albedo = texture2D(texture, texcoord) * color;
    if (albedo.a < 0.1) discard;
    /* DRAWBUFFERS:0 */
    gl_FragData[0] = vec4(albedo.rgb * texture2D(lightmap, lmcoord).rgb, albedo.a);
}
