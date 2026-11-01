//! Typed identifiers: a root's id and a version's are both UUIDs, and a
//! method taking two of them must not compile with them swapped.

use facet::Facet;
use uuid::Uuid;

/// A transparent newtype over `Uuid`, minted by the server.
macro_rules! uuid_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Facet)]
        #[repr(C)]
        pub struct $name(pub Uuid);

        impl $name {
            #[must_use]
            pub const fn new(id: Uuid) -> Self {
                Self(id)
            }

            /// A fresh random id.
            #[cfg(feature = "server")]
            #[must_use]
            pub fn generate() -> Self {
                Self(Uuid::new_v4())
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                std::fmt::Display::fmt(&self.0, f)
            }
        }
    };
}

uuid_id!(
    /// A root: a tree of files and folders an app keeps its things in
    /// (`signal/impulse-responses`, a song's `session/<song>`).
    RootId
);
uuid_id!(
    /// One version of one file: what a save, a copy or a restore wrote.
    VersionId
);
uuid_id!(
    /// A name a person gave a version ("Mix B for the client").
    LabelId
);
uuid_id!(
    /// A grant of capabilities over a folder to a person.
    GrantId
);
uuid_id!(
    /// An upload in progress. Lives until it lands or is aborted.
    UploadId
);

/// What a file's bytes are: their sha256, as 64 lowercase hex digits.
///
/// Derived from the bytes, not minted, so the same bytes are one blob in
/// the store however many paths, versions and roots hold them — and the
/// same address a document's blobs use (`architect::blob`). It is also a
/// file's etag: a save can say "replace exactly this" ([`crate::Expect`]).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Facet)]
#[facet(transparent)]
#[repr(C)]
pub struct ContentId(pub String);

impl ContentId {
    #[must_use]
    pub fn new(hex: impl Into<String>) -> Self {
        Self(hex.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether this is a well-formed sha256 (64 hex digits).
    #[must_use]
    pub fn is_sha256(&self) -> bool {
        self.0.len() == 64 && self.0.bytes().all(|b| b.is_ascii_hexdigit())
    }
}

impl std::fmt::Display for ContentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
