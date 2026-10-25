use glam::{Vec2, vec2};

use crate::circuit_data;

/// Game units per metre for the real circuits (cars are a little large for
/// them, which keeps the map to a size you can drive across).
pub const UNITS_PER_METRE: f32 = 1.0;

/// Circuit outlines as closed control-point loops in game units, each
/// starting at its start/finish line and running in the racing direction.
/// `layout` places them on the map. The first entry is the home circuit;
/// the rest are the real circuits' outlines (see `circuit_data`).
pub fn circuit_shapes() -> Vec<(&'static str, Vec<Vec2>)> {
    let mut circuits = Vec::new();

    // ── Stumbler Ring (home circuit, centre of the map) — the race track ──
    // Clockwise loop with no crossings: main straight south, sweeping turn
    // one, chicane on the back straight, esses up the west side, a hairpin
    // and a flowing north section back onto the straight.
    circuits.push(("STUMBLER RING", vec![
        vec2(1000.0, 0.0), vec2(1000.0, -350.0), vec2(1000.0, -700.0),
        // Turn one
        vec2(970.0, -860.0), vec2(880.0, -960.0), vec2(740.0, -1010.0),
        // Back straight with a chicane
        vec2(500.0, -1020.0), vec2(300.0, -1000.0), vec2(200.0, -950.0),
        vec2(100.0, -1010.0), vec2(-50.0, -1030.0), vec2(-300.0, -1000.0),
        // South-west corner
        vec2(-480.0, -930.0), vec2(-580.0, -800.0), vec2(-600.0, -620.0),
        // West esses
        vec2(-560.0, -400.0), vec2(-640.0, -200.0), vec2(-560.0, 0.0), vec2(-600.0, 200.0),
        // North-west hairpin
        vec2(-560.0, 380.0), vec2(-440.0, 480.0), vec2(-260.0, 500.0),
        // North section
        vec2(-50.0, 460.0), vec2(150.0, 350.0), vec2(350.0, 330.0),
        vec2(550.0, 420.0), vec2(700.0, 560.0), vec2(850.0, 600.0),
        // Final corner onto the straight
        vec2(960.0, 540.0), vec2(1000.0, 420.0), vec2(1000.0, 250.0),
    ]));

    for &(name, _length, outline) in circuit_data::CIRCUITS {
        circuits.push((name, outline.iter().map(|&[x, z]| vec2(x, z) * UNITS_PER_METRE).collect()));
    }

    circuits
}
