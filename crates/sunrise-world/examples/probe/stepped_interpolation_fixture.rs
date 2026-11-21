//! The Rust side of tools/test_stepped_interpolation_parity.py: replays the
//! harness's events (VanillaSteppedInterpolationOracle's input format) on
//! `SteppedInterpolation`, `mob_motion` and `RemotePlayerMotion`, and writes the
//! entity after each one.
use std::fmt::Write as _;
use sunrise_world::stepped_interpolation::{
    Path, PosRot, RemotePlayerMotion, Start, SteppedInterpolation, mob_motion,
};

/// `Entity.setRot`: yaw and pitch wrapped, pitch clamped (`setXRot`).
fn set_rot(entity: &mut PosRot, yaw: f32, pitch: f32) {
    entity.yaw = yaw % 360.0;
    entity.pitch = (pitch % 360.0 % 360.0).clamp(-90.0, 90.0);
}

pub fn main() {
    let args: Vec<String> = crate::args().collect();
    let input = std::fs::read_to_string(&args[1]).expect("input");
    let mut out = String::new();
    let mut handler = SteppedInterpolation::new(0);
    let mut entity = PosRot::default();
    let mut motion = [0.0f64; 3];
    let mut player = None::<RemotePlayerMotion>;
    for line in input.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        let num = |i: usize| f[i].parse::<f64>().expect("number");
        let what = match f[0] {
            "case" => {
                handler = SteppedInterpolation::new(f[1].parse().expect("steps"));
                entity = PosRot {
                    position: [num(2), num(3), num(4)],
                    yaw: f[5].parse().expect("yaw"),
                    pitch: f[6].parse().expect("pitch"),
                };
                motion = [0.0; 3];
                player = (f.get(7) == Some(&"P")).then(RemotePlayerMotion::default);
                "c"
            }
            "move" => {
                let mut i = 2;
                let path = match f[1] {
                    "L" => {
                        i += 3;
                        Some(Path::Linear([num(2), num(3), num(4)]))
                    }
                    "S" => {
                        let n: usize = f[2].parse().expect("count");
                        i = 3;
                        let mut steps = Vec::new();
                        for _ in 0..n {
                            steps.push((
                                [num(i), num(i + 1), num(i + 2)],
                                f[i + 3].parse().expect("ticks"),
                            ));
                            i += 4;
                        }
                        Some(Path::Stepped(steps))
                    }
                    _ => None,
                };
                let rotation = (f[i] == "R").then(|| {
                    (
                        f[i + 1].parse().expect("yaw"),
                        f[i + 2].parse().expect("pitch"),
                    )
                });
                match handler.interpolate_to(entity, path, rotation) {
                    Start::Snap(at) => {
                        entity.position = at.position;
                        set_rot(&mut entity, at.yaw, at.pitch);
                        "s"
                    }
                    Start::Unchanged => "k",
                    Start::Interpolating => "i",
                }
            }
            // Entity.lerpMotion, RemotePlayer.lerpMotion for a player.
            "motion" => {
                let to = [num(1), num(2), num(3)];
                match player.as_mut() {
                    Some(p) => p.lerp_to(to),
                    None => motion = to,
                }
                "m"
            }
            "tick" => {
                let what = match handler.interpolate() {
                    Some(at) => {
                        entity.position = at.position;
                        set_rot(&mut entity, at.yaw, at.pitch);
                        "t"
                    }
                    None => "n",
                };
                // LivingEntity.aiStep, RemotePlayer.aiStep for a player.
                motion = match player.as_mut() {
                    Some(p) => p.step(motion),
                    None => mob_motion(motion, handler.active()),
                };
                what
            }
            other => panic!("unknown event {other}"),
        };
        let p = entity.position;
        writeln!(
            out,
            "{what} {:x} {:x} {:x} {:x} {:x} {:x} {:x} {:x}",
            p[0].to_bits(),
            p[1].to_bits(),
            p[2].to_bits(),
            entity.yaw.to_bits(),
            entity.pitch.to_bits(),
            motion[0].to_bits(),
            motion[1].to_bits(),
            motion[2].to_bits()
        )
        .expect("write");
    }
    std::fs::write(&args[2], out).expect("output");
}
