//! `#[derive(Entity)]` — the architect macro.
//!
//! See the crate-level docs in `architect` for the conceptual overview.
//! This file is the emission engine: parse the `#[architect(...)]`
//! container + field attributes, then synthesise the wire types, the
//! repo trait, and (under `--features server`) the SeaORM bridge.

use heck::{ToPascalCase, ToSnakeCase, ToTitleCase};
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{
    Data, DeriveInput, Expr, Field, Fields, Ident, LitBool, LitStr, Result, Type, parse_macro_input,
};

// ── Attribute parsing ─────────────────────────────────────────────────

#[derive(Default)]
struct ContainerAttrs {
    table_name: Option<String>,
    emit_repo: bool,
    /// Also emit the client-side state layer (store binding, data hooks,
    /// optimistic mutations), gated on the user crate's `atom` + `vox`
    /// features. Requires `repo`.
    emit_store: bool,
    /// Also emit typed form-field bindings (`<E>CreateFields` /
    /// `<E>UpdateFields` + hooks) over the Create/Update payloads, gated
    /// on the user crate's `form` feature.
    emit_form: bool,
    /// Also emit the live-event story: the `<E>Event` enum, the
    /// `<E>Events` subscribe trait, the `<E>Evented<R>` publish-through
    /// wrapper whose `Services` bundle mounts repo + events together, and
    /// (with `store`) the client subscription hook. Requires `repo`.
    emit_events: bool,
    /// Also emit the CRDT story: the `EntityCrdt` impl (field ↔ LoroMap
    /// codec), the `<E>RepoLoro` Loro-backed repo, and (with the user
    /// crate's `atom` feature) the replica-backed hooks + write actions.
    /// Gated on the user crate's `crdt` feature. Requires `repo`.
    emit_crdt: bool,
    /// `#[architect(page_size = N)]` — rows per page for the derived list
    /// hook (default 100).
    page_size: Option<u32>,
}

#[derive(Default)]
struct FieldAttrs {
    primary_key: bool,
    auto_increment: Option<bool>,
    on_create: Option<Expr>,
    on_update: Option<Expr>,
    filterable: bool,
    sortable: bool,
    fulltext: bool,
    exclude_create: bool,
    exclude_update: bool,
    /// Store this field as a JSON column in the SeaORM model. Required
    /// for `Vec<T>` / structured types — `sea_orm::Value` has no
    /// blanket `From<Vec<T>>` impl, so without this attribute the
    /// generated DeriveEntityModel fails to compile on `server`
    /// builds. Container-shaped fields (`Vec<T>` except `Vec<u8>`,
    /// maps, sets) that lack this attribute are a **derive-time error**
    /// so the failure points at the declaration, not at SeaORM
    /// internals. The field type must be `serde::Serialize +
    /// DeserializeOwned`; sea-orm's `with-json` feature handles the
    /// rest. No-op when the `server` feature isn't active.
    json: bool,
    /// `#[architect(form(optional))]` — the generated form field accepts
    /// an empty value (String fields only; the default is required.
    /// Using it on a non-String field is a derive-time error).
    form_optional: bool,
    /// `#[architect(form(label = "…"))]` — display label for the
    /// generated form field (default: Title Case of the field name).
    form_label: Option<String>,
}

fn parse_container_attrs(attrs: &[syn::Attribute]) -> Result<ContainerAttrs> {
    let mut out = ContainerAttrs::default();
    for attr in attrs {
        if !attr.path().is_ident("architect") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("table_name") {
                let s: LitStr = meta.value()?.parse()?;
                out.table_name = Some(s.value());
            } else if meta.path.is_ident("repo") {
                out.emit_repo = true;
            } else if meta.path.is_ident("store") {
                out.emit_store = true;
            } else if meta.path.is_ident("form") {
                out.emit_form = true;
            } else if meta.path.is_ident("events") {
                out.emit_events = true;
            } else if meta.path.is_ident("crdt") {
                out.emit_crdt = true;
            } else if meta.path.is_ident("page_size") {
                let n: syn::LitInt = meta.value()?.parse()?;
                out.page_size = Some(n.base10_parse()?);
            } else {
                return Err(meta.error("unknown architect container attribute"));
            }
            Ok(())
        })?;
    }
    Ok(out)
}

fn parse_field_attrs(field: &Field) -> Result<FieldAttrs> {
    let mut out = FieldAttrs::default();
    for attr in &field.attrs {
        if !attr.path().is_ident("architect") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            let p = &meta.path;
            if p.is_ident("primary_key") {
                out.primary_key = true;
            } else if p.is_ident("auto_increment") {
                let b: LitBool = meta.value()?.parse()?;
                out.auto_increment = Some(b.value);
            } else if p.is_ident("on_create") {
                let e: Expr = meta.value()?.parse()?;
                out.on_create = Some(e);
            } else if p.is_ident("on_update") {
                let e: Expr = meta.value()?.parse()?;
                out.on_update = Some(e);
            } else if p.is_ident("filterable") {
                out.filterable = true;
            } else if p.is_ident("sortable") {
                out.sortable = true;
            } else if p.is_ident("fulltext") {
                out.fulltext = true;
            } else if p.is_ident("json") {
                out.json = true;
            } else if p.is_ident("form") {
                meta.parse_nested_meta(|inner| {
                    if inner.path.is_ident("optional") {
                        out.form_optional = true;
                    } else if inner.path.is_ident("label") {
                        let s: LitStr = inner.value()?.parse()?;
                        out.form_label = Some(s.value());
                    } else {
                        return Err(inner.error("unknown architect form attribute"));
                    }
                    Ok(())
                })?;
            } else if p.is_ident("exclude") {
                meta.parse_nested_meta(|inner| {
                    if inner.path.is_ident("create") {
                        out.exclude_create = true;
                    } else if inner.path.is_ident("update") {
                        out.exclude_update = true;
                    } else {
                        return Err(inner.error("unknown exclude target"));
                    }
                    Ok(())
                })?;
            } else {
                return Err(meta.error("unknown architect field attribute"));
            }
            Ok(())
        })?;
    }
    Ok(out)
}

// ── Derive entry point ────────────────────────────────────────────────

#[proc_macro_derive(Entity, attributes(architect))]
pub fn derive_entity(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand(input) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

struct ParsedField<'a> {
    ident: &'a Ident,
    ty: &'a Type,
    attrs: FieldAttrs,
    /// Pass-through attributes from the source field — everything
    /// that isn't `#[architect(...)]`. Forwarded onto the emitted
    /// `<Entity>Create` field so `#[dummy(faker = "...")]` annotations
    /// (and any other field-level derive helpers like `#[serde(...)]`)
    /// reach the generated Dummy impl that `seed_fake_<entity>`
    /// drives.
    forward_attrs: Vec<syn::Attribute>,
}

