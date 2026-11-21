//! The copy a mod reads of a baked model (`RenderAssets::baked_model`): every quad field as the
//! mesher holds it, the particle named by its atlas layer.
use super::*;
use crate::models::catalog::BakedFace;
use sunrise_assets::{bake::ModelQuad, model::Material};

fn quad(facing: FaceDirection, cull: Option<FaceDirection>, sprite: &str) -> ModelQuad {
    ModelQuad {
        positions: [
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 1.0],
            [1.0, 1.0, 1.0],
            [1.0, 1.0, 0.0],
        ],
        uv: [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]],
        facing,
        cull,
        material: Material {
            sprite: sprite.into(),
            force_translucent: false,
        },
        tint_index: 0,
        shade_direction_override: None,
        light_emission: 7,
        light: [7, 0],
        color: None,
        ambient_occlusion: None,
        render_type: None,
        shade: false,
    }
}

#[test]
fn a_view_copies_each_quad_and_names_the_particle_sprite() {
    let model = BakedModel {
        ambient_occlusion: false,
        particle: Some(3),
        faces: vec![
            BakedFace::new(
                quad(
                    FaceDirection::Up,
                    Some(FaceDirection::Up),
                    "minecraft:block/stone",
                ),
                3,
                true,
                false,
                None,
            ),
            BakedFace::new(
                quad(FaceDirection::West, None, "minecraft:block/dirt"),
                4,
                true,
                false,
                None,
            ),
        ],
        cube: None,
    };
    let view = view(&model, |layer| {
        assert_eq!(layer, 3);
        Some("minecraft:block/stone".into())
    });
    assert!(!view.ambient_occlusion);
    assert_eq!(view.particle.as_deref(), Some("minecraft:block/stone"));
    assert_eq!(view.quads.len(), 2);
    let top = &view.quads[0];
    assert_eq!(top.positions, model.faces[0].quad.positions);
    assert_eq!(top.uvs, model.faces[0].quad.uv);
    assert_eq!(top.sprite, "minecraft:block/stone");
    assert_eq!(top.facing, Direction::Up);
    assert_eq!(top.cull, Some(Direction::Up));
    assert_eq!(top.tint_index, 0);
    assert!(!top.shade);
    assert_eq!(top.light_emission, 7);
    assert_eq!(view.quads[1].facing, Direction::West);
    assert_eq!(view.quads[1].cull, None);
    assert_eq!(view.quads[1].sprite, "minecraft:block/dirt");
}

#[test]
fn a_model_without_a_named_particle_sprite_has_none() {
    let model = BakedModel {
        ambient_occlusion: true,
        particle: Some(9),
        faces: Vec::new(),
        cube: None,
    };
    let view = view(&model, |_| None);
    assert_eq!(view.particle, None);
    assert!(view.quads.is_empty());
    let none = BakedModel {
        particle: None,
        ..model
    };
    assert_eq!(view_of(&none).particle, None);
}

fn view_of(model: &BakedModel) -> BakedModelView {
    view(model, |_| panic!("no particle layer to name"))
}

#[test]
fn every_face_direction_maps_to_the_api_direction_of_the_same_side() {
    for (face, side) in [
        (FaceDirection::Down, Direction::Down),
        (FaceDirection::Up, Direction::Up),
        (FaceDirection::North, Direction::North),
        (FaceDirection::South, Direction::South),
        (FaceDirection::West, Direction::West),
        (FaceDirection::East, Direction::East),
    ] {
        assert_eq!(direction(face), side);
    }
}
