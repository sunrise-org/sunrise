#version 120
/* RENDERTARGETS: 1,2 */
varying vec2 uv;
void main() {
    gl_FragData[0] = vec4(uv, 0.25, 1.0);
    gl_FragData[1] = vec4(0.5, uv, 1.0);
}
