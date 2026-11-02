//! Owned query fixtures isolate the native movement handler from world geometry.
use mag_gameplay::acquisition_spawn_blockers::*;
use std::collections::VecDeque;
struct World {
    position: [f32; 3],
    contacts: VecDeque<Option<Contact>>,
    hits: VecDeque<Option<CastHit>>,
    moves: Vec<[f32; 3]>,
    cast_targets: Vec<[f32; 3]>,
}
impl BlockerQueries for World {
    fn position(&self) -> [f32; 3] {
        self.position
    }
    fn closest_spawn_contact(&mut self) -> Result<Option<Contact>, &'static str> {
        Ok(self.contacts.pop_front().expect("explicit contact fixture"))
    }
    fn cast_world(&mut self, target: [f32; 3]) -> Result<Option<CastHit>, &'static str> {
        self.cast_targets.push(target);
        Ok(self.hits.pop_front().expect("explicit world cast fixture"))
    }
    fn move_character(&mut self, target: [f32; 3]) -> Result<(), &'static str> {
        self.position = target;
        self.moves.push(target);
        Ok(())
    }
}
fn world(contacts: Vec<Option<Contact>>, hits: Vec<Option<CastHit>>) -> World {
    World {
        position: [0.; 3],
        contacts: contacts.into(),
        hits: hits.into(),
        moves: vec![],
        cast_targets: vec![],
    }
}
fn contact(depth: f32) -> Option<Contact> {
    Some(Contact {
        signed_distance: -depth,
        normal: [1., 0., 0.],
    })
}
#[test]
fn character_moves_to_full_clear_target_without_epsilon_in_hit_fraction() {
    let mut w = world(vec![contact(0.1), None], vec![None]);
    assert!(resolve_character(&mut w).unwrap());
    assert!((w.position[0] - 0.101).abs() < 1e-7);
    assert_eq!(w.moves.len(), 1);
    let mut deep = world(vec![contact(0.125)], vec![]);
    assert!(!resolve_character(&mut deep).unwrap());
    assert!(deep.moves.is_empty());
}
#[test]
fn blocked_tangent_does_not_move_before_validation() {
    let mut w = world(
        vec![contact(0.1)],
        vec![Some(CastHit {
            fraction: 0.5,
            normal: [-1., 0., 0.],
        })],
    );
    assert!(!resolve_character(&mut w).unwrap());
    assert_eq!(w.position, [0.; 3]);
    assert!(w.moves.is_empty());
}
#[test]
fn second_hit_keeps_both_intermediate_moves_before_unresolved_result() {
    let n = std::f32::consts::FRAC_1_SQRT_2;
    let mut w = world(
        vec![contact(0.1)],
        vec![
            Some(CastHit {
                fraction: 0.5,
                normal: [-n, n, 0.],
            }),
            Some(CastHit {
                fraction: 0.5,
                normal: [0., 0., 1.],
            }),
        ],
    );
    assert!(!resolve_character(&mut w).unwrap());
    assert_eq!(w.moves.len(), 2);
    assert!((w.moves[0][0] - 0.05).abs() < 1e-7);
    assert!((w.position[0] - 0.075).abs() < 1e-6);
    assert!((w.position[1] - 0.025).abs() < 1e-6);
    assert!((w.cast_targets[0][0] - 0.101).abs() < 1e-7);
}
#[test]
fn two_outer_pushes_are_followed_by_final_contact_check_without_third_move() {
    let mut w = world(
        vec![contact(0.1), contact(0.1), contact(0.1)],
        vec![None, None],
    );
    assert!(!resolve_character(&mut w).unwrap());
    assert_eq!(w.moves.len(), 2);
    assert!((w.position[0] - 0.202).abs() < 1e-7);
}
