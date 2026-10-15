//! Turbo pads: a flat strip of glowing chevrons on the ground that fires CTR's turbo pad
//! ([`ctr_kart::Kart::turbo_pad`]) every tick the kart is on it. CTR marks its pads in the
//! track's quadblocks (`COLL_STEP_TRIGGER_TURBO_PAD`); here a course names them
//! (`courses.json`'s `"pads"`, `docs/courses.md`), the host hands them over ([`PadField::set`]),
//! each is laid onto the ground under it once that ground has streamed in, and the session
//! fires the ones it drives over ([`crate::Session::set_pads`]).

use benilla_assets::coords::wow_to_bevy;
use benilla_world::collision::WorldCollision;
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

/// A CTR turbo pad's side, level units: CTR's pads are single trigger quadblocks, and across its
/// tracks both kinds measure a median 384 by 384 (the disc's quadblocks with `quadFlags & 0x40`
/// and turbo `terrain_type`).
const PAD_LU: f32 = 384.0;
/// A pad's length along the course and width across it, yards: CTR's, to the kart as CTR's kart.
pub const PAD_LENGTH: f32 = PAD_LU * crate::METERS_PER_LU / crate::METERS_PER_YARD;
pub const PAD_WIDTH: f32 = PAD_LENGTH;
/// How far above or below the pad (yards) the kart's origin still counts as on it.
pub const PAD_REACH: f32 = 1.5;
/// Pads are laid (their ground found and their mesh built) within this many yards of the camera.
const LAY_RANGE: f32 = 300.0;
/// How far above the ground (yards) the decal floats, against z-fighting.
const LIFT: f32 = 0.06;
/// The chevrons' pulse, Hz.
const PULSE_HZ: f32 = 1.5;

/// One pad: its centre on the ground and its forward, both Bevy world.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pad {
    pub centre: Vec3,
    /// Unit, level: the way the pad points.
    pub forward: Vec3,
}

impl Pad {
    /// A pad at WoW `x, y, z` pointing along WoW orientation `o` (radians from +x north toward
    /// +y west, as a creature's facing).
    pub fn from_wow(x: f32, y: f32, z: f32, o: f32) -> Self {
        Self {
            centre: wow_to_bevy([x, y, z]),
            forward: wow_to_bevy([o.cos(), o.sin(), 0.0]).normalize_or(Vec3::NEG_Z),
        }
    }

    fn right(&self) -> Vec3 {
        self.forward.cross(Vec3::Y)
    }

    /// Whether a kart whose origin is at `p` (Bevy world) is on the pad.
    pub fn contains(&self, p: Vec3) -> bool {
        let d = p - self.centre;
        d.dot(self.forward).abs() <= PAD_LENGTH * 0.5
            && d.dot(self.right()).abs() <= PAD_WIDTH * 0.5
            && d.y.abs() <= PAD_REACH
    }

    /// The pad-local point `(across, along)` (yards, +along forward) in the world, at `height`.
    fn at(&self, across: f32, along: f32) -> Vec3 {
        self.centre + self.right() * across + self.forward * along
    }
}

/// The pad's mesh, Bevy world, each vertex dropped onto the ground `height` finds under it: a
/// dim green plate and three bright green chevrons pointing forward; colours in the vertices.
pub fn pad_mesh(pad: &Pad, mut height: impl FnMut(Vec3) -> Option<f32>) -> Option<Mesh> {
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut colors: Vec<[f32; 4]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut quad = |corners: [(f32, f32); 4], color: [f32; 4]| -> Option<()> {
        let base = positions.len() as u32;
        for (x, z) in corners {
            let p = pad.at(x, z);
            let y = height(p)? + LIFT;
            positions.push([p.x, y, p.z]);
            colors.push(color);
        }
        indices.extend([base, base + 2, base + 1, base, base + 3, base + 2]);
        Some(())
    };
    let (hw, hl) = (PAD_WIDTH * 0.5, PAD_LENGTH * 0.5);
    // The plate, in strips so it follows the ground's slope.
    const STRIPS: usize = 4;
    for i in 0..STRIPS {
        let z0 = -hl + PAD_LENGTH * i as f32 / STRIPS as f32;
        let z1 = -hl + PAD_LENGTH * (i + 1) as f32 / STRIPS as f32;
        quad(
            [(-hw, z0), (hw, z0), (hw, z1), (-hw, z1)],
            [0.05, 0.55, 0.12, 0.45],
        )?;
    }
    // Three chevrons, each two arms meeting at a tip ahead.
    let (arm, depth) = (PAD_LENGTH * 0.09, PAD_LENGTH * 0.18);
    let reach = hw * 0.8;
    for k in 0..3 {
        let tip = -hl + PAD_LENGTH * (0.36 + 0.29 * k as f32);
        let tail = tip - depth;
        let color = [0.35, 1.0, 0.3, 0.95];
        // Left arm: from the left tail to the tip, `arm` thick along the course.
        quad(
            [
                (-reach, tail - arm),
                (0.0, tip - arm),
                (0.0, tip),
                (-reach, tail),
            ],
            color,
        )?;
        quad(
            [
                (0.0, tip - arm),
                (reach, tail - arm),
                (reach, tail),
                (0.0, tip),
            ],
            color,
        )?;
    }
    let n = positions.len();
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; n]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32(indices));
    Some(mesh)
}

/// A pad the host named, and once laid, its ground and its drawn entity.
struct Spot {
    pad: Pad,
    laid: Option<(Pad, Entity)>,
}

