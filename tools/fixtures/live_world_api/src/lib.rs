//! Independent public-API fixture; see README.md for the source-only validation boundary.
//! The caller supplies kinematic poses. There is no physics solver or external native library.
mod record;

use mod_api::text::TextContent;
use mod_api::{
    self, AttachmentError, BlockUpdateFlags, Compound, Holder, Hooks, HostTransaction,
    LevelReadError, LevelRef, LiveWorldError, LiveWorldLimits, LiveWorldPose, LiveWorldRecovery,
    LiveWorldRef, LiveWorldSaveReceipt, LiveWorldTransferCell, LiveWorldTransferLimits,
    LiveWorldTransferReceipt, LiveWorldTransferRef, Registrar, RuntimeGenerator, RuntimeLevel,
    ServerHost, Tag, Text, TicketController, TicketOwner,
};
use record::{OWNER, Phase, Record};
use std::sync::{Mutex, OnceLock};

const SOURCE: &str = "live_world_fixture:local";
const DESTINATION: &str = "minecraft:overworld";
const OWNER_POSITION: [i32; 3] = [0, 64, 0];
const WORLD_Y_OFFSET: i32 = 96;
const MOTION_STEPS: u32 = 120;

#[derive(Clone, Copy, Debug)]
enum Action {
    Initialize,
    Project,
    Capture,
    Return,
    Refuse,
    Checkpoint,
    Run,
    Pause,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Initialization {
    MarkerBarrier,
    Seed,
    PayloadBarrier,
}
#[derive(Debug)]
enum Error {
    Live(LiveWorldError),
    LevelRead(LevelReadError),
    Attachment(AttachmentError),
    Refused(&'static str),
    Write(String),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Live(v) => write!(f, "{v:?}"),
            Self::LevelRead(v) => write!(f, "{v:?}"),
            Self::Attachment(v) => write!(f, "{v:?}"),
            Self::Refused(v) => f.write_str(v),
            Self::Write(v) => f.write_str(v),
        }
    }
}
impl From<LiveWorldError> for Error {
    fn from(value: LiveWorldError) -> Self {
        Self::Live(value)
    }
}
impl From<AttachmentError> for Error {
    fn from(value: AttachmentError) -> Self {
        Self::Attachment(value)
    }
}
impl From<LevelReadError> for Error {
    fn from(value: LevelReadError) -> Self {
        Self::LevelRead(value)
    }
}
type Result<T> = std::result::Result<T, Error>;

#[derive(Default)]
struct State {
    source: Option<LevelRef>,
    destination: Option<LevelRef>,
    record: Option<Record>,
    projection: Option<LiveWorldRef>,
    source_chunks: Vec<[i32; 2]>,
    held: Vec<(LevelRef, [i32; 2])>,
    loaded: bool,
    restore: bool,
    project: bool,
    action: Option<Action>,
    pending: Option<(LiveWorldTransferRef, Record)>,
    initialization: Option<Initialization>,
    checkpoint_pending: bool,
    last_transfer: Option<LiveWorldTransferReceipt>,
    last_checkpoint: Option<LiveWorldSaveReceipt>,
    motion: Option<(u32, LiveWorldPose)>,
    durable_epoch: Option<u64>,
    last: String,
    fenced: bool,
}
#[derive(Default)]
struct Fixture {
    controller: OnceLock<TicketController>,
    state: Mutex<State>,
}

