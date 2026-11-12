//! `process-view-demo`: the lab's process view (`demo::Orbits`) served
//! from a program of its own. The app starts it with the socket in
//! `ARCHITECT_VIEW_FD`; run by hand it says so and exits.

fn main() -> std::process::ExitCode {
    #[cfg(target_os = "linux")]
    {
        use architect_process_view::{demo, guest};
        match guest::serve(|ctx| Box::new(demo::Orbits::new(ctx))) {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("process-view-demo: {e}");
                std::process::ExitCode::FAILURE
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("process-view-demo: process views run on Linux only for now");
        std::process::ExitCode::FAILURE
    }
}
