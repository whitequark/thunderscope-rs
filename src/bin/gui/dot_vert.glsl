#version 300 es
precision mediump float;

#define PI 3.1415926

const vec2 quad[] = vec2[](
    vec2(-1.0f, 1.0f),
    vec2(-1.0f,-1.0f),
    vec2( 1.0f, 1.0f),
    vec2( 1.0f,-1.0f)
);

uniform vec2 resolution;
uniform uint sample_count;

in float adc_data;

out vec2 relative_offset;
flat out float adc_data_scaled;

void main() {
    const float point_size = 2.0f;

    adc_data_scaled = (1.0f + adc_data) / 2.0f;

    vec2 point_offset;
    point_offset.x = (float(resolution.x) / float(sample_count)) * float(gl_InstanceID);
    point_offset.y = resolution.y * (0.5f + adc_data / 2.0f);

    relative_offset = quad[gl_VertexID];
    vec2 screen_position = vec2(point_offset + relative_offset * point_size);
    gl_Position = vec4(screen_position * 2.0f / resolution - vec2(1.0f, 1.0f), 0.0, 1.0);
}
