//! Bot sight lines answered ahead of a serial bot loop, on worker threads.
//!
//! A sight line's geometry answer (does the static world block it, and if
//! not, does a placed structure) depends only on its two endpoints and the
//! geometry standing during the phase. Its query visits the same nodes in
//! the same order whatever work budget it is given, so it spends the same
//! work every time the budget covers it, and fails identically otherwise.
//!
//! Before a bot loop, `SightPrefetch::compute` answers the lines that loop is
//! expected to ask, spread over worker threads, each under the phase's whole
//! work budget. The loop itself is unchanged and still runs on the tick
//! thread in its usual order. When it asks a line, a prefetched answer is
//! taken only if the work still left this tick covers the work that answer
//! spent, and that same work is charged. Anything else (a line nobody
//! predicted, or one the remaining budget would not cover) is asked of the
//! geometry directly, as before. Every result, budget and receipt is
//! therefore the same as a run without workers; the workers only move work
//! off the tick thread. Nothing here is shared mutably between threads:
//! workers read the geometry and each returns its own answers, which are
//! merged by line before the loop starts.
use crate::bot_firearms::Structures;
use crate::collision::{CollisionWorld, PROJECTILE_QUERY_LAYER};
use std::cell::Cell;
use std::collections::HashMap;

type Point = [f64; 3];
type Key = ([u64; 3], [u64; 3]);

fn key(from: Point, to: Point) -> Key {
    (from.map(f64::to_bits), to.map(f64::to_bits))
}

/// A query's answer and the work it spent.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Answer<T> {
    result: T,
    work: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Line {
    world: Answer<Result<bool, ()>>,
    /// Asked only when the world leaves the line clear, as the loop does.
    structures: Option<Answer<Result<bool, &'static str>>>,
}

/// How a phase's prefetched lines were used, for diagnostics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Usage {
    pub(crate) prefetched: u64,
    pub(crate) hits: u64,
    pub(crate) misses: u64,
}

impl Usage {
    fn add(&mut self, other: Usage) {
        self.prefetched += other.prefetched;
        self.hits += other.hits;
        self.misses += other.misses;
    }
}

impl<W: std::io::Write> crate::Engine<W> {
    /// Answer bot sight lines on `threads` threads (1, the default, keeps
    /// every query on the tick thread). Results are identical for any count.
    pub fn set_bot_threads(&mut self, threads: usize) {
        self.bot_threads = threads.max(1);
    }

    /// Prefetched sight lines so far: answered ahead, then used or missed.
    pub fn bot_sight_prefetch_usage(&self) -> (u64, u64, u64) {
        let u = self.bot_sight_usage;
        (u.prefetched, u.hits, u.misses)
    }

    pub(crate) fn record_sight_usage(&mut self, prefetch: Option<SightPrefetch>) {
        if let Some(prefetch) = prefetch {
            self.bot_sight_usage.add(prefetch.usage());
        }
    }
}

#[derive(Default)]
pub(crate) struct SightPrefetch {
    lines: HashMap<Key, Line>,
    hits: Cell<u64>,
    misses: Cell<u64>,
}

/// Does the static world block the line for bullets and sight? `Err` is a
/// failed or over-budget query.
fn world_blocks(
    world: &CollisionWorld,
    from: Point,
    to: Point,
    work: &mut usize,
) -> Result<bool, ()> {
    let row = world.query_row(PROJECTILE_QUERY_LAYER);
    world
        .raycast_row_bounded(from, to, row, work)
        .map(|query| query.nearest.is_some())
        .map_err(|_| ())
}

fn answer<T>(budget: usize, query: impl FnOnce(&mut usize) -> T) -> Answer<T> {
    let mut left = budget;
    let result = query(&mut left);
    Answer {
        result,
        work: budget - left,
    }
}

fn ask(
    world: &CollisionWorld,
    structures: Option<Structures<'_>>,
    budget: usize,
    from: Point,
    to: Point,
) -> Line {
    let world = answer(budget, |work| world_blocks(world, from, to, work));
    let structures = match (world.result, structures) {
        (Ok(false), Some(structures)) => {
            Some(answer(budget, |work| structures.raw_blocks(from, to, work)))
        }
        _ => None,
    };
    Line { world, structures }
}

impl SightPrefetch {
    /// Answer `lines` on up to `threads` threads (the calling thread is one of
    /// them), each under the whole `budget`. Duplicate lines are asked once.
    pub(crate) fn compute(
        world: &CollisionWorld,
        structures: Option<Structures<'_>>,
        budget: usize,
        lines: &[(Point, Point)],
        threads: usize,
    ) -> Self {
        let mut unique = Vec::with_capacity(lines.len());
        let mut seen = std::collections::HashSet::with_capacity(lines.len());
        for &(from, to) in lines {
            if seen.insert(key(from, to)) {
                unique.push((from, to));
            }
        }
        let answer_all = |chunk: &[(Point, Point)]| -> Vec<(Key, Line)> {
            chunk
                .iter()
                .map(|&(from, to)| (key(from, to), ask(world, structures, budget, from, to)))
                .collect()
        };
        let threads = threads.clamp(1, unique.len().max(1));
        let answered: Vec<(Key, Line)> = if threads == 1 {
            answer_all(&unique)
        } else {
            let size = unique.len().div_ceil(threads);
            std::thread::scope(|scope| {
                let mut chunks = unique.chunks(size);
                let first = chunks.next().unwrap_or(&[]);
                let workers: Vec<_> = chunks
                    .map(|chunk| scope.spawn(move || answer_all(chunk)))
                    .collect();
                let mut answered = answer_all(first);
                for worker in workers {
                    answered.extend(worker.join().expect("sight prefetch worker panicked"));
                }
                answered
            })
        };
        Self {
            lines: answered.into_iter().collect(),
            ..Default::default()
        }
    }

