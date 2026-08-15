//! `clients!` — generate a client-registry struct from a service list.
//!
//! A consumer app talking to a multi-service backend ends up with one
//! vox-generated `<Trait>Client` per service, all constructed from the
//! same [`vox::Caller`]. Hand-writing that registry is pure ceremony —
//! a struct with N fields plus a constructor with N identical
//! `FooClient::new(caller.clone())` lines that must be edited in two
//! places every time a service lands.
//!
//! `clients!` collapses the registry to its field list:
//!
//! ```ignore
//! architect::clients! {
//!     /// Service clients for a DAW connection.
//!     pub struct DawClients {
//!         pub(crate) transport: TransportClient,
//!         pub(crate) markers: MarkersClient,
//!         pub(crate) tracks: TracksClient,
//!     }
//! }
//!
//! let clients = DawClients::new(caller);
//! clients.transport.play().await?;
//! ```
//!
//! The generated struct additionally retains the original `Caller`
//! (via a `caller()` accessor) so consumers can build further service
//! clients over the same connection, and derives `Clone` — every vox
//! client is a cheap handle.

/// Generate a client-registry struct: one field per vox client type,
/// a `new(Caller)` that fans the caller out to every client, and the
/// retained caller for late-bound extras. See the module docs.
#[cfg(feature = "vox")]
#[macro_export]
macro_rules! clients {
    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident {
            $(
                $(#[$fmeta:meta])*
                $fvis:vis $field:ident : $client:ty
            ),* $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone)]
        $vis struct $name {
            $(
                $(#[$fmeta])*
                $fvis $field: $client,
            )*
            /// Original caller — kept so additional service clients can
            /// be built on the same connection.
            __caller: $crate::vox::Caller,
        }

        impl $name {
            /// Build every client over one shared caller.
            $vis fn new(caller: $crate::vox::Caller) -> Self {
                Self {
                    $(
                        $field: <$client>::new(caller.clone()),
                    )*
                    __caller: caller,
                }
            }

            /// The underlying caller — build additional service clients
            /// on the same connection.
            $vis fn caller(&self) -> &$crate::vox::Caller {
                &self.__caller
            }
        }
    };
}