fn expand(input: DeriveInput) -> Result<TokenStream2> {
    let ident = input.ident.clone();
    let vis = input.vis.clone();
    let container = parse_container_attrs(&input.attrs)?;

    let named = match &input.data {
        Data::Struct(s) => match &s.fields {
            Fields::Named(n) => &n.named,
            _ => {
                return Err(syn::Error::new_spanned(
                    &ident,
                    "architect::Entity requires named fields",
                ));
            }
        },
        _ => {
            return Err(syn::Error::new_spanned(
                &ident,
                "architect::Entity requires a struct",
            ));
        }
    };

    let mut parsed: Vec<ParsedField> = Vec::with_capacity(named.len());
    for f in named.iter() {
        let attrs = parse_field_attrs(f)?;
        // Pass-through attrs end up on the emitted `Create`
        // struct (whose only derive is `Facet` + optional
        // `Dummy`). Strip helper attributes whose owning
        // derive ISN'T applied to `Create` — otherwise rustc
        // errors "cannot find attribute `serde` in this scope"
        // because the attribute has no claimant on the Create
        // struct. `#[serde(...)]` is the common case (entities
        // that double as a markdown/JSON wire format on the
        // ORIGINAL struct don't want their field-level serde
        // attrs leaked onto the synthetic Create).
        let forward_attrs: Vec<syn::Attribute> = f
            .attrs
            .iter()
            .filter(|a| {
                !a.path().is_ident("architect")
                    && !a.path().is_ident("serde")
                    && !a.path().is_ident("schemars")
            })
            .cloned()
            .collect();
        parsed.push(ParsedField {
            ident: f.ident.as_ref().unwrap(),
            ty: &f.ty,
            attrs,
            forward_attrs,
        });
    }

    let pk = parsed.iter().find(|f| f.attrs.primary_key).ok_or_else(|| {
        syn::Error::new_spanned(
            &ident,
            "architect::Entity needs a #[architect(primary_key)] field",
        )
    })?;
    let pk_ident = pk.ident;
    let pk_ty = pk.ty;

    if container.emit_store && !container.emit_repo {
        return Err(syn::Error::new_spanned(
            &ident,
            "#[architect(store)] requires #[architect(repo)] — the generated \
             data hooks call the <Entity>Repo vox client",
        ));
    }
    if container.emit_events && !container.emit_repo {
        return Err(syn::Error::new_spanned(
            &ident,
            "#[architect(events)] requires #[architect(repo)] — the \
             <Entity>Evented wrapper publishes through the repo trait",
        ));
    }
    if container.emit_crdt && !container.emit_repo {
        return Err(syn::Error::new_spanned(
            &ident,
            "#[architect(crdt)] requires #[architect(repo)] — the \
             <Entity>RepoLoro newtype implements the <Entity>Repo trait",
        ));
    }
    if container.emit_crdt && type_last_ident(pk_ty).as_deref() != Some("Uuid") {
        return Err(syn::Error::new_spanned(
            pk_ident,
            "#[architect(crdt)] requires a `Uuid` primary key (the \
             EntityCrdt trait keys rows by Uuid)",
        ));
    }

    // ── Field-level misuse caught at derive time ──
    //
    // Both of these used to surface late (a raw SeaORM rustc error the
    // first time the consumer's `server` feature compiled) or not at all
    // (form attributes silently doing nothing). Fail at the declaration
    // instead, with the fix in the message.
    for f in &parsed {
        // Container-shaped field types (`Vec<T>`, maps, sets — and
        // `Option` of each) have no `sea_orm::Value` impl, so the
        // generated `DeriveEntityModel` only compiles when the column is
        // stored as JSON. `Vec<u8>` is exempt: SeaORM handles it
        // natively as a bytes column.
        if !f.attrs.json
            && let Some(container_name) = json_requiring_container(f.ty)
        {
            return Err(syn::Error::new_spanned(
                f.ty,
                format!(
                    "`{container_name}<…>` fields need `#[architect(json)]` — \
                     sea_orm::Value has no impl for plain container types, so \
                     the generated SeaORM model only compiles when the column \
                     is stored as JSON (the field type must be \
                     serde::Serialize + DeserializeOwned)"
                ),
            ));
        }

        let has_form_attrs = f.attrs.form_optional || f.attrs.form_label.is_some();
        if has_form_attrs {
            // `form(…)` attributes only affect the emitted form bindings;
            // without the container flag they'd be silently ignored.
            if !container.emit_form {
                return Err(syn::Error::new_spanned(
                    f.ident,
                    "#[architect(form(…))] has no effect without the \
                     container-level `form` flag — add `form` to \
                     `#[architect(…)]` on the struct, or remove the field \
                     attribute",
                ));
            }
            // A field excluded from both payloads never appears in either
            // generated Fields struct.
            let in_create_form = !f.attrs.exclude_create && f.attrs.on_create.is_none();
            let in_update_form =
                !f.attrs.exclude_update && !f.attrs.primary_key && f.attrs.on_update.is_none();
            if !in_create_form && !in_update_form {
                return Err(syn::Error::new_spanned(
                    f.ident,
                    "#[architect(form(…))] has no effect here — this field \
                     appears in neither the Create nor the Update payload \
                     (excluded / populated by on_create/on_update), so no \
                     form field is generated for it",
                ));
            }
        }
        // `form(optional)` is implemented for String fields only: other
        // types validate through `validate::parse::<T>` (FromStr), where
        // an empty input is already a parse failure.
        if f.attrs.form_optional && !is_string_type(f.ty) {
            return Err(syn::Error::new_spanned(
                f.ident,
                "#[architect(form(optional))] is only supported on `String` \
                 fields — other types validate via `FromStr`, so an empty \
                 input can't succeed; remove `form(optional)` or change the \
                 field type to `String`",
            ));
        }
    }

    let create_ident = format_ident!("{}Create", ident);
    let update_ident = format_ident!("{}Update", ident);
    let list_ident = format_ident!("{}List", ident);
    let repo_ident = format_ident!("{}Repo", ident);
    let storage_ident = format_ident!("{}RepoStorage", ident);

    // Wire struct stays in user code (the derive can't modify it). User
    // writes `#[derive(architect::Entity, facet::Facet, Clone, Debug, PartialEq)]`.
    let wire_struct = quote! {};

    // ── Create payload ──
    let create_fields: Vec<_> = parsed
        .iter()
        .filter(|f| !f.attrs.exclude_create && f.attrs.on_create.is_none())
        .collect();
    let create_field_defs = create_fields.iter().map(|f| {
        let id = f.ident;
        let ty = f.ty;
        let fwd = &f.forward_attrs;
        quote! { #(#fwd)* pub #id: #ty }
    });
    let create_struct = quote! {
        #[cfg_attr(feature = "fake", derive(::architect::fake::Dummy))]
        #[derive(Clone, Debug, PartialEq, ::architect::facet::Facet)]
        #vis struct #create_ident {
            #(#create_field_defs,)*
        }
    };

    // ── Update payload (all Option<T>) ──
    let update_fields: Vec<_> = parsed
        .iter()
        .filter(|f| !f.attrs.exclude_update && !f.attrs.primary_key && f.attrs.on_update.is_none())
        .collect();
    let update_field_defs = update_fields.iter().map(|f| {
        let id = f.ident;
        let ty = f.ty;
        quote! { pub #id: ::core::option::Option<#ty> }
    });
    let update_struct = quote! {
        #[cfg_attr(feature = "fake", derive(::architect::fake::Dummy))]
        #[derive(Clone, Debug, PartialEq, ::architect::facet::Facet, Default)]
        #vis struct #update_ident {
            #(#update_field_defs,)*
        }
    };

    // ── List payload ──
    let list_struct = quote! {
        #[cfg_attr(feature = "fake", derive(::architect::fake::Dummy))]
        #[derive(Clone, Debug, PartialEq, ::architect::facet::Facet)]
        #vis struct #list_ident {
            pub items: ::std::vec::Vec<#ident>,
            pub total: u32,
            pub page: ::architect::Page,
        }
    };

    // ── Repo trait ──
    //
    // `cfg_attr` gates the `#[vox::service]` decoration on the user
    // crate's `vox` feature. With the feature on, vox generates the
    // dispatcher + client types as usual. With it off, the trait is
    // just a plain Rust 2024 async trait and no RPC machinery
    // compiles. This is the seam that lets a consumer use the trait
    // in-process without pulling vox.
    let repo_trait = if container.emit_repo {
        quote! {
            #[cfg_attr(feature = "vox", ::vox::service)]
            #vis trait #repo_ident {
                async fn get(&self, id: #pk_ty)
                    -> ::core::result::Result<#ident, ::architect::RepoError>;
                async fn list(
                    &self,
                    page: ::architect::Page,
                    sort: ::core::option::Option<::architect::Sort>,
                    filter: ::core::option::Option<::architect::Filter>,
                ) -> ::core::result::Result<#list_ident, ::architect::RepoError>;
                async fn create(&self, input: #create_ident)
                    -> ::core::result::Result<#ident, ::architect::RepoError>;
                async fn update(&self, id: #pk_ty, input: #update_ident)
                    -> ::core::result::Result<#ident, ::architect::RepoError>;
                async fn delete(&self, id: #pk_ty)
                    -> ::core::result::Result<(), ::architect::RepoError>;
            }
        }
    } else {
        quote! {}
    };

    // ── Repo as a Layer service ──
    //
    // The `<Entity>Repo` trait is an all-async `#[vox::service]`, exactly
    // the shape `#[architect::rpc]` turns into a composable `Service`
    // token. Emit the same surface for repos so they drop into a
    // `layers![…]` bundle and `.provide(backend)` / `.into_router()` like
    // any other service. Because several entities share one module, the
    // token is uniquely named `<Entity>RepoLayer` (not the bare `Service`
    // that rpc-derive — one trait per module — can get away with).
    //
    // We reference the identifiers `#[vox::service]` emits from the repo
    // trait directly: `<Entity>RepoDispatcher::new(backend)` (the vox
    // Handler; architect's blanket `DynHandler` covers it) and the
    // `<snake>_service_descriptor()` free fn.
    let repo_layer = if container.emit_repo {
        let layer_token = format_ident!("{}Layer", repo_ident);
        let repo_snake = repo_ident.to_string().to_snake_case();
        let descriptor_fn = format_ident!("{}_service_descriptor", repo_snake);
        let dispatcher_ident = format_ident!("{}Dispatcher", repo_ident);
        let layer_fn = format_ident!("{}_layer", repo_snake);
        quote! {
            /// Deferred-bind Layer token for the repository service. Acts
            /// as a one-element [`architect::Layer`]; compose it with
            /// [`architect::Layer::merge`] and bind a backend at
            /// [`architect::Layer::provide`] time. Any backend that
            /// implements `#repo_ident` (the SeaORM storage, an in-memory
            /// repo, a third-party impl) is interchangeable here.
            #[cfg(feature = "vox")]
            #[derive(Debug, Default, Clone, Copy)]
            #vis struct #layer_token;

            #[cfg(feature = "vox")]
            impl ::architect::BindAny for #layer_token {
                fn descriptor(&self) -> &'static ::architect::vox::ServiceDescriptor {
                    #descriptor_fn()
                }
            }

            #[cfg(feature = "vox")]
            impl<B> ::architect::Bind<B> for #layer_token
            where
                B: #repo_ident
                    + ::core::clone::Clone
                    + ::core::marker::Send
                    + ::core::marker::Sync
                    + 'static,
            {
                fn bind_into(self, backend: &B, router: &mut ::architect::LayerRouter) {
                    use ::architect::LayerSink as _;
                    router.add_mounted(::architect::Mounted::new(
                        #descriptor_fn(),
                        #dispatcher_ident::new(backend.clone()),
                    ));
                }
            }

            #[cfg(feature = "vox")]
            impl<R> ::architect::Append<R> for #layer_token {
                type Output = ::architect::Cons<#layer_token, R>;
                fn append(self, rhs: R) -> Self::Output {
                    ::architect::Cons::new(self, rhs)
                }
            }

            #[cfg(feature = "vox")]
            impl ::architect::Descriptors for #layer_token {
                fn collect(
                    &self,
                    out: &mut ::std::vec::Vec<&'static ::architect::vox::ServiceDescriptor>,
                ) {
                    out.push(::architect::BindAny::descriptor(self));
                }
            }

            /// Immediate-bind shortcut: wrap a backend in the repo's vox
            /// dispatcher and return a `Mounted`, ready to drop into
            /// someone else's bundle via [`architect::Layer::merge`]
            /// (e.g. to override the repo impl for one service).
            #[cfg(feature = "vox")]
            #vis fn #layer_fn<B>(backend: B) -> ::architect::Mounted
            where
                B: #repo_ident
                    + ::core::clone::Clone
                    + ::core::marker::Send
                    + ::core::marker::Sync
                    + 'static,
            {
                ::architect::Mounted::new(#descriptor_fn(), #dispatcher_ident::new(backend))
            }
        }
    } else {
        quote! {}
    };

    // ── Fake-data seeding helper ──
    //
    // Emit a free `seed_fake_<entity>` function that loops, generates
    // a `<Entity>Create` via fake-rs, and feeds each one into the
    // repo. Gated on the user crate's `fake` feature, plus only
    // emitted when a repo trait exists to call into.
    let seed_fn_ident = format_ident!("seed_fake_{}", ident.to_string().to_snake_case());
    let seed_fn = if container.emit_repo {
        quote! {
            /// Generate `count` faked `#ident` instances and persist
            /// them via `repo.create`. `count` accepts a single number
            /// (`500`) or a range (`10..30` / `10..=30`) for a random
            /// quantity per call. See `architect::seed::SeedCount`.
            #[cfg(feature = "fake")]
            #vis async fn #seed_fn_ident<R>(
                repo: &R,
                count: impl ::architect::seed::SeedCount,
            ) -> ::core::result::Result<
                ::std::vec::Vec<#ident>,
                ::architect::RepoError,
            >
            where
                R: #repo_ident + ?::core::marker::Sized,
            {
                use ::fake::Fake as _;
                let n = ::architect::seed::SeedCount::pick(&count);
                let mut out = ::std::vec::Vec::with_capacity(n);
                for _ in 0..n {
                    let payload: #create_ident = ::fake::Faker.fake();
                    out.push(repo.create(payload).await?);
                }
                ::core::result::Result::Ok(out)
            }
        }
    } else {
        quote! {}
    };

    // ── Server-only emission ──
    let server_block = build_server_block(
        &ident,
        &vis,
        &container,
        &parsed,
        pk_ident,
        pk_ty,
        &create_ident,
        &update_ident,
        &list_ident,
        &repo_ident,
        &storage_ident,
        &create_fields,
        &update_fields,
    );

    // ── Client-state emission (`store`) ──
    let store_block = build_store_block(
        &ident,
        &vis,
        &container,
        &parsed,
        pk_ident,
        pk_ty,
        &create_ident,
        &update_ident,
        &repo_ident,
        &update_fields,
    );

    // ── Typed form emission (`form`) ──
    let form_block = build_form_block(
        &ident,
        &vis,
        &container,
        &create_ident,
        &update_ident,
        &create_fields,
        &update_fields,
    );

    // ── Live-event emission (`events`) ──
    let events_block = build_events_block(
        &ident,
        &vis,
        &container,
        pk_ty,
        &create_ident,
        &update_ident,
        &list_ident,
        &repo_ident,
    );

    // ── CRDT emission (`crdt`) ──
    let crdt_block = build_crdt_block(
        &ident,
        &vis,
        &container,
        &parsed,
        pk_ident,
        &create_ident,
        &update_ident,
        &list_ident,
        &repo_ident,
        &update_fields,
    )?;

    Ok(quote! {
        #wire_struct
        #create_struct
        #update_struct
        #list_struct
        #repo_trait
        #repo_layer
        #seed_fn
        #server_block
        #store_block
        #form_block
        #events_block
        #crdt_block
    })
}

