use mag_gameplay::collision::{CollisionLimits, CollisionWorld};
use mag_gameplay::telemetry::{TickRecorder, TickSample};
use mag_gameplay::{
    Engine, EngineResources, IO_DEADLINE, MAX_LOG, Response, parse_request, read_line,
};
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn main() -> io::Result<()> {
    let mut bind = SocketAddr::from(([127, 0, 0, 1], 31000));
    let mut log: Option<PathBuf> = None;
    let mut telemetry_path: Option<PathBuf> = None;
    let mut tick_budget_us = None;
    let mut collision_path: Option<PathBuf> = None;
    let mut navmesh_path: Option<PathBuf> = None;
    let mut bot_population_path: Option<PathBuf> = None;
    let mut bot_fill_total: usize = 0;
    let mut spawn_path: Option<PathBuf> = None;
    let mut objective_path: Option<PathBuf> = None;
    let mut item_path: Option<PathBuf> = None;
    let mut blast_rules_path: Option<PathBuf> = None;
    let mut retail_metadata_path: Option<PathBuf> = None;
    let mut bootstrap_loadouts_path: Option<PathBuf> = None;
    let mut fall_rules_path: Option<PathBuf> = None;
    let mut skill_path: Option<PathBuf> = None;
    let mut retail_armory_path: Option<PathBuf> = None;
    let mut leadership_passives_path: Option<PathBuf> = None;
    let mut leadership_role_policy_path: Option<PathBuf> = None;
    let mut structure_path: Option<PathBuf> = None;
    let mut bunker_path: Option<PathBuf> = None;
    let mut bunker_exit_policy_path: Option<PathBuf> = None;
    let mut wire_profiles_path: Option<PathBuf> = None;
    let mut collision_query = None;
    let mut development_dummy = false;
    let mut development_dummy_position = None;
    let mut development_control = false;
    let mut development_grenade_radius = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--bot-population" => {
                bot_population_path = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("--bot-population requires a path"))?
                        .into(),
                );
            }
            "--bot-fill-total" => {
                bot_fill_total = args
                    .next()
                    .ok_or_else(|| io::Error::other("--bot-fill-total requires a count"))?
                    .parse()
                    .map_err(|_| io::Error::other("invalid --bot-fill-total"))?;
            }
            "--navmesh-world" => {
                navmesh_path = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("--navmesh-world requires a path"))?
                        .into(),
                );
            }
            "--retail-combat-metadata" => {
                retail_metadata_path = Some(
                    args.next()
                        .ok_or_else(|| {
                            io::Error::other("--retail-combat-metadata requires an absolute path")
                        })?
                        .into(),
                );
            }
            "--character-blast-rules" => {
                blast_rules_path = Some(
                    args.next()
                        .ok_or_else(|| {
                            io::Error::other("--character-blast-rules requires an absolute path")
                        })?
                        .into(),
                );
            }
            "--wire-profiles" => {
                wire_profiles_path = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("--wire-profiles requires a path"))?
                        .into(),
                );
            }
            "--tick-telemetry" => {
                telemetry_path = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("--tick-telemetry requires a NEW path"))?
                        .into(),
                );
            }
            "--tick-budget-ms" => {
                let value: f64 = args
                    .next()
                    .ok_or_else(|| io::Error::other("--tick-budget-ms requires a number"))?
                    .parse()
                    .map_err(|_| io::Error::other("invalid --tick-budget-ms"))?;
                if !value.is_finite() || !(0.001..=1000.0).contains(&value) {
                    return Err(io::Error::other("tick budget must be .001..1000 ms"));
                }
                tick_budget_us = Some((value * 1000.0).round() as u64);
            }
            "--fall-damage-rules" => {
                fall_rules_path = Some(
                    args.next()
                        .ok_or_else(|| {
                            io::Error::other("--fall-damage-rules requires an absolute path")
                        })?
                        .into(),
                );
            }
            "--leadership-passives" => {
                leadership_passives_path = Some(
                    args.next()
                        .ok_or_else(|| {
                            io::Error::other("--leadership-passives requires an absolute path")
                        })?
                        .into(),
                );
            }
            "--leadership-role-policy" => {
                leadership_role_policy_path = Some(
                    args.next()
                        .ok_or_else(|| {
                            io::Error::other("--leadership-role-policy requires an absolute path")
                        })?
                        .into(),
                );
            }
            "--retail-armory-catalog" => {
                retail_armory_path = Some(
                    args.next()
                        .ok_or_else(|| {
                            io::Error::other("--retail-armory-catalog requires an absolute path")
                        })?
                        .into(),
                );
            }
            "--skill-catalog" => {
                skill_path = Some(
                    args.next()
                        .ok_or_else(|| {
                            io::Error::other("--skill-catalog requires an absolute path")
                        })?
                        .into(),
                );
            }
            "--structure-catalog" => {
                structure_path = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("--structure-catalog requires a path"))?
                        .into(),
                );
            }
            "--bunker-exit-policy" => {
                bunker_exit_policy_path =
                    Some(PathBuf::from(args.next().ok_or_else(|| {
                        io::Error::other("--bunker-exit-policy requires a path")
                    })?));
            }
            "--bunker-catalog" => {
                bunker_path = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("--bunker-catalog requires a path"))?
                        .into(),
                );
            }
            "--bootstrap-loadouts" => {
                bootstrap_loadouts_path = Some(
                    args.next()
                        .ok_or_else(|| {
                            io::Error::other("--bootstrap-loadouts requires an absolute path")
                        })?
                        .into(),
                );
            }
            "--item-catalog" => {
                item_path = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("--item-catalog requires a path"))?
                        .into(),
                );
            }
            "--objective-catalog" => {
                objective_path = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("--objective-catalog requires a path"))?
                        .into(),
                );
            }
            "--spawn-catalog" => {
                spawn_path = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("--spawn-catalog requires a path"))?
                        .into(),
                );
            }
            "--collision-world" => {
                collision_path = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("--collision-world requires a path"))?
                        .into(),
                );
            }
            "--collision-query" => {
                let start = args.next().ok_or_else(|| {
                    io::Error::other("--collision-query requires START END as x,y,z")
                })?;
                let end = args.next().ok_or_else(|| {
                    io::Error::other("--collision-query requires START END as x,y,z")
                })?;
                collision_query = Some((parse_point(&start)?, parse_point(&end)?));
            }
            "--development-dummy-position" => {
                let value = args.next().ok_or_else(|| {
                    io::Error::other("--development-dummy-position requires x,y,z")
                })?;
                let [x, y, z] = parse_point(&value)?;
                let position = mag_gameplay::replication::CharacterPosition {
                    x: x as f32,
                    y: y as f32,
                    z: z as f32,
                };
                development_dummy_position = Some(position);
            }
            "--development-dummy" => development_dummy = true,
            "--development-control" => development_control = true,
            "--development-grenade-radius" => development_grenade_radius = true,
            "--bind" => {
                bind = args
                    .next()
                    .ok_or_else(|| io::Error::other("--bind requires an address"))?
                    .parse()
                    .map_err(|_| io::Error::other("invalid --bind address"))?
            }
            "--log" => {
                log = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("--log requires a new path"))?
                        .into(),
                )
            }
            _ => {
                return Err(io::Error::other(
                    "usage: mag-gameplay --item-catalog ITEMS.json --spawn-catalog SPAWNS.json --log NEW_FILE [--wire-profiles PROFILES.json] [--objective-catalog OBJECTIVES.json] [--character-blast-rules /absolute/BLAST.json] [--retail-combat-metadata /absolute/RETAIL.json] [--fall-damage-rules /absolute/FALL.json] [--skill-catalog /absolute/SKILLS.json] [--retail-armory-catalog /absolute/ARMORY.json] [--leadership-passives /absolute/PASSIVES.json --leadership-role-policy /absolute/ROLES.json] [--bind IP:PORT] [--development-control] [--development-grenade-radius] [--development-dummy] [--development-dummy-position x,y,z] [--collision-world WORLD.json] [--navmesh-world MESH.mag-nav] [--bot-fill-total N --bot-population POPULATION.json] [--tick-telemetry NEW_FILE] [--tick-budget-ms MS]; query only: mag-gameplay --collision-world WORLD.json --collision-query x,y,z x,y,z",
                ));
            }
        }
    }
    if development_dummy_position.is_some() && !development_dummy {
        return Err(io::Error::other(
            "--development-dummy-position requires --development-dummy",
        ));
    }
    // Geometry is loaded before opening logs/listeners. A bad file cannot
    // silently fall back to an empty collision world or accept a reservation.
    let collision_world = collision_path
        .as_ref()
        .map(|path| {
            CollisionWorld::load(path, CollisionLimits::default()).map_err(io::Error::other)
        })
        .transpose()?;
    let navmesh_world = if let Some(path) = navmesh_path {
        let world = collision_world
            .as_ref()
            .ok_or_else(|| io::Error::other("--navmesh-world requires --collision-world"))?;
        if std::fs::metadata(&path)?.len() > mag_gameplay::navmesh::MAX_ARTIFACT_JSON_BYTES {
            return Err(io::Error::other("navmesh artifact size limit"));
        }
        let artifact: mag_gameplay::navmesh::Artifact =
            serde_json::from_slice(&std::fs::read(path)?).map_err(io::Error::other)?;
        let hash = mag_gameplay::navmesh::sha256(&std::fs::read(collision_path.as_ref().unwrap())?);
        let mesh =
            mag_gameplay::navmesh::NavMesh::load(artifact, &hash, &world.metadata().map_name)
                .map_err(io::Error::other)?;
        mesh.validate_coverage(world).map_err(io::Error::other)?;
        Some(mesh)
    } else {
        None
    };
    if let Some((start, end)) = collision_query {
        if log.is_some() || telemetry_path.is_some() || tick_budget_us.is_some() {
            return Err(io::Error::other(
                "--collision-query is standalone; omit --log and telemetry flags",
            ));
        }
        let world = collision_world
            .as_ref()
            .ok_or_else(|| io::Error::other("--collision-query requires --collision-world"))?;
        let query = world
            .raycast_segment(start, end)
            .map_err(io::Error::other)?;
        println!(
            "{}",
            serde_json::json!({"event":"collision_query", "source":collision_path,
            "stats":world.stats(), "query":query, "authority":"diagnostic_only"})
        );
        return Ok(());
    }
    if tick_budget_us.is_some() && telemetry_path.is_none() {
        return Err(io::Error::other(
            "--tick-budget-ms requires --tick-telemetry",
        ));
    }
    // Validate once before listening; the installed catalog resolves ordinary retail hits.
    let mut retail_combat_catalog = None;
    let retail_metadata = match &retail_metadata_path {
        Some(path) => {
            if !path.is_absolute() {
                return Err(io::Error::other(
                    "retail combat metadata requires an absolute path",
                ));
            }
            let data = mag_gameplay::retail_combat::RetailCombatCatalog::load(path)
                .map_err(|error| io::Error::other(error.to_string()))?;
            let info = Some(
                serde_json::json!({"path":path,"application_id":data.application_id(),"scope":"ordinary_client_reported_bullets",
                "bullets":data.bullets().len(),"grenades":data.grenades().len(),"armor":data.armor().len(),"body":data.body().len(),
                "healing":data.healing().len(),"authority":data.authority()}),
            );
            retail_combat_catalog = Some(data);
            info
        }
        None => None,
    };
    let spawn_path = spawn_path
        .ok_or_else(|| io::Error::other("--spawn-catalog is required for gameplay startup"))?;
    let catalog = mag_gameplay::spawn_catalog::SpawnCatalog::load(&spawn_path)?;
    let collision_info = collision_world.as_ref().map(|world| serde_json::json!({
        "source":collision_path,"metadata":world.metadata(),"stats":world.stats(),"bullet_authority":"audit_only","grenade_policy":"disk_triangles_three_body_samples","grenade_radius_enabled":development_grenade_radius,"active_map_name":catalog.map_name(),"map_matches_active":world.metadata().map_name == catalog.map_name()}));
    let objective_catalog = objective_path
        .as_ref()
        .map(mag_gameplay::objective_catalog::ObjectiveCatalog::load)
        .transpose()?;
    if let Some(objectives) = &objective_catalog {
        objectives
            .validate_match(catalog.map_name(), catalog.match_profile())
            .map_err(io::Error::other)?;
    }
    let objective_info = objective_catalog.as_ref().map(|objectives| serde_json::json!({"path":objective_path,"object_count":objectives.object_count(),"source":objectives.source()}));
    let item_path = item_path
        .ok_or_else(|| io::Error::other("--item-catalog is required for gameplay startup"))?;
    let blast_rules_path =
        blast_rules_path.unwrap_or_else(|| item_path.with_file_name("character-blast-rules.json"));
    if !blast_rules_path.is_absolute() {
        return Err(io::Error::other(
            "character-blast rules require an absolute path",
        ));
    }
    let mut item_catalog = mag_gameplay::item_catalog::ItemCatalog::load_with_blast_rules(
        &item_path,
        &blast_rules_path,
    )?;
    if let Some(path) = &bootstrap_loadouts_path {
        if !path.is_absolute() {
            return Err(io::Error::other(
                "--bootstrap-loadouts requires an absolute path",
            ));
        }
        item_catalog = item_catalog.with_bootstrap_loadouts(
            mag_gameplay::bootstrap_loadouts::BootstrapLoadouts::load(path)?,
        );
    }
    item_catalog
        .validate_bootstrap_faction(catalog.match_profile().defending_faction)
        .map_err(io::Error::other)?;
    if let Some(combat) = retail_combat_catalog {
        item_catalog = item_catalog.with_retail_combat(combat);
    }
    if let Some(path) = &retail_armory_path {
        if !path.is_absolute() {
            return Err(io::Error::other(
                "--retail-armory-catalog requires an absolute path",
            ));
        }
        item_catalog =
            item_catalog.with_retail_armory(mag_gameplay::retail_armory::RetailArmory::load(path)?);
    }
    let item_catalog = Arc::new(item_catalog);
    let wire_profiles_path =
        wire_profiles_path.unwrap_or_else(|| item_path.with_file_name("wire-profiles.json"));
    let wire_profiles = mag_gameplay::wire_profile::WireProfileCatalog::load(&wire_profiles_path)?;
    if skill_path.as_ref().is_some_and(|path| !path.is_absolute()) {
        return Err(io::Error::other(
            "--skill-catalog requires an absolute path",
        ));
    }
    let skill_path = match skill_path {
        Some(path) => path,
        None => std::env::current_dir()?
            .join(
                item_path
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new(".")),
            )
            .join("prototype-skills.json"),
    };
    let skill_catalog = mag_gameplay::skill_catalog::SkillCatalog::load(&skill_path)?;
    let leadership = match (&leadership_passives_path, &leadership_role_policy_path) {
        (None, None) => None,
        (Some(passives), Some(policy)) => {
            if !passives.is_absolute() || !policy.is_absolute() {
                return Err(io::Error::other(
                    "leadership configuration paths must be absolute",
                ));
            }
            let catalog = mag_gameplay::leadership_passives::LeadershipCatalog::load(passives)?;
            let policy = mag_gameplay::leadership_runtime::RolePolicy::load(policy)?;
            if policy.has_development_bindings() && !development_control {
                return Err(io::Error::other(
                    "leadership development bindings require --development-control",
                ));
            }
            Some((catalog, policy))
        }
        _ => {
            return Err(io::Error::other(
                "--leadership-passives and --leadership-role-policy must be supplied together",
            ));
        }
    };
    let structure_path =
        structure_path.unwrap_or_else(|| item_path.with_file_name("prototype-structures.json"));
    let bunker_path =
        bunker_path.unwrap_or_else(|| item_path.with_file_name("prototype-bunkers.json"));
    let bunker_world = if catalog.match_profile().has_bunkers {
        mag_gameplay::structures::BunkerWorld::load(
            &bunker_path,
            &structure_path,
            catalog.map_name(),
        )?
    } else {
        mag_gameplay::structures::BunkerWorld::without_bunkers(&structure_path)?
    };
    let bunker_exit_policy = bunker_exit_policy_path
        .as_ref()
        .map(mag_gameplay::vehicle_exit::BunkerExitPolicy::load)
        .transpose()?;
    let bunker_info = serde_json::json!({"active":bunker_world.active_count(),"unresolved":bunker_world.unresolved_count(),"placement_source":bunker_path,"combat_source":structure_path});
    if let Some(position) = development_dummy_position {
        mag_gameplay::dummy::respawn_at_with_catalog_and_profile(
            2,
            0,
            position,
            &item_catalog,
            wire_profiles.profile(20230).map_err(io::Error::other)?,
        )
        .map_err(io::Error::other)?;
    }
    let fall_rules = fall_rules_path
        .as_ref()
        .map(|path| {
            if !path.is_absolute() {
                return Err(io::Error::other(
                    "--fall-damage-rules requires an absolute path",
                ));
            }
            mag_gameplay::fall_damage::Rules::load(path)
        })
        .transpose()?;
    let actual_dummy_position = if development_dummy {
        Some(
            development_dummy_position
                .or_else(|| catalog.alternate_position())
                .ok_or_else(|| io::Error::other("missing_alternate_spawn"))?,
        )
    } else {
        None
    };
    let path = log.ok_or_else(|| {
        io::Error::other("--log is required; existing logs are never overwritten")
    })?;
    let telemetry = telemetry_path
        .as_ref()
        .map(|path| {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)
                .map(|writer| TickRecorder::start(writer, MAX_LOG))
        })
        .transpose()?;
    let writer = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    let mut resources =
        EngineResources::new(item_catalog, catalog, bunker_world).with_wire_profiles(wire_profiles);
    if let Some(policy) = bunker_exit_policy {
        resources = resources
            .with_bunker_exit_policy(policy)
            .map_err(io::Error::other)?;
    }
    let mut engine =
        Engine::from_resources(writer, MAX_LOG, resources).with_skill_catalog(skill_catalog);
    if let Some(objectives) = objective_catalog {
        engine = engine
            .with_objective_catalog(objectives)
            .map_err(io::Error::other)?;
    }
    if let Some(rules) = fall_rules {
        engine = engine.with_fall_damage_rules(rules);
    }
    if let Some(world) = collision_world {
        engine = engine.with_collision_world(world);
    }
    if let Some(mesh) = navmesh_world {
        engine = engine.with_navmesh_world(mesh);
    }
    if development_control {
        engine = engine.with_development_control();
    }
    if development_grenade_radius {
        engine = engine.with_development_grenade_radius();
    }
    if development_dummy {
        engine = engine.with_development_dummy();
    }
    if let Some(position) = development_dummy_position {
        engine = engine
            .with_development_dummy_position(position)
            .map_err(io::Error::other)?;
    }
    if let Some((catalog, policy)) = leadership {
        engine = engine
            .with_leadership_passives(catalog, policy)
            .map_err(io::Error::other)?;
    }
    if bot_fill_total > 0 {
        let path = bot_population_path
            .as_ref()
            .ok_or_else(|| io::Error::other("--bot-fill-total requires --bot-population"))?;
        let population = mag_gameplay::bot_registry::BotPopulation::load(path)
            .map_err(|e| io::Error::other(e.to_string()))?;
        engine = engine
            .with_bot_population(population, bot_fill_total)
            .map_err(io::Error::other)?;
    }
    let engine = Arc::new(Mutex::new(engine));
    let listener = TcpListener::bind(bind)?;
    let started = Instant::now();
    let active = Arc::new(AtomicUsize::new(0));
    let subscriber: SharedSubscriber = Arc::new(Mutex::new(None));
    spawn_world_scheduler(
        Arc::clone(&engine),
        Arc::clone(&subscriber),
        started,
        telemetry.clone(),
        tick_budget_us,
    );
    println!(
        "{}",
        serde_json::json!({"event":"listening","protocol":mag_gameplay::BRIDGE_VERSION,"address":listener.local_addr()?.to_string(),"collision":collision_info,"spawn_catalog":spawn_path,"objective_catalog":objective_info,"item_catalog":item_path,"character_blast_rules":blast_rules_path,"retail_combat_metadata":retail_metadata,"bootstrap_loadouts":bootstrap_loadouts_path,"fall_damage_rules":fall_rules_path,"skill_catalog":skill_path,"retail_armory_catalog":retail_armory_path,"leadership_passives":leadership_passives_path,"leadership_role_policy":leadership_role_policy_path,"bunkers":bunker_info,"tick_telemetry":telemetry_path,"tick_budget_us":tick_budget_us,"scheduler":"independent_50ms_when_subscribed","bot_fill_total":bot_fill_total,"bot_population":bot_population_path,"development_dummy_position":actual_dummy_position})
    );
    for connection in listener.incoming() {
        let stream = connection?;
        if active
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < 8).then_some(n + 1)
            })
            .is_err()
        {
            drop(stream);
            continue;
        }
        let engine = Arc::clone(&engine);
        let active = Arc::clone(&active);
        let telemetry = telemetry.clone();
        let subscriber = Arc::clone(&subscriber);
        std::thread::spawn(move || {
            let slot = ConnectionSlot(Some(active));
            if let Err(failure) = serve(
                stream,
                engine,
                started,
                telemetry,
                tick_budget_us,
                subscriber,
                slot,
            ) {
                // Fixed categories only: no payload, request ID, credentials or
                // operating-system error text can reach this diagnostic.
                let _ = writeln!(io::stderr().lock(), "{}", failure.diagnostic());
            }
        });
    }
    Ok(())
}
#[derive(Clone, Copy, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum ConnectionStage {
    ReadRequest,
    EncodeResponse,
    WriteResponse,
}

