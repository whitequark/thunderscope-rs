#version 300 es
precision highp float;

uniform bool draw_lines;
uniform vec3 channel_color;

flat in vec2 prim_size;
in vec2 prim_offset;

out vec4 frag_color;

void main() {
    float alpha = 1.0f;
    if (draw_lines) {
        if (prim_offset.x < 0.0f) { // endcap
            alpha = 0.5f;
            vec2 norm_offset = vec2(
                prim_offset.x / prim_size.y,
                prim_offset.y / prim_size.y
            );
            alpha = 1.0f - dot(norm_offset, norm_offset) * 2.0f;
        } else if (prim_offset.x > prim_size.x) { // endcap
            vec2 norm_offset = vec2(
                (prim_offset.x - prim_size.x) / prim_size.y,
                prim_offset.y / prim_size.y
            );
            alpha = 1.0f - dot(norm_offset, norm_offset) * 2.0f;
        } else { // body
            float norm_offset = prim_offset.y / prim_size.y;
            alpha = 1.0f - norm_offset * norm_offset;
        }
    } else {
        vec2 norm_offset = prim_offset / prim_size;
        alpha = 1.0f - dot(norm_offset, norm_offset);
    }
    frag_color = vec4(channel_color, alpha);
}
