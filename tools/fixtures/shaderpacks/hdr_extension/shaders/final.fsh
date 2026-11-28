#version 120

// Who owns the tone curve (shaders.properties tests this option): the pack, or with it on, Sunrise.
//#define SUNRISE_CURVE

uniform sampler2D colortex0;
#ifdef SUNRISE_HDR
uniform int sunriseHdr;
uniform float sunriseHeadroom;
uniform float sunrisePaperWhite;
uniform float sunrisePeak;
uniform float sunriseBlackLevel;
#endif

varying vec2 texcoord;

void main() {
    // The world as the renderer's own programs left it in colortex0, in vanilla's gamma, to light.
    vec3 linearColor = pow(texture2D(colortex0, texcoord).rgb, vec3(2.2));
    if (texcoord.y > 0.9) {
        // The ramp, across the top tenth: 16 bands of 0.5, 1.0, ... 8.0 times paper white.
        linearColor = vec3(0.5 * (floor(texcoord.x * 16.0) + 1.0));
    } else if (texcoord.y > 0.8) {
        // The display, in the next tenth, in seven parts. Black where Sunrise's HDR isn't served.
        float part = floor(texcoord.x * 7.0);
        linearColor = vec3(0.0);
#ifdef SUNRISE_HDR
#ifdef SUNRISE_CURVE
        bool curveAsked = true;
#else
        bool curveAsked = false;
#endif
#ifdef SUNRISE_HDR_TONEMAP_PACK
        bool packOwnsCurve = true;
#else
        bool packOwnsCurve = false;
#endif
        // Each value scaled to at most paper white, so no curve or hold reaches it: 203 and 1000 nits are the
        // harness's paper white and peak, 0.1 nits its black level with HDR on.
        if (part == 0.0) linearColor = vec3(float(sunriseHdr));
        if (part == 1.0) linearColor = vec3(sunriseHeadroom / 8.0);
        if (part == 2.0) linearColor = vec3(sunrisePaperWhite / 203.0);
        if (part == 3.0) linearColor = vec3(sunrisePeak / 1000.0);
        if (part == 4.0) linearColor = vec3(sunriseBlackLevel / 0.2);
        // Green where the curve's owner is the one the option asked for, red where it isn't.
        if (part == 5.0) linearColor = curveAsked != packOwnsCurve ? vec3(0.0, 1.0, 0.0) : vec3(1.0, 0.0, 0.0);
#endif
    }
#ifdef SUNRISE_HDR
#ifdef SUNRISE_HDR_TONEMAP_PACK
    // The pack owns the tone curve: what it writes is what the display shows, up to its peak.
    linearColor = min(linearColor, vec3(sunriseHeadroom));
#endif
    // Linear light, 1.0 at paper white: Sunrise stores it in the frame's encoding.
    gl_FragData[0] = vec4(linearColor, 1.0);
#else
    // Anywhere else: the SDR picture, clipped at white and in the 2.2 power, as before.
    gl_FragData[0] = vec4(pow(min(linearColor, vec3(1.0)), vec3(1.0 / 2.2)), 1.0);
#endif
    // The strip's last part: 6 times paper white past the pack's own hold, which Sunrise holds at the display's
    // peak, eases with its curve, or clips at white on an SDR frame. White on the SDR path.
    if (texcoord.y <= 0.9 && texcoord.y > 0.8 && floor(texcoord.x * 7.0) == 6.0) {
#ifdef SUNRISE_HDR
        gl_FragData[0] = vec4(6.0, 6.0, 6.0, 1.0);
#else
        gl_FragData[0] = vec4(1.0);
#endif
    }
}