fn cells() -> Vec<[i32; 3]> {
    let mut selected = Vec::new();
    for x in 0..5 {
        for z in 0..5 {
            if [x, z] != [2, 2] {
                selected.push([x, 64, z]);
            }
        }
    }
    selected.extend([[0, 65, 0], [1, 65, 0], [17, 64, 0]]);
    selected.sort_unstable();
    selected
}
fn columns(selected: &[[i32; 3]]) -> Vec<[i32; 2]> {
    let mut result: Vec<_> = selected
        .iter()
        .map(|p| [p[0].div_euclid(16), p[2].div_euclid(16)])
        .collect();
    result.sort_unstable();
    result.dedup();
    result
}
fn world_cell(p: [i32; 3]) -> [i32; 3] {
    [p[0], p[1] + WORLD_Y_OFFSET, p[2]]
}
fn first_pose() -> LiveWorldPose {
    LiveWorldPose::at([0.0, f64::from(WORLD_Y_OFFSET), 0.0])
}
fn item(slot: i8, id: &str, count: i32) -> Tag {
    Tag::compound([
        ("Slot", Tag::Byte(slot)),
        ("id", Tag::String(id.into())),
        ("count", Tag::Int(count)),
    ])
}
fn compound(value: Tag) -> Compound {
    let Tag::Compound(value) = value else {
        unreachable!()
    };
    value
}
fn save_record(host: &mut dyn ServerHost, source: LevelRef, value: Record) -> Result<()> {
    OWNER.set(&mut source.attachments(host), Holder::Level, value)?;
    Ok(())
}
fn verify_publication(
    host: &mut dyn ServerHost,
    source: LevelRef,
    projection: mod_api::LiveWorldRef,
    expected: &Record,
) -> Result<()> {
    // Ordinary Host.transaction close is void. Closure Ok is not a successful root decision.
    let actual_record = OWNER
        .get_existing(&mut source.attachments(host), Holder::Level)?
        .ok_or(Error::Refused("post-close source record is missing"))?;
    if record::encode(&actual_record)? != record::encode(expected)? {
        return Err(Error::Refused(
            "post-close source record differs from proposed record",
        ));
    }
    let accepted = expected
        .accepted
        .as_ref()
        .ok_or(Error::Refused("expected accepted sample is missing"))?;
    let actual = projection.sample(host)?;
    let bits = |pose: LiveWorldPose| {
        pose.translation
            .into_iter()
            .chain(pose.pivot)
            .chain(pose.rotation)
            .map(f64::to_bits)
            .collect::<Vec<_>>()
    };
    if actual.revision != accepted.revision
        || bits(actual.previous) != bits(accepted.previous)
        || bits(actual.current) != bits(accepted.current)
    {
        return Err(Error::Refused(
            "post-close actual sample differs from proposed receipt",
        ));
    }
    Ok(())
}
fn checkpoint(host: &mut dyn ServerHost, state: &mut State) -> Result<()> {
    let receipt = state.source.unwrap().checkpoint(host)?;
    state.durable_epoch = Some(receipt.barrier_epoch);
    state.last = format!("checkpoint receipt={receipt:?}");
    state.last_checkpoint = Some(receipt);
    Ok(())
}

