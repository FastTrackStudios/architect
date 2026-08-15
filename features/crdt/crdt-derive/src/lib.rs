//! `entity_crdt!` — generate `EntityCrdt` + `*RepoLoro` from a
//! single declaration.
//!
//! ## Example
//!
//! ```ignore
//! use crdt::entity_crdt;
//! use project_proto::{Task, TaskCreate, TaskList, TaskRepo, TaskUpdate};
//!
//! entity_crdt! {
//!     pub Task,                          // base name → TaskEntity, TaskRepoLoro
//!     root = "tasks",
//!     fields {
//!         id: uuid (pk),
//!         project_id: uuid,
//!         title: str (sortable),
//!         done: bool (sortable),
//!     },
//! }
//! ```
//!
//! Expands to ≈80 lines of `EntityCrdt` impl + `TaskRepoLoro`
//! newtype + `TaskRepo` forward impl that previously had to be
//! written by hand.
//!
//! ## Naming convention
//!
//! From a base name `Foo` the macro reaches for:
//!
//! | Item | Looked up as |
//! |------|--------------|
//! | wire | `Foo` |
//! | create payload | `FooCreate` |
//! | update payload | `FooUpdate` |
//! | list wrapper | `FooList` |
//! | architect-emitted repo trait | `FooRepo` |
//!
//! The macro emits `FooEntity` (marker) + `FooRepoLoro`
//! (Loro-backed `FooRepo` impl) into the calling module.
//!
//! ## Supported field types
//!
//! `uuid`, `opt_uuid`, `str`, `opt_str`, `bool`, `opt_bool`,
//! `i64`, `opt_i64`, `dt`, `opt_dt`, `string_list`. Each maps
//! to the matching `crdt::codec::{read,write}_<ty>` helper.
//!
//! ## Field flags
//!
//! - `(pk)` — primary key. Skipped in `apply_update` (the macro
//!   relies on the wire's `Create` payload generating a fresh
//!   UUID via `Uuid::new_v4()`; if your PK isn't a Uuid this
//!   macro isn't for you yet).
//! - `(sortable)` — included in the `sort_items` match. Other
//!   names are rejected with `RepoError::Internal`.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{
    Ident, LitStr, Token, Visibility,
    parse::{Parse, ParseStream},
    parse_macro_input,
    punctuated::Punctuated,
};

// ── AST ───────────────────────────────────────────────────────────────

struct EntityCrdtInput {
    vis: Visibility,
    base: Ident,
    root: LitStr,
    fields: Vec<FieldSpec>,
    /// When set, the macro additionally manages `created_at` /
    /// `updated_at` fields: fills both with `Utc::now()` in
    /// `from_create`, codecs them as `dt` in encode/decode, and
    /// bumps `updated_at` at the end of every `apply_update`.
    /// Matches architect's `#[architect(exclude(create, update),
    /// on_create = Utc::now())]` audit-pair convention.
    audit_timestamps: bool,
}

struct FieldSpec {
    name: Ident,
    kind: FieldKind,
    flags: FieldFlags,
}

#[derive(Default)]
struct FieldFlags {
    pk: bool,
    sortable: bool,
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum FieldKind {
    Uuid,
    OptUuid,
    Str,
    OptStr,
    Bool,
    OptBool,
    I32,
    OptI32,
    I64,
    OptI64,
    Dt,
    OptDt,
    StringList,
}

impl FieldKind {
    fn from_ident(id: &Ident) -> Option<Self> {
        Some(match id.to_string().as_str() {
            "uuid" => Self::Uuid,
            "opt_uuid" => Self::OptUuid,
            "str" => Self::Str,
            "opt_str" => Self::OptStr,
            "bool" => Self::Bool,
            "opt_bool" => Self::OptBool,
            "i32" => Self::I32,
            "opt_i32" => Self::OptI32,
            "i64" => Self::I64,
            "opt_i64" => Self::OptI64,
            "dt" => Self::Dt,
            "opt_dt" => Self::OptDt,
            "string_list" => Self::StringList,
            _ => return None,
        })
    }

    fn writer(self) -> Ident {
        format_ident!(
            "write_{}",
            match self {
                Self::Uuid => "uuid",
                Self::OptUuid => "opt_uuid",
                Self::Str => "str",
                Self::OptStr => "opt_str",
                Self::Bool => "bool",
                Self::OptBool => "opt_bool",
                // i32 / opt_i32 ride i64 codecs with a cast at the
                // emit site; no native i32 codec exists in
                // crdt::codec today.
                Self::I32 => "i64",
                Self::OptI32 => "opt_i64",
                Self::I64 => "i64",
                Self::OptI64 => "opt_i64",
                Self::Dt => "dt",
                Self::OptDt => "opt_dt",
                Self::StringList => "string_list",
            }
        )
    }