/// Emit the entity's live-event story:
///
/// ```ignore
/// enum ExampleEvent { Upserted(Example), Deleted(Uuid) }      // wire type
/// #[vox::service] trait ExampleEvents { async fn subscribe(&self, sink: Tx<ExampleEvent>) … }
/// struct ExampleEvented<R> { … }   // publish-through repo wrapper + subscribe host
/// struct ExampleEventsLayer;       // Layer token for the subscribe service
/// impl Services for ExampleEvented<R>   // bundles repo + events: into_router() mounts both
/// fn use_example_events()          // (with `store`) client hook: events → the optimistic store
/// ```
///
/// The server wraps its backend **once** (`ExampleEvented::new(repo)`) and
/// every successful `create`/`update`/`delete` broadcasts to every
/// subscriber; `hub()` exposes the `architect::PubSub` so hosts can
/// publish from outside the repo too (pollers, CRDT change hooks). The
/// client hook folds events into the derived store, making every
/// store-rendered page live.
#[allow(clippy::too_many_arguments)]
fn build_events_block(
    ident: &Ident,
    vis: &syn::Visibility,
    container: &ContainerAttrs,
    pk_ty: &Type,
    create_ident: &Ident,
    update_ident: &Ident,
    list_ident: &Ident,
    repo_ident: &Ident,
) -> TokenStream2 {
    if !container.emit_events {
        return quote! {};
    }

    let snake = ident.to_string().to_snake_case();
    let event_ident = format_ident!("{}Event", ident);
    let events_trait = format_ident!("{}Events", ident);
    let evented_ident = format_ident!("{}Evented", ident);
    let events_layer_token = format_ident!("{}Layer", events_trait);
    let events_client = format_ident!("{}Client", events_trait);
    let events_dispatcher = format_ident!("{}Dispatcher", events_trait);
    let events_descriptor_fn = format_ident!(
        "{}_service_descriptor",
        events_trait.to_string().to_snake_case()
    );
    let repo_layer_token = format_ident!("{}Layer", repo_ident);
    let use_events_fn = format_ident!("use_{}_events", snake);
    let apply_fn = format_ident!("__apply_{}_event", snake);
    let use_store_fn = format_ident!("use_{}_store", snake);
    let store_alias = format_ident!("{}Store", ident);

    let doc_event = format!(
        "One change to the [`{ident}`] set. `Upserted` covers create *and* \
         update — the client folds it with a single authoritative \
         `Store::put`."
    );
    let doc_evented = format!(
        "Publish-through wrapper over any [`{repo_ident}`] backend: every \
         successful write broadcasts an [`{event_ident}`] to every \
         [`{events_trait}::subscribe`] sink. Its `Services` bundle mounts \
         the repo **and** the subscribe service, so \
         `{evented_ident}::new(backend).into_router()` serves both. Wrap \
         **once** per process — the hub lives inside, and per-connection \
         routers must share it."
    );

    // The client hook only exists when the store layer is also emitted.
    let client_hook = if container.emit_store {
        quote! {
            /// Live entity events → the derived store. Mount once (at the
            /// app root): subscribes for the app's lifetime, folds every
            /// server-pushed change into the optimistic store — making
            /// every store-rendered page **live** — and re-subscribes
            /// when the connection reconnects.
            #[cfg(all(feature = "atom", feature = "vox"))]
            #vis fn #use_events_fn() {
                let conn = ::architect::use_connection::<::architect::vox::Caller>();
                let store = #use_store_fn();
                ::architect::use_store_stream(
                    store,
                    move |sink| async move {
                        match conn.state() {
                            ::architect::ConnectionState::Ready(caller) => {
                                #events_client::new(caller).subscribe(sink).await.is_ok()
                            }
                            _ => false,
                        }
                    },
                    #apply_fn,
                );
            }

            #[cfg(all(feature = "atom", feature = "vox"))]
            #[doc(hidden)]
            fn #apply_fn(store: &#store_alias, event: #event_ident) {
                match event {
                    #event_ident::Snapshot(rows) => store.hydrate(rows),
                    #event_ident::Upserted(row) => store.put(row),
                    #event_ident::Deleted(id) => store.remove_real(&id),
                }
            }
        }
    } else {
        quote! {}
    };

    quote! {
        #[doc = #doc_event]
        #[cfg_attr(feature = "fake", derive(::architect::fake::Dummy))]
        #[derive(Clone, Debug, PartialEq, ::architect::facet::Facet)]
        #[repr(u8)]
        #vis enum #event_ident {
            /// The current full row set — the **first** event every
            /// subscriber receives (effect-`SubscriptionRef` semantics:
            /// current state, then changes), so subscribing alone fully
            /// hydrates a client store.
            Snapshot(::std::vec::Vec<#ident>),
            /// A row was created or updated; here is its authoritative state.
            Upserted(#ident),
            /// A row was deleted.
            Deleted(#pk_ty),
        }

        /// Streaming sibling trait: pass a channel sink in, receive every
        /// subsequent change on it until the connection closes.
        #[cfg(feature = "vox")]
        #[::vox::service]
        #vis trait #events_trait {
            /// Attach `sink` to the event feed. The call stays in flight for
            /// the life of the subscription (vox channels are request-scoped)
            /// — events flow on the channel while it runs; cancel the call to
            /// unsubscribe. Errors resolve the call immediately.
            async fn subscribe(
                &self,
                sink: ::vox::Tx<#event_ident>,
            ) -> ::core::result::Result<(), ::architect::RepoError>;
        }

        #[doc = #doc_evented]
        #[cfg(all(feature = "vox", not(target_arch = "wasm32")))]
        #[derive(Clone)]
        #vis struct #evented_ident<R> {
            inner: R,
            hub: ::architect::PubSub<#event_ident>,
        }

        #[cfg(all(feature = "vox", not(target_arch = "wasm32")))]
        impl<R> #evented_ident<R> {
            #vis fn new(inner: R) -> Self {
                Self {
                    inner,
                    hub: ::architect::PubSub::sliding(256),
                }
            }

            /// The shared fan-out hub — for publishing changes that don't
            /// come through the repo (host pollers, CRDT merge hooks, …).
            #vis fn hub(&self) -> ::architect::PubSub<#event_ident> {
                self.hub.clone()
            }
        }

        #[cfg(all(feature = "vox", not(target_arch = "wasm32")))]
        impl<R> #repo_ident for #evented_ident<R>
        where
            R: #repo_ident + ::core::marker::Send + ::core::marker::Sync + 'static,
        {
            async fn get(
                &self,
                id: #pk_ty,
            ) -> ::core::result::Result<#ident, ::architect::RepoError> {
                self.inner.get(id).await
            }
            async fn list(
                &self,
                page: ::architect::Page,
                sort: ::core::option::Option<::architect::Sort>,
                filter: ::core::option::Option<::architect::Filter>,
            ) -> ::core::result::Result<#list_ident, ::architect::RepoError> {
                self.inner.list(page, sort, filter).await
            }
            async fn create(
                &self,
                input: #create_ident,
            ) -> ::core::result::Result<#ident, ::architect::RepoError> {
                let row = self.inner.create(input).await?;
                self.hub.publish(#event_ident::Upserted(row.clone()));
                ::core::result::Result::Ok(row)
            }
            async fn update(
                &self,
                id: #pk_ty,
                input: #update_ident,
            ) -> ::core::result::Result<#ident, ::architect::RepoError> {
                let row = self.inner.update(id, input).await?;
                self.hub.publish(#event_ident::Upserted(row.clone()));
                ::core::result::Result::Ok(row)
            }
            async fn delete(
                &self,
                id: #pk_ty,
            ) -> ::core::result::Result<(), ::architect::RepoError> {
                // The pk is `Clone`, not necessarily `Copy` (String pks):
                // the inner call consumes one copy, the broadcast the other.
                self.inner.delete(id.clone()).await?;
                self.hub.publish(#event_ident::Deleted(id));
                ::core::result::Result::Ok(())
            }
        }

        #[cfg(all(feature = "vox", not(target_arch = "wasm32")))]
        impl<R> #events_trait for #evented_ident<R>
        where
            R: #repo_ident
                + ::core::clone::Clone
                + ::core::marker::Send
                + ::core::marker::Sync
                + 'static,
        {
            async fn subscribe(
                &self,
                sink: ::vox::Tx<#event_ident>,
            ) -> ::core::result::Result<(), ::architect::RepoError> {
                // effect-SubscriptionRef semantics: park the subscriber
                // (its mailbox collects every change from this instant),
                // read the current row set with no lock held, then deliver
                // the snapshot ahead of the collected changes.
                let pending = self.hub.begin_attach(sink);
                let snapshot = self
                    .inner
                    .list(
                        ::architect::Page { index: 0, size: u32::MAX },
                        ::core::option::Option::None,
                        ::core::option::Option::None,
                    )
                    .await;
                match snapshot {
                    ::core::result::Result::Ok(list) => {
                        self.hub.complete_attach(
                            pending,
                            ::core::option::Option::Some(#event_ident::Snapshot(list.items)),
                        );
                        // vox scopes channels to their request: delivering
                        // the response terminates the sink. Hold the request
                        // open for the life of the subscription — the client
                        // unsubscribes by cancelling the call.
                        ::core::future::pending::<()>().await;
                        ::core::result::Result::Ok(())
                    }
                    ::core::result::Result::Err(e) => {
                        self.hub.abort_attach(pending);
                        ::core::result::Result::Err(e)
                    }
                }
            }
        }

        /// Deferred-bind Layer token for the subscribe service — composes
        /// through `layers![…]` exactly like the repo's token.
        #[cfg(all(feature = "vox", not(target_arch = "wasm32")))]
        #[derive(Debug, Default, Clone, Copy)]
        #vis struct #events_layer_token;

        #[cfg(all(feature = "vox", not(target_arch = "wasm32")))]
        impl ::architect::BindAny for #events_layer_token {
            fn descriptor(&self) -> &'static ::architect::vox::ServiceDescriptor {
                #events_descriptor_fn()
            }
        }

        #[cfg(all(feature = "vox", not(target_arch = "wasm32")))]
        impl<B> ::architect::Bind<B> for #events_layer_token
        where
            B: #events_trait
                + ::core::clone::Clone
                + ::core::marker::Send
                + ::core::marker::Sync
                + 'static,
        {
            fn bind_into(self, backend: &B, router: &mut ::architect::LayerRouter) {
                use ::architect::LayerSink as _;
                router.add_mounted(::architect::Mounted::new(
                    #events_descriptor_fn(),
                    #events_dispatcher::new(backend.clone()),
                ));
            }
        }

        #[cfg(all(feature = "vox", not(target_arch = "wasm32")))]
        impl<R> ::architect::Append<R> for #events_layer_token {
            type Output = ::architect::Cons<#events_layer_token, R>;
            fn append(self, rhs: R) -> Self::Output {
                ::architect::Cons::new(self, rhs)
            }
        }

        #[cfg(all(feature = "vox", not(target_arch = "wasm32")))]
        impl ::architect::Descriptors for #events_layer_token {
            fn collect(
                &self,
                out: &mut ::std::vec::Vec<&'static ::architect::vox::ServiceDescriptor>,
            ) {
                out.push(::architect::BindAny::descriptor(self));
            }
        }

        /// The wrapper provides both services: CRUD + the event feed.
        #[cfg(all(feature = "vox", not(target_arch = "wasm32")))]
        impl<R> ::architect::Services for #evented_ident<R>
        where
            R: #repo_ident
                + ::core::clone::Clone
                + ::core::marker::Send
                + ::core::marker::Sync
                + 'static,
        {
            fn layers() -> impl ::architect::Layer<Self> {
                ::architect::layers![#repo_layer_token, #events_layer_token]
            }
        }

        #client_hook
    }
}