impl Fixture {
    fn hold(
        &self,
        host: &mut dyn ServerHost,
        state: &mut State,
        level: LevelRef,
        chunks: &[[i32; 2]],
    ) -> Result<()> {
        let controller = *self
            .controller
            .get()
            .ok_or(Error::Refused("controller unavailable"))?;
        for &chunk in chunks {
            if state.held.contains(&(level, chunk)) {
                continue;
            }
            let handle = level
                .chunk_ref(host, chunk)
                .ok_or(Error::Refused("chunk handle unavailable"))?;
            handle.set_mod_forced(
                host,
                controller,
                TicketOwner::Block(OWNER_POSITION),
                true,
                false,
            );
            // False can mean this exact owned ticket already exists after a restart.
            let owned = level
                .mod_forced_chunks(host, controller)
                .ok_or(Error::Refused("ticket read unavailable"))?;
            if !owned.contains(&(TicketOwner::Block(OWNER_POSITION), chunk, false)) {
                return Err(Error::Refused("bootstrap ticket was not admitted"));
            }
            state.held.push((level, chunk));
        }
        Ok(())
    }
    fn release(&self, host: &mut dyn ServerHost, state: &mut State) {
        let Some(&controller) = self.controller.get() else {
            return;
        };
        for (level, chunk) in state.held.drain(..) {
            if let Some(chunk) = level.chunk_ref(host, chunk) {
                chunk.set_mod_forced(
                    host,
                    controller,
                    TicketOwner::Block(OWNER_POSITION),
                    false,
                    false,
                );
            }
        }
    }
    fn bootstrap(&self, host: &mut dyn ServerHost, state: &mut State) -> Result<bool> {
        let source = state.source.unwrap();
        let destination = state.destination.unwrap();
        let chunks = state.source_chunks.clone();
        self.hold(host, state, source, &chunks)?;
        // Finite temporary coverage of this fixture's start and short kinematic sequence.
        let target: Vec<_> = (-1..=2)
            .flat_map(|x| (-1..=2).map(move |z| [x, z]))
            .collect();
        self.hold(host, state, destination, &target)?;
        Ok(chunks
            .iter()
            .all(|&[x, z]| source.should_tick_blocks_at(host, [x * 16, 64, z * 16]) == Some(true))
            && target.iter().all(|&[x, z]| {
                destination.should_tick_blocks_at(host, [x * 16, 160, z * 16]) == Some(true)
            }))
    }
    fn load(&self, host: &mut dyn ServerHost, state: &mut State) -> Result<()> {
        let source = state.source.unwrap();
        // Global callback current-level identity is unspecified: name the real source holder.
        state.record = OWNER.get_existing(&mut source.attachments(host), Holder::Level)?;
        match source.saved_projection(host) {
            Ok(saved) => {
                if saved.recovery != LiveWorldRecovery::Ready {
                    return Err(LiveWorldError::MigrationRequired.into());
                }
                if state.record.as_ref().map(|r| r.phase) != Some(Phase::Projected) {
                    return Err(Error::Refused(
                        "saved projection lacks its same-source Projected marker",
                    ));
                }
                state.destination = Some(
                    host.level(&saved.destination)
                        .ok_or(Error::Refused("saved destination is not open"))?,
                );
                // Actual saved columns, never the bootstrap recipe, control restore residency.
                state.source_chunks = saved.source_chunks;
                state.restore = true;
            }
            Err(LiveWorldError::NotSaved) => {
                state.source_chunks = columns(&cells());
                match state.record.as_ref().map(|r| r.phase) {
                    Some(Phase::LocalOwned) => state.project = true,
                    Some(Phase::Initializing) => {
                        return Err(Error::Refused(
                            "incomplete explicit initialization requires inspection",
                        ));
                    }
                    Some(Phase::Projected) => {
                        return Err(Error::Refused(
                            "Projected marker exists without saved descriptor",
                        ));
                    }
                    Some(Phase::WorldOwned) | None => {}
                }
            }
            Err(error) => return Err(error.into()),
        }
        // Reconcile only this controller/owner's leftover temporary tickets after a crash.
        let controller = *self.controller.get().unwrap();
        for level in [source, state.destination.unwrap()] {
            let tickets = level
                .mod_forced_chunks(host, controller)
                .ok_or(Error::Refused("ticket read unavailable"))?;
            for (owner, chunk, natural) in tickets {
                if owner == TicketOwner::Block(OWNER_POSITION) {
                    let chunk = level
                        .chunk_ref(host, chunk)
                        .ok_or(Error::Refused("saved ticket handle unavailable"))?;
                    chunk.set_mod_forced(host, controller, owner, false, natural);
                }
            }
        }
        state.loaded = true;
        Ok(())
    }
    fn initialize(&self, host: &mut dyn ServerHost, state: &mut State) -> Result<()> {
        let source = state.source.unwrap();
        let members = cells();
        let destination = state.destination.unwrap();
        let air = host
            .block_state_id("minecraft:air")
            .ok_or(Error::Refused("air unavailable"))?;
        if !members.iter().all(|&p| {
            let target = world_cell(p);
            destination.should_tick_blocks_at(host, target) == Some(true)
                && destination.block_state(host, target) == Some(air)
        }) {
            return Err(Error::Refused(
                "initialization requires the inspected reserved parent cells to remain resident and empty",
            ));
        }
        if state.initialization.is_none() {
            if state.record.is_some() {
                return Err(Error::Refused(
                    "same-source marker exists; initialization cannot repeat",
                ));
            }
            // The explicit command is the opt-in. Absence alone never authorizes creation.
            match source.saved_projection(host) {
                Err(LiveWorldError::NotSaved) => {}
                Err(error) => return Err(error.into()),
                Ok(_) => {
                    return Err(Error::Refused(
                        "initialization requires an absent saved projection",
                    ));
                }
            }
            if !members
                .iter()
                .all(|&p| source.block_state(host, p) == Some(air))
            {
                return Err(Error::Refused(
                    "initialization refuses existing or unreadable source cells",
                ));
            }
            let record = Record::new(Phase::Initializing);
            save_record(host, source, record.clone())?;
            state.record = Some(record.clone());
            state.initialization = Some(Initialization::MarkerBarrier);
        }
        // Persist the first-use marker before any seed writes. A partial seed never gets reseeded.
        if state.initialization == Some(Initialization::MarkerBarrier) {
            checkpoint(host, state)?;
            state.initialization = Some(Initialization::Seed);
        }
        if state.initialization == Some(Initialization::Seed) {
            let flags = BlockUpdateFlags::NEIGHBORS | BlockUpdateFlags::CLIENTS;
            for &position in &members {
                let name = match position {
                    [0, 65, 0] => "minecraft:chest",
                    [1, 65, 0] => "minecraft:furnace",
                    _ => "minecraft:stone",
                };
                let block = host
                    .block_state_id(name)
                    .ok_or(Error::Refused("fixture block unavailable"))?;
                if !source
                    .set_block(host, position, block, flags)
                    .map_err(|e| Error::Write(format!("{e:?}")))?
                {
                    return Err(Error::Refused("seed block write refused"));
                }
            }
            let chest = host
                .block_entity(source, [0, 65, 0])
                .ok_or(Error::Refused("chest unavailable"))?;
            if !chest
                .merge_data(
                    host,
                    compound(Tag::compound([(
                        "Items",
                        Tag::List(vec![item(0, "minecraft:stone", 7)]),
                    )])),
                )
                .map_err(|e| Error::Write(format!("{e:?}")))?
            {
                return Err(Error::Refused("chest seed data was not accepted"));
            }
            let furnace = host
                .block_entity(source, [1, 65, 0])
                .ok_or(Error::Refused("furnace unavailable"))?;
            if !furnace
                .merge_data(
                    host,
                    compound(Tag::compound([
                        (
                            "Items",
                            Tag::List(vec![
                                item(0, "minecraft:raw_iron", 64),
                                item(1, "minecraft:coal", 1),
                            ]),
                        ),
                        ("lit_time_remaining", Tag::Int(2000)),
                        ("lit_total_time", Tag::Int(2000)),
                        ("cooking_time_spent", Tag::Int(40)),
                        ("cooking_total_time", Tag::Int(200)),
                    ])),
                )
                .map_err(|e| Error::Write(format!("{e:?}")))?
            {
                return Err(Error::Refused("furnace seed data was not accepted"));
            }
            let mut record = state.record.clone().unwrap();
            record.phase = Phase::LocalOwned;
            save_record(host, source, record.clone())?;
            state.record = Some(record);
            state.initialization = Some(Initialization::PayloadBarrier);
        }
        checkpoint(host, state)?;
        state.initialization = None;
        state.project = true;
        Ok(())
    }
    fn project(&self, host: &mut dyn ServerHost, state: &mut State) -> Result<()> {
        let source = state.source.unwrap();
        if state.restore {
            let projection = source.restore_projection(host)?;
            state.projection = Some(projection);
            let actual = projection.sample(host)?;
            let accepted = state
                .record
                .as_ref()
                .and_then(|record| record.accepted.as_ref())
                .ok_or(Error::Refused("saved accepted receipt is missing"))?;
            let same_bits = |a: LiveWorldPose, b: LiveWorldPose| {
                a.translation.map(f64::to_bits) == b.translation.map(f64::to_bits)
                    && a.pivot.map(f64::to_bits) == b.pivot.map(f64::to_bits)
                    && a.rotation.map(f64::to_bits) == b.rotation.map(f64::to_bits)
            };
            if actual.revision != accepted.revision
                || !same_bits(actual.current, accepted.current)
                || !same_bits(actual.previous, actual.current)
            {
                return Err(Error::Refused(
                    "restored current receipt bits/revision or settled previous differ",
                ));
            }
            // Keep the full historical publish receipt in the original persistent record;
            // the reopened presentation/contact pair intentionally settles at its exact current pose.
            state.restore = false;
            state.last = format!(
                "restored current/revision bits validated; settled actual sample={actual:?}"
            );
        } else {
            let mut record = state
                .record
                .clone()
                .ok_or(Error::Refused("initialize must be explicit"))?;
            if record.phase != Phase::LocalOwned {
                return Err(Error::Refused("source does not own payload"));
            }
            // Separate binding frontier after an actual transfer receipt. Backend must journal
            // creation with metadata in this transaction, or refuse unsupported nesting.
            let projection = host.transaction(|host, _tx| -> Result<_> {
                let projection = source.project(
                    host,
                    state.destination.unwrap(),
                    &cells(),
                    first_pose(),
                    LiveWorldLimits::default(),
                )?;
                record.phase = Phase::Projected;
                record.accepted = Some(projection.sample(host)?.into());
                save_record(host, source, record.clone())?;
                Ok(projection)
            })?;
            verify_publication(host, source, projection, &record)?;
            state.record = Some(record);
            state.projection = Some(projection);
            state.project = false;
            state.checkpoint_pending = true;
            return Ok(());
        }
        // Accepted binding derives retention. Bootstrap tickets are not permanent forceload.
        self.release(host, state);
        Ok(())
    }
    fn transfer(&self, host: &mut dyn ServerHost, state: &mut State, capture: bool) -> Result<()> {
        let source = state.source.unwrap();
        let destination = state.destination.unwrap();
        let mut next = state
            .record
            .clone()
            .ok_or(Error::Refused("fixture is not initialized"))?;
        if capture && next.phase != Phase::WorldOwned {
            return Err(Error::Refused("capture requires WorldOwned"));
        }
        if !capture && !matches!(next.phase, Phase::Projected | Phase::LocalOwned) {
            return Err(Error::Refused("return requires source ownership"));
        }
        if let Some(projection) = state.projection {
            let mut actual: Vec<_> = projection.cells(host)?.into_iter().map(|c| c.pos).collect();
            actual.sort_unstable();
            if actual != cells() {
                return Err(Error::Refused(
                    "fixture membership changed; inspect instead of transferring a partial payload",
                ));
            }
        }
        let mappings: Vec<_> = cells()
            .into_iter()
            .map(|p| {
                if capture {
                    LiveWorldTransferCell::new(destination, world_cell(p), source, p)
                } else {
                    LiveWorldTransferCell::new(source, p, destination, world_cell(p))
                }
            })
            .collect();
        let prepared = source.prepare_transfer(host, &mappings, transfer_limits())?;
        next.phase = if capture {
            Phase::LocalOwned
        } else {
            Phase::WorldOwned
        };
        next.accepted = None;
        next.steps = 0;
        let staged = host.transaction(|host, tx| -> Result<_> {
            let planned = prepared.stage(host, tx)?;
            next.planned_epoch = Some(planned.planned_epoch);
            save_record(host, source, next.clone())?;
            Ok(planned)
        });
        if let Err(error) = staged {
            if !matches!(error, Error::Live(LiveWorldError::Storage)) {
                let _ = prepared.cancel(host);
            }
            return Err(error);
        }
        state.motion = None;
        state.pending = Some((prepared, next));
        state.last = "stage returned planned epoch; actual receipt pending".into();
        Ok(())
    }
    fn finish_transfer(&self, host: &mut dyn ServerHost, state: &mut State) -> Result<()> {
        let (prepared, mut next) = state.pending.clone().unwrap();
        let receipt = prepared.receipt(host)?;
        if receipt.moved_cells != cells().len() as u32 {
            return Err(Error::Refused("unexpected moved-cell receipt"));
        }
        let actual = OWNER
            .get_existing(&mut state.source.unwrap().attachments(host), Holder::Level)?
            .ok_or(Error::Refused(
                "completed transfer source record is missing",
            ))?;
        if record::encode(&actual)? != record::encode(&next)? {
            return Err(Error::Refused("completed transfer opaque record differs"));
        }
        next.transfer_epoch = Some(receipt.barrier_epoch);
        state.durable_epoch = Some(receipt.barrier_epoch);
        state.projection = None;
        state.project = next.phase == Phase::LocalOwned;
        state.pending = None;
        state.record = Some(next.clone());
        state.last = format!("actual transfer receipt={receipt:?}");
        state.last_transfer = Some(receipt);
        // Persist the now-known exact receipt through a separate acknowledged barrier.
        state.checkpoint_pending = true;
        save_record(host, state.source.unwrap(), next)?;
        Ok(())
    }
    fn refusal(&self, host: &mut dyn ServerHost, state: &mut State) -> Result<()> {
        if state.record.as_ref().map(|r| r.phase) != Some(Phase::WorldOwned) {
            return Err(Error::Refused("refuse requires WorldOwned after return"));
        }
        let source = state.source.unwrap();
        let world = state.destination.unwrap();
        let selected = cells();
        let before = (source.revisions(host)?, world.revisions(host)?);
        let occupied = world_cell(selected[2]);
        let air = host
            .block_state_id("minecraft:air")
            .ok_or(Error::Refused("air unavailable"))?;
        if source.block_state(host, selected[0]) != Some(air)
            || [selected[0], selected[1]].into_iter().any(|p| {
                world
                    .block_state(host, world_cell(p))
                    .is_none_or(|s| s == air)
            })
        {
            return Err(Error::Refused(
                "probe requires two real sources and an empty first destination",
            ));
        }
        if world.block_state(host, occupied).is_none_or(|s| s == air) {
            return Err(Error::Refused(
                "second destination is not actually occupied",
            ));
        }
        let mappings = [
            LiveWorldTransferCell::new(world, world_cell(selected[0]), source, selected[0]),
            LiveWorldTransferCell::new(world, world_cell(selected[1]), world, occupied),
        ];
        match source.prepare_transfer(host, &mappings, transfer_limits()) {
            Err(LiveWorldError::Obstructed) => {
                if before != (source.revisions(host)?, world.revisions(host)?) {
                    return Err(Error::Refused(
                        "occupied preparation changed actual revisions",
                    ));
                }
                state.last = "occupied second destination refused before preparation; actual revisions unchanged".into();
                self.release(host, state);
                Ok(())
            }
            Err(error) => Err(error.into()),
            Ok(prepared) => {
                prepared.cancel(host)?;
                Err(Error::Refused(
                    "occupied request unexpectedly prepared; canceled, refusal not proven",
                ))
            }
        }
    }
    fn motion(&self, host: &mut dyn ServerHost, state: &mut State) -> Result<()> {
        let Some((step, base)) = state.motion else {
            return Ok(());
        };
        if step == MOTION_STEPS {
            state.motion = None;
            return Ok(());
        }
        let projection = state
            .projection
            .ok_or(Error::Refused("no active projection"))?;
        let mut record = state.record.clone().unwrap();
        let mut requested = base;
        requested.translation[0] += f64::from(step + 1) * 0.01;
        let yaw = 2.0 * base.rotation[1].atan2(base.rotation[3])
            + f64::from(step + 1) * 0.25_f64.to_radians();
        requested.rotation = [0.0, (yaw * 0.5).sin(), 0.0, (yaw * 0.5).cos()];
        host.transaction(|host, _tx| -> Result<()> {
            let current = projection.sample(host)?;
            let accepted = projection.publish_pose(host, current.revision, requested)?;
            record.accepted = Some(accepted.into()); // Exact returned canonical sample, encoded as IEEE bits.
            record.steps = step + 1;
            save_record(host, state.source.unwrap(), record.clone())
        })?;
        verify_publication(host, state.source.unwrap(), projection, &record)?;
        state.record = Some(record);
        state.motion = Some((step + 1, base));
        Ok(())
    }
    fn advance(&self, host: &mut dyn ServerHost, state: &mut State) -> Result<()> {
        if !host.runtime_levels().is_open(SOURCE) {
            return Ok(());
        }
        if !state.loaded {
            self.load(host, state)?;
        }
        if state.pending.is_some() {
            return self.finish_transfer(host, state);
        }
        if state.checkpoint_pending {
            checkpoint(host, state)?;
            state.checkpoint_pending = false;
            if !state.project {
                self.release(host, state);
            }
            return Ok(());
        }
        if state.restore || state.project {
            if !self.bootstrap(host, state)? {
                return Err(LiveWorldError::NotResident.into());
            }
            return self.project(host, state);
        }
        if let Some(action) = state.action {
            if matches!(
                action,
                Action::Initialize
                    | Action::Project
                    | Action::Capture
                    | Action::Return
                    | Action::Refuse
            ) && !self.bootstrap(host, state)?
            {
                return Err(LiveWorldError::NotResident.into());
            }
            match action {
                Action::Initialize => self.initialize(host, state)?,
                Action::Project => {
                    if state.record.as_ref().map(|r| r.phase) != Some(Phase::LocalOwned) {
                        return Err(Error::Refused("project requires LocalOwned"));
                    }
                    state.project = true;
                }
                Action::Capture => self.transfer(host, state, true)?,
                Action::Return => self.transfer(host, state, false)?,
                Action::Refuse => self.refusal(host, state)?,
                Action::Checkpoint => checkpoint(host, state)?,
                Action::Run => {
                    let projection = state
                        .projection
                        .ok_or(Error::Refused("no active projection"))?;
                    state.motion = Some((0, projection.sample(host)?.current));
                }
                Action::Pause => state.motion = None,
            }
            state.action = None;
        }
        self.motion(host, state)
    }
}
fn feedback(host: &mut dyn ServerHost, key: &str, fallback: &str, value: String) {
    let _ = host
        .commands()
        .feedback(Text::new(TextContent::Translatable {
            key: key.into(),
            fallback: Some(fallback.into()),
            arguments: vec![Text::literal(value)],
        }));
}

