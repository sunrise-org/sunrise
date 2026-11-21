//! A retained payload callback fired outside the world (`callback_owner::call`).
use mod_api::{
    ClientHost, Hooks, IPayloadContext, IPayloadHandler, ModEntry, PayloadBytes, PayloadId,
    Registrar, ServerHost,
};
use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};

const NAME: &str = "game::payload_transport::phase_refusal_tests::a_payload_handler_outside_the_world_is_refused_not_a_panic";
const CHANNEL: &str = "refusalprobe:message";

/// A handler that reads the world through the Host, as a mod's handler may.
struct Reads {
    entered: AtomicU32,
    completed: AtomicU32,
}
impl IPayloadHandler for Reads {
    fn handle(
        &self,
        _: PayloadBytes,
        context: &mut dyn IPayloadContext,
    ) -> Result<(), mod_api::PayloadHandleError> {
        self.entered.fetch_add(1, Ordering::Relaxed);
        let host = context
            .server_host()
            .map_err(|e| mod_api::PayloadHandleError::Failed(format!("{e:?}")))?;
        let _ = host.level("minecraft:overworld");
        self.completed.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}
struct Declares(Arc<Reads>);
#[mod_api::__subscriptions]
impl Hooks for Declares {
    fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        registrar
            .payload_handlers()
            .registrar("1")
            .play_to_server(PayloadId(CHANNEL.into()), self.0.clone());
    }
}

/// The context a handler is given: the server host is the game.
struct Context<'g, 'a>(&'g mut crate::game::Game<'a>);
impl IPayloadContext for Context<'_, '_> {
    fn player(&self) -> Result<mod_api::PlayerRef, mod_api::PayloadContextError> {
        Err(mod_api::PayloadContextError::NoPlayer)
    }
    fn flow(&self) -> mod_api::PayloadDirection {
        mod_api::PayloadDirection::ToServer
    }
    fn protocol(&self) -> mod_api::PayloadProtocol {
        mod_api::PayloadProtocol::Play
    }
    fn server_host(&mut self) -> Result<&mut dyn ServerHost, mod_api::PayloadContextError> {
        Ok(&mut *self.0)
    }
    fn client_host(&mut self) -> Result<&mut dyn ClientHost, mod_api::PayloadContextError> {
        Err(mod_api::PayloadContextError::WrongSide)
    }
    fn reply(
        &mut self,
        _: &PayloadId,
        _: PayloadBytes,
    ) -> Result<mod_api::PayloadAccepted, mod_api::PayloadSendError> {
        unreachable!("the handler never replies")
    }
    fn check_payload_token(
        &self,
        _: &mod_api::PayloadToken,
    ) -> Result<(), mod_api::PayloadSendError> {
        unreachable!("the handler never sends")
    }
    fn send_to(
        &mut self,
        _: &[mod_api::PlayerRef],
        _: &mod_api::PayloadId,
        _: mod_api::PayloadBytes,
    ) -> Result<u32, mod_api::PayloadSendError> {
        unreachable!()
    }
    fn disconnect(&mut self, _: mod_api::Text) {
        unreachable!("the handler never disconnects")
    }
    fn enqueue_work(
        &mut self,
        _: Arc<dyn mod_api::PayloadWork>,
    ) -> Result<mod_api::PayloadWorkStatus, mod_api::PayloadContextError> {
        unreachable!("the handler queues no work")
    }
}

/// Callbacks run in ticks and on the network, after the thread reached World. Fired in Setup and in
/// Reloading anyway, a handler's world read is a refused call, not the no-scope panic, and the
/// owner is not poisoned: a second call in World runs the handler to its end.
#[test]
#[ignore = "daily"]
fn a_payload_handler_outside_the_world_is_refused_not_a_panic() {
    use mod_api::__private::{ModPhase, PhaseOwner};
    if super::super::native_entity_refs::tests::run_fresh_cases(
        NAME,
        "SUNRISE_PAYLOAD_PHASE_CHILD",
        &["valid"],
    ) {
        return;
    }
    let reads = Arc::new(Reads {
        entered: AtomicU32::new(0),
        completed: AtomicU32::new(0),
    });
    let mods: mod_api::Mods = vec![ModEntry::new(
        "refusalprobe",
        Box::new(Declares(reads.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        super::super::native_entity_refs::tests::with_game(|game| {
            let receiver = mod_api::__private::game::installed_payloads()
                .unwrap()
                .handler(
                    CHANNEL,
                    mod_api::PayloadProtocol::Play,
                    mod_api::PayloadDirection::ToServer,
                )
                .unwrap();
            for reloading in [false, true] {
                let setup = PhaseOwner::scope(ModPhase::Setup);
                let _reload = reloading.then(|| setup.reloading().unwrap());
                let refused = receiver
                    .handle(PayloadBytes(vec![]), &mut Context(game))
                    .unwrap_err();
                assert!(
                    matches!(refused, mod_api::PayloadHandleError::Failed(_)),
                    "{refused:?}"
                );
                assert_eq!(reads.completed.load(Ordering::Relaxed), 0);
                assert!(receiver.enabled(), "a refusal does not poison the owner");
            }
            let _world = PhaseOwner::scope(ModPhase::World);
            receiver
                .handle(PayloadBytes(vec![]), &mut Context(game))
                .unwrap();
            assert_eq!(reads.entered.load(Ordering::Relaxed), 3);
            assert_eq!(reads.completed.load(Ordering::Relaxed), 1);
            assert!(receiver.enabled());
        });
    });
}