struct ConnectionFailure {
    stage: ConnectionStage,
    kind: io::ErrorKind,
}
impl ConnectionFailure {
    fn io(stage: ConnectionStage, error: io::Error) -> Self {
        Self {
            stage,
            kind: error.kind(),
        }
    }
    fn diagnostic(&self) -> serde_json::Value {
        let category = match (self.stage, self.kind) {
            (ConnectionStage::EncodeResponse, _) => "encoding_failed",
            (_, io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock) => "timeout",
            (_, io::ErrorKind::InvalidData) => "invalid_framing",
            (_, io::ErrorKind::UnexpectedEof) => "incomplete_request",
            (
                _,
                io::ErrorKind::BrokenPipe
                | io::ErrorKind::ConnectionReset
                | io::ErrorKind::ConnectionAborted
                | io::ErrorKind::NotConnected,
            ) => "disconnected",
            _ => "io_error",
        };
        serde_json::json!({"event":"connection_error", "stage":self.stage, "category":category})
    }
}

fn serve(
    mut stream: TcpStream,
    engine: Arc<Mutex<Engine<std::fs::File>>>,
    started: Instant,
    telemetry: Option<TickRecorder>,
    tick_budget_us: Option<u64>,
    subscriber: SharedSubscriber,
    slot: ConnectionSlot,
) -> Result<(), ConnectionFailure> {
    let line = read_line(&mut stream, IO_DEADLINE)
        .map_err(|error| ConnectionFailure::io(ConnectionStage::ReadRequest, error))?;
    if serde_json::from_slice::<serde_json::Value>(&line)
        .ok()
        .is_some_and(|v| v["op"] == "subscribe_world")
    {
        return serve_subscription(stream, &line, engine, subscriber, slot);
    }
    let response = match parse_request(&line) {
        Ok(request) => {
            let wall = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let update = telemetry
                .as_ref()
                .is_some_and(|_| request.is_gameplay_update());
            let waiting = Instant::now();
            let started_us = started.elapsed().as_micros() as u64;
            let (response, wait_us, engine_us) = match engine.lock() {
                Ok(mut engine) => {
                    let wait_us = waiting.elapsed().as_micros() as u64;
                    let processing = Instant::now();
                    let response = engine.handle(request, started.elapsed(), wall);
                    (response, wait_us, processing.elapsed().as_micros() as u64)
                }
                Err(_) => (
                    Response::error(String::new(), "engine_unavailable"),
                    waiting.elapsed().as_micros() as u64,
                    0,
                ),
            };
            // The engine lock has been released before telemetry enqueue.
            if let Some(recorder) = telemetry.as_ref() {
                recorder.record(TickSample::new(
                    update,
                    started_us,
                    wait_us,
                    engine_us,
                    response.ok,
                    tick_budget_us,
                ));
            }
            response
        }
        Err(error) => Response::error(error.request_id, error.code),
    };
    let mut reply = serde_json::to_vec(&response).map_err(|_| ConnectionFailure {
        stage: ConnectionStage::EncodeResponse,
        kind: io::ErrorKind::InvalidData,
    })?;
    reply.push(b'\n');
    stream
        .set_write_timeout(Some(IO_DEADLINE))
        .and_then(|_| stream.write_all(&reply))
        .map_err(|error| ConnectionFailure::io(ConnectionStage::WriteResponse, error))
}

