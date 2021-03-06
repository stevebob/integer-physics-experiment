use fnv::FnvHashMap;
use cgmath::{InnerSpace, Vector2, vec2};
use pixel_num::sub_pixel_i64::{self, SubPixelI64};
use shape::Shape;
use axis_aligned_rect::AxisAlignedRect;
use loose_quad_tree::LooseQuadTree;
use line_segment::LineSegment;
use best::BestMap;
use num::Zero;

#[derive(Default, Debug)]
pub struct InputModel {
    left: SubPixelI64,
    right: SubPixelI64,
    up: SubPixelI64,
    down: SubPixelI64,
}

fn clamp_float(value: f32) -> SubPixelI64 {
    SubPixelI64::new_pixels_f32(value).clamp_zero_one_pixel()
}

impl InputModel {
    pub fn set_left(&mut self, value: f32) {
        self.left = clamp_float(value);
    }
    pub fn set_right(&mut self, value: f32) {
        self.right = clamp_float(value);
    }
    pub fn set_up(&mut self, value: f32) {
        self.up = clamp_float(value);
    }
    pub fn set_down(&mut self, value: f32) {
        self.down = clamp_float(value);
    }
    fn horizontal(&self) -> SubPixelI64 {
        self.right - self.left
    }
    fn vertical(&self) -> SubPixelI64 {
        self.down - self.up
    }
    fn movement(&self) -> Vector2<SubPixelI64> {
        sub_pixel_i64::normalize_vector_if_longer_than_one(vec2(
            self.horizontal(),
            self.vertical(),
        ))
    }
}

fn update_player_velocity(
    _current_velocity: Vector2<SubPixelI64>,
    input_model: &InputModel,
) -> Vector2<SubPixelI64> {
    const MULTIPLIER: i64 = 4;
    input_model.movement() * SubPixelI64::new(MULTIPLIER)
}

pub type EntityId = u32;

#[derive(Default)]
struct EntityIdAllocator {
    next: u32,
}

impl EntityIdAllocator {
    fn allocate(&mut self) -> EntityId {
        let id = self.next;
        self.next += 1;
        id
    }
    fn reset(&mut self) {
        self.next = 0;
    }
}

#[derive(Debug)]
struct SpatialInfo {
    entity_id: EntityId,
}

type SpatialLooseQuadTree = LooseQuadTree<SpatialInfo, SubPixelI64>;

pub struct RenderUpdate<'a> {
    pub position: Vector2<SubPixelI64>,
    pub shape: &'a Shape<SubPixelI64>,
    pub colour: [f32; 3],
}

pub struct GameState {
    player_id: Option<EntityId>,
    entity_id_allocator: EntityIdAllocator,
    position: FnvHashMap<EntityId, Vector2<SubPixelI64>>,
    shape: FnvHashMap<EntityId, Shape<SubPixelI64>>,
    colour: FnvHashMap<EntityId, [f32; 3]>,
    velocity: FnvHashMap<EntityId, Vector2<SubPixelI64>>,
    quad_tree: SpatialLooseQuadTree,
}

enum MovementStep {
    NoMovement,
    NoCollision {
        destination: Vector2<SubPixelI64>,
    },
    Collision {
        allowed_movement: Vector2<SubPixelI64>,
        destination: Vector2<SubPixelI64>,
        line_segment: LineSegment<SubPixelI64>,
    },
}

fn movement_step(
    id: EntityId,
    position: Vector2<SubPixelI64>,
    position_table: &FnvHashMap<EntityId, Vector2<SubPixelI64>>,
    shape_table: &FnvHashMap<EntityId, Shape<SubPixelI64>>,
    quad_tree: &SpatialLooseQuadTree,
    movement: Vector2<SubPixelI64>,
) -> MovementStep {
    if movement.x.is_zero() && movement.y.is_zero() {
        return MovementStep::NoMovement;
    }
    if let Some(shape) = shape_table.get(&id) {
        let mut closest_collision = BestMap::new();
        let start_aabb = shape.aabb(position);
        let end_aabb = shape.aabb(position + movement);
        let aabb = start_aabb.union(&end_aabb);
        quad_tree.for_each_intersection(
            &aabb,
            |_other_aabb,
             SpatialInfo {
                 entity_id: other_id,
             }| {
                if *other_id != id {
                    if let Some(stationary_position) = position_table.get(other_id) {
                        if let Some(stationary_shape) = shape_table.get(other_id) {
                            if let Some(collision_info) = shape.movement_collision_test(
                                position,
                                stationary_shape,
                                *stationary_position,
                                movement,
                            ) {
                                closest_collision.insert_le(
                                    collision_info.magnitude2,
                                    (
                                        collision_info.allowed_movement,
                                        collision_info.line_segment,
                                    ),
                                );
                            }
                        }
                    }
                }
            },
        );
        return match closest_collision.into_value() {
            None => MovementStep::NoCollision {
                destination: position + movement,
            },
            Some((allowed_movement, line_segment)) => MovementStep::Collision {
                allowed_movement,
                destination: position + allowed_movement,
                line_segment,
            },
        };
    }
    MovementStep::NoMovement
}

