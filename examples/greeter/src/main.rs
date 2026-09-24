//! The server: one backend, every wire.
//!
//! ```text
//! GREETER_BIND=127.0.0.1:4040 cargo run -p example-greeter
//! curl -XPOST localhost:4040/greeter/greet -d '{"name":"cody"}'
//! curl -N   localhost:4040/greeter/greeted            # Server-Sent Events
//! curl -XPOST localhost:4040/note/create -d '{"input":{"title":"hi","body":"there"}}'
//! ```

use architect::Services as _;
use architect::host::EngineHost;
use example_greeter::Memory;

#[derive(architect::Config)]
#[architect(prefix = "GREETER")]
struct Config {
    #[architect(default = "127.0.0.1:4040")]
    bind: String,
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    architect::host::boot("info");
    let config = Config::from_env()?;
    let backend = Memory::default();
    EngineHost::new(backend.clone().into_router(), config.bind)
        .plugin(backend.into_http_router())
        .serve()
        .await?;
    Ok(())
}
