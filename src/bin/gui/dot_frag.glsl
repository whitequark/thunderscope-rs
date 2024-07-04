#version 300 es
precision mediump float;

uniform vec3 channel_color;
uniform vec2 resolution;

flat in float adc_data_scaled;
in vec2 relative_offset;

out vec4 frag_color;

void main() {
    float a = sqrt(relative_offset.x * relative_offset.x +
                   relative_offset.y * relative_offset.y);

    frag_color = vec4(channel_color, 1.0f - a * a);
}