fn position_after_movement(
    id: EntityId,
    position_table: &FnvHashMap<EntityId, Vector2<SubPixelI64>>,
    shape_table: &FnvHashMap<EntityId, Shape<SubPixelI64>>,
    quad_tree: &SpatialLooseQuadTree,
    mut movement: Vector2<SubPixelI64>,
) -> Option<Vector2<SubPixelI64>> {
    let mut position = if let Some(position) = position_table.get(&id) {
        *position
    } else {
        return None;
    };
    const MAX_ITERATIONS: usize = 16;
    for _ in 0..MAX_ITERATIONS {
        match movement_step(
            id,
            position,
            position_table,
            shape_table,
            quad_tree,
            movement,
        ) {
            MovementStep::NoMovement => return Some(position),
            MovementStep::NoCollision { destination } => return Some(destination),
            MovementStep::Collision {
                allowed_movement,
                destination,
                line_segment,
            } => {
                position = destination;
                let remaining_movement =
                    sub_pixel_i64::vector_to_f32_sub_pixel(movement - allowed_movement);
                let collision_surface_direction =
                    sub_pixel_i64::vector_to_f32_sub_pixel(line_segment.vector())
                        .normalize();
                let slide_movement_float =
                    remaining_movement.project_on(collision_surface_direction);
                let remaining_to_slide_direction =
                    (slide_movement_float - remaining_movement).normalize();
                let padding = remaining_to_slide_direction * 0.1
                    * sub_pixel_i64::SUB_PIXELS_PER_PIXEL as f32;
                let slide_movement = sub_pixel_i64::vector_from_f32_sub_pixel(
                    slide_movement_float + padding,
                );
                if sub_pixel_i64::vector_is_zero(slide_movement) {
                    break;
                }
                movement = slide_movement
            }
        }
    }
    Some(position)
}

impl GameState {
    pub fn new(size_hint: Vector2<f32>) -> Self {
        Self {
            player_id: None,
            entity_id_allocator: Default::default(),
            position: Default::default(),
            shape: Default::default(),
            colour: Default::default(),
            velocity: Default::default(),
            quad_tree: LooseQuadTree::new(vec2(
                SubPixelI64::new_pixels_f32(size_hint.x),
                SubPixelI64::new_pixels_f32(size_hint.y),
            )),
        }
    }
    fn clear(&mut self) {
        self.player_id = None;
        self.entity_id_allocator.reset();
        self.position.clear();
        self.shape.clear();
        self.colour.clear();
        self.velocity.clear();
    }
    fn add_entity(
        &mut self,
        position: Vector2<f32>,
        shape: Shape<SubPixelI64>,
        colour: [f32; 3],
    ) -> EntityId {
        let id = self.entity_id_allocator.allocate();
        let position = vec2(
            SubPixelI64::new_pixels_f32(position.x),
            SubPixelI64::new_pixels_f32(position.y),
        );
        self.position.insert(id, position);

        self.quad_tree
            .insert(shape.aabb(position), SpatialInfo { entity_id: id });
        self.shape.insert(id, shape);
        self.colour.insert(id, colour);
        id
    }
    pub fn init_demo(&mut self) {
        self.clear();
        let player_id = self.add_entity(
            vec2(200., 50.),
            Shape::AxisAlignedRect(AxisAlignedRect::new(vec2(
                SubPixelI64::new_pixels_f32(32.),
                SubPixelI64::new_pixels_f32(64.),
            ))),
            [1., 0., 0.],
        );

        self.player_id = Some(player_id);
        self.velocity.insert(
            player_id,
            vec2(
                SubPixelI64::new_pixels_f32(0.),
                SubPixelI64::new_pixels_f32(0.),
            ),
        );

        self.add_entity(
            vec2(50., 200.),
            Shape::AxisAlignedRect(AxisAlignedRect::new(vec2(
                SubPixelI64::new_pixels_f32(400.),
                SubPixelI64::new_pixels_f32(20.),
            ))),
            [1., 1., 0.],
        );

        self.add_entity(
            vec2(20., 20.),
            Shape::LineSegment(LineSegment::new(
                vec2(Zero::zero(), Zero::zero()),
                vec2(
                    SubPixelI64::new_pixels_f32(50.),
                    SubPixelI64::new_pixels_f32(100.),
                ),
            )),
            [0., 1., 0.],
        );
    }
    pub fn render_updates(&self) -> impl Iterator<Item = RenderUpdate> {
        let position = &self.position;
        position.iter().filter_map(move |(id, &position)| {
            self.shape.get(id).and_then(|shape| {
                self.colour.get(id).map(|&colour| RenderUpdate {
                    position,
                    shape,
                    colour,
                })
            })
        })
    }
    pub fn update(&mut self, input_model: &InputModel) {
        let player_id = self.player_id.expect("No player id");
        if let Some(velocity) = self.velocity.get_mut(&player_id) {
            *velocity = update_player_velocity(*velocity, input_model);
        }
        for (id, velocity) in self.velocity.iter() {
            if let Some(position) = position_after_movement(
                *id,
                &self.position,
                &self.shape,
                &self.quad_tree,
                *velocity,
            ) {
                self.position.insert(*id, position);
            }
        }
    }
}