#[mod_api::sunrise_mod]
impl Hooks for Fixture {
    fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        registrar
            .register_attachment_type("live_world_fixture:owner", false)
            .unwrap();
        self.controller
            .set(registrar.register_ticket_controller("bootstrap", false))
            .unwrap();
    }
    fn register_commands(
        &self,
        _: &mut dyn ServerHost,
        event: &mut mod_api::RegisterCommandsEvent<'_>,
    ) {
        for verb in [
            "initialize",
            "project",
            "run",
            "pause",
            "checkpoint",
            "capture",
            "return",
            "refuse",
            "read",
            "status",
        ] {
            event.commands.push(mod_api::Command::op(
                &format!("live_world_fixture {verb}"),
                2,
            ));
        }
    }
    fn run_mod_command(
        &self,
        host: &mut dyn ServerHost,
        event: &mut mod_api::RunModCommandEvent<'_>,
    ) -> mod_api::Flow {
        if event.command.literals.first().map(String::as_str) != Some("live_world_fixture") {
            return mod_api::Flow::Continue;
        }
        let mut state = self.state.lock().unwrap();
        let action = match event.command.literals.get(1).map(String::as_str) {
            Some("initialize") => Some(Action::Initialize),
            Some("project") => Some(Action::Project),
            Some("run") => Some(Action::Run),
            Some("pause") => Some(Action::Pause),
            Some("checkpoint") => Some(Action::Checkpoint),
            Some("capture") => Some(Action::Capture),
            Some("return") => Some(Action::Return),
            Some("refuse") => Some(Action::Refuse),
            Some("read" | "status") => {
                let source = state.source;
                let world = state.destination;
                let source_chest = source
                    .and_then(|l| host.block_entity(l, [0, 65, 0]))
                    .and_then(|b| b.data(host));
                let source_furnace = source
                    .and_then(|l| host.block_entity(l, [1, 65, 0]))
                    .and_then(|b| b.data(host));
                let world_chest = world
                    .and_then(|l| host.block_entity(l, world_cell([0, 65, 0])))
                    .and_then(|b| b.data(host));
                let world_furnace = world
                    .and_then(|l| host.block_entity(l, world_cell([1, 65, 0])))
                    .and_then(|b| b.data(host));
                let marker =
                    source.map(|l| OWNER.get_existing(&mut l.attachments(host), Holder::Level));
                let value = format!(
                    "marker={marker:?}; saved={:?}; sample={:?}; members={:?}; revisions={:?}/{:?}; source_chest={source_chest:?}; source_furnace={source_furnace:?}; world_chest={world_chest:?}; world_furnace={world_furnace:?}; queued={:?}; pending={:?}; checkpoint_pending={}; last_transfer={:?}; last_checkpoint={:?}; moving={}; durable_epoch={:?}; fenced={}; last={}",
                    source.map(|l| l.saved_projection(host)),
                    state.projection.map(|p| p.sample(host)),
                    state.projection.map(|p| p.cells(host)),
                    source.map(|l| l.revisions(host)),
                    world.map(|l| l.revisions(host)),
                    state.action,
                    state.pending.as_ref().map(|p| p.0),
                    state.checkpoint_pending,
                    state.last_transfer,
                    state.last_checkpoint,
                    state.motion.is_some(),
                    state.durable_epoch,
                    state.fenced,
                    state.last
                );
                feedback(
                    host,
                    "live_world_fixture.status",
                    "Live-world fixture: %s",
                    value,
                );
                return mod_api::Flow::Cancel;
            }
            _ => return mod_api::Flow::Continue,
        };
        // A command only queues work. No transfer happens during a level or guest turn.
        if state.action.is_some()
            || state.pending.is_some()
            || state.checkpoint_pending
            || state.fenced
        {
            feedback(
                host,
                "live_world_fixture.busy",
                "Live-world fixture refused: %s",
                format!(
                    "queued={:?}; pending={}; fenced={}",
                    state.action,
                    state.pending.is_some(),
                    state.fenced
                ),
            );
        } else {
            state.action = action;
            feedback(
                host,
                "live_world_fixture.queued",
                "Live-world fixture queued: %s",
                format!("{action:?}"),
            );
        }
        mod_api::Flow::Cancel
    }
    fn server_started(&self, host: &mut dyn ServerHost, _: &mut mod_api::ServerStartedEvent) {
        let opened = host.runtime_levels().open(
            SOURCE,
            RuntimeLevel::new("minecraft:overworld", RuntimeGenerator::Void, 1022),
        );
        let mut state = self.state.lock().unwrap();
        *state = State::default();
        match opened {
            Ok(source) => {
                state.source = Some(source);
                state.destination = host.level(DESTINATION);
            }
            Err(error) => {
                state.fenced = true;
                state.last = format!("runtime open refused: {error:?}");
            }
        }
    }
    fn server_tick(&self, host: &mut dyn ServerHost, event: &mut mod_api::ServerTickEvent<'_>) {
        if !matches!(
            event.phase,
            mod_api::ServerTickPhase::ServerTickStart {
                world_running: true
            }
        ) {
            return;
        }
        let mut state = self.state.lock().unwrap();
        if state.fenced || state.source.is_none() || state.destination.is_none() {
            return;
        }
        if let Err(error) = self.advance(host, &mut state) {
            state.last = error.to_string();
            match error {
                Error::Live(LiveWorldError::Busy | LiveWorldError::NotResident) => {}
                Error::Live(LiveWorldError::Storage) => {
                    state.fenced = true;
                    state.motion = None;
                }
                Error::Live(LiveWorldError::NotCommitted) => {
                    // NotCommitted guarantees the staged batch and its metadata did not commit.
                    state.pending = None;
                    state.action = None;
                    state.motion = None;
                    self.release(host, &mut state);
                }
                _ => {
                    state.action = None;
                    state.motion = None;
                    state.project = false;
                    state.restore = false;
                    if !state.loaded
                        || state.pending.is_some()
                        || state.initialization.is_some()
                        || state.checkpoint_pending
                    {
                        state.fenced = true;
                    }
                    self.release(host, &mut state);
                }
            }
        }
    }
    fn server_stopping(&self, host: &mut dyn ServerHost, _: &mut mod_api::ServerStoppingEvent) {
        let mut state = self.state.lock().unwrap();
        if state.fenced {
            return;
        }
        if state.loaded && state.pending.is_none() {
            if let Err(error) = checkpoint(host, &mut state) {
                state.last = format!("stop checkpoint refused: {error}");
            }
        }
        self.release(host, &mut state);
    }
}

// The byte ceiling covers the actual common world/player/metadata afterimages, not just
// these few moved cells. This stays within the host's bounded common checkpoint ceiling.
fn transfer_limits() -> LiveWorldTransferLimits {
    LiveWorldTransferLimits {
        bytes: 64 * 1024 * 1024,
        ..Default::default()
    }
}