fn parse_point(value: &str) -> io::Result<[f64; 3]> {
    let parts: Vec<_> = value.split(',').collect();
    if parts.len() != 3 {
        return Err(io::Error::other("point requires x,y,z"));
    }
    let mut result = [0.0_f64; 3];
    for (out, part) in result.iter_mut().zip(parts) {
        *out = part
            .parse()
            .map_err(|_| io::Error::other("invalid point coordinate"))?;
        if !out.is_finite() {
            return Err(io::Error::other("point must be finite"));
        }
    }
    Ok(result)
}

struct ConnectionSlot(Option<Arc<AtomicUsize>>);
impl Drop for ConnectionSlot {
    fn drop(&mut self) {
        if let Some(active) = self.0.take() {
            active.fetch_sub(1, Ordering::AcqRel);
        }
    }
}
struct WorldSubscriber {
    generation: u64,
    world_id: u32,
    request_id: String,
    sender: SyncSender<Response>,
}
type SharedSubscriber = Arc<Mutex<Option<WorldSubscriber>>>;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SubscribeWorld {
    op: String,
    v: u8,
    request_id: String,
    world_id: u32,
}
fn write_response(stream: &mut TcpStream, response: &Response) -> Result<(), ConnectionFailure> {
    let mut bytes = serde_json::to_vec(response).map_err(|_| ConnectionFailure {
        stage: ConnectionStage::EncodeResponse,
        kind: io::ErrorKind::InvalidData,
    })?;
    bytes.push(b'\n');
    stream
        .set_write_timeout(Some(IO_DEADLINE))
        .and_then(|_| stream.write_all(&bytes))
        .map_err(|e| ConnectionFailure::io(ConnectionStage::WriteResponse, e))
}
fn serve_subscription(
    mut stream: TcpStream,
    line: &[u8],
    engine: Arc<Mutex<Engine<std::fs::File>>>,
    shared: SharedSubscriber,
    slot: ConnectionSlot,
) -> Result<(), ConnectionFailure> {
    let request = serde_json::from_slice::<SubscribeWorld>(line)
        .ok()
        .filter(|r| {
            r.op == "subscribe_world"
                && r.v == mag_gameplay::BRIDGE_VERSION
                && (1..=64).contains(&r.request_id.len())
                && r.request_id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        });
    let Some(request) = request else {
        return write_response(
            &mut stream,
            &Response::error(String::new(), "invalid_subscription"),
        );
    };
    let (sender, receiver) = mpsc::sync_channel(32);
    static NEXT_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let generation = NEXT_GENERATION.fetch_add(1, Ordering::Relaxed);
    {
        let mut guard = shared.lock().map_err(|_| ConnectionFailure {
            stage: ConnectionStage::WriteResponse,
            kind: io::ErrorKind::Other,
        })?;
        if guard.is_some() {
            drop(guard);
            return write_response(
                &mut stream,
                &Response::error(request.request_id, "world_subscription_exists"),
            );
        }
        let mut engine = engine.lock().map_err(|_| ConnectionFailure {
            stage: ConnectionStage::WriteResponse,
            kind: io::ErrorKind::Other,
        })?;
        if !engine.accepts_subscription_world(request.world_id) {
            drop(engine);
            drop(guard);
            return write_response(
                &mut stream,
                &Response::error(request.request_id, "subscription_world_mismatch"),
            );
        }
        engine.enable_world_ticks();
        // Queue the acknowledgement before exposing the sender to the scheduler.
        sender
            .send(Response {
                v: mag_gameplay::BRIDGE_VERSION,
                request_id: request.request_id.clone(),
                ok: true,
                error: None,
                messages: Vec::new(),
                rewards: Vec::new(),
                match_terminal: None,
            })
            .unwrap();
        *guard = Some(WorldSubscriber {
            generation,
            world_id: request.world_id,
            request_id: request.request_id,
            sender,
        });
    }
    // Persistent subscriptions have their own single slot, separate from request workers.
    drop(slot);
    let result = (|| {
        loop {
            if shared
                .lock()
                .ok()
                .is_none_or(|guard| !guard.as_ref().is_some_and(|s| s.generation == generation))
            {
                return Err(ConnectionFailure {
                    stage: ConnectionStage::WriteResponse,
                    kind: io::ErrorKind::ConnectionAborted,
                });
            }
            match receiver.recv_timeout(Duration::from_secs(2)) {
                Ok(response) => write_response(&mut stream, &response)?,
                Err(_) => {
                    return Err(ConnectionFailure {
                        stage: ConnectionStage::WriteResponse,
                        kind: io::ErrorKind::ConnectionAborted,
                    });
                }
            }
        }
    })();
    if let Ok(mut guard) = shared.lock()
        && guard.as_ref().is_some_and(|s| s.generation == generation)
    {
        *guard = None;
    }
    result
}
fn spawn_world_scheduler(
    engine: Arc<Mutex<Engine<std::fs::File>>>,
    subscriber: SharedSubscriber,
    started: Instant,
    telemetry: Option<TickRecorder>,
    budget: Option<u64>,
) {
    std::thread::spawn(move || {
        let interval = Duration::from_millis(50);
        let mut deadline = Instant::now() + interval;
        let mut heartbeat = Instant::now();
        loop {
            std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
            let now = Instant::now();
            // Skip missed deadlines: never run an unbounded burst of catch-up steps.
            deadline += interval;
            if deadline <= now {
                deadline = now + interval;
            }
            let target = subscriber.lock().ok().and_then(|s| {
                s.as_ref().map(|s| {
                    (
                        s.generation,
                        s.world_id,
                        s.request_id.clone(),
                        s.sender.clone(),
                    )
                })
            });
            let Some((generation, world_id, request_id, sender)) = target else {
                continue;
            };
            let wall = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let waiting = Instant::now();
            let start_us = started.elapsed().as_micros() as u64;
            let (response, wait_us, engine_us) = match engine.lock() {
                Ok(mut engine) => {
                    let wait = waiting.elapsed().as_micros() as u64;
                    let processing = Instant::now();
                    let response = engine.tick_world(started.elapsed(), wall, request_id);
                    (response, wait, processing.elapsed().as_micros() as u64)
                }
                Err(_) => continue,
            };
            if let Some(recorder) = telemetry.as_ref() {
                recorder.record(
                    TickSample::new(true, start_us, wait_us, engine_us, response.ok, budget)
                        .independent(),
                );
            }
            let mismatch = response.messages.iter().any(|m| m.world_id != world_id);
            if mismatch || !response.ok {
                if let Ok(mut guard) = subscriber.lock()
                    && guard.as_ref().is_some_and(|s| s.generation == generation)
                {
                    *guard = None;
                }
                continue;
            }
            let due = heartbeat.elapsed() >= Duration::from_secs(1);
            if response.messages.is_empty()
                && response.rewards.is_empty()
                && response.match_terminal.is_none()
                && !due
            {
                continue;
            }
            heartbeat = Instant::now();
            if let Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) =
                sender.try_send(response)
            {
                // Closing the stream is an explicit gap; Horizon releases affected sessions.
                if let Ok(mut guard) = subscriber.lock()
                    && guard.as_ref().is_some_and(|s| s.generation == generation)
                {
                    *guard = None;
                }
            }
        }
    });
}
