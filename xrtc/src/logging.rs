use std::panic::PanicInfo;

use backtrace::Backtrace;
use tracing::error;
use tracing_log::LogTracer;
use tracing_subscriber::filter;
use tracing_subscriber::fmt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::Layer;
use tracing_subscriber::Registry;

fn log_panic(panic: &PanicInfo) {
    let backtrace = Backtrace::new();
    let backtrace = format!("{backtrace:?}");
    if let Some(location) = panic.location() {
        error!(
            message = %panic,
            backtrace = %backtrace,
            panic.file = location.file(),
            panic.line = location.line(),
            panic.column = location.column(),
        );
    } else {
        error!(message = %panic, backtrace = %backtrace);
    }
}

pub fn set_panic_hook() {
    // Set a panic hook that records the panic as a `tracing` event at the
    // `ERROR` verbosity level.
    //
    // If we are currently in a span when the panic occurred, the logged event
    // will include the current span, allowing the context in which the panic
    // occurred to be recorded.
    std::panic::set_hook(Box::new(|panic| {
        log_panic(panic);
    }));
}

pub fn init() {
    set_panic_hook();

    let subscriber = Registry::default();

    // Stderr
    let subscriber = subscriber.with(
        fmt::layer().with_writer(std::io::stderr).with_filter(
            filter::EnvFilter::builder()
                .with_default_directive(filter::LevelFilter::INFO.into())
                .from_env_lossy(),
        ),
    );

    // Enable log compatible layer to convert log record to tracing span.
    LogTracer::init().unwrap();

    tracing::subscriber::set_global_default(subscriber).unwrap();
}