    fn reader(self) -> Ident {
        format_ident!(
            "read_{}",
            match self {
                Self::Uuid => "uuid",
                Self::OptUuid => "opt_uuid",
                Self::Str => "str",
                Self::OptStr => "opt_str",
                Self::Bool => "bool",
                Self::OptBool => "opt_bool",
                Self::I32 => "i64",
                Self::OptI32 => "opt_i64",
                Self::I64 => "i64",
                Self::OptI64 => "opt_i64",
                Self::Dt => "dt",
                Self::OptDt => "opt_dt",
                Self::StringList => "string_list",
            }
        )
    }

    /// `true` when the field type takes `&value` rather than `value`
    /// in its setter.
    fn needs_ref(self) -> bool {
        matches!(self, Self::Str | Self::StringList)
    }
}

// ── Parser ────────────────────────────────────────────────────────────

impl Parse for EntityCrdtInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let vis: Visibility = input.parse()?;
        let base: Ident = input.parse()?;
        input.parse::<Token![,]>()?;

        let mut root: Option<LitStr> = None;
        let mut fields: Option<Vec<FieldSpec>> = None;
        let mut audit_timestamps = false;

        while !input.is_empty() {
            let key: Ident = input.parse()?;
            match key.to_string().as_str() {
                "root" => {
                    input.parse::<Token![=]>()?;
                    root = Some(input.parse()?);
                }
                "fields" => {
                    let content;
                    syn::braced!(content in input);
                    let parsed: Punctuated<FieldSpec, Token![,]> =
                        content.parse_terminated(FieldSpec::parse, Token![,])?;
                    fields = Some(parsed.into_iter().collect());
                }
                "audit_timestamps" => {
                    audit_timestamps = true;
                }
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!(
                            "unknown entity_crdt key `{other}` (expected `root`, `fields`, or `audit_timestamps`)"
                        ),
                    ));
                }
            }
            // Optional trailing comma between top-level entries.
            let _ = input.parse::<Token![,]>();
        }

        let root = root.ok_or_else(|| syn::Error::new(base.span(), "missing `root = \"...\"`"))?;
        let fields =
            fields.ok_or_else(|| syn::Error::new(base.span(), "missing `fields { ... }` block"))?;
        if fields.is_empty() {
            return Err(syn::Error::new(
                base.span(),
                "fields block must not be empty",
            ));
        }

        Ok(Self {
            vis,
            base,
            root,
            fields,
            audit_timestamps,
        })
    }
}

impl Parse for FieldSpec {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let name: Ident = input.parse()?;
        input.parse::<Token![:]>()?;
        let kind_ident: Ident = input.parse()?;
        let kind = FieldKind::from_ident(&kind_ident).ok_or_else(|| {
            syn::Error::new(
                kind_ident.span(),
                format!(
                    "unknown field type `{}` (supported: uuid, opt_uuid, str, opt_str, bool, opt_bool, i64, opt_i64, dt, opt_dt, string_list)",
                    kind_ident
                ),
            )
        })?;

        let mut flags = FieldFlags::default();
        if input.peek(syn::token::Paren) {
            let content;
            syn::parenthesized!(content in input);
            let punctuated: Punctuated<Ident, Token![,]> =
                content.parse_terminated(Ident::parse, Token![,])?;
            for flag in punctuated {
                match flag.to_string().as_str() {
                    "pk" => flags.pk = true,
                    "sortable" => flags.sortable = true,
                    other => {
                        return Err(syn::Error::new(
                            flag.span(),
                            format!("unknown field flag `{other}` (supported: `pk`, `sortable`)"),
                        ));
                    }
                }
            }
        }
        Ok(Self { name, kind, flags })
    }
}

// ── Expansion ─────────────────────────────────────────────────────────