/// True when the field's type is a bare `String` (path's last segment).
fn is_string_type(ty: &Type) -> bool {
    matches!(ty, Type::Path(p) if p.path.segments.last().is_some_and(|s| s.ident == "String"))
}

/// Emit typed form-field bindings over the Create/Update payloads, gated
/// on the user crate's `form` feature (`form = ["architect/form"]`).
///
/// For `Example` this emits, roughly:
///
/// ```ignore
/// struct ExampleCreateFields { name: Field<String>, description: Field<String> }
/// fn use_example_create_fields() -> ExampleCreateFields;     // empty fields
/// impl ExampleCreateFields {
///     fn submit(&self) -> Option<ExampleCreate>;  // validate all → typed payload
///     fn is_dirty(&self) -> bool;  fn reset(&self);
/// }
/// struct ExampleUpdateFields { … }                            // same, seeded
/// fn use_example_update_fields(initial: &Example) -> ExampleUpdateFields;
/// impl ExampleUpdateFields { fn submit(&self) -> Option<ExampleUpdate>; … }
/// ```
///
/// Validation comes from the field's **type** plus the
/// `#[architect(form(…))]` attributes: `String` fields are required
/// (Title-Case label) unless `form(optional)`; any other type goes
/// through `validate::parse::<T>` (so it must be `FromStr + Display`).
/// The struct produced by `submit()` is the same wire payload the derived
/// mutations take — form → payload → optimistic write, typed throughout.
fn build_form_block(
    ident: &Ident,
    vis: &syn::Visibility,
    container: &ContainerAttrs,
    create_ident: &Ident,
    update_ident: &Ident,
    create_fields: &[&ParsedField],
    update_fields: &[&ParsedField],
) -> TokenStream2 {
    if !container.emit_form {
        return quote! {};
    }

    let snake = ident.to_string().to_snake_case();

    // One generated block per payload kind. `seed` is how a field's
    // initial string value is produced: empty for create, from the
    // current entity row for update.
    let build = |fields: &[&ParsedField],
                 payload_ident: &Ident,
                 fields_ident: Ident,
                 hook_ident: Ident,
                 seeded: bool,
                 wrap_some: bool| {
        let field_defs = fields.iter().map(|f| {
            let id = f.ident;
            let ty = f.ty;
            if is_string_type(ty) {
                quote! { pub #id: ::architect::form::Field<::std::string::String> }
            } else {
                quote! { pub #id: ::architect::form::Field<#ty> }
            }
        });

        let field_inits = fields.iter().map(|f| {
            let id = f.ident;
            let ty = f.ty;
            let label = f
                .attrs
                .form_label
                .clone()
                .unwrap_or_else(|| id.to_string().to_title_case());
            let validator = if is_string_type(ty) {
                if f.attrs.form_optional {
                    quote! { ::architect::form::validate::optional() }
                } else {
                    quote! { ::architect::form::validate::required(#label) }
                }
            } else {
                quote! { ::architect::form::validate::parse::<#ty>(#label) }
            };
            let initial = if !seeded {
                quote! { ::std::string::String::new() }
            } else if is_string_type(ty) {
                quote! { initial.#id.clone() }
            } else {
                quote! { ::std::string::ToString::to_string(&initial.#id) }
            };
            quote! { #id: ::architect::form::use_field(#initial, #validator) }
        });

        let validate_binds = fields.iter().map(|f| {
            let id = f.ident;
            quote! { let #id = self.#id.validate().ok(); }
        });
        let payload_assigns = fields.iter().map(|f| {
            let id = f.ident;
            if wrap_some {
                quote! { #id: ::core::option::Option::Some(#id?) }
            } else {
                quote! { #id: #id? }
            }
        });
        let dirty_checks = fields.iter().map(|f| {
            let id = f.ident;
            quote! { self.#id.is_dirty() }
        });
        let resets = fields.iter().map(|f| {
            let id = f.ident;
            quote! { self.#id.reset(); }
        });

        let hook_params = if seeded {
            quote! { initial: &#ident }
        } else {
            quote! {}
        };
        let doc_fields = format!(
            "Typed form fields over [`{payload_ident}`] — one \
             [`Field`](::architect::form::Field) per payload field, \
             validation derived from the entity declaration."
        );
        let doc_submit = format!(
            "Validate every field (revealing errors on the untouched \
             ones), and return the typed [`{payload_ident}`] when the \
             whole form passes — ready to hand to the derived mutations."
        );

        quote! {
            #[doc = #doc_fields]
            #[cfg(feature = "form")]
            #[derive(Clone, Copy, PartialEq)]
            #vis struct #fields_ident {
                #(#field_defs,)*
            }

            #[doc = #doc_fields]
            #[cfg(feature = "form")]
            #vis fn #hook_ident(#hook_params) -> #fields_ident {
                #fields_ident {
                    #(#field_inits,)*
                }
            }

            #[cfg(feature = "form")]
            impl #fields_ident {
                #[doc = #doc_submit]
                #vis fn submit(&self) -> ::core::option::Option<#payload_ident> {
                    #(#validate_binds)*
                    ::core::option::Option::Some(#payload_ident {
                        #(#payload_assigns,)*
                    })
                }

                /// True once any field differs from its initial value.
                #vis fn is_dirty(&self) -> bool {
                    false #(|| #dirty_checks)*
                }

                /// Reset every field to its initial value, clearing
                /// errors and touched state.
                #vis fn reset(&self) {
                    #(#resets)*
                }
            }
        }
    };

    let create_block = build(
        create_fields,
        create_ident,
        format_ident!("{}CreateFields", ident),
        format_ident!("use_{}_create_fields", snake),
        false,
        false,
    );
    let update_block = build(
        update_fields,
        update_ident,
        format_ident!("{}UpdateFields", ident),
        format_ident!("use_{}_update_fields", snake),
        true,
        true,
    );

    quote! {
        #create_block
        #update_block
    }
}

/// Emit the client-side state layer for the entity — the optimistic store
/// binding, typed data hooks, and optimistic mutations — gated on the user
/// crate's `atom` + `vox` features (the same convention as `server` /
/// `fake`: the consumer declares `atom = ["architect/atom"]`).
///
/// For `Example` this emits, roughly:
///
/// ```ignore
/// type ExampleClientError = ClientError<RepoError>;
/// type ExampleStore = Store<Example, ExampleClientError>;
/// impl StoreEntity for Example { type Key = Uuid; … }
/// impl Example { fn draft(input: &ExampleCreate) -> Example { … } }
/// fn provide_example_store() -> ExampleStore;             // app root
/// fn use_example_store() -> ExampleStore;
/// fn use_example(id: String) -> AtomResult<Example, ExampleClientError>;
/// fn use_example_list() -> AtomResult<Vec<(Id<Uuid>, Example)>, ExampleClientError>;
/// struct ExampleMutations { … }                            // create/update/delete
/// fn use_example_mutations() -> ExampleMutations;
/// ```
///
/// Requirements: the primary key must be `Clone + Eq + Hash + FromStr`
/// with a `Display`able parse error (`Uuid`, `String`, integer ids all
/// qualify — `Copy` is *not* required, so non-`Copy` keys like `String`
/// work; the emitted code clones at the few places a key crosses an
/// optimistic-patch closure and the server-call future). The hooks read
/// a `Connection<<Entity>RepoClient>` from context, provided at the app
/// root with `architect::use_connect`.
#[allow(clippy::too_many_arguments)]
fn build_store_block(
    ident: &Ident,
    vis: &syn::Visibility,
    container: &ContainerAttrs,
    parsed: &[ParsedField],
    _pk_ident: &Ident,
    pk_ty: &Type,
    create_ident: &Ident,
    update_ident: &Ident,
    repo_ident: &Ident,
    update_fields: &[&ParsedField],
) -> TokenStream2 {
    if !container.emit_store {
        return quote! {};
    }

    let snake = ident.to_string().to_snake_case();
    let invalidate_key = container
        .table_name
        .clone()
        .unwrap_or_else(|| snake.clone());

    let client_error_alias = format_ident!("{}ClientError", ident);
    let store_alias = format_ident!("{}Store", ident);
    let mutations_ident = format_ident!("{}Mutations", ident);
    let repo_client = format_ident!("{}Client", repo_ident);
    let provide_store_fn = format_ident!("provide_{}_store", snake);
    let use_store_fn = format_ident!("use_{}_store", snake);
    let use_one_fn = format_ident!("use_{}", snake);
    let use_list_fn = format_ident!("use_{}_list", snake);
    let use_muts_fn = format_ident!("use_{}_mutations", snake);
    let parse_fn = format_ident!("__parse_{}_id", snake);
    let provide_fn = format_ident!("provide_{}", snake);
    let key_const = format_ident!("{}_REACTIVITY_KEY", snake.to_uppercase());
    let page_size = container.page_size.unwrap_or(100);
    // The feature's whole client-side provision in one root call: the
    // store, plus (with `events`) the live subscription that keeps it fed.
    let provide_events = if container.emit_events {
        let use_events_fn = format_ident!("use_{}_events", snake);
        quote! { #use_events_fn(); }
    } else {
        quote! {}
    };
    let doc_provide = format!(
        "Provide the whole `{ident}` client layer at the app root: the \
         shared optimistic store{}. Call once, after `architect::use_app`.",
        if container.emit_events {
            " plus the live event subscription that keeps it fed"
        } else {
            ""
        }
    );

    // The primary-key accessor for `StoreEntity::key`.
    let pk_field = _pk_ident;

    // `draft` field assignments: a client-side placeholder row built from
    // the Create payload. `on_create` expressions are evaluated locally as
    // previews (the reconcile replaces the row with the server's truth);
    // excluded fields without an expression fall back to `Default`.
    let draft_assigns = parsed.iter().map(|f| {
        let id = f.ident;
        if let Some(e) = &f.attrs.on_create {
            quote! { #id: #e }
        } else if f.attrs.exclude_create {
            quote! { #id: ::core::default::Default::default() }
        } else {
            quote! { #id: input.#id.clone() }
        }
    });

    // Optimistic update patch: apply each `Some` field of the Update
    // payload, then preview the `on_update` expressions (e.g. a touched
    // `updated_at`) the server will set authoritatively.
    let patch_assigns = update_fields.iter().map(|f| {
        let id = f.ident;
        quote! {
            if let ::core::option::Option::Some(v) = patch_input.#id {
                row.#id = v;
            }
        }
    });
    let patch_touches = parsed
        .iter()
        .filter(|f| f.attrs.on_update.is_some())
        .map(|f| {
            let id = f.ident;
            let e = f.attrs.on_update.as_ref().unwrap();
            quote! { row.#id = #e; }
        });

    let doc_store = format!(
        "The shared optimistic cache of [`{ident}`] rows \
         (stale-while-revalidate reads, optimistic writes)."
    );
    let doc_use_one = format!(
        "One [`{ident}`] by id — cache-first: `Success` straight from the \
         store when the row is cached (instant after a list visit, and it \
         reflects optimistic edits), else the fallback fetch's phase. \
         `match` the returned phase in the page."
    );
    let doc_use_list = format!(
        "The [`{ident}`] list as one `AtomResult` (rows are the value, the \
         backing fetch the phase). Tracks the `\"{invalidate_key}\"` \
         reactivity key when a `Reactivity` registry is provided, so \
         settled mutations re-fetch it."
    );
    let doc_muts = format!(
        "Optimistic write actions for [`{ident}`]: each patches the store \
         instantly, then reconciles against the server — rolling back (and \
         reporting to the app's `Notifications`, when provided) on failure."
    );

    quote! {
        /// What this entity's client hooks fail with: the typed repo error
        /// (`App(RepoError::NotFound)`, …) or an infrastructure failure.
        #[cfg(all(feature = "atom", feature = "vox"))]
        #vis type #client_error_alias = ::architect::ClientError<::architect::RepoError>;

        #[doc = #doc_store]
        #[cfg(all(feature = "atom", feature = "vox"))]
        #vis type #store_alias = ::architect::Store<#ident, #client_error_alias>;

        #[cfg(all(feature = "atom", feature = "vox"))]
        impl ::architect::StoreEntity for #ident {
            type Key = #pk_ty;
            fn key(&self) -> #pk_ty {
                self.#pk_field.clone()
            }
        }

        #[cfg(all(feature = "atom", feature = "vox"))]
        impl #ident {
            /// Build an unsaved placeholder row from a Create payload, for
            /// an optimistic insert. `on_create` expressions are evaluated
            /// client-side as previews; the store keys the row by a temp
            /// id and swaps in the server's authoritative row on
            /// reconcile, so none of this is persisted.
            #vis fn draft(input: &#create_ident) -> Self {
                Self {
                    #(#draft_assigns,)*
                }
            }
        }

        /// Create the entity's store once at the app root and provide it
        /// as context (alongside `architect::use_connect` for the repo
        /// client).
        #[cfg(all(feature = "atom", feature = "vox"))]
        #vis fn #provide_store_fn() -> #store_alias {
            let store = ::architect::use_store::<#ident, #client_error_alias>();
            ::architect::dioxus::prelude::use_context_provider(move || store)
        }

        #[doc = #doc_store]
        #[cfg(all(feature = "atom", feature = "vox"))]
        #vis fn #use_store_fn() -> #store_alias {
            ::architect::dioxus::prelude::use_context()
        }

        #[doc = #doc_provide]
        #[cfg(all(feature = "atom", feature = "vox"))]
        #vis fn #provide_fn() -> #store_alias {
            let store = #provide_store_fn();
            #provide_events
            store
        }

        /// The entity's reactivity key — track it in custom list-shaped
        /// hooks; the derived mutations invalidate it after settling.
        #vis const #key_const: &'static str = #invalidate_key;

        #[cfg(all(feature = "atom", feature = "vox"))]
        #[doc(hidden)]
        fn #parse_fn(raw: &str) -> ::core::result::Result<#pk_ty, #client_error_alias> {
            raw.parse::<#pk_ty>().map_err(|e| {
                ::architect::ClientError::App(::architect::RepoError::InvalidInput(
                    ::std::format!("bad id `{raw}`: {e}"),
                ))
            })
        }

        #[doc = #doc_use_one]
        #[cfg(all(feature = "atom", feature = "vox"))]
        #vis fn #use_one_fn(
            id: ::std::string::String,
        ) -> ::architect::AtomResult<#ident, #client_error_alias> {
            let conn = ::architect::use_connection::<::architect::vox::Caller>();
            let store = #use_store_fn();
            ::architect::use_store_entry(store, id, #parse_fn, move |key| async move {
                let caller = ::architect::ready_or_pending!(conn);
                let client = #repo_client::new(caller);
                ::core::option::Option::Some(
                    client.get(key).await.map_err(::architect::ClientError::from),
                )
            })
        }

        #[doc = #doc_use_list]
        #[cfg(all(feature = "atom", feature = "vox"))]
        #[allow(clippy::type_complexity)]
        #vis fn #use_list_fn() -> ::architect::AtomResult<
            ::std::vec::Vec<(::architect::Id<#pk_ty>, #ident)>,
            #client_error_alias,
        > {
            let conn = ::architect::use_connection::<::architect::vox::Caller>();
            let store = #use_store_fn();
            let reactivity = ::architect::try_use_reactivity();
            ::architect::use_store_list(store, move || async move {
                if let ::core::option::Option::Some(r) = reactivity {
                    r.track(#invalidate_key);
                }
                let caller = ::architect::ready_or_pending!(conn);
                let client = #repo_client::new(caller);
                ::core::option::Option::Some(
                    client
                        .list(
                            ::architect::Page { index: 0, size: #page_size },
                            ::core::option::Option::None,
                            ::core::option::Option::None,
                        )
                        .await
                        .map(|l| l.items)
                        .map_err(::architect::ClientError::from),
                )
            })
        }

        #[doc = #doc_muts]
        #[cfg(all(feature = "atom", feature = "vox"))]
        #[derive(Clone, Copy)]
        #vis struct #mutations_ident {
            conn: ::architect::Connection<::architect::vox::Caller>,
            store: #store_alias,
            create_m: ::architect::Mutation<#client_error_alias>,
            write_m: ::architect::Mutation<#client_error_alias>,
        }

        #[doc = #doc_muts]
        #[cfg(all(feature = "atom", feature = "vox"))]
        #vis fn #use_muts_fn() -> #mutations_ident {
            #mutations_ident {
                conn: ::architect::use_connection::<::architect::vox::Caller>(),
                store: #use_store_fn(),
                create_m: ::architect::use_mutation().invalidating(&[#invalidate_key]),
                write_m: ::architect::use_mutation().invalidating(&[#invalidate_key]),
            }
        }

        #[cfg(all(feature = "atom", feature = "vox"))]
        impl #mutations_ident {
            /// Optimistically add a row: a draft appears instantly, then
            /// reconciles to the server's row (or rolls back on failure).
            /// No-op while the connection isn't up.
            #vis fn create(&self, input: #create_ident) {
                let ::core::option::Option::Some(caller) = self.conn.ready() else {
                    return;
                };
                let client = #repo_client::new(caller);
                let draft = #ident::draft(&input);
                self.create_m.run(
                    self.store,
                    move |s| s.insert_optimistic(draft).0,
                    move || async move {
                        client
                            .create(input)
                            .await
                            .map(::core::option::Option::Some)
                            .map_err(::architect::ClientError::from)
                    },
                );
            }

            /// Optimistically patch a row in place (each `Some` field of
            /// the Update payload, plus `on_update` previews), then
            /// reconcile against the server's row.
            #vis fn update(&self, id: #pk_ty, input: #update_ident) {
                let ::core::option::Option::Some(caller) = self.conn.ready() else {
                    return;
                };
                let client = #repo_client::new(caller);
                let patch_input = input.clone();
                // The key is `Clone`, not necessarily `Copy` (String pks):
                // the optimistic patch and the server call each get their
                // own copy.
                let patch_id = id.clone();
                self.write_m.run(
                    self.store,
                    move |s| {
                        s.update_optimistic(::architect::Id::Real(patch_id), move |row| {
                            #(#patch_assigns)*
                            #(#patch_touches)*
                        })
                    },
                    move || async move {
                        client
                            .update(id, input)
                            .await
                            .map(::core::option::Option::Some)
                            .map_err(::architect::ClientError::from)
                    },
                );
            }

            /// Optimistically remove a row (it vanishes now; restored —
            /// and reported — if the server rejects it).
            #vis fn delete(&self, id: #pk_ty) {
                let ::core::option::Option::Some(caller) = self.conn.ready() else {
                    return;
                };
                let client = #repo_client::new(caller);
                let patch_id = id.clone();
                self.write_m.run(
                    self.store,
                    move |s| s.remove_optimistic(::architect::Id::Real(patch_id)),
                    move || async move {
                        client
                            .delete(id)
                            .await
                            .map(|()| ::core::option::Option::None)
                            .map_err(::architect::ClientError::from)
                    },
                );
            }

            /// The last create failure (a create form usually stays
            /// on-screen, so this is worth rendering inline).
            #vis fn create_error(&self) -> ::core::option::Option<#client_error_alias> {
                self.create_m.error()
            }

            /// The last update/delete failure.
            #vis fn write_error(&self) -> ::core::option::Option<#client_error_alias> {
                self.write_m.error()
            }

            /// True while any of this handle's calls is in flight.
            #vis fn is_pending(&self) -> bool {
                self.create_m.is_pending() || self.write_m.is_pending()
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn build_server_block(
    ident: &Ident,
    vis: &syn::Visibility,
    container: &ContainerAttrs,
    parsed: &[ParsedField],
    _pk_ident: &Ident,
    pk_ty: &Type,
    create_ident: &Ident,
    update_ident: &Ident,
    list_ident: &Ident,
    repo_ident: &Ident,
    storage_ident: &Ident,
    create_fields: &[&ParsedField],
    update_fields: &[&ParsedField],
) -> TokenStream2 {
    let table_name = container
        .table_name
        .clone()
        .unwrap_or_else(|| ident.to_string().to_snake_case());
    let storage_mod = format_ident!("__{}_storage", ident.to_string().to_snake_case());
    // The repo's Layer token, emitted at parent scope in `expand`.
    let layer_token = format_ident!("{}Layer", repo_ident);

    // Model fields with sea_orm attrs.
    let model_fields = parsed.iter().map(|f| {
        let id = f.ident;
        let ty = f.ty;
        let mut sea_attrs: Vec<TokenStream2> = Vec::new();
        if f.attrs.primary_key {
            let auto = f.attrs.auto_increment.unwrap_or(false);
            if auto {
                sea_attrs.push(quote! { primary_key });
            } else {
                sea_attrs.push(quote! { primary_key, auto_increment = false });
            }
        }
        if f.attrs.json {
            sea_attrs.push(quote! { column_type = "Json" });
        }
        let sea = if sea_attrs.is_empty() {
            quote! {}
        } else {
            quote! { #[sea_orm(#(#sea_attrs),*)] }
        };
        quote! { #sea pub #id: #ty }
    });

    // From<Model> for wire.
    let from_field_assigns = parsed.iter().map(|f| {
        let id = f.ident;
        quote! { #id: m.#id }
    });

    // ActiveModel field assignments for create.
    let create_active_assigns = parsed.iter().map(|f| {
        let id = f.ident;
        if let Some(e) = &f.attrs.on_create {
            quote! { #id: ::sea_orm::Set(#e) }
        } else if f.attrs.exclude_create {
            quote! { #id: ::sea_orm::NotSet }
        } else {
            quote! { #id: ::sea_orm::Set(input.#id) }
        }
    });

    // ActiveModel field assignments for update.
    let update_active_assigns = update_fields.iter().map(|f| {
        let id = f.ident;
        quote! {
            if let ::core::option::Option::Some(v) = input.#id {
                am.#id = ::sea_orm::Set(v);
            }
        }
    });

    let touch_updated = parsed
        .iter()
        .filter(|f| f.attrs.on_update.is_some())
        .map(|f| {
            let id = f.ident;
            let e = f.attrs.on_update.as_ref().unwrap();
            quote! { am.#id = ::sea_orm::Set(#e); }
        });

    let _ = (create_fields, list_ident, repo_ident);

    // Sort match arms — emit one arm per `sortable` field, mapping the
    // wire-facing snake_case field name to its SeaORM Column variant.
    let sort_arms = parsed.iter().filter(|f| f.attrs.sortable).map(|f| {
        let id = f.ident;
        let field_name = id.to_string();
        let col_variant = format_ident!("{}", id.to_string().to_pascal_case());
        quote! {
            #field_name => {
                query = match order {
                    ::architect::SortOrder::Asc => query.order_by(Column::#col_variant, Order::Asc),
                    ::architect::SortOrder::Desc => query.order_by(Column::#col_variant, Order::Desc),
                };
            }
        }
    });

    quote! {
        #[cfg(feature = "server")]
        #[doc(hidden)]
        pub mod #storage_mod {
            // User's types (Uuid, DateTime<Utc>, etc.) come via super::*.
            // We deliberately avoid `use ::sea_orm::entity::prelude::*`
            // because it re-exports DateTime/etc and collides with the
            // user's chrono import.
            use super::*;
            use ::sea_orm::{
                ActiveModelBehavior, ActiveModelTrait, DbErr,
                DeriveEntityModel, DerivePrimaryKey, DeriveRelation,
                EntityTrait, EnumIter, IntoActiveModel, Order,
                PaginatorTrait, PrimaryKeyTrait, PrimaryKeyToColumn,
                QueryOrder, Set,
            };

            #[cfg_attr(feature = "fake", derive(::architect::fake::Dummy))]
            #[derive(Clone, Debug, PartialEq, ::sea_orm::DeriveEntityModel)]
            #[sea_orm(table_name = #table_name)]
            pub struct Model {
                #(#model_fields,)*
            }

            #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
            pub enum Relation {}

            impl ActiveModelBehavior for ActiveModel {}

            impl ::core::convert::From<Model> for #ident {
                fn from(m: Model) -> Self {
                    Self {
                        #(#from_field_assigns,)*
                    }
                }
            }

            #[derive(Clone)]
            pub struct #storage_ident<C: ::architect::storage::DbConn> {
                db: C,
            }

            impl<C: ::architect::storage::DbConn> #storage_ident<C> {
                pub fn new(db: C) -> Self { Self { db } }
            }

            fn map_err(e: ::sea_orm::DbErr) -> ::architect::RepoError {
                ::architect::RepoError::Internal(e.to_string())
            }

            impl<C: ::architect::storage::DbConn> super::#repo_ident for #storage_ident<C> {
                async fn get(&self, id: #pk_ty)
                    -> ::core::result::Result<#ident, ::architect::RepoError>
                {
                    Entity::find_by_id(id).one(&self.db).await
                        .map_err(map_err)?
                        .map(#ident::from)
                        .ok_or(::architect::RepoError::NotFound)
                }

                async fn list(
                    &self,
                    page: ::architect::Page,
                    sort: ::core::option::Option<::architect::Sort>,
                    _filter: ::core::option::Option<::architect::Filter>,
                ) -> ::core::result::Result<#list_ident, ::architect::RepoError> {
                    // Filter is intentionally ignored for now — the
                    // wire-side `Filter { raw: String }` is a placeholder
                    // for the structured predicate AST that lands when
                    // the macro grows column-typed filtering.
                    let mut query = Entity::find();
                    if let ::core::option::Option::Some(s) = sort.as_ref() {
                        let order = s.order;
                        match s.field.as_str() {
                            #(#sort_arms,)*
                            other => {
                                return ::core::result::Result::Err(
                                    ::architect::RepoError::InvalidInput(
                                        ::std::format!("unsortable field: {}", other)
                                    )
                                );
                            }
                        }
                    }
                    let size = page.size.max(1) as u64;
                    let p = query.paginate(&self.db, size);
                    let total = p.num_items().await.map_err(map_err)? as u32;
                    let items = p.fetch_page(page.index as u64).await
                        .map_err(map_err)?
                        .into_iter()
                        .map(#ident::from)
                        .collect();
                    Ok(#list_ident { items, total, page })
                }

                async fn create(&self, input: #create_ident)
                    -> ::core::result::Result<#ident, ::architect::RepoError>
                {
                    let am = ActiveModel {
                        #(#create_active_assigns,)*
                    };
                    let m = am.insert(&self.db).await.map_err(map_err)?;
                    Ok(#ident::from(m))
                }

                async fn update(&self, id: #pk_ty, input: #update_ident)
                    -> ::core::result::Result<#ident, ::architect::RepoError>
                {
                    let existing = Entity::find_by_id(id).one(&self.db).await
                        .map_err(map_err)?
                        .ok_or(::architect::RepoError::NotFound)?;
                    let mut am: ActiveModel = existing.into();
                    #(#update_active_assigns)*
                    #(#touch_updated)*
                    let m = am.update(&self.db).await.map_err(map_err)?;
                    Ok(#ident::from(m))
                }

                async fn delete(&self, id: #pk_ty)
                    -> ::core::result::Result<(), ::architect::RepoError>
                {
                    let res = Entity::delete_by_id(id).exec(&self.db).await
                        .map_err(map_err)?;
                    if res.rows_affected == 0 {
                        return ::core::result::Result::Err(::architect::RepoError::NotFound);
                    }
                    ::core::result::Result::Ok(())
                }
            }

        }

        // Only the `<Ident>RepoStorage` type leaks into the parent
        // module. The SeaORM `Model` / `Entity` / `Column` / `Relation`
        // / `ActiveModel` items stay inside `__<snake>_storage` —
        // re-exporting them by their bare names collides when multiple
        // `#[derive(Entity)]` structs share a module. Code that wants
        // to reach into them (rare; the repo trait covers normal use)
        // can name `__<snake>_storage::Entity` directly.
        #[cfg(feature = "server")]
        #vis use #storage_mod::#storage_ident;

        // The generated storage backend is the canonical single-service
        // backend: it provides exactly the repo. Declaring its `Services`
        // bundle lets `<Entity>RepoStorage::new(db).into_router()` work
        // out of the box, swappable with any other repo backend.
        #[cfg(all(feature = "server", feature = "vox"))]
        impl<C: ::architect::storage::DbConn> ::architect::Services for #storage_ident<C> {
            fn layers() -> impl ::architect::Layer<Self> {
                ::architect::layers![#layer_token]
            }
        }
    }
}

// ── `#[derive(JsonField)]` — JSON-column support ─────────────────────
//
// Companion derive for any `enum` or `struct` that appears in an
// `#[derive(Entity)]` field carrying `#[architect(json)]`. SeaORM's
// `From<T> for sea_orm::Value`, `TryGetable`, `ValueType`, and
// `Nullable` are the four traits a column type must satisfy; this
// derive emits all four (cfg-gated on the user's `server` feature)
// via a `serde_json` round-trip.
//
// Vec<T> columns can't use this directly — orphan rules forbid
// `impl From<Vec<X>> for sea_orm::Value`. Wrap in a newtype:
//
// ```ignore
// #[derive(architect::JsonField, facet::Facet, Clone, Debug, PartialEq,
//          serde::Serialize, serde::Deserialize)]
// pub struct LineItems(pub Vec<InvoiceLineItem>);
// ```
//
// Requirements on the annotated type:
// - `serde::Serialize + serde::de::DeserializeOwned` (the trip is
//   `serde_json::to_value` / `serde_json::from_value`).
// - The user's crate must depend on `serde_json` for the emitted
//   code to compile under the `server` feature.
#[proc_macro_derive(JsonField)]
pub fn derive_json_field(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let ident = &input.ident;
    let (impl_g, ty_g, where_g) = input.generics.split_for_impl();
    let type_name = ident.to_string();

    quote! {
        #[cfg(feature = "server")]
        const _: () = {
            extern crate serde_json as __serde_json;

            impl #impl_g ::core::convert::From<#ident #ty_g> for ::sea_orm::Value #where_g {
                fn from(v: #ident #ty_g) -> Self {
                    ::sea_orm::Value::Json(
                        __serde_json::to_value(&v).ok().map(::std::boxed::Box::new),
                    )
                }
            }

            impl #impl_g ::sea_orm::TryGetable for #ident #ty_g #where_g {
                fn try_get_by<I: ::sea_orm::ColIdx>(
                    res: &::sea_orm::QueryResult,
                    idx: I,
                ) -> ::core::result::Result<Self, ::sea_orm::TryGetError> {
                    let v: __serde_json::Value =
                        <__serde_json::Value as ::sea_orm::TryGetable>::try_get_by(res, idx)?;
                    __serde_json::from_value(v).map_err(|e| {
                        ::sea_orm::TryGetError::DbErr(::sea_orm::DbErr::Json(e.to_string()))
                    })
                }
            }

            impl #impl_g ::sea_orm::sea_query::ValueType for #ident #ty_g #where_g {
                fn try_from(
                    v: ::sea_orm::Value,
                ) -> ::core::result::Result<Self, ::sea_orm::sea_query::ValueTypeErr> {
                    match v {
                        ::sea_orm::Value::Json(::core::option::Option::Some(b)) => {
                            __serde_json::from_value::<Self>(*b)
                                .map_err(|_| ::sea_orm::sea_query::ValueTypeErr)
                        }
                        _ => ::core::result::Result::Err(::sea_orm::sea_query::ValueTypeErr),
                    }
                }

                fn type_name() -> ::std::string::String {
                    ::std::string::String::from(#type_name)
                }

                fn array_type() -> ::sea_orm::sea_query::ArrayType {
                    ::sea_orm::sea_query::ArrayType::Json
                }

                fn column_type() -> ::sea_orm::sea_query::ColumnType {
                    ::sea_orm::sea_query::ColumnType::Json
                }
            }

            impl #impl_g ::sea_orm::sea_query::Nullable for #ident #ty_g #where_g {
                fn null() -> ::sea_orm::Value {
                    ::sea_orm::Value::Json(::core::option::Option::None)
                }
            }
        };
    }
    .into()
}

// ── CRDT emission ─────────────────────────────────────────────────────

/// Last path segment of a type, e.g. `chrono::DateTime<Utc>` → `DateTime`.
fn type_last_ident(ty: &Type) -> Option<String> {
    match ty {
        Type::Path(p) => p.path.segments.last().map(|s| s.ident.to_string()),
        _ => None,
    }
}

/// First generic argument of the last path segment (`Option<T>` → `T`).
fn type_first_generic(ty: &Type) -> Option<&Type> {
    let Type::Path(p) = ty else { return None };
    let seg = p.path.segments.last()?;
    let syn::PathArguments::AngleBracketed(args) = &seg.arguments else {
        return None;
    };
    args.args.iter().find_map(|a| match a {
        syn::GenericArgument::Type(t) => Some(t),
        _ => None,
    })
}

/// The container shape (after unwrapping one `Option`) that requires
/// `#[architect(json)]` for the SeaORM column emission, or `None` for
/// types SeaORM maps natively. `Vec<u8>` is native (bytes column);
/// every other `Vec<T>`, plus the std maps/sets, has no
/// `sea_orm::Value` impl and must be stored as JSON.
fn json_requiring_container(ty: &Type) -> Option<String> {
    let base = if type_last_ident(ty).as_deref() == Some("Option") {
        type_first_generic(ty)?
    } else {
        ty
    };
    let last = type_last_ident(base)?;
    match last.as_str() {
        "Vec" => {
            let inner = type_first_generic(base).and_then(type_last_ident);
            if inner.as_deref() == Some("u8") {
                None // Vec<u8> → native bytes column.
            } else {
                Some(last)
            }
        }
        "VecDeque" | "HashMap" | "BTreeMap" | "HashSet" | "BTreeSet" => Some(last),
        _ => None,
    }
}

/// How a field maps onto `crdt::codec`. Mirrors the kinds the hand-
/// written `entity_crdt!` macro supports; the derive infers them from
/// the field's Rust type instead of a declaration.
#[derive(Copy, Clone, PartialEq, Eq)]
enum CrdtKind {
    Uuid,
    Str,
    Bool,
    I32,
    I64,
    U32,
    Dt,
    StringList,
}

impl CrdtKind {
    /// `(kind, optional)` for a field type, or None if unsupported.
    fn of(ty: &Type) -> Option<(CrdtKind, bool)> {
        let last = type_last_ident(ty)?;
        if last == "Option" {
            let inner = type_first_generic(ty)?;
            return Self::base_of(inner).map(|k| (k, true));
        }
        Self::base_of(ty).map(|k| (k, false))
    }

    fn base_of(ty: &Type) -> Option<CrdtKind> {
        match type_last_ident(ty)?.as_str() {
            "Uuid" => Some(Self::Uuid),
            "String" => Some(Self::Str),
            "bool" => Some(Self::Bool),
            "i32" => Some(Self::I32),
            "i64" => Some(Self::I64),
            "u32" => Some(Self::U32),
            "DateTime" => Some(Self::Dt),
            "Vec" => match type_first_generic(ty).and_then(type_last_ident)?.as_str() {
                "String" => Some(Self::StringList),
                _ => None,
            },
            _ => None,
        }
    }

    /// Codec function suffix (`write_{suffix}` / `read_{suffix}`).
    fn suffix(self, optional: bool) -> String {
        let base = match self {
            Self::Uuid => "uuid",
            Self::Str => "str",
            Self::Bool => "bool",
            Self::I32 => "i32",
            Self::I64 => "i64",
            Self::U32 => "u32",
            Self::Dt => "dt",
            Self::StringList => "string_list",
        };
        if optional {
            format!("opt_{base}")
        } else {
            base.to_string()
        }
    }

    /// Shape a value expression for the writer's parameter type.
    fn write_arg(self, optional: bool, value: TokenStream2) -> TokenStream2 {
        match (self, optional) {
            (Self::Str, false) => quote! { &#value },
            (Self::Str, true) => quote! { #value.as_deref() },
            (Self::StringList, false) => quote! { &#value },
            (Self::StringList, true) => quote! { #value.as_deref() },
            _ => value,
        }
    }
}

/// Emit the entity's CRDT story (under the user crate's `crdt` feature,
/// convention `crdt = ["dep:crdt"]`):
///
/// ```ignore
/// impl crdt::EntityCrdt for Example { … }      // field ↔ LoroMap codec
/// struct ExampleRepoLoro { … }                 // Loro-backed ExampleRepo
/// // and, with the `atom` feature too (convention adds `crdt?/dioxus`):
/// fn use_example_crdt_list() -> AtomResult<Vec<Example>, RepoError>;
/// fn use_example_crdt(id: Uuid) -> AtomResult<Example, RepoError>;
/// struct ExampleCrdtActions { … }              // create/update/delete, local-first
/// fn use_example_crdt_actions() -> ExampleCrdtActions;
/// ```
///
/// The hooks read the app's `use_synced_doc` replica from context and
/// re-read on every doc revision — local writes and remote peers' edits
/// render through the same path, no optimistic machinery, no rollback.
#[allow(clippy::too_many_arguments)]
fn build_crdt_block(
    ident: &Ident,
    vis: &syn::Visibility,
    container: &ContainerAttrs,
    parsed: &[ParsedField],
    pk_ident: &Ident,
    create_ident: &Ident,
    update_ident: &Ident,
    list_ident: &Ident,
    repo_ident: &Ident,
    update_fields: &[&ParsedField],
) -> Result<TokenStream2> {
    if !container.emit_crdt {
        return Ok(quote! {});
    }

    let snake = ident.to_string().to_snake_case();
    let root = container
        .table_name
        .clone()
        .unwrap_or_else(|| snake.clone());
    let repo_loro_ident = format_ident!("{}RepoLoro", ident);
    let actions_ident = format_ident!("{}CrdtActions", ident);
    let use_list_fn = format_ident!("use_{}_crdt_list", snake);
    let use_one_fn = format_ident!("use_{}_crdt", snake);
    let use_actions_fn = format_ident!("use_{}_crdt_actions", snake);

    // Resolve every field's codec kind up front so unsupported types
    // fail with a pointed error instead of a generated-code mismatch.
    let mut kinds: Vec<(CrdtKind, bool)> = Vec::with_capacity(parsed.len());
    for f in parsed {
        let Some(kind) = CrdtKind::of(f.ty) else {
            return Err(syn::Error::new_spanned(
                f.ty,
                "#[architect(crdt)]: unsupported field type for the LoroMap \
                 codec (supported: Uuid, String, bool, i32, i64, u32, \
                 DateTime<Utc>, Vec<String>, and Option of each)",
            ));
        };
        kinds.push(kind);
    }

    // `from_create`: same policy as the SeaORM storage path — `on_create`
    // expressions win, excluded fields fall back to Default, the rest
    // come from the payload.
    let from_create_assigns = parsed.iter().map(|f| {
        let id = f.ident;
        if let Some(e) = &f.attrs.on_create {
            quote! { #id: #e }
        } else if f.attrs.exclude_create {
            quote! { #id: ::core::default::Default::default() }
        } else {
            quote! { #id: c.#id }
        }
    });

    let encode_calls = parsed.iter().zip(&kinds).map(|(f, (kind, opt))| {
        let id = f.ident;
        let key = LitStr::new(&id.to_string(), id.span());
        let writer = format_ident!("write_{}", kind.suffix(*opt));
        let arg = kind.write_arg(*opt, quote! { e.#id });
        quote! { ::crdt::codec::#writer(m, #key, #arg)?; }
    });

    let decode_assigns = parsed.iter().zip(&kinds).map(|(f, (kind, opt))| {
        let id = f.ident;
        let key = LitStr::new(&id.to_string(), id.span());
        let reader = format_ident!("read_{}", kind.suffix(*opt));
        quote! { #id: ::crdt::codec::#reader(m, #key)? }
    });

    // `apply_update`: write each `Some` field of the Update payload …
    let apply_update_arms = update_fields.iter().map(|f| {
        let id = f.ident;
        let key = LitStr::new(&id.to_string(), id.span());
        let (kind, opt) = CrdtKind::of(f.ty).expect("validated above");
        let writer = format_ident!("write_{}", kind.suffix(opt));
        let arg = kind.write_arg(opt, quote! { v });
        quote! {
            if let ::core::option::Option::Some(v) = u.#id {
                ::crdt::codec::#writer(m, #key, #arg)?;
            }
        }
    });
    // … then refresh the `on_update` fields (e.g. `updated_at`), exactly
    // like the SeaORM update path does.
    let apply_update_touches = parsed
        .iter()
        .zip(&kinds)
        .filter(|(f, _)| f.attrs.on_update.is_some())
        .map(|(f, (kind, opt))| {
            let id = f.ident;
            let key = LitStr::new(&id.to_string(), id.span());
            let e = f.attrs.on_update.as_ref().unwrap();
            let writer = format_ident!("write_{}", kind.suffix(*opt));
            let arg = kind.write_arg(*opt, quote! { __touch });
            quote! {
                {
                    let __touch = #e;
                    ::crdt::codec::#writer(m, #key, #arg)?;
                }
            }
        });

    let sort_arms = parsed.iter().filter(|f| f.attrs.sortable).map(|f| {
        let id = f.ident;
        let key = LitStr::new(&id.to_string(), id.span());
        quote! { #key => items.sort_by(|a, b| a.#id.cmp(&b.#id)), }
    });

    let doc_repo_loro = format!(
        "Loro-backed [`{ident}`] repo — a typed view over a \
         [`crdt::CrdtDoc`]'s `\"{root}\"` container. Same `{repo_ident}` \
         surface as the SeaORM storage, but rows live in the replica: \
         writes are local-first and merge across peers."
    );
    let doc_use_list = format!(
        "Every [`{ident}`] in the app's synced doc (provided by \
         `crdt::use_synced_doc`), re-read on every doc revision — local \
         writes and remote peers' edits render through the same path."
    );
    let doc_use_one = format!(
        "One [`{ident}`] from the synced doc by id, re-read on every doc \
         revision. `Error(NotFound)` until a peer (or you) creates it."
    );
    let doc_actions = format!(
        "Local-first write actions for [`{ident}`]: each writes the \
         replica synchronously-fast (no server round-trip, works offline) \
         and the change syncs + merges in the background. There is no \
         rollback arm — concurrent edits merge instead of conflicting."
    );

    Ok(quote! {
        #[cfg(feature = "crdt")]
        impl ::crdt::EntityCrdt for #ident {
            type Wire = #ident;
            type Create = #create_ident;
            type Update = #update_ident;
            type List = #list_ident;

            const ROOT: &'static str = #root;

            fn id(e: &#ident) -> ::uuid::Uuid {
                e.#pk_ident
            }

            fn from_create(c: #create_ident) -> #ident {
                #ident { #(#from_create_assigns,)* }
            }

            fn encode_into(
                m: &::crdt::loro::LoroMap,
                e: &#ident,
            ) -> ::core::result::Result<(), ::architect::RepoError> {
                #(#encode_calls)*
                ::core::result::Result::Ok(())
            }

            fn decode_from(
                m: &::crdt::loro::LoroMap,
            ) -> ::core::result::Result<#ident, ::architect::RepoError> {
                ::core::result::Result::Ok(#ident { #(#decode_assigns,)* })
            }

            fn apply_update(
                m: &::crdt::loro::LoroMap,
                u: #update_ident,
            ) -> ::core::result::Result<(), ::architect::RepoError> {
                #(#apply_update_arms)*
                #(#apply_update_touches)*
                ::core::result::Result::Ok(())
            }

            fn sort_items(
                items: &mut [#ident],
                field: &str,
                order: ::architect::SortOrder,
            ) -> ::core::result::Result<(), ::architect::RepoError> {
                match field {
                    #(#sort_arms)*
                    other => {
                        return ::core::result::Result::Err(::architect::RepoError::InvalidInput(
                            ::std::format!("unknown sort field: {other}"),
                        ));
                    }
                }
                if order == ::architect::SortOrder::Desc {
                    items.reverse();
                }
                ::core::result::Result::Ok(())
            }

            fn build_list(
                items: ::std::vec::Vec<#ident>,
                total: u32,
                page: ::architect::Page,
            ) -> #list_ident {
                #list_ident { items, total, page }
            }
        }

        #[doc = #doc_repo_loro]
        #[cfg(feature = "crdt")]
        #[derive(Clone)]
        #vis struct #repo_loro_ident {
            inner: ::crdt::LoroRepo<#ident>,
        }

        #[cfg(feature = "crdt")]
        impl #repo_loro_ident {
            #vis fn new(doc: &::crdt::CrdtDoc) -> Self {
                Self { inner: doc.repo() }
            }

            /// The generic repo underneath (sync reads, text ops).
            #vis fn inner(&self) -> &::crdt::LoroRepo<#ident> {
                &self.inner
            }
        }

        #[cfg(feature = "crdt")]
        impl #repo_ident for #repo_loro_ident {
            async fn get(
                &self,
                id: ::uuid::Uuid,
            ) -> ::core::result::Result<#ident, ::architect::RepoError> {
                self.inner.get(id).await
            }
            async fn list(
                &self,
                page: ::architect::Page,
                sort: ::core::option::Option<::architect::Sort>,
                filter: ::core::option::Option<::architect::Filter>,
            ) -> ::core::result::Result<#list_ident, ::architect::RepoError> {
                self.inner.list(page, sort, filter).await
            }
            async fn create(
                &self,
                input: #create_ident,
            ) -> ::core::result::Result<#ident, ::architect::RepoError> {
                self.inner.create(input).await
            }
            async fn update(
                &self,
                id: ::uuid::Uuid,
                input: #update_ident,
            ) -> ::core::result::Result<#ident, ::architect::RepoError> {
                self.inner.update(id, input).await
            }
            async fn delete(
                &self,
                id: ::uuid::Uuid,
            ) -> ::core::result::Result<(), ::architect::RepoError> {
                self.inner.delete(id).await
            }
        }

        #[doc = #doc_use_list]
        #[cfg(all(feature = "crdt", feature = "atom"))]
        #vis fn #use_list_fn() -> ::architect::AtomResult<
            ::std::vec::Vec<#ident>,
            ::architect::RepoError,
        > {
            ::crdt::use_crdt_list::<#ident>()
        }

        #[doc = #doc_use_one]
        #[cfg(all(feature = "crdt", feature = "atom"))]
        #vis fn #use_one_fn(
            id: ::uuid::Uuid,
        ) -> ::architect::AtomResult<#ident, ::architect::RepoError> {
            ::crdt::use_crdt_entry::<#ident>(id)
        }

        #[doc = #doc_actions]
        #[cfg(all(feature = "crdt", feature = "atom"))]
        #[derive(Clone, Copy)]
        #vis struct #actions_ident {
            handle: ::crdt::DocHandle,
            notify: ::core::option::Option<::architect::Notifications>,
        }

        #[doc = #doc_actions]
        #[cfg(all(feature = "crdt", feature = "atom"))]
        #vis fn #use_actions_fn() -> #actions_ident {
            #actions_ident {
                handle: ::crdt::use_doc_handle(),
                notify: ::architect::try_use_notifications(),
            }
        }

        #[cfg(all(feature = "crdt", feature = "atom"))]
        impl #actions_ident {
            /// Add a row to the replica — applied locally now, synced +
            /// merged in the background.
            #vis fn create(&self, input: #create_ident) {
                let ::core::option::Option::Some(repo) = self.handle.repo::<#ident>() else {
                    return;
                };
                let notify = self.notify;
                ::architect::dioxus::dioxus_core::spawn_forever(async move {
                    if let ::core::result::Result::Err(e) = repo.create(input).await
                        && let ::core::option::Option::Some(n) = notify
                    {
                        n.error(::std::format!("create failed: {e}"));
                    }
                });
            }

            /// Patch a row in the replica (each `Some` field of the
            /// Update payload).
            #vis fn update(&self, id: ::uuid::Uuid, input: #update_ident) {
                let ::core::option::Option::Some(repo) = self.handle.repo::<#ident>() else {
                    return;
                };
                let notify = self.notify;
                ::architect::dioxus::dioxus_core::spawn_forever(async move {
                    if let ::core::result::Result::Err(e) = repo.update(id, input).await
                        && let ::core::option::Option::Some(n) = notify
                    {
                        n.error(::std::format!("update failed: {e}"));
                    }
                });
            }

            /// Remove a row from the replica.
            #vis fn delete(&self, id: ::uuid::Uuid) {
                let ::core::option::Option::Some(repo) = self.handle.repo::<#ident>() else {
                    return;
                };
                let notify = self.notify;
                ::architect::dioxus::dioxus_core::spawn_forever(async move {
                    if let ::core::result::Result::Err(e) = repo.delete(id).await
                        && let ::core::option::Option::Some(n) = notify
                    {
                        n.error(::std::format!("delete failed: {e}"));
                    }
                });
            }
        }
    })
}