/// The pads of the map we are on, as the host set them.
#[derive(Resource, Default)]
pub struct PadField {
    spots: Vec<Spot>,
    /// What [`PadField::set`] last had, to keep a repeat from relaying them.
    key: Vec<Pad>,
    material: Option<Handle<StandardMaterial>>,
    /// The laid pads changed since [`PadField::take_changed`] last looked.
    changed: bool,
}

impl PadField {
    /// The pads to lay and fire from now on; the same list again changes nothing.
    pub fn set(&mut self, pads: Vec<Pad>, commands: &mut Commands) {
        if pads == self.key {
            return;
        }
        for spot in self.spots.drain(..) {
            if let Some((_, e)) = spot.laid {
                commands.entity(e).despawn();
            }
        }
        self.spots = pads.iter().map(|&pad| Spot { pad, laid: None }).collect();
        self.key = pads;
        self.changed = true;
    }

    /// The pads laid on the ground so far, when they changed since the last look.
    pub fn take_changed(&mut self) -> Option<Vec<Pad>> {
        std::mem::take(&mut self.changed).then(|| {
            self.spots
                .iter()
                .filter_map(|s| s.laid.map(|(p, _)| p))
                .collect()
        })
    }
}

/// The ground's height under `p` (Bevy world), the first face down from well above it.
fn ground_y(collide: &WorldCollision, p: Vec3) -> Option<f32> {
    let top = p.y + 40.0;
    collide
        .ray_body(Vec3::new(p.x, top, p.z), Dir3::NEG_Y, 120.0)
        .map(|hit| top - hit.distance)
}

/// Lays the pads near the camera onto the ground once it is solid under them, and pulses their
/// glow.
pub fn lay(
    mut commands: Commands,
    time: Res<Time>,
    mut field: ResMut<PadField>,
    collide: WorldCollision,
    camera: Query<&GlobalTransform, With<benilla_world::view::WorldCamera>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if field.spots.is_empty() {
        return;
    }
    let material = field
        .material
        .get_or_insert_with(|| {
            materials.add(StandardMaterial {
                base_color: Color::WHITE,
                unlit: true,
                alpha_mode: AlphaMode::Blend,
                double_sided: true,
                cull_mode: None,
                ..default()
            })
        })
        .clone();
    let glow = 0.75 + 0.25 * (std::f32::consts::TAU * PULSE_HZ * time.elapsed_secs()).sin();
    if let Some(m) = materials.get_mut(&material) {
        m.base_color = Color::linear_rgba(1.0, 1.0, 1.0, glow);
    }
    let Ok(eye) = camera.single().map(GlobalTransform::translation) else {
        return;
    };
    let mut laid_any = false;
    for spot in field.spots.iter_mut().filter(|s| s.laid.is_none()) {
        let flat = Vec3::new(spot.pad.centre.x - eye.x, 0.0, spot.pad.centre.z - eye.z);
        if flat.length() > LAY_RANGE {
            continue;
        }
        // The course's height (else the camera's) is only where the look down starts.
        let (x, z) = (spot.pad.centre.x, spot.pad.centre.z);
        let Some(y) = [spot.pad.centre.y, eye.y]
            .into_iter()
            .find_map(|start| ground_y(&collide, Vec3::new(x, start, z)))
        else {
            continue;
        };
        let pad = Pad {
            centre: Vec3::new(x, y, z),
            ..spot.pad
        };
        let Some(mesh) = pad_mesh(&pad, |p| ground_y(&collide, Vec3::new(p.x, y, p.z))) else {
            continue;
        };
        let entity = commands
            .spawn((
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(material.clone()),
                Transform::IDENTITY,
                bevy::camera::visibility::NoFrustumCulling,
                Name::new("turbo pad"),
            ))
            .id();
        spot.laid = Some((pad, entity));
        laid_any = true;
    }
    field.changed |= laid_any;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pad_points_along_its_orientation_and_holds_a_kart_on_it_only() {
        // Facing north (+x): Bevy's −z.
        let pad = Pad::from_wow(100.0, 50.0, 10.0, 0.0);
        assert!((pad.forward - Vec3::NEG_Z).length() < 1e-5);
        assert_eq!(pad.centre, wow_to_bevy([100.0, 50.0, 10.0]));
        let on = |dx: f32, dy: f32, dz: f32| pad.contains(pad.centre + Vec3::new(dx, dy, dz));
        assert!(on(0.0, 0.3, 0.0));
        assert!(on(0.0, 0.0, -PAD_LENGTH * 0.49), "the front end");
        assert!(on(PAD_WIDTH * 0.49, 0.0, 0.0), "the side");
        assert!(!on(0.0, 0.0, PAD_LENGTH * 0.51), "behind it");
        assert!(!on(PAD_WIDTH * 0.51, 0.0, 0.0), "beside it");
        assert!(!on(0.0, PAD_REACH + 0.1, 0.0), "over it in the air");
        // Facing west (+y, a quarter turn): Bevy's −x.
        let west = Pad::from_wow(0.0, 0.0, 0.0, std::f32::consts::FRAC_PI_2);
        assert!((west.forward - Vec3::NEG_X).length() < 1e-5);
    }

    #[test]
    fn the_mesh_drapes_over_the_ground_or_waits_for_it() {
        let pad = Pad::from_wow(0.0, 0.0, 0.0, 0.0);
        let mesh = pad_mesh(&pad, |p| Some(p.z * 0.1)).expect("ground everywhere");
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(ps)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("positions");
        };
        // A plate of four strips and three two-armed chevrons, each a quad.
        assert_eq!(ps.len(), (4 + 3 * 2) * 4);
        for p in ps {
            assert!((p[1] - (p[2] * 0.1 + LIFT)).abs() < 1e-4, "{p:?}");
        }
        assert!(pad_mesh(&pad, |p| (p.x < 1.0).then_some(0.0)).is_none());
    }
}
