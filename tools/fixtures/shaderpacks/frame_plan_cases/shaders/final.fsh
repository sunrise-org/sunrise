#version 120
uniform sampler2D colortex3;
varying vec2 uv;
void main() {
    gl_FragColor = texture2D(colortex3, uv);
}
