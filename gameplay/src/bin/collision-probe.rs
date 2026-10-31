//! Offline batch query verification and timing against disk-loaded neutral data.
use mag_gameplay::collision::{CollisionLimits, CollisionWorld};
use serde::Deserialize;
use std::{io, path::PathBuf, time::Instant};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Queries {
    format: String,
    schema_version: u32,
    queries: Vec<Query>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Query {
    id: String,
    start: [f64; 3],
    end: [f64; 3],
    expected: Expected,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Expected {
    Miss,
    Hit {
        instance_id: String,
        triangle_index: usize,
        fraction: f64,
        position: [f64; 3],
    },
}
fn main() -> io::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 3 {
        return Err(io::Error::other(
            "usage: collision-probe WORLD.json QUERIES.json REPEATS",
        ));
    }
    let repeats: usize = args[2]
        .parse()
        .map_err(|_| io::Error::other("invalid repeats"))?;
    let query_path = PathBuf::from(&args[1]);
    if std::fs::metadata(&query_path)?.len() > 64 * 1024 * 1024 {
        return Err(io::Error::other("query file too large"));
    }
    let queries: Queries = serde_json::from_slice(&std::fs::read(query_path)?)?;
    if queries.format != "mag_collision_queries"
        || queries.schema_version != 1
        || queries.queries.is_empty()
        || repeats == 0
        || queries
            .queries
            .len()
            .checked_mul(repeats)
            .is_none_or(|n| n > 2_000_000)
    {
        return Err(io::Error::other("invalid query schema or workload limits"));
    }
    let started = Instant::now();
    let world =
        CollisionWorld::load(&args[0], CollisionLimits::default()).map_err(io::Error::other)?;
    let load_ns = started.elapsed().as_nanos();
    let mut hit_count = 0;
    for query in &queries.queries {
        let result = world
            .raycast_segment(query.start, query.end)
            .map_err(io::Error::other)?;
        let valid = match (&query.expected, result.nearest) {
            (Expected::Miss, None) => true,
            (
                Expected::Hit {
                    instance_id,
                    triangle_index,
                    fraction,
                    position,
                },
                Some(hit),
            ) => {
                hit_count += 1;
                fraction.is_finite()
                    && position.iter().all(|v| v.is_finite())
                    && hit.instance_id == *instance_id
                    && hit.triangle_index == *triangle_index
                    && (hit.fraction - fraction).abs() <= 1e-8
                    && hit
                        .position
                        .iter()
                        .zip(position)
                        .all(|(a, b)| (a - b).abs() <= 1e-8)
            }
            _ => false,
        };
        if !valid {
            return Err(io::Error::other(format!(
                "query expectation failed: {}",
                query.id
            )));
        }
    }
    let mut costs = Vec::with_capacity(queries.queries.len() * repeats);
    for _ in 0..repeats {
        for query in &queries.queries {
            let started = Instant::now();
            std::hint::black_box(
                world
                    .raycast_segment(query.start, query.end)
                    .map_err(io::Error::other)?,
            );
            costs.push(started.elapsed().as_nanos() as u64);
        }
    }
    costs.sort_unstable();
    let percentile = |percent: usize| costs[(costs.len() * percent).div_ceil(100) - 1];
    println!(
        "{}",
        serde_json::json!({"event":"collision_probe", "world_path":args[0],
        "metadata":world.metadata(), "stats":world.stats(), "load_ms":load_ns as f64/1e6,
        "verified_queries":queries.queries.len(), "hits":hit_count,
        "misses":queries.queries.len()-hit_count, "timed_queries":costs.len(),
        "query_us":{"p50":percentile(50) as f64/1000.0,"p95":percentile(95) as f64/1000.0,
          "p99":percentile(99) as f64/1000.0,"max":costs.last().unwrap().to_owned() as f64/1000.0},
        "authority":"diagnostic_only"})
    );
    Ok(())
}
