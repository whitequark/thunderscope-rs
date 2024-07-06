#version 300 es
precision highp float;

const float thickness = 2.0f;

const vec2 point_quad[] = vec2[](
    vec2(-1.0f, 1.0f),
    vec2(-1.0f,-1.0f),
    vec2( 1.0f, 1.0f),
    vec2( 1.0f,-1.0f)
);

const vec2 line_quad[] = vec2[](
    vec2(0.0f,  1.0f),
    vec2(0.0f, -1.0f),
    vec2(1.0f,  1.0f),
    vec2(1.0f, -1.0f)
);

mat2 line_rotation(vec2 line_a, vec2 line_b) {
    vec2 norm_ab = normalize(line_b - line_a);
    return mat2(
        norm_ab.x,  norm_ab.y,
        norm_ab.y, -norm_ab.x
    );
}

uniform vec2 resolution;
uniform int sample_count;
uniform bool draw_lines;

in float sample_value0;
in float sample_value1;

flat out vec2 prim_size;
out vec2 prim_offset;

vec2 project_sample(int index, float value) {
    return vec2(
        float(resolution.x) * (float(index) / float(sample_count - 1)),
        float(resolution.y) * (0.5f + value / 2.0f)
    );
}

void main() {
    vec2 screen_position;
    if (draw_lines) {
        if (gl_InstanceID + 1 == sample_count) {
            gl_Position = vec4(0.0f, 0.0f, 0.0f, 0.0f);
            return;
        }
        vec2 line_a = project_sample(gl_InstanceID + 0, sample_value0);
        vec2 line_b = project_sample(gl_InstanceID + 1, sample_value1);
        prim_size = vec2(distance(line_a, line_b), thickness);
        prim_offset = line_quad[gl_VertexID] *
            mat2(prim_size.x + 2.0f * thickness, 0.0f, 0.0f, thickness) -
            vec2(thickness, 0.0f);
        screen_position = line_a + prim_offset * line_rotation(line_a, line_b);
    } else /* draw points */ {
        vec2 point = project_sample(gl_InstanceID, sample_value0);
        prim_size = vec2(thickness, thickness);
        prim_offset = point_quad[gl_VertexID] * prim_size;
        screen_position = point + prim_offset;
    }
    gl_Position = vec4(screen_position * 2.0f / resolution - vec2(1.0f, 1.0f), 0.0, 1.0);
}
