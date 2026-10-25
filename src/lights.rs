use glam::Vec3;

pub const MAX_LIGHTS: usize = 8;

/// A dynamic light source. Supports point lights and spot lights.
#[derive(Clone, Copy)]
pub struct Light {
    pub pos: Vec3,
    pub radius: f32,        // falloff distance
    pub dir: Vec3,          // direction (only for spots)
    pub cos_half_angle: f32, // 0.0 = point light, >0 = spot (cosine of half-angle)
    pub color: Vec3,
    pub intensity: f32,
}

impl Light {
    pub fn spot(pos: Vec3, dir: Vec3, radius: f32, half_angle_deg: f32, color: Vec3, intensity: f32) -> Self {
        Self {
            pos, radius, dir: dir.normalize(),
            cos_half_angle: half_angle_deg.to_radians().cos(),
            color, intensity,
        }
    }
}

/// Packed light data ready to be sent as uniforms.
/// Layout: 3 vec4s per light × MAX_LIGHTS, plus light_count.
pub struct LightUniforms {
    pub data: [[f32; 4]; MAX_LIGHTS * 3], // 3 vec4s per light
    pub count: u32,
}

impl LightUniforms {
    pub fn from_lights(lights: &[Light]) -> Self {
        let mut data = [[0.0_f32; 4]; MAX_LIGHTS * 3];
        let count = lights.len().min(MAX_LIGHTS);

        for (i, light) in lights.iter().take(MAX_LIGHTS).enumerate() {
            let base = i * 3;
            data[base]     = [light.pos.x, light.pos.y, light.pos.z, light.radius];
            data[base + 1] = [light.dir.x, light.dir.y, light.dir.z, light.cos_half_angle];
            data[base + 2] = [light.color.x, light.color.y, light.color.z, light.intensity];
        }

        Self { data, count: count as u32 }
    }
}