#[proc_macro]
pub fn entity_crdt(tokens: TokenStream) -> TokenStream {
    let input = parse_macro_input!(tokens as EntityCrdtInput);
    expand(input)
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

fn expand(input: EntityCrdtInput) -> syn::Result<TokenStream2> {
    let EntityCrdtInput {
        vis,
        base,
        root,
        fields,
        audit_timestamps,
    } = input;

    // Names derived from base.
    let wire = base.clone();
    let create = format_ident!("{}Create", base);
    let update = format_ident!("{}Update", base);
    let list = format_ident!("{}List", base);
    let repo_trait = format_ident!("{}Repo", base);
    let entity = format_ident!("{}Entity", base);
    let repo_loro = format_ident!("{}RepoLoro", base);

    // Find the PK field. Must be a single Uuid.
    let pk = fields.iter().find(|f| f.flags.pk).ok_or_else(|| {
        syn::Error::new(
            base.span(),
            "exactly one field must be marked `(pk)` (and it must be `uuid`)",
        )
    })?;
    if pk.kind != FieldKind::Uuid {
        return Err(syn::Error::new(
            pk.name.span(),
            "primary key must be of type `uuid`",
        ));
    }
    let pk_name = &pk.name;

    // `from_create`: take every NON-PK field from Create, generate
    // a fresh Uuid for the PK. With `audit_timestamps`, the audit
    // pair is appended at the end with `Utc::now()` since those
    // fields are excluded from the Create payload by architect.
    let from_create_fields: Vec<_> = fields
        .iter()
        .map(|f| {
            let n = &f.name;
            if f.flags.pk {
                quote! { #n: ::uuid::Uuid::new_v4() }
            } else {
                quote! { #n: c.#n }
            }
        })
        .chain(if audit_timestamps {
            vec![
                quote! { created_at: ::chrono::Utc::now() },
                quote! { updated_at: ::chrono::Utc::now() },
            ]
        } else {
            vec![]
        })
        .collect();

    // `encode_into`: write every field.
    let audit_encode = if audit_timestamps {
        quote! {
            ::crdt::codec::write_dt(m, "created_at", e.created_at)?;
            ::crdt::codec::write_dt(m, "updated_at", e.updated_at)?;
        }
    } else {
        quote! {}
    };
    let encode_calls = fields.iter().map(|f| {
        let n = &f.name;
        let key = LitStr::new(&n.to_string(), n.span());
        let writer = f.kind.writer();
        match f.kind {
            FieldKind::OptStr => quote! {
                ::crdt::codec::#writer(m, #key, e.#n.as_deref())?;
            },
            FieldKind::StringList => quote! {
                ::crdt::codec::#writer(m, #key, &e.#n)?;
            },
            // i32 / opt_i32 ride the i64 codec with explicit casts.
            FieldKind::I32 => quote! {
                ::crdt::codec::#writer(m, #key, e.#n as i64)?;
            },
            FieldKind::OptI32 => quote! {
                ::crdt::codec::#writer(m, #key, e.#n.map(|v| v as i64))?;
            },
            k if k.needs_ref() => quote! {
                ::crdt::codec::#writer(m, #key, &e.#n)?;
            },
            _ => quote! {
                ::crdt::codec::#writer(m, #key, e.#n)?;
            },
        }
    });

    // `decode_from`: read every field, build the wire.
    let audit_decode: Vec<_> = if audit_timestamps {
        vec![
            quote! { created_at: ::crdt::codec::read_dt(m, "created_at")? },
            quote! { updated_at: ::crdt::codec::read_dt(m, "updated_at")? },
        ]
    } else {
        vec![]
    };
    // Bump updated_at at the end of apply_update so audited rows
    // get a fresh modified-time even when the Update payload itself
    // doesn't carry one.
    let audit_update_bump = if audit_timestamps {
        quote! { ::crdt::codec::write_dt(m, "updated_at", ::chrono::Utc::now())?; }
    } else {
        quote! {}
    };
    let decode_fields = fields.iter().map(|f| {
        let n = &f.name;
        let key = LitStr::new(&n.to_string(), n.span());
        let reader = f.kind.reader();
        match f.kind {
            FieldKind::I32 => quote! {
                #n: ::crdt::codec::#reader(m, #key)? as i32
            },
            FieldKind::OptI32 => quote! {
                #n: ::crdt::codec::#reader(m, #key)?.map(|v| v as i32)
            },
            _ => quote! {
                #n: ::crdt::codec::#reader(m, #key)?
            },
        }
    });

    // `apply_update`: write fields that are Some in the Update.
    // Skip the PK (architect's Update struct excludes it).
    let apply_update_arms = fields.iter().filter(|f| !f.flags.pk).map(|f| {
        let n = &f.name;
        let key = LitStr::new(&n.to_string(), n.span());
        let writer = f.kind.writer();
        match f.kind {
            FieldKind::OptStr => quote! {
                if let Some(v) = u.#n {
                    ::crdt::codec::#writer(m, #key, v.as_deref())?;
                }
            },
            FieldKind::StringList => quote! {
                if let Some(v) = u.#n {
                    ::crdt::codec::#writer(m, #key, &v)?;
                }
            },
            FieldKind::I32 => quote! {
                if let Some(v) = u.#n {
                    ::crdt::codec::#writer(m, #key, v as i64)?;
                }
            },
            FieldKind::OptI32 => quote! {
                if let Some(v) = u.#n {
                    ::crdt::codec::#writer(m, #key, v.map(|x| x as i64))?;
                }
            },
            k if k.needs_ref() => quote! {
                if let Some(v) = u.#n {
                    ::crdt::codec::#writer(m, #key, &v)?;
                }
            },
            _ => quote! {
                if let Some(v) = u.#n {
                    ::crdt::codec::#writer(m, #key, v)?;
                }
            },
        }
    });

    // `sort_items`: one arm per (sortable) field.
    let sort_arms = fields.iter().filter(|f| f.flags.sortable).map(|f| {
        let n = &f.name;
        let key = LitStr::new(&n.to_string(), n.span());
        match f.kind {
            FieldKind::Str => quote! {
                #key => items.sort_by(|a, b| a.#n.cmp(&b.#n)),
            },
            FieldKind::OptStr => quote! {
                #key => items.sort_by(|a, b| a.#n.cmp(&b.#n)),
            },
            _ => quote! {
                #key => items.sort_by(|a, b| a.#n.cmp(&b.#n)),
            },
        }
    });

    Ok(quote! {
        // ── Marker ─────────────────────────────────────────────────
        #vis struct #entity;

        // ── EntityCrdt impl ────────────────────────────────────────
        impl ::crdt::EntityCrdt for #entity {
            type Wire = #wire;
            type Create = #create;
            type Update = #update;
            type List = #list;

            const ROOT: &'static str = #root;

            fn id(e: &#wire) -> ::uuid::Uuid {
                e.#pk_name
            }

            fn from_create(c: #create) -> #wire {
                #wire { #(#from_create_fields),* }
            }

            fn encode_into(m: &::loro::LoroMap, e: &#wire) -> ::core::result::Result<(), ::architect::RepoError> {
                #(#encode_calls)*
                #audit_encode
                Ok(())
            }

            fn decode_from(m: &::loro::LoroMap) -> ::core::result::Result<#wire, ::architect::RepoError> {
                Ok(#wire { #(#decode_fields),* #(, #audit_decode)* })
            }

            fn apply_update(m: &::loro::LoroMap, u: #update) -> ::core::result::Result<(), ::architect::RepoError> {
                #(#apply_update_arms)*
                #audit_update_bump
                Ok(())
            }

            fn build_list(items: Vec<#wire>, total: u32, page: ::architect::Page) -> #list {
                #list { items, total, page }
            }

            fn sort_items(items: &mut [#wire], field: &str, order: ::architect::SortOrder)
                -> ::core::result::Result<(), ::architect::RepoError>
            {
                match field {
                    #(#sort_arms)*
                    other => return Err(::architect::RepoError::Internal(
                        format!("unknown sort field: {other}")
                    )),
                }
                if order == ::architect::SortOrder::Desc {
                    items.reverse();
                }
                Ok(())
            }
        }

        // ── RepoLoro newtype ───────────────────────────────────────
        #[derive(Clone)]
        #vis struct #repo_loro {
            inner: ::crdt::LoroRepo<#entity>,
        }

        impl #repo_loro {
            pub fn new(doc: &::crdt::CrdtDoc) -> Self {
                Self { inner: doc.repo() }
            }
        }

        impl #repo_trait for #repo_loro {
            async fn get(&self, id: ::uuid::Uuid) -> ::core::result::Result<#wire, ::architect::RepoError> {
                self.inner.get(id).await
            }
            async fn list(
                &self,
                page: ::architect::Page,
                sort: ::core::option::Option<::architect::Sort>,
                filter: ::core::option::Option<::architect::Filter>,
            ) -> ::core::result::Result<#list, ::architect::RepoError> {
                self.inner.list(page, sort, filter).await
            }
            async fn create(&self, input: #create) -> ::core::result::Result<#wire, ::architect::RepoError> {
                self.inner.create(input).await
            }
            async fn update(&self, id: ::uuid::Uuid, input: #update) -> ::core::result::Result<#wire, ::architect::RepoError> {
                self.inner.update(id, input).await
            }
            async fn delete(&self, id: ::uuid::Uuid) -> ::core::result::Result<(), ::architect::RepoError> {
                self.inner.delete(id).await
            }
        }
    })
}
