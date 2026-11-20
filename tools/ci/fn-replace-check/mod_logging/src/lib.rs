use mod_api::content::crash::{ModCall, enter_native_mod};
use mod_api::{
    BroadcastMessageEvent, ClientHost, ClientTickPostEvent, Flow, Hooks, Host, HostTransaction,
    LogLevel, LogOutcome, ModLogger, Registrar, RegistrationContext, ServerHost,
    ServerStartedEvent,
};
use std::sync::OnceLock;

const MARKER: &str = "mod-logger fixture: ";

#[derive(Default)]
struct Fixture {
    logger: OnceLock<ModLogger>,
    client_once: OnceLock<()>,
}

#[mod_api::sunrise_mod]
impl Hooks for Fixture {
    fn register(&self, _: &mut dyn Registrar, cx: &RegistrationContext) {
        let logger = {
            let _other = enter_native_mod(ModCall {
                id: "fnr_log_wrong",
                hook: "register",
                part: None,
            });
            cx.logger()
        };
        assert!(logger.enabled(LogLevel::Info));
        assert_eq!(
            logger.log(LogLevel::Info, format_args!("{MARKER}registered {}", 17)),
            LogOutcome::Queued,
        );
        assert!(self.logger.set(logger).is_ok());
    }

    fn client_tick_post(&self, _: &mut dyn ClientHost, _: &mut ClientTickPostEvent) {
        self.client_once.get_or_init(|| {
            let logger = self.logger.get().unwrap();
            assert_eq!(
                logger.log(LogLevel::Warn, format_args!("{MARKER}client first")),
                LogOutcome::Queued,
            );
            assert_eq!(
                logger.log(LogLevel::Error, format_args!("{MARKER}client second")),
                LogOutcome::Queued,
            );
        });
    }

    fn server_started(&self, host: &mut dyn ServerHost, event: &mut ServerStartedEvent) {
        assert!(event.dedicated);
        let logger = self.logger.get().unwrap();
        let result: Result<(), ()> = host.transaction(|_, _| {
            assert_eq!(
                logger.log(LogLevel::Warn, format_args!("{MARKER}before rollback")),
                LogOutcome::Queued,
            );
            Err(())
        });
        assert_eq!(result, Err(()));
        assert_eq!(
            logger.log(LogLevel::Error, format_args!("{MARKER}after rollback")),
            LogOutcome::Queued,
        );
    }

    fn broadcast_message(&self, host: &dyn Host, event: &mut BroadcastMessageEvent<'_>) -> Flow {
        let plain = host.text_string(event.line).unwrap();
        assert!(
            !plain.contains(MARKER),
            "fixture log leaked to chat: {plain}"
        );
        Flow::Continue
    }
}
