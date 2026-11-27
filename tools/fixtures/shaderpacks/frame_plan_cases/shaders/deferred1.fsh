#version 120
/* RENDERTARGETS: 2 */
uniform sampler2D colortex2;
varying vec2 uv;
void main() {
    gl_FragData[0] = texture2D(colortex2, uv) * 0.5 + vec4(0.0, 0.25, 0.0, 0.5);
}
