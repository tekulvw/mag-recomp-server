// SYNTHETIC: the mesh is the generated floor stack of `goal_tests`; no game data.
use super::*;
use std::sync::{Arc, mpsc};

fn mesh() -> Arc<NavMesh> {
    Arc::new(super::goal_tests::stack())
}

/// The mesh with the one shared query context of [`SHARED_QUERY_ENV`].
fn shared_mesh() -> Arc<NavMesh> {
    let mut mesh = super::goal_tests::stack();
    mesh.share_one_query();
    Arc::new(mesh)
}

/// Route on another thread; returns when it finished.
fn background_route(mesh: &Arc<NavMesh>) -> mpsc::Receiver<Instant> {
    let (done, finished) = mpsc::channel();
    let mesh = Arc::clone(mesh);
    std::thread::spawn(move || {
        let route = mesh.route([-4.6, 0., -4.6], [0.7, 0., 0.7], 0.3).unwrap();
        assert_eq!(route.status, Status::Complete);
        done.send(Instant::now()).unwrap();
    });
    finished
}

#[test]
fn a_query_runs_while_another_thread_holds_a_query_context() {
    let mesh = mesh();
    // As a worker's long search would.
    let busy = mesh.lock_native().unwrap();
    background_route(&mesh)
        .recv_timeout(Duration::from_secs(30))
        .expect("the query waited for the other context");
    drop(busy);
    assert_eq!(mesh.query_contexts().0, 2);
}

#[test]
fn a_background_query_does_not_wait_for_the_world_tick() {
    let mesh = mesh();
    let _tick = mesh.foreground();
    let began = Instant::now();
    let at = background_route(&mesh).recv().unwrap();
    assert!(at.duration_since(began) < BACKGROUND_WAIT_LIMIT);
}

#[test]
fn idle_query_contexts_are_reused() {
    let mesh = mesh();
    for _ in 0..4 {
        background_route(&mesh).recv().unwrap();
    }
    let (made, bytes) = mesh.query_contexts();
    assert_eq!(made, 1);
    // 16,384 search nodes: about 0.6 MiB of Detour state per context.
    assert!((256 << 10..2 << 20).contains(&bytes), "{bytes}");
}

#[test]
fn a_background_query_waits_for_the_world_tick_that_is_running() {
    let mesh = shared_mesh();
    let tick = mesh.foreground();
    let finished = background_route(&mesh);
    // The tick's own queries go straight through.
    let route = mesh.route([-4.6, 0., -4.6], [0.7, 0., 0.7], 0.3).unwrap();
    assert_eq!(route.status, Status::Complete);
    assert!(
        finished.recv_timeout(Duration::from_millis(50)).is_err(),
        "the background query ran during the tick"
    );
    let released = Instant::now();
    drop(tick);
    let at = finished.recv().unwrap();
    assert!(at >= released);
}

#[test]
fn a_background_query_runs_at_once_between_ticks() {
    let mesh = shared_mesh();
    drop(mesh.foreground());
    let began = Instant::now();
    background_route(&mesh).recv().unwrap();
    assert!(began.elapsed() < BACKGROUND_WAIT_LIMIT);
}

#[test]
fn a_world_that_never_stops_ticking_cannot_starve_background_queries() {
    let mesh = shared_mesh();
    let _tick = mesh.foreground();
    let began = Instant::now();
    let at = background_route(&mesh).recv().unwrap();
    assert!(at.duration_since(began) >= BACKGROUND_WAIT_LIMIT);
}

#[test]
fn the_guard_restores_the_thread_and_nests() {
    let mesh = shared_mesh();
    let other = super::goal_tests::stack();
    let outer = mesh.foreground();
    {
        let _inner = other.foreground();
        assert!(FOREGROUND.with(Cell::get));
    }
    assert!(FOREGROUND.with(Cell::get), "the outer tick still runs");
    assert_eq!(mesh.foreground.load(Ordering::Acquire), 1);
    drop(outer);
    assert!(!FOREGROUND.with(Cell::get));
    assert_eq!(mesh.foreground.load(Ordering::Acquire), 0);
}