// ── Derive-time validation tests ──────────────────────────────────────
//
// These drive `expand()` directly (no proc-macro harness needed): build
// a `DeriveInput` with `parse_quote!` and assert on the `Ok`/`Err`
// shape. Every derive-time error added to `expand` gets a test here;
// the *behavior* of the emitted code is covered by the example app's
// test crates (`example-tests-native`, `app-tests-e2e`).

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    fn expand_err(input: DeriveInput) -> String {
        expand(input)
            .expect_err("expected a derive-time error")
            .to_string()
    }

    fn expand_ok(input: DeriveInput) -> String {
        expand(input)
            .expect("expected the derive to expand")
            .to_string()
    }

    // ── #[architect(json)] required on container-shaped fields ──

    #[test]
    fn vec_field_without_json_errors() {
        let err = expand_err(parse_quote! {
            #[architect(table_name = "things", repo)]
            pub struct Thing {
                #[architect(primary_key)]
                pub id: Uuid,
                pub tags: Vec<String>,
            }
        });
        assert!(err.contains("#[architect(json)]"), "got: {err}");
        assert!(err.contains("Vec<…>"), "got: {err}");
    }

    #[test]
    fn option_vec_field_without_json_errors() {
        let err = expand_err(parse_quote! {
            #[architect(repo)]
            pub struct Thing {
                #[architect(primary_key)]
                pub id: Uuid,
                pub maybe_tags: Option<Vec<String>>,
            }
        });
        assert!(err.contains("#[architect(json)]"), "got: {err}");
    }

    #[test]
    fn map_field_without_json_errors() {
        let err = expand_err(parse_quote! {
            #[architect(repo)]
            pub struct Thing {
                #[architect(primary_key)]
                pub id: Uuid,
                pub extras: HashMap<String, String>,
            }
        });
        assert!(err.contains("HashMap<…>"), "got: {err}");
    }

    #[test]
    fn vec_u8_field_needs_no_json() {
        // SeaORM maps Vec<u8> natively (bytes column) — no false positive.
        expand_ok(parse_quote! {
            #[architect(repo)]
            pub struct Thing {
                #[architect(primary_key)]
                pub id: Uuid,
                pub payload: Vec<u8>,
            }
        });
    }

    #[test]
    fn vec_field_with_json_is_accepted() {
        expand_ok(parse_quote! {
            #[architect(repo)]
            pub struct Thing {
                #[architect(primary_key)]
                pub id: Uuid,
                #[architect(json)]
                pub tags: Vec<String>,
            }
        });
    }

    // ── form attribute misuse ──

    #[test]
    fn form_optional_on_non_string_errors() {
        let err = expand_err(parse_quote! {
            #[architect(repo, form)]
            pub struct Thing {
                #[architect(primary_key)]
                pub id: Uuid,
                #[architect(form(optional))]
                pub count: u32,
            }
        });
        assert!(err.contains("only supported on `String`"), "got: {err}");
    }

    #[test]
    fn form_attr_without_container_form_flag_errors() {
        let err = expand_err(parse_quote! {
            #[architect(repo)]
            pub struct Thing {
                #[architect(primary_key)]
                pub id: Uuid,
                #[architect(form(label = "Name"))]
                pub name: String,
            }
        });
        assert!(err.contains("container-level `form` flag"), "got: {err}");
    }

    #[test]
    fn form_attr_on_excluded_field_errors() {
        // Excluded from both payloads → no form field is ever generated.
        let err = expand_err(parse_quote! {
            #[architect(repo, form)]
            pub struct Thing {
                #[architect(primary_key)]
                pub id: Uuid,
                pub name: String,
                #[architect(exclude(create, update), form(label = "Created"))]
                pub created_at: DateTime<Utc>,
            }
        });
        assert!(
            err.contains("neither the Create nor the Update"),
            "got: {err}"
        );
    }

    #[test]
    fn form_optional_on_string_is_accepted() {
        expand_ok(parse_quote! {
            #[architect(repo, form)]
            pub struct Thing {
                #[architect(primary_key)]
                pub id: Uuid,
                #[architect(form(optional, label = "Notes"))]
                pub notes: String,
            }
        });
    }

    // ── pk-type threading + crdt's Uuid requirement ──

    #[test]
    fn string_pk_with_store_and_events_expands() {
        let out = expand_ok(parse_quote! {
            #[architect(table_name = "tags", repo, store, events, form)]
            pub struct Tag {
                #[architect(primary_key, auto_increment = false)]
                pub slug: String,
                pub label: String,
            }
        });
        // The pk type is threaded through, not hardcoded to Uuid.
        assert!(out.contains("TagEvent"), "events emission missing: {out}");
        assert!(
            !out.contains(":: uuid :: Uuid"),
            "Uuid leaked into a String-pk expansion"
        );
    }

    #[test]
    fn crdt_requires_a_uuid_pk() {
        let err = expand_err(parse_quote! {
            #[architect(repo, crdt)]
            pub struct Thing {
                #[architect(primary_key)]
                pub slug: String,
            }
        });
        assert!(err.contains("requires a `Uuid` primary key"), "got: {err}");
    }
}