    fn line(&self, from: Point, to: Point) -> Option<&Line> {
        let line = self.lines.get(&key(from, to));
        let counter = if line.is_some() {
            &self.hits
        } else {
            &self.misses
        };
        counter.set(counter.get() + 1);
        line
    }

    pub(crate) fn usage(&self) -> Usage {
        Usage {
            prefetched: self.lines.len() as u64,
            hits: self.hits.get(),
            misses: self.misses.get(),
        }
    }
}

/// Take a prefetched answer if `work` covers it, charging its work.
fn replay<T: Copy>(answer: Option<Answer<T>>, work: &mut usize) -> Option<T> {
    let answer = answer.filter(|a| a.work <= *work)?;
    *work -= answer.work;
    Some(answer.result)
}

/// The static world's answer for a sight line, from `prefetch` when it holds
/// one the remaining `work` covers.
pub(crate) fn world_blocked(
    prefetch: Option<&SightPrefetch>,
    world: &CollisionWorld,
    from: Point,
    to: Point,
    work: &mut usize,
) -> Result<bool, ()> {
    let prefetched = prefetch
        .and_then(|p| p.line(from, to))
        .map(|line| line.world);
    replay(prefetched, work).unwrap_or_else(|| world_blocks(world, from, to, work))
}

/// The placed structures' raw answer for a sight line the world left clear.
pub(crate) fn structures_blocked(
    prefetch: Option<&SightPrefetch>,
    structures: Structures<'_>,
    from: Point,
    to: Point,
    work: &mut usize,
) -> Result<bool, &'static str> {
    let prefetched = prefetch
        .and_then(|p| p.lines.get(&key(from, to)))
        .and_then(|line| line.structures);
    replay(prefetched, work).unwrap_or_else(|| structures.raw_blocks(from, to, work))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn floor_with_wall() -> CollisionWorld {
        // A floor and a wall across x = 10, from z = 0 to 20, 4 m high.
        let world = serde_json::json!({"format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
            "metadata":{"map_name":"synthetic_prefetch","build":"synthetic","coverage":"provisional",
                "provenance":["Synthetic prefetch floor"],"unknowns":["Not a game map"],
                "physics_regions":[{"min":[-1.,-5.,-1.],"max":[41.,8.,41.],"ground_verified":true,
                    "obstacles_verified":true,"provenance":["Synthetic floor"]}]},
            "instances":[
                {"id":"floor","vertices_xyz":[[0.,0.,0.],[40.,0.,0.],[40.,0.,40.],[0.,0.,40.]],
                 "triangles":[[0,2,1],[0,3,2]]},
                {"id":"wall","vertices_xyz":[[10.,0.,0.],[10.,4.,0.],[10.,4.,20.],[10.,0.,20.]],
                 "triangles":[[0,1,2],[0,2,3],[0,2,1],[0,3,2]]}]});
        CollisionWorld::from_bytes(
            &serde_json::to_vec(&world).unwrap(),
            crate::collision::CollisionLimits::default(),
        )
        .unwrap()
    }

    fn lines() -> Vec<(Point, Point)> {
        let mut lines = Vec::new();
        for i in 0..40 {
            let z = f64::from(i);
            // Through the wall where it stands, past its end otherwise.
            lines.push(([2., 1.5, z], [30., 1.5, 39. - z]));
            lines.push(([2., 1.5, z], [8., 1.2, z * 0.5]));
        }
        lines
    }

    #[test]
    fn prefetched_answers_and_charges_match_direct_queries_for_any_thread_count() {
        let world = floor_with_wall();
        let lines = lines();
        for budget in [usize::MAX / 2, 64, 3, 0] {
            // Direct, serial answers and the work left after each.
            let mut direct = Vec::new();
            let mut work = budget;
            for &(from, to) in &lines {
                direct.push((world_blocks(&world, from, to, &mut work), work));
            }
            if budget > 1 << 20 {
                assert!(direct.iter().any(|(r, _)| *r == Ok(true)));
                assert!(direct.iter().any(|(r, _)| *r == Ok(false)));
            }
            for threads in [1, 2, 4, 7] {
                let prefetch = SightPrefetch::compute(&world, None, budget, &lines, threads);
                let mut work = budget;
                let replayed: Vec<_> = lines
                    .iter()
                    .map(|&(from, to)| {
                        (
                            world_blocked(Some(&prefetch), &world, from, to, &mut work),
                            work,
                        )
                    })
                    .collect();
                assert_eq!(replayed, direct, "budget {budget}, {threads} threads");
                let usage = prefetch.usage();
                assert_eq!((usage.hits, usage.misses), (lines.len() as u64, 0));
            }
        }
    }

    #[test]
    fn unpredicted_lines_fall_back_to_the_geometry() {
        let world = floor_with_wall();
        let lines = lines();
        let prefetch = SightPrefetch::compute(&world, None, 1 << 20, &lines[..10], 3);
        let (mut a, mut b) = (1 << 20, 1 << 20);
        for &(from, to) in &lines {
            assert_eq!(
                world_blocked(Some(&prefetch), &world, from, to, &mut a),
                world_blocked(None, &world, from, to, &mut b)
            );
            assert_eq!(a, b);
        }
        let usage = prefetch.usage();
        assert_eq!(usage.prefetched, 10);
        assert_eq!((usage.hits, usage.misses), (10, lines.len() as u64 - 10));
    }
}
