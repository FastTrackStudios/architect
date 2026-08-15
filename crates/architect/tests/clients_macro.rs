//! Compile-shape test for `architect::clients!` — the registry struct,
//! fan-out constructor, and retained caller. Runs only with the `vox`
//! feature (the macro is gated on it).
#![cfg(feature = "vox")]

use vox::Caller;

/// Stand-in for a vox-generated `<Trait>Client`: `new(Caller)` + Clone.
#[derive(Clone)]
struct FakeClient {
    _caller: Caller,
}

impl FakeClient {
    fn new(caller: Caller) -> Self {
        Self { _caller: caller }
    }
}

architect::clients! {
    /// Registry over two services.
    pub struct TestClients {
        pub transport: FakeClient,
        pub(crate) markers: FakeClient,
    }
}

/// Never called — proves the generated constructor and accessor
/// typecheck against a real `Caller`.
#[allow(dead_code)]
fn constructs(caller: Caller) {
    let clients = TestClients::new(caller);
    let _ = clients.caller();
    let _ = clients.transport.clone();
    let _ = clients.markers.clone();
    let _ = clients.clone();
}

#[test]
fn clients_macro_compiles() {}
