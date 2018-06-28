/// Common transform logic
// requires: quat.vert

uniform sampler2D t_Entries;

struct Transform {
    vec3 pos;
    float scale;
    vec4 rot;
};

/// Apply the transform to a homogeneous vector.
vec3 transform(Transform t, vec4 hv) {
    return t.scale * qrot(t.rot, hv.xyz) + t.pos * hv.w;
}

/// Read a transform entry.
Transform fetch_entry_transform(int entry) {
    if (entry >= 0) {
        vec4 m_pos = texelFetch(t_Entries, ivec2(0, entry), 0);
        vec4 m_rot = texelFetch(t_Entries, ivec2(1, entry), 0);
        Transform tr = Transform(m_pos.xyz, m_pos.w, m_rot);
        return tr;
    } else {
        Transform tr = Transform(vec3(0.0), 1.0, vec4(0.0, 0.0, 0.0, 1.0));
        return tr;
    }
}
