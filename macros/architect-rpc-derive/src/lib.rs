//! `architect-rpc-derive` — the `#[architect::rpc]` proc-macro.
//!
//! Given a trait declaration, emit a complete RPC face — async client,
//! server-side host, hidden async mirror — so the **same trait** is
//! callable in-process (sync where natural) and across the network
//! (async, vox-served).
//!
//! The macro inspects each method, classifies it sync vs. async, and
//! adapts what it emits to the trait's natural shape:
//!
//! - **All-sync**  → bridge marshals every call onto a `Dispatcher`,
//!   emits an async mirror used by the vox client/host.
//! - **All-async** → trait already its own RPC face. Decorate with
//!   `#[vox::service]` directly; `<T>Host` is a thin newtype.
//! - **Mixed**    → sync methods bridged, async methods pass through.
//!
//! See `docs/content/reference/_index.md` (the "`#[architect::rpc]` mechanics" section)
//! for the object-safety rules and argument-rewriting table.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{
    FnArg, ItemTrait, Pat, PatIdent, ReturnType, TraitItem, TraitItemFn, Type, parse_macro_input,
    parse_quote,
};

/// `#[architect::rpc]` — annotate a trait to derive its RPC face.
///
/// Usage:
///
/// ```ignore
/// #[architect::rpc]
/// pub trait Tracks {
///     fn all(&self) -> Vec<Track>;
///     fn set_muted(&self, guid: &str, muted: bool) -> Result<(), DawError>;
/// }
/// ```
///
/// emits, in addition to the input trait:
///
/// - `TracksRpc` — hidden async mirror, vox::service-decorated. The
///   trait `#[vox::service]` would generate clients and dispatchers
///   against; user code touches it only by name when mounting.
/// - `TracksHost<S, D>` — server-side wrapper accepting any `impl
///   Tracks` plus a `Dispatcher`, implementing `TracksRpc` by
///   marshaling each sync method through the dispatcher.
/// - `TracksClient` — type alias for the vox-emitted `TracksRpcClient`,
///   so callers see a clean name.
///
/// # Streams: `#[subscribe]`
///
/// A method marked `#[subscribe]` is a stream declaration, not a
/// callable method — it names an event type and is stripped from the
/// trait:
///
/// ```ignore
/// #[architect::rpc]
/// pub trait Tracks {
///     fn all(&self) -> Vec<Track>;
///     /// Every track-set change, as it happens.
///     #[subscribe]
///     fn events(&self) -> TrackEvent;
/// }
/// ```
///
/// emits (vox-gated) the `TracksStream` sibling service
/// (`async fn events(&self, sink: Tx<TrackEvent>)`), the
/// `TracksStreamSource` backend contract (`fn events_hub(&self) ->
/// &PubSub<TrackEvent>` — the backend owns and publishes into the
/// hub), and `stream_serve` / `stream_layer` / `StreamService` mount
/// verbs. Declarations with filter params (`#[subscribe] fn events(&self,
/// filter: F) -> E`) ask the backend for `fn events_attach(&self,
/// filter: F, sink: Tx<E>)` instead, so filtering and
/// snapshot-then-changes (`PubSub::begin_attach`) stay backend-owned.
#[proc_macro_attribute]
pub fn rpc(args: TokenStream, input: TokenStream) -> TokenStream {
    let mut rpc_args = RpcArgs::default();
    let parser = syn::meta::parser(|meta| rpc_args.parse(&meta));
    parse_macro_input!(args with parser);

    let trait_item = parse_macro_input!(input as ItemTrait);

    match expand(trait_item, rpc_args) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// Arguments to `#[architect::rpc(...)]`. All optional; the bare form
/// is the common case.
#[derive(Default)]
struct RpcArgs {
    /// `sync_client` — additionally emit the `<Trait>SyncClient`
    /// blocking facade (vox-gated, native only). Opt-in so consumers
    /// that never block don't carry the extra surface.
    sync_client: bool,
    /// `context = SomeType` — the ambient per-call context. Declared
    /// once on the trait: every method receives it without repeating
    /// the parameter (sync methods get `ctx: &T`, async and wire
    /// surfaces get owned `T`), and the client gains a `.scoped(ctx)`
    /// wrapper so call sites set it once per scope.
    context: Option<Type>,
    /// `scopes(name: Type, ...)` — the trait's leveled call scopes, in
    /// nesting order (e.g. `scopes(project: ProjectContext, track:
    /// TrackRef)`). Emits one scope view per level over the local direct
    /// view: methods whose leading parameters match the first N scope
    /// types are callable from level N with those parameters elided —
    /// `daw.tracks_direct().project(ctx).track(tr).set_volume(0.9)`.
    scopes: Vec<ScopeDecl>,
    /// `ops` / `ops(Src as Dst, ...)` — additionally emit the
    /// `<Trait>Op` / `<Trait>OpOutput` reified-call enums plus an
    /// `apply` that replays an op against any backend. Substitution
    /// pairs (`ProjectContext as ProjectArg`) swap a parameter type
    /// for a deferred wire representation resolved at apply time via
    /// `architect::ops::ResolveArg`.
    ops: Option<Vec<OpsSubst>>,
}

/// One `name: Type` scope declaration inside `scopes(...)`.
struct ScopeDecl {
    name: syn::Ident,
    ty: Type,
}

impl syn::parse::Parse for ScopeDecl {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let name: syn::Ident = input.parse()?;
        input.parse::<syn::Token![:]>()?;
        let ty: Type = input.parse()?;
        Ok(ScopeDecl { name, ty })
    }
}

/// One `Src as Dst` substitution inside `ops(...)`: parameters of
/// type `src` are carried on the op enum as `dst` and resolved back
/// at `apply` time.
struct OpsSubst {
    src: Type,
    dst: Type,
}

impl syn::parse::Parse for OpsSubst {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let src: Type = input.parse()?;
        input.parse::<syn::Token![as]>()?;
        let dst: Type = input.parse()?;
        Ok(OpsSubst { src, dst })
    }
}

impl RpcArgs {
    fn parse(&mut self, meta: &syn::meta::ParseNestedMeta) -> syn::Result<()> {
        if meta.path.is_ident("sync_client") {
            self.sync_client = true;
            return Ok(());
        }
        if meta.path.is_ident("context") {
            self.context = Some(meta.value()?.parse()?);
            return Ok(());
        }
        if meta.path.is_ident("scopes") {
            if !self.scopes.is_empty() {
                return Err(meta.error("duplicate `scopes` argument"));
            }
            let content;
            syn::parenthesized!(content in meta.input);
            self.scopes = content
                .parse_terminated(<ScopeDecl as syn::parse::Parse>::parse, syn::Token![,])?
                .into_iter()
                .collect();
            if self.scopes.is_empty() {
                return Err(meta.error("`scopes(...)` needs at least one `name: Type`"));
            }
            return Ok(());
        }
        if meta.path.is_ident("ops") {
            if self.ops.is_some() {
                return Err(meta.error("duplicate `ops` argument"));
            }
            if meta.input.peek(syn::token::Paren) {
                let content;
                syn::parenthesized!(content in meta.input);
                let pairs = content
                    .parse_terminated(<OpsSubst as syn::parse::Parse>::parse, syn::Token![,])?
                    .into_iter()
                    .collect();
                self.ops = Some(pairs);
            } else {
                self.ops = Some(Vec::new());
            }
            return Ok(());
        }
        Err(meta.error(
            "unknown #[architect::rpc] argument — supported: `sync_client` \
             (emit the blocking <Trait>SyncClient facade), `context = Type` \
             (ambient per-call context threaded through every method), \
             `scopes(name: Type, ...)` (leveled local scope views over the \
             direct view), `ops` / `ops(Src as Dst, ...)` (emit the \
             <Trait>Op reified-call enum + apply). Backend requirements are \
             declared per backend at the bundle site, not on the trait.",
        ))
    }
}

/// `#[derive(HasDispatcher)]` — emit the boilerplate `HasDispatcher`
/// impl for a backend whose dispatcher is default-constructible.
///
/// Without arguments the impl points at
/// `::architect::dispatch::DefaultDispatcher` — the build's default
/// (`TokioBlockingDispatcher` when architect's `dispatch-tokio`
/// feature is on, `CurrentThreadDispatcher` otherwise):
///
/// ```ignore
/// #[derive(architect::HasDispatcher)]
/// pub struct TaskBackend { /* … */ }
/// ```
///
/// An explicit `#[dispatch(SomeDispatcher)]` attribute overrides the
/// type (any path is accepted; it must implement
/// `architect::dispatch::Dispatcher + Default`):
///
/// ```ignore
/// #[derive(architect::HasDispatcher)]
/// #[dispatch(architect::dispatch::CurrentThreadDispatcher)]
/// pub struct TestBackend { /* … */ }
/// ```
///
/// Backends whose dispatcher needs runtime state (a REAPER main-thread
/// queue handle, etc.) keep writing the manual impl — the derive only
/// covers the `Default`-constructible case, which is the overwhelming
/// majority in server binaries.
#[proc_macro_derive(HasDispatcher, attributes(dispatch))]
pub fn derive_has_dispatcher(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as syn::DeriveInput);
    match expand_has_dispatcher(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

fn expand_has_dispatcher(input: syn::DeriveInput) -> syn::Result<TokenStream2> {
    let mut dispatcher_ty: Option<Type> = None;
    for attr in &input.attrs {
        if attr.path().is_ident("dispatch") {
            if dispatcher_ty.is_some() {
                return Err(syn::Error::new_spanned(
                    attr,
                    "duplicate #[dispatch(...)] attribute — a backend has exactly one dispatcher",
                ));
            }
            dispatcher_ty = Some(attr.parse_args::<Type>()?);
        }
    }
    let dispatcher_ty: Type =
        dispatcher_ty.unwrap_or_else(|| parse_quote! { ::architect::dispatch::DefaultDispatcher });

    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    Ok(quote! {
        impl #impl_generics ::architect::HasDispatcher for #name #ty_generics #where_clause {
            type Dispatcher = #dispatcher_ty;

            fn dispatcher(&self) -> Self::Dispatcher {
                <#dispatcher_ty as ::core::default::Default>::default()
            }
        }
    })
}

/// Main expansion entry. Split out from the proc-macro shim so it can
/// be exercised from unit tests once they exist.
fn expand(trait_item: ItemTrait, args: RpcArgs) -> syn::Result<TokenStream2> {
    let trait_name = &trait_item.ident;
    let vis = &trait_item.vis;
    let rpc_trait_name = format_ident!("{}Rpc", trait_name);
    // Bridge struct is hidden — users only see `serve`. The name is
    // doc(hidden) and underscored so it doesn't show up in completions.
    let host_name = format_ident!("__{}Bridge", trait_name);
    let client_name = format_ident!("{}Client", trait_name);
    let rpc_client_name = format_ident!("{}RpcClient", trait_name);
    let rpc_dispatcher_name = format_ident!("{}RpcDispatcher", trait_name);

    // Classify + validate each method. `#[subscribe]` declarations are
    // split off — they're stream declarations, not callable methods.
    let mut methods = Vec::new();
    let mut subscriptions = Vec::new();
    for item in &trait_item.items {
        if let TraitItem::Fn(method) = item {
            if has_subscribe_attr(method) {
                subscriptions.push(classify_subscription(method)?);
            } else {
                methods.push(classify_method(method)?);
            }
        }
    }

    let shape = classify_shape(&methods);

    // Always emit the user's trait — adding the `Send + Sync + 'static`
    // bound so it's safe to share through `Arc<dyn Trait>` from inside
    // the bridge. We don't mutate the input AST; we re-emit with the
    // augmented supertraits. `#[subscribe]` declarations are dropped
    // here — they materialize as the stream sibling, not as methods.
    let ctx = args.context.as_ref();
    let user_trait = emit_user_trait(&trait_item, shape, ctx);

    let (mirror_trait, host_struct, host_impl) = match shape {
        Shape::Empty | Shape::AllAsync => {
            // The trait is already its own RPC face. We still emit a
            // mirror alias + host so consumer wiring is uniform across
            // shapes — `<T>Host::new(impl T)` mounts the same way the
            // sync bridge does.
            (
                quote! {},
                emit_passthrough_host(trait_name, &host_name, vis),
                emit_passthrough_impl(trait_name, &host_name, &methods, ctx),
            )
        }
        Shape::AllSync | Shape::Mixed => (
            emit_mirror_trait(&rpc_trait_name, vis, &methods, ctx),
            emit_bridge_host(trait_name, &host_name, vis),
            emit_bridge_impl(
                trait_name,
                &host_name,
                &rpc_trait_name,
                &methods,
                matches!(shape, Shape::AllSync),
                ctx,
            ),
        ),
    };

    // Client alias — only emitted when the consumer crate enables its
    // `vox` feature, since the underlying `<T>RpcClient` type is
    // generated by `#[vox::service]` and otherwise doesn't exist. For
    // AllAsync the vox-generated client is named off the user trait
    // directly, so no alias is needed.
    let client_alias = match shape {
        Shape::Empty => quote! {},
        Shape::AllAsync => {
            // vox::service is applied to the user trait directly, so it
            // emits `<Trait>Client`, `<Trait>Dispatcher`, and
            // `<snake_name>_service_descriptor`. Architect downstream
            // (serve / layer) refers to the Rpc-suffixed names, so
            // alias them through.
            let vox_dispatcher = format_ident!("{}Dispatcher", trait_name);
            let vox_descriptor = format_ident!(
                "{}_service_descriptor",
                to_snake_case(&trait_name.to_string())
            );
            let rpc_descriptor = format_ident!(
                "{}_rpc_service_descriptor",
                to_snake_case(&trait_name.to_string())
            );
            quote! {
                #[cfg(feature = "vox")]
                #vis use #vox_dispatcher as #rpc_dispatcher_name;
                #[cfg(feature = "vox")]
                #vis use #vox_descriptor as #rpc_descriptor;
            }
        }
        Shape::AllSync | Shape::Mixed => quote! {
            /// Async caller proxy — type alias for the vox-emitted
            /// `<Trait>RpcClient` generated from the hidden mirror.
            #[cfg(feature = "vox")]
            #vis type #client_name = #rpc_client_name;
        },
    };

    // `serve` — the public mount verb. Users write
    // `router.mount(marker::serve(backend, dispatcher))` and never see
    // the hidden bridge struct or the `<T>RpcDispatcher` plumbing.
    let serve_fn = emit_serve_fn(trait_name, &host_name, &rpc_dispatcher_name, vis, shape);

    // `layer` — bundles (descriptor, serve(backend)) into an
    // `architect::Mounted` so callers can compose mounting via
    // `Layer::merge` instead of writing per-service `.with(...)` calls.
    let layer_fn = emit_layer_fn(trait_name, vis, shape);

    // `prelude` — crate-root-ready re-exports with the trait name baked
    // in, so proto crates glob one module per service instead of
    // hand-renaming five items.
    let prelude = emit_prelude(
        trait_name,
        &client_name,
        &rpc_trait_name,
        &rpc_dispatcher_name,
        vis,
        shape,
        &subscriptions,
        args.sync_client,
        args.context.is_some(),
        args.ops.is_some(),
    );

    // Stream sibling — emitted when the trait declares `#[subscribe]`
    // methods: the `<Trait>Stream` vox service, the `<Trait>StreamSource`
    // backend contract, and the stream mount verbs.
    let stream_block = emit_stream_block(trait_name, vis, &subscriptions);

    // Blocking facade over the async client — sync code talking to a
    // remote backend. Opt-in via `#[architect::rpc(sync_client)]`.
    let sync_client = if args.sync_client {
        emit_sync_client(trait_name, &client_name, vis, shape, &methods, ctx)
    } else {
        quote! {}
    };

    // Scoped client — emitted with `context = T`: binds the ambient
    // context once, then every call goes context-free.
    let scoped_client = match ctx {
        Some(ctx_ty) if !matches!(shape, Shape::Empty) => {
            let actual_client = match shape {
                Shape::AllAsync => client_name.clone(),
                _ => rpc_client_name.clone(),
            };
            emit_scoped_client(trait_name, &actual_client, vis, &methods, ctx_ty)
        }
        _ => quote! {},
    };

    // Reified-call enums — emitted with `ops`: `<Trait>Op` (one
    // variant per sync method) + `<Trait>OpOutput` + `apply`.
    let ops_block = match &args.ops {
        Some(substs) => emit_ops_block(trait_name, vis, &methods, ctx, substs)?,
        None => quote! {},
    };

    // Bare module-scope aliases completing the uniform five-name set
    // (`Service` / `layer` / `serve` are already emitted bare):
    // `Dispatcher` and `descriptor`. One `#[rpc]` trait per module is
    // already the rule (the `Service` token collides otherwise), so
    // these can't clash. Proto-crate modules then re-export the whole
    // surface with a single `pub use service::*;`.
    let bare_aliases = match shape {
        Shape::Empty => quote! {},
        _ => {
            let descriptor_fn = format_ident!(
                "{}_rpc_service_descriptor",
                to_snake_case(&trait_name.to_string())
            );
            quote! {
                #[cfg(feature = "vox")]
                #vis use #rpc_dispatcher_name as Dispatcher;
                #[cfg(feature = "vox")]
                #vis use #descriptor_fn as descriptor;
            }
        }
    };

    if !args.scopes.is_empty() && ctx.is_some() {
        return Err(syn::Error::new_spanned(
            &trait_item.ident,
            "#[architect::rpc]: `scopes(...)` and `context = ...` cannot be \
             combined yet — scopes elide leading parameters the trait \
             declares, while context injects one the trait doesn't.",
        ));
    }
    let direct_view = emit_direct_view(&trait_item, vis, trait_name, ctx);
    let scope_views = emit_scope_views(&trait_item, vis, trait_name, &args.scopes)?;

    Ok(quote! {
        #user_trait
        #mirror_trait
        #host_struct
        #host_impl
        #client_alias
        #serve_fn
        #layer_fn
        #stream_block
        #sync_client
        #scoped_client
        #direct_view
        #scope_views
        #ops_block
        #bare_aliases
        #prelude
    })
}

/// The **direct view** — the local/in-process twin of the generated client:
/// a zero-cost wrapper over `&B` where every trait method is an *inherent*
/// method, so a backend implementing many services never needs UFCS to
/// disambiguate colliding names (`Tracks::set_volume` vs
/// `Routing::set_volume`), and callers don't need the trait in scope.
///
/// ```text
/// use daw_proto::TracksDirectExt as _;
/// daw.tracks_direct().set_volume(ctx, track, 0.9)?;   // no UFCS, ever
/// ```
///
/// Sync methods only (the in-process face); async methods keep their trait
/// form. With an ambient `context = T`, the view's methods take `ctx: &T`
/// exactly like the trait's sync face.
fn emit_direct_view(
    trait_item: &ItemTrait,
    vis: &syn::Visibility,
    trait_name: &syn::Ident,
    ctx: Option<&Type>,
) -> TokenStream2 {
    let direct_name = format_ident!("{}Direct", trait_name);
    let ext_name = format_ident!("{}DirectExt", trait_name);
    let accessor = format_ident!("{}_direct", to_snake_case(&trait_name.to_string()));

    let mut methods = Vec::new();
    for item in &trait_item.items {
        let TraitItem::Fn(f) = item else { continue };
        if has_subscribe_attr(f) || f.sig.asyncness.is_some() {
            continue;
        }
        let mut sig = f.sig.clone();
        // Mirror the emitted user trait's sync face: the ambient context
        // rides as a borrowed second parameter.
        if let Some(ctx_ty) = ctx {
            sig.inputs.insert(1, parse_quote! { ctx: &#ctx_ty });
        }
        let name = &sig.ident;
        let docs: Vec<_> = f
            .attrs
            .iter()
            .filter(|a| a.path().is_ident("doc"))
            .collect();
        let arg_idents: Vec<syn::Ident> = sig
            .inputs
            .iter()
            .filter_map(|arg| match arg {
                FnArg::Typed(t) => match &*t.pat {
                    syn::Pat::Ident(p) => Some(p.ident.clone()),
                    _ => None,
                },
                FnArg::Receiver(_) => None,
            })
            .collect();
        methods.push(quote! {
            #(#docs)*
            #[inline]
            #vis #sig {
                <B as #trait_name>::#name(self.0, #(#arg_idents),*)
            }
        });
    }

    if methods.is_empty() {
        return quote! {};
    }

    let direct_doc = format!(
        "Collision-free inherent-method view over a local `{trait_name}`          backend — the in-process twin of the generated client. Obtain via          [`{ext_name}::{accessor}`]."
    );
    let ext_doc = format!(
        "Blanket accessor: `backend.{accessor}()` → [`{direct_name}`]."
    );
    quote! {
        #[doc = #direct_doc]
        #vis struct #direct_name<'a, B: #trait_name + ?Sized>(#vis &'a B);

        impl<'a, B: #trait_name + ?Sized> ::core::clone::Clone for #direct_name<'a, B> {
            fn clone(&self) -> Self {
                Self(self.0)
            }
        }
        impl<'a, B: #trait_name + ?Sized> ::core::marker::Copy for #direct_name<'a, B> {}

        impl<'a, B: #trait_name + ?Sized> #direct_name<'a, B> {
            #(#methods)*
        }

        #[doc = #ext_doc]
        #vis trait #ext_name: #trait_name {
            fn #accessor(&self) -> #direct_name<'_, Self> {
                #direct_name(self)
            }
        }
        impl<B: #trait_name + ?Sized> #ext_name for B {}
    }
}

/// Leveled **scope views** over the direct view (`scopes(project:
/// ProjectContext, track: TrackRef)`): one wrapper per level holding the
/// backend ref plus the scope values bound so far. A method belongs to
/// level N when its leading parameter types equal the first N scope types
/// (matched structurally, by token equality); it is emitted on that level's
/// view with those parameters elided — the view passes its stored values.
///
/// ```text
/// let track = daw.tracks_direct().project(ctx).track(TrackRef::guid(g));
/// track.set_volume(0.9)?;          // project + track ride along
/// track.set_muted(true)?;
/// ```
fn emit_scope_views(
    trait_item: &ItemTrait,
    vis: &syn::Visibility,
    trait_name: &syn::Ident,
    scopes: &[ScopeDecl],
) -> syn::Result<TokenStream2> {
    if scopes.is_empty() {
        return Ok(quote! {});
    }
    let direct_name = format_ident!("{}Direct", trait_name);
    let ty_key = |ty: &Type| quote!(#ty).to_string();
    let scope_keys: Vec<String> = scopes.iter().map(|s| ty_key(&s.ty)).collect();

    // CamelCase view name per level from the scope's declared name.
    let camel = |ident: &syn::Ident| -> String {
        let s = ident.to_string();
        let mut out = String::new();
        let mut up = true;
        for c in s.chars() {
            if c == '_' {
                up = true;
            } else if up {
                out.extend(c.to_uppercase());
                up = false;
            } else {
                out.push(c);
            }
        }
        out
    };
    let view_names: Vec<syn::Ident> = scopes
        .iter()
        .map(|s| format_ident!("{}{}Scope", trait_name, camel(&s.name)))
        .collect();

    // Assign every sync method its scope depth (0 = direct-view only).
    let mut per_level: Vec<Vec<TokenStream2>> = vec![Vec::new(); scopes.len()];
    for item in &trait_item.items {
        let TraitItem::Fn(f) = item else { continue };
        if has_subscribe_attr(f) || f.sig.asyncness.is_some() {
            continue;
        }
        let typed: Vec<&syn::PatType> = f
            .sig
            .inputs
            .iter()
            .filter_map(|a| match a {
                FnArg::Typed(t) => Some(t),
                FnArg::Receiver(_) => None,
            })
            .collect();
        let mut depth = 0usize;
        while depth < scopes.len()
            && depth < typed.len()
            && ty_key(&typed[depth].ty) == scope_keys[depth]
        {
            depth += 1;
        }
        if depth == 0 {
            continue;
        }
        let level = depth - 1;
        let name = &f.sig.ident;
        let docs: Vec<_> = f
            .attrs
            .iter()
            .filter(|a| a.path().is_ident("doc"))
            .collect();
        // Remaining (unelided) parameters + their idents.
        let rest: Vec<&syn::PatType> = typed[depth..].to_vec();
        let rest_idents: Vec<syn::Ident> = rest
            .iter()
            .filter_map(|t| match &*t.pat {
                syn::Pat::Ident(p) => Some(p.ident.clone()),
                _ => None,
            })
            .collect();
        if rest_idents.len() != rest.len() {
            return Err(syn::Error::new_spanned(
                &f.sig,
                "#[architect::rpc(scopes)]: method parameters must be plain \
                 identifiers",
            ));
        }
        let scope_field_names: Vec<&syn::Ident> =
            scopes[..depth].iter().map(|s| &s.name).collect();
        let ret = &f.sig.output;
        per_level[level].push(quote! {
            #(#docs)*
            #[inline]
            #vis fn #name(&self, #(#rest),*) #ret {
                <B as #trait_name>::#name(
                    self.backend,
                    #(::core::clone::Clone::clone(&self.#scope_field_names),)*
                    #(#rest_idents),*
                )
            }
        });
    }

    // Emit each level's view + the constructor from the level below.
    let mut out = TokenStream2::new();
    for (level, view_name) in view_names.iter().enumerate() {
        let held = &scopes[..=level];
        let field_names: Vec<&syn::Ident> = held.iter().map(|s| &s.name).collect();
        let field_tys: Vec<&Type> = held.iter().map(|s| &s.ty).collect();
        let methods = &per_level[level];
        let deeper = scopes.get(level + 1).map(|next| {
            let next_view = &view_names[level + 1];
            let next_name = &next.name;
            let next_ty = &next.ty;
            let doc = format!("Narrow to a specific `{next_name}` scope.");
            quote! {
                #[doc = #doc]
                #vis fn #next_name(&self, #next_name: #next_ty) -> #next_view<'a, B> {
                    #next_view {
                        backend: self.backend,
                        #(#field_names: ::core::clone::Clone::clone(&self.#field_names),)*
                        #next_name,
                    }
                }
            }
        });
        let accessors = held.iter().map(|s| {
            let name = &s.name;
            let ty = &s.ty;
            quote! {
                #[inline]
                #vis fn #name(&self) -> &#ty {
                    &self.#name
                }
            }
        });
        let doc = format!(
            "`{trait_name}` scoped to a bound `{}` — the elided leading \
             parameters ride along on every call. Obtained via the scope \
             chain starting at `{}Direct`.",
            field_names
                .iter()
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join("` + `"),
            trait_name,
        );
        out.extend(quote! {
            #[doc = #doc]
            #vis struct #view_name<'a, B: #trait_name + ?Sized> {
                backend: &'a B,
                #(#field_names: #field_tys,)*
            }

            impl<'a, B: #trait_name + ?Sized> ::core::clone::Clone for #view_name<'a, B> {
                fn clone(&self) -> Self {
                    Self {
                        backend: self.backend,
                        #(#field_names: ::core::clone::Clone::clone(&self.#field_names),)*
                    }
                }
            }

            impl<'a, B: #trait_name + ?Sized> #view_name<'a, B> {
                /// The scoped backend.
                #vis fn backend(&self) -> &'a B {
                    self.backend
                }
                #(#accessors)*
                #deeper
                #(#methods)*
            }
        });
    }

    // Entry: the first scope's constructor hangs off the direct view.
    let first = &scopes[0];
    let first_name = &first.name;
    let first_ty = &first.ty;
    let first_view = &view_names[0];
    let doc = format!("Bind a `{first_name}` scope — the entry to the scope chain.");
    out.extend(quote! {
        impl<'a, B: #trait_name + ?Sized> #direct_name<'a, B> {
            #[doc = #doc]
            #vis fn #first_name(&self, #first_name: #first_ty) -> #first_view<'a, B> {
                #first_view { backend: self.0, #first_name }
            }
        }
    });
    Ok(out)
}

/// Split a syntactically-literal `Result<O, E>` into its parts. This
/// must MATCH vox's own caller-signature rule (vox-macros-core
/// `as_result`): only a path whose last segment is exactly `Result`
/// with two type arguments counts — type aliases (`DawResult<T>`) are
/// opaque to both layers and are treated as plain payload types.
fn as_syntactic_result(ty: &Type) -> Option<(Type, Type)> {
    let Type::Path(p) = ty else { return None };
    let seg = p.path.segments.last()?;
    if seg.ident != "Result" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(args) = &seg.arguments else {
        return None;
    };
    let mut types = args.args.iter().filter_map(|a| match a {
        syn::GenericArgument::Type(t) => Some(t.clone()),
        _ => None,
    });
    let ok = types.next()?;
    let err = types.next()?;
    if types.next().is_some() {
        return None;
    }
    Some((ok, err))
}

/// Emit the `<Trait>SyncClient` blocking facade: one sync method per
/// trait method, each driving the async vox client through a
/// [`BlockingCaller`]. The signatures mirror what vox emits on the
/// async client (rpc.fallible caller-signature rule): `Result<O, E>`
/// methods return `Result<O, VoxError<E>>`, everything else returns
/// `Result<T, VoxError>`.
///
/// Emitted only under `#[architect::rpc(sync_client)]`. Gated on the
/// consumer's `vox` feature and native targets — wasm has no blocking.
/// Requires `architect/sync-client` in the consumer's dependency
/// (compile error names `BlockingCaller` if missing). For in-process
/// backends skip the facade entirely: the user trait is already sync
/// and free.
fn emit_sync_client(
    trait_name: &syn::Ident,
    client_name: &syn::Ident,
    vis: &syn::Visibility,
    shape: Shape,
    methods: &[Method],
    ctx: Option<&Type>,
) -> TokenStream2 {
    if matches!(shape, Shape::Empty) {
        return quote! {};
    }
    let sync_client_name = format_ident!("{}SyncClient", trait_name);
    let ctx_param = ctx.map(|t| quote! { ctx: #t, });
    let ctx_arg = ctx.map(|_| quote! { ctx, });

    let facade_methods = methods.iter().map(|m| {
        let name = &m.decl.sig.ident;
        let inputs = &m.mirror_inputs;
        let arg_idents = &m.arg_idents;
        let docs: Vec<_> = m
            .decl
            .attrs
            .iter()
            .filter(|a| a.path().is_ident("doc"))
            .collect();
        let ret = match &m.return_ty {
            ReturnType::Default => quote! {
                ::core::result::Result<(), ::architect::vox::VoxError>
            },
            ReturnType::Type(_, ty) => match as_syntactic_result(ty) {
                Some((ok, err)) => quote! {
                    ::core::result::Result<#ok, ::architect::vox::VoxError<#err>>
                },
                None => quote! {
                    ::core::result::Result<#ty, ::architect::vox::VoxError>
                },
            },
        };
        // `mirror_inputs` includes the receiver; strip it — the facade
        // method declares its own `&self`.
        let typed_args = inputs.iter().filter(|a| matches!(a, FnArg::Typed(_)));
        quote! {
            #(#docs)*
            pub fn #name(&self, #ctx_param #(#typed_args),*) -> #ret {
                self.caller.block_on(self.inner.#name(#ctx_arg #(#arg_idents),*))
            }
        }
    });

    let doc = format!(
        "Blocking facade over [`{client_name}`]: each call drives the          async client to completion on the [`BlockingCaller`]'s          runtime. For sync code talking to a *remote* backend —          in-process callers use the [`{trait_name}`] trait directly.          Never call from inside an async context.

         [`BlockingCaller`]: ::architect::BlockingCaller"
    );

    quote! {
        #[doc = #doc]
        #[cfg(all(feature = "vox", not(target_arch = "wasm32")))]
        #[derive(Clone)]
        #vis struct #sync_client_name {
            inner: #client_name,
            caller: ::architect::BlockingCaller,
        }

        #[cfg(all(feature = "vox", not(target_arch = "wasm32")))]
        impl #sync_client_name {
            /// Wrap an established async client with a blocking driver.
            #vis fn new(inner: #client_name, caller: ::architect::BlockingCaller) -> Self {
                Self { inner, caller }
            }

            #(#facade_methods)*
        }
    }
}

/// Emit the `prelude` module: every public item this expansion
/// produces, re-exported under names that stay unique after a glob —
/// the module-scoped `Service` / `layer` / `serve` get the trait name
/// baked in (`TracksService`, `tracks_layer`, `tracks_serve`). A proto
/// crate's root then collapses its per-service re-export block to:
///
/// ```ignore
/// pub use service::{bookings::prelude::*, slots::prelude::*};
/// ```
///
/// The vox-only items carry the same `#[cfg(feature = "vox")]` gates
/// as their definitions, so the glob works in non-vox builds too (it
/// just re-exports the bare trait).
#[allow(clippy::too_many_arguments)]
fn emit_prelude(
    trait_name: &syn::Ident,
    client_name: &syn::Ident,
    rpc_trait_name: &syn::Ident,
    rpc_dispatcher_name: &syn::Ident,
    vis: &syn::Visibility,
    shape: Shape,
    subs: &[Subscription],
    sync_client: bool,
    has_context: bool,
    has_ops: bool,
) -> TokenStream2 {
    let snake = to_snake_case(&trait_name.to_string());
    let service_alias = format_ident!("{}Service", trait_name);
    let layer_alias = format_ident!("{snake}_layer");
    let serve_alias = format_ident!("{snake}_serve");
    let descriptor_fn = format_ident!("{snake}_rpc_service_descriptor");

    // Stream-sibling re-exports — names are already trait-prefixed
    // except the module-scoped verbs/token, which get renamed here.
    let stream_items = if subs.is_empty() {
        quote! {}
    } else {
        let source_trait = format_ident!("{}StreamSource", trait_name);
        let stream_trait = format_ident!("{}Stream", trait_name);
        let stream_client = format_ident!("{}StreamClient", trait_name);
        let stream_dispatcher = format_ident!("{}StreamDispatcher", trait_name);
        let stream_service_alias = format_ident!("{}StreamService", trait_name);
        let stream_layer_alias = format_ident!("{snake}_stream_layer");
        let stream_serve_alias = format_ident!("{snake}_stream_serve");
        let stream_descriptor_fn = format_ident!("{snake}_stream_service_descriptor");
        quote! {
            #[cfg(feature = "vox")]
            pub use super::{
                #source_trait, #stream_trait, #stream_client, #stream_dispatcher,
                #stream_descriptor_fn,
                StreamService as #stream_service_alias,
                stream_layer as #stream_layer_alias,
                stream_serve as #stream_serve_alias,
            };
        }
    };

    let scoped_items = if has_context && !matches!(shape, Shape::Empty) {
        let scoped_name = format_ident!("{}ScopedClient", trait_name);
        quote! {
            #[cfg(feature = "vox")]
            pub use super::#scoped_name;
        }
    } else {
        quote! {}
    };

    let sync_items = if sync_client && !matches!(shape, Shape::Empty) {
        let sync_client_name = format_ident!("{}SyncClient", trait_name);
        quote! {
            #[cfg(all(feature = "vox", not(target_arch = "wasm32")))]
            pub use super::#sync_client_name;
        }
    } else {
        quote! {}
    };

    // Op enums are plain data — no vox gate; names are already
    // trait-prefixed so they stay glob-safe.
    let ops_items = if has_ops {
        let op_name = format_ident!("{}Op", trait_name);
        let op_output_name = format_ident!("{}OpOutput", trait_name);
        quote! {
            pub use super::{#op_name, #op_output_name};
        }
    } else {
        quote! {}
    };

    let vox_items = match shape {
        Shape::Empty => quote! {},
        // AllAsync: vox::service decorates the user trait directly, so
        // the client is already `<Trait>Client` and the mirror trait
        // doesn't exist. The dispatcher/descriptor go through the
        // Rpc-suffixed aliases emitted alongside.
        Shape::AllAsync => quote! {
            #[cfg(feature = "vox")]
            pub use super::{
                #client_name, #rpc_dispatcher_name, #descriptor_fn,
                Service as #service_alias,
                layer as #layer_alias,
                serve as #serve_alias,
            };
        },
        Shape::AllSync | Shape::Mixed => quote! {
            #[cfg(feature = "vox")]
            pub use super::{
                #client_name, #rpc_trait_name, #rpc_dispatcher_name, #descriptor_fn,
                Service as #service_alias,
                layer as #layer_alias,
                serve as #serve_alias,
            };
        },
    };

    quote! {
        /// Glob-safe re-exports for this service: the trait itself plus
        /// (with the `vox` feature) the client, dispatcher, descriptor,
        /// and the `Service` token / `layer` / `serve` verbs renamed
        /// with the trait's name baked in. Intended for proto-crate
        /// roots: `pub use my_service::prelude::*;`.
        #vis mod prelude {
            pub use super::#trait_name;
            #vox_items
            #stream_items
            #sync_items
            #scoped_items
            #ops_items
        }
    }
}

/// PascalCase a snake_case identifier — op-enum variant names are the
/// method names recased (`set_muted` → `SetMuted`).
fn to_pascal_case(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut upper_next = true;
    for ch in input.chars() {
        if ch == '_' {
            upper_next = true;
        } else if upper_next {
            out.extend(ch.to_uppercase());
            upper_next = false;
        } else {
            out.push(ch);
        }
    }
    out
}

/// Emit the reified-call surface requested by `#[architect::rpc(ops)]`:
///
/// - `<Trait>Op` — one struct variant per **sync** method, fields =
///   the method's owned wire arguments (async methods don't reify —
///   ops exist to batch/replay synchronous calls).
/// - `<Trait>OpOutput` — one variant per method wrapping its return.
/// - `impl <Trait>Op { fn apply(self, backend) }` — replays the op
///   against any `impl Trait`.
///
/// Substitution pairs (`ops(ProjectContext as ProjectArg)`) swap every
/// parameter of the source type for the target type on the enum, and
/// `apply` grows a `resolver` argument that converts each deferred
/// value back via [`architect::ops::ResolveArg`] before the call —
/// this is what lets a batch step reference the output of an earlier
/// step instead of a literal.
///
/// Both enums derive `Clone, Debug, Facet` (facet via architect's
/// unconditional re-export) so they can ride any vox wire unchanged.
fn emit_ops_block(
    trait_name: &syn::Ident,
    vis: &syn::Visibility,
    methods: &[Method],
    ctx: Option<&Type>,
    substs: &[OpsSubst],
) -> syn::Result<TokenStream2> {
    let sync_methods: Vec<&Method> = methods
        .iter()
        .filter(|m| !m.is_async && !m.ops_skip)
        .collect();
    if sync_methods.is_empty() {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "#[architect::rpc(ops)] requires at least one sync method — \
             ops reify synchronous calls for batching/replay; an \
             all-async trait has nothing to reify.",
        ));
    }

    let op_name = format_ident!("{}Op", trait_name);
    let op_output_name = format_ident!("{}OpOutput", trait_name);

    // Type-token comparison for substitution matching: normalize both
    // sides through `quote` so spacing differences don't matter.
    let subst_for = |ty: &Type| -> Option<&OpsSubst> {
        let ty_str = quote!(#ty).to_string();
        substs.iter().find(|s| {
            let src = &s.src;
            quote!(#src).to_string() == ty_str
        })
    };

    let ctx_field = ctx.map(|t| quote! { ctx: #t, });

    let mut op_variants = Vec::new();
    let mut output_variants = Vec::new();
    let mut apply_arms = Vec::new();

    for m in &sync_methods {
        let method_name = &m.decl.sig.ident;
        let variant = format_ident!("{}", to_pascal_case(&method_name.to_string()));
        let docs: Vec<_> = m
            .decl
            .attrs
            .iter()
            .filter(|a| a.path().is_ident("doc"))
            .collect();

        // Per-arg: field type after substitution + how apply rebuilds
        // the call argument (resolve substituted, `&` re-borrowed).
        let mut fields = Vec::new();
        let mut binds = Vec::new();
        let mut resolves = Vec::new();
        let mut call_args = Vec::new();

        let typed_inputs = m.mirror_inputs.iter().filter_map(|a| match a {
            FnArg::Typed(t) => Some(t),
            FnArg::Receiver(_) => None,
        });
        for (pat_ty, (ident, &was_ref)) in
            typed_inputs.zip(m.arg_idents.iter().zip(m.arg_was_ref.iter()))
        {
            let owned_ty = &*pat_ty.ty;
            if let Some(sub) = subst_for(owned_ty) {
                let dst = &sub.dst;
                let src = &sub.src;
                fields.push(quote! { #ident: #dst });
                resolves.push(quote! {
                    let #ident: #src =
                        ::architect::ops::ResolveArg::resolve_arg(resolver, #ident)?;
                });
            } else {
                fields.push(quote! { #ident: #owned_ty });
            }
            binds.push(quote! { #ident });
            if was_ref {
                call_args.push(quote! { &#ident });
            } else {
                call_args.push(quote! { #ident });
            }
        }

        // Variant shape: unit when field-free (and no ambient ctx).
        let has_fields = !fields.is_empty() || ctx.is_some();
        if has_fields {
            op_variants.push(quote! {
                #(#docs)*
                #variant { #ctx_field #(#fields),* }
            });
        } else {
            op_variants.push(quote! {
                #(#docs)*
                #variant
            });
        }

        // Output variant wraps the method's return; unit for `()`.
        let (out_variant, wrap): (TokenStream2, bool) = match &m.return_ty {
            ReturnType::Default => (quote! { #variant }, false),
            ReturnType::Type(_, ty) => (quote! { #variant(#ty) }, true),
        };
        output_variants.push(out_variant);

        let ctx_bind = ctx.map(|_| quote! { ctx, });
        let ctx_call = ctx.map(|_| quote! { &ctx, });
        let pattern = if has_fields {
            quote! { Self::#variant { #ctx_bind #(#binds),* } }
        } else {
            quote! { Self::#variant }
        };
        let call = quote! { backend.#method_name(#ctx_call #(#call_args),*) };
        // `resolves` is empty when the method has no substituted args
        // (and always, without `ops(...)` pairs) — the block collapses
        // to the bare call.
        let body = if wrap {
            quote! { { #(#resolves)* #op_output_name::#variant(#call) } }
        } else {
            quote! { { #(#resolves)* #call; #op_output_name::#variant } }
        };
        apply_arms.push(quote! { #pattern => #body, });
    }

    // With substitutions, `apply` takes a resolver and is fallible;
    // without, it's infallible and resolver-free.
    let dst_bounds: Vec<TokenStream2> = substs
        .iter()
        .map(|s| {
            let src = &s.src;
            let dst = &s.dst;
            quote! { ::architect::ops::ResolveArg<#dst, #src> }
        })
        .collect();

    let op_doc = format!(
        "Reified call into [`{trait_name}`] — one variant per sync \
         method, carrying the owned wire arguments. Build a program of \
         these, ship it as data, replay with [`{op_name}::apply`]."
    );
    let output_doc = format!(
        "Return value of one applied [`{op_name}`] — one variant per \
         method, wrapping that method's return type."
    );

    let apply_fn = if substs.is_empty() {
        quote! {
            /// Replay this op against a backend.
            #vis fn apply<__B>(self, backend: &__B) -> #op_output_name
            where
                __B: #trait_name + ?::core::marker::Sized,
            {
                match self { #(#apply_arms)* }
            }
        }
    } else {
        quote! {
            /// Replay this op against a backend, resolving deferred
            /// arguments through `resolver` first. Fails only if the
            /// resolver rejects a deferred value.
            #vis fn apply<__B, __R>(
                self,
                backend: &__B,
                resolver: &__R,
            ) -> ::core::result::Result<
                #op_output_name,
                <__R as ::architect::ops::OpResolver>::Error,
            >
            where
                __B: #trait_name + ?::core::marker::Sized,
                __R: ::architect::ops::OpResolver #(+ #dst_bounds)*,
            {
                Ok(match self { #(#apply_arms)* })
            }
        }
    };

    Ok(quote! {
        #[doc = #op_doc]
        #[repr(u8)]
        #[derive(Clone, Debug, ::architect::facet::Facet)]
        #vis enum #op_name {
            #(#op_variants),*
        }

        #[doc = #output_doc]
        #[repr(u8)]
        #[derive(Clone, Debug, ::architect::facet::Facet)]
        #vis enum #op_output_name {
            #(#output_variants),*
        }

        impl #op_name {
            #apply_fn
        }
    })
}

/// Snake-case an UpperCamelCase identifier. Mirrors what `#[vox::service]`
/// does when it derives `<snake_name>_service_descriptor` from the
/// trait's name, so we can refer to that emitted function by name from
/// inside the `layer()` body.
fn to_snake_case(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 4);
    let mut prev_lower = false;
    for ch in input.chars() {
        if ch.is_uppercase() {
            if prev_lower {
                out.push('_');
            }
            for lc in ch.to_lowercase() {
                out.push(lc);
            }
            prev_lower = false;
        } else {
            out.push(ch);
            prev_lower = ch.is_alphanumeric();
        }
    }
    out
}

/// Emit:
///
/// - `pub fn layer(backend) -> architect::Mounted` — immediate-bind
///   shortcut. Useful when a caller already has the backend and just
///   wants a bolt-on `Mounted` to drop into someone else's `Layer`.
/// - `pub struct Service;` + `impl architect::Bind<B> for Service` —
///   the deferred-bind token. Architectures compose these into
///   `architect::Layer<B>` and bind once at `.provide(B)` time.
fn emit_layer_fn(trait_name: &syn::Ident, vis: &syn::Visibility, shape: Shape) -> TokenStream2 {
    let descriptor_fn = format_ident!(
        "{}_rpc_service_descriptor",
        to_snake_case(&trait_name.to_string())
    );

    let (bounds, immediate_doc) = match shape {
        Shape::Empty => return quote! {},
        Shape::AllAsync => (
            quote! {
                S: #trait_name + ::architect::MaybeSendSync + 'static,
            },
            "Async-native trait — no `HasDispatcher` requirement.",
        ),
        Shape::AllSync | Shape::Mixed => (
            quote! {
                S: #trait_name
                    + ::architect::HasDispatcher
                    + ::architect::MaybeSendSync
                    + 'static,
            },
            "Sync/mixed trait — backend must implement `HasDispatcher`.",
        ),
    };

    quote! {
        /// Immediate-bind shortcut: wrap a backend in this service's
        /// vox dispatcher and return a `Mounted`. For deferred
        /// binding, prefer `Service` + [`architect::Layer::merge`] +
        /// [`architect::Layer::provide`].
        ///
        #[doc = #immediate_doc]
        #[cfg(feature = "vox")]
        #vis fn layer<S>(backend: S) -> ::architect::Mounted
        where
            #bounds
        {
            ::architect::Mounted::new(#descriptor_fn(), serve(backend))
        }

        /// Deferred-bind service token. Acts as a one-element
        /// [`architect::Layer`]; compose with [`architect::Layer::merge`]
        /// and bind a backend at [`architect::Layer::provide`] time.
        #[cfg(feature = "vox")]
        #[derive(Debug, Default, Clone, Copy)]
        #vis struct Service;

        #[cfg(feature = "vox")]
        impl ::architect::BindAny for Service {
            fn descriptor(&self) -> &'static ::architect::vox::ServiceDescriptor {
                #descriptor_fn()
            }
        }

        #[cfg(feature = "vox")]
        impl<S> ::architect::Bind<S> for Service
        where
            S: ::core::clone::Clone,
            #bounds
        {
            fn bind_into(
                self,
                backend: &S,
                router: &mut ::architect::LayerRouter,
            ) {
                use ::architect::LayerSink as _;
                router.add_mounted(::architect::Mounted::new(
                    #descriptor_fn(),
                    serve(backend.clone()),
                ));
            }
        }

        // ─ Composition impls ───────────────────────────────────────
        // `Layer<B>` itself is blanket-implemented by architect for
        // any `T: Bind<B> + Descriptors + Sized`, so the Service
        // token gets it for free once we emit Bind / Descriptors.
        // We still need `Append<R>` (the structural cons-concat)
        // emitted per-token so `layers![a, b]` builds a chain.

        #[cfg(feature = "vox")]
        impl<R> ::architect::Append<R> for Service {
            type Output = ::architect::Cons<Service, R>;
            fn append(self, rhs: R) -> Self::Output {
                ::architect::Cons::new(self, rhs)
            }
        }

        #[cfg(feature = "vox")]
        impl ::architect::Descriptors for Service {
            fn collect(
                &self,
                out: &mut ::std::vec::Vec<&'static ::architect::vox::ServiceDescriptor>,
            ) {
                out.push(::architect::BindAny::descriptor(self));
            }
        }

    }
}

/// Emit the `serve` free function — the public mount verb.
///
/// Sync / mixed: takes a backend + dispatcher, builds the hidden bridge,
/// wraps it in the vox-emitted dispatcher, returns the mountable.
///
/// All-async: takes just a backend (the trait is its own RPC face).
fn emit_serve_fn(
    trait_name: &syn::Ident,
    host_name: &syn::Ident,
    rpc_dispatcher_name: &syn::Ident,
    vis: &syn::Visibility,
    shape: Shape,
) -> TokenStream2 {
    match shape {
        Shape::Empty => quote! {},
        Shape::AllAsync => quote! {
            /// Wrap a backend in the vox-emitted dispatcher so it can be
            /// mounted on a vox router. The trait is async-native, so no
            /// thread dispatcher is required.
            #[cfg(feature = "vox")]
            #vis fn serve<S>(backend: S) -> #rpc_dispatcher_name<#host_name<S>>
            where
                S: #trait_name + ::architect::MaybeSendSync + 'static,
            {
                #rpc_dispatcher_name::new(#host_name::new(backend))
            }
        },
        Shape::AllSync | Shape::Mixed => quote! {
            /// Wrap a backend in the vox-emitted dispatcher so it can
            /// be mounted on a vox router. Each call to a sync trait
            /// method is marshaled through the backend's dispatcher
            /// (pulled via `HasDispatcher`) to the thread where the
            /// backend's work runs.
            #[cfg(feature = "vox")]
            #vis fn serve<S>(
                backend: S,
            ) -> #rpc_dispatcher_name<#host_name<S, <S as ::architect::HasDispatcher>::Dispatcher>>
            where
                S: #trait_name
                    + ::architect::HasDispatcher
                    + ::architect::MaybeSendSync
                    + 'static,
            {
                let dispatcher = ::architect::HasDispatcher::dispatcher(&backend);
                #rpc_dispatcher_name::new(#host_name::new(backend, dispatcher))
            }
        },
    }
}

// ── Subscription classification + emission ─────────────────────────────

/// One `#[subscribe]` declaration: `fn name(&self, filters..) -> EventTy`.
#[derive(Debug)]
struct Subscription {
    name: syn::Ident,
    event_ty: Type,
    /// Filter params in owned form (what crosses the wire), excluding
    /// the receiver.
    mirror_inputs: Vec<FnArg>,
    arg_idents: Vec<syn::Ident>,
    arg_was_ref: Vec<bool>,
    docs: Vec<syn::Attribute>,
}

fn has_subscribe_attr(method: &TraitItemFn) -> bool {
    method.attrs.iter().any(|a| a.path().is_ident("subscribe"))
}

/// `#[ops(skip)]` on a trait method — recognized (and stripped) by the
/// rpc macro so the method stays callable but is not reified into the
/// `<Trait>Op` enum.
fn has_ops_skip_attr(method: &TraitItemFn) -> bool {
    method.attrs.iter().any(|a| {
        a.path().is_ident("ops")
            && a.parse_args::<syn::Ident>()
                .map(|id| id == "skip")
                .unwrap_or(false)
    })
}

/// Strip `#[ops(...)]` marker attributes before re-emitting a method —
/// they are directives to this macro, not real attributes.
fn strip_ops_attrs(method: &mut TraitItemFn) {
    method.attrs.retain(|a| !a.path().is_ident("ops"));
}

fn classify_subscription(method: &TraitItemFn) -> syn::Result<Subscription> {
    if method.sig.asyncness.is_some() {
        return Err(syn::Error::new_spanned(
            &method.sig,
            "#[subscribe] declarations are sync markers — write \
             `fn events(&self) -> EventType;`, not `async fn`. The async \
             subscribe RPC is emitted on the `<Trait>Stream` sibling.",
        ));
    }
    let event_ty = match &method.sig.output {
        ReturnType::Type(_, ty) => (**ty).clone(),
        ReturnType::Default => {
            return Err(syn::Error::new_spanned(
                &method.sig,
                "#[subscribe] declarations name their event type as the \
                 return type: `fn events(&self) -> EventType;`.",
            ));
        }
    };
    match method.sig.inputs.first() {
        Some(FnArg::Receiver(rec)) if rec.reference.is_some() && rec.mutability.is_none() => {}
        _ => {
            return Err(syn::Error::new_spanned(
                &method.sig,
                "#[subscribe] declarations must take `&self` as the first \
                 parameter.",
            ));
        }
    }

    let mut mirror_inputs = Vec::new();
    let mut arg_idents = Vec::new();
    let mut arg_was_ref = Vec::new();
    for (i, input) in method.sig.inputs.iter().enumerate() {
        if let FnArg::Typed(pat_ty) = input {
            let ident = match &*pat_ty.pat {
                Pat::Ident(PatIdent { ident, .. }) => ident.clone(),
                _ => format_ident!("__arg{i}"),
            };
            let (owned_ty, was_ref) = owned_form(&pat_ty.ty);
            let mut mirror_pat = pat_ty.clone();
            mirror_pat.ty = Box::new(owned_ty);
            mirror_pat.pat = Box::new(parse_quote! { #ident });
            mirror_inputs.push(FnArg::Typed(mirror_pat));
            arg_idents.push(ident);
            arg_was_ref.push(was_ref);
        }
    }

    let docs = method
        .attrs
        .iter()
        .filter(|a| a.path().is_ident("doc"))
        .cloned()
        .collect();

    Ok(Subscription {
        name: method.sig.ident.clone(),
        event_ty,
        mirror_inputs,
        arg_idents,
        arg_was_ref,
        docs,
    })
}

/// Emit the stream sibling for a trait with `#[subscribe]` declarations.
///
/// For `#[architect::rpc] trait Fx { #[subscribe] fn events(&self) -> FxEvent; }`:
///
/// - `FxStreamSource` — the backend contract. Parameterless
///   subscriptions ask for a hub accessor (`fn events_hub(&self) ->
///   &PubSub<FxEvent>`); the bridge attaches sinks to it, and the
///   backend publishes into it. Subscriptions **with** filter params
///   ask for an attach method (`fn events_attach(&self, args.., sink)`)
///   so the backend owns the filtering.
/// - `FxStream` — the `#[vox::service]` sibling: one
///   `async fn events(&self, args.., sink: Tx<FxEvent>)` per
///   declaration. Vox emits `FxStreamClient` / `FxStreamDispatcher` /
///   `fx_stream_service_descriptor` from it.
/// - `__FxStreamHost<S>` — hidden adapter implementing `FxStream` over
///   any `S: FxStreamSource`.
/// - `stream_serve` / `stream_layer` / `StreamService` — mount verbs,
///   parallel to the base trait's `serve` / `layer` / `Service`.
///
/// Snapshot-then-changes: construct the hub with
/// `PubSub::sliding(n).with_replay(1)` and publish full-state events —
/// late subscribers get the last event on attach. Subscriptions that
/// need a real snapshot read (the Entity-events pattern) use a filter
/// param + `begin_attach`/`complete_attach` inside their `_attach` impl.
fn emit_stream_block(
    trait_name: &syn::Ident,
    vis: &syn::Visibility,
    subs: &[Subscription],
) -> TokenStream2 {
    if subs.is_empty() {
        return quote! {};
    }

    let snake = to_snake_case(&trait_name.to_string());
    let source_trait = format_ident!("{}StreamSource", trait_name);
    let stream_trait = format_ident!("{}Stream", trait_name);
    let stream_host = format_ident!("__{}StreamHost", trait_name);
    let stream_dispatcher = format_ident!("{}StreamDispatcher", trait_name);
    let stream_descriptor_fn = format_ident!("{snake}_stream_service_descriptor");

    // Backend contract methods.
    let source_methods = subs.iter().map(|s| {
        let ev = &s.event_ty;
        let docs = &s.docs;
        if s.arg_idents.is_empty() {
            let hub_fn = format_ident!("{}_hub", s.name);
            let doc = format!(
                "Fan-out hub backing the `{}` subscription. Construct it \
                 once on the backend (e.g. `PubSub::sliding(256)`, add \
                 `.with_replay(1)` for state-shaped events) and publish \
                 into it; the stream host attaches every subscriber sink.",
                s.name
            );
            quote! {
                #(#docs)*
                #[doc = #doc]
                fn #hub_fn(&self) -> &::architect::PubSub<#ev>;
            }
        } else {
            let attach_fn = format_ident!("{}_attach", s.name);
            let inputs = &s.mirror_inputs;
            let doc = format!(
                "Attach `sink` to the filtered `{}` feed. The backend owns \
                 the filter semantics — typically a per-filter `PubSub`, or \
                 `begin_attach`/`complete_attach` for snapshot-then-changes.",
                s.name
            );
            quote! {
                #(#docs)*
                #[doc = #doc]
                fn #attach_fn(&self, #(#inputs,)* sink: ::architect::vox::Tx<#ev>);
            }
        }
    });

    // Sibling vox-service methods.
    let stream_methods = subs.iter().map(|s| {
        let name = &s.name;
        let ev = &s.event_ty;
        let inputs = &s.mirror_inputs;
        let docs = &s.docs;
        quote! {
            #(#docs)*
            async fn #name(&self, #(#inputs,)* sink: ::architect::vox::Tx<#ev>);
        }
    });

    // Host adapter bodies.
    let host_methods = subs.iter().map(|s| {
        let name = &s.name;
        let ev = &s.event_ty;
        let inputs = &s.mirror_inputs;
        if s.arg_idents.is_empty() {
            let hub_fn = format_ident!("{}_hub", s.name);
            quote! {
                async fn #name(&self, sink: ::architect::vox::Tx<#ev>) {
                    self.inner.#hub_fn().attach(sink);
                    // vox scopes channels to their request: delivering the
                    // response terminates the sink. Hold the request open for
                    // the life of the subscription — the client unsubscribes
                    // by cancelling the call (dropping its future), which
                    // drops this handler and closes the channel.
                    ::core::future::pending::<()>().await;
                }
            }
        } else {
            let attach_fn = format_ident!("{}_attach", s.name);
            let call_args = s
                .arg_idents
                .iter()
                .zip(s.arg_was_ref.iter())
                .map(|(id, &was_ref)| {
                    if was_ref {
                        quote! { &#id }
                    } else {
                        quote! { #id }
                    }
                });
            quote! {
                async fn #name(&self, #(#inputs,)* sink: ::architect::vox::Tx<#ev>) {
                    self.inner.#attach_fn(#(#call_args,)* sink);
                    // Hold the request open — see the parameterless variant.
                    ::core::future::pending::<()>().await;
                }
            }
        }
    });

    let source_doc = format!(
        "Backend contract for [`{trait_name}`]'s `#[subscribe]` streams. \
         Implement on the backend alongside `{trait_name}`; mount with \
         `stream_layer` / `StreamService` next to the base service."
    );
    let stream_doc = format!(
        "Streaming sibling of [`{trait_name}`]: pass a channel sink in, \
         receive events on it until the connection closes. Clients use \
         `{trait_name}StreamClient` (see `architect::use_stream`)."
    );

    quote! {
        #[doc = #source_doc]
        #[cfg(feature = "vox")]
        #vis trait #source_trait {
            #(#source_methods)*
        }

        #[doc = #stream_doc]
        #[cfg(feature = "vox")]
        #[::architect::vox::service]
        #vis trait #stream_trait {
            #(#stream_methods)*
        }

        /// Internal adapter: implements the stream sibling over any
        /// backend that fulfills the `StreamSource` contract. Hidden —
        /// mount via `stream_serve` / `stream_layer`.
        #[doc(hidden)]
        #[cfg(feature = "vox")]
        #vis struct #stream_host<S>
        where
            S: #source_trait + ::architect::MaybeSendSync + 'static,
        {
            inner: ::std::sync::Arc<S>,
        }

        #[cfg(feature = "vox")]
        impl<S> #stream_host<S>
        where
            S: #source_trait + ::architect::MaybeSendSync + 'static,
        {
            pub fn new(inner: S) -> Self {
                Self { inner: ::std::sync::Arc::new(inner) }
            }
        }

        #[cfg(feature = "vox")]
        impl<S> ::core::clone::Clone for #stream_host<S>
        where
            S: #source_trait + ::architect::MaybeSendSync + 'static,
        {
            fn clone(&self) -> Self {
                Self { inner: ::std::sync::Arc::clone(&self.inner) }
            }
        }

        #[cfg(feature = "vox")]
        impl<S> #stream_trait for #stream_host<S>
        where
            S: #source_trait + ::architect::MaybeSendSync + 'static,
        {
            #(#host_methods)*
        }

        /// Wrap a backend's stream source in the vox-emitted dispatcher
        /// so the subscription service can be mounted on a vox router.
        #[cfg(feature = "vox")]
        #vis fn stream_serve<S>(backend: S) -> #stream_dispatcher<#stream_host<S>>
        where
            S: #source_trait + ::architect::MaybeSendSync + 'static,
        {
            #stream_dispatcher::new(#stream_host::new(backend))
        }

        /// Immediate-bind shortcut for the stream sibling — parallel to
        /// `layer` for the base service.
        #[cfg(feature = "vox")]
        #vis fn stream_layer<S>(backend: S) -> ::architect::Mounted
        where
            S: #source_trait + ::architect::MaybeSendSync + 'static,
        {
            ::architect::Mounted::new(#stream_descriptor_fn(), stream_serve(backend))
        }

        /// Deferred-bind token for the stream sibling — slots into
        /// `layers![Service, StreamService]` next to the base token.
        #[cfg(feature = "vox")]
        #[derive(Debug, Default, Clone, Copy)]
        #vis struct StreamService;

        #[cfg(feature = "vox")]
        impl ::architect::BindAny for StreamService {
            fn descriptor(&self) -> &'static ::architect::vox::ServiceDescriptor {
                #stream_descriptor_fn()
            }
        }

        #[cfg(feature = "vox")]
        impl<S> ::architect::Bind<S> for StreamService
        where
            S: ::core::clone::Clone
                + #source_trait
                + ::architect::MaybeSendSync
                + 'static,
        {
            fn bind_into(
                self,
                backend: &S,
                router: &mut ::architect::LayerRouter,
            ) {
                use ::architect::LayerSink as _;
                router.add_mounted(::architect::Mounted::new(
                    #stream_descriptor_fn(),
                    stream_serve(backend.clone()),
                ));
            }
        }

        #[cfg(feature = "vox")]
        impl<R> ::architect::Append<R> for StreamService {
            type Output = ::architect::Cons<StreamService, R>;
            fn append(self, rhs: R) -> Self::Output {
                ::architect::Cons::new(self, rhs)
            }
        }

        #[cfg(feature = "vox")]
        impl ::architect::Descriptors for StreamService {
            fn collect(
                &self,
                out: &mut ::std::vec::Vec<&'static ::architect::vox::ServiceDescriptor>,
            ) {
                out.push(::architect::BindAny::descriptor(self));
            }
        }
    }
}

/// Emit the `<Trait>ScopedClient` for `context = T` traits: binds the
/// ambient context once, then exposes every method context-free. The
/// raw vox client (which takes `ctx` as its first wire argument on
/// every call) stays available for callers juggling several scopes.
fn emit_scoped_client(
    trait_name: &syn::Ident,
    actual_client: &syn::Ident,
    vis: &syn::Visibility,
    methods: &[Method],
    ctx_ty: &Type,
) -> TokenStream2 {
    let scoped_name = format_ident!("{}ScopedClient", trait_name);

    let scoped_methods = methods.iter().map(|m| {
        let name = &m.decl.sig.ident;
        let arg_idents = &m.arg_idents;
        let docs: Vec<_> = m
            .decl
            .attrs
            .iter()
            .filter(|a| a.path().is_ident("doc"))
            .collect();
        let ret = match &m.return_ty {
            ReturnType::Default => quote! {
                ::core::result::Result<(), ::architect::vox::VoxError>
            },
            ReturnType::Type(_, ty) => match as_syntactic_result(ty) {
                Some((ok, err)) => quote! {
                    ::core::result::Result<#ok, ::architect::vox::VoxError<#err>>
                },
                None => quote! {
                    ::core::result::Result<#ty, ::architect::vox::VoxError>
                },
            },
        };
        let typed_args = m
            .mirror_inputs
            .iter()
            .filter(|a| matches!(a, FnArg::Typed(_)));
        quote! {
            #(#docs)*
            pub async fn #name(&self, #(#typed_args),*) -> #ret {
                self.inner
                    .#name(::core::clone::Clone::clone(&self.ctx), #(#arg_idents),*)
                    .await
            }
        }
    });

    let doc = format!(
        "Context-bound view of the `{trait_name}` client: the ambient          context set by `.scoped(ctx)` rides along on every call, so          call sites stop threading it manually. Cheap to re-scope —          `with_ctx` returns a sibling bound to a different context."
    );

    quote! {
        #[cfg(feature = "vox")]
        impl #actual_client {
            /// Bind an ambient context to this client — every call on
            /// the returned scoped client carries it automatically.
            #vis fn scoped(self, ctx: #ctx_ty) -> #scoped_name {
                #scoped_name { inner: self, ctx }
            }
        }

        #[doc = #doc]
        #[cfg(feature = "vox")]
        #[derive(Clone)]
        #vis struct #scoped_name {
            inner: #actual_client,
            ctx: #ctx_ty,
        }

        #[cfg(feature = "vox")]
        impl #scoped_name {
            /// The bound context.
            #vis fn ctx(&self) -> &#ctx_ty {
                &self.ctx
            }

            /// Rebind to a different context, keeping the connection.
            #vis fn with_ctx(mut self, ctx: #ctx_ty) -> Self {
                self.ctx = ctx;
                self
            }

            #(#scoped_methods)*
        }
    }
}

// ── Method classification ──────────────────────────────────────────────

#[derive(Debug)]
struct Method {
    decl: TraitItemFn,
    is_async: bool,
    /// Owned (non-reference) parameter list rebuilt for the async
    /// mirror trait. Borrowed args (`&str`, etc.) get rewritten here
    /// to owned types so the bridge's closures can capture them.
    mirror_inputs: Vec<FnArg>,
    /// Argument idents in order — needed when emitting the call from
    /// the bridge body back into the sync trait.
    arg_idents: Vec<syn::Ident>,
    /// Whether each argument was originally a reference (so the
    /// bridge passes `&owned` when calling back into the sync trait).
    arg_was_ref: Vec<bool>,
    return_ty: ReturnType,
    /// `#[ops(skip)]` — excluded from the `<Trait>Op` reified-call
    /// enum (e.g. a return type phon cannot lower, like a tuple).
    ops_skip: bool,
}

fn classify_method(method: &TraitItemFn) -> syn::Result<Method> {
    // Reject what the bridge can't currently express.
    if !method.sig.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &method.sig,
            "#[architect::rpc] cannot expose methods with generic \
             type or const parameters — the trait must be object-safe.",
        ));
    }

    if let Some(FnArg::Receiver(rec)) = method.sig.inputs.first() {
        if rec.mutability.is_some() {
            return Err(syn::Error::new_spanned(
                rec,
                "#[architect::rpc] methods must take `&self`, not `&mut self`. \
                 Use interior mutability in the backend if needed.",
            ));
        }
        if rec.reference.is_none() {
            return Err(syn::Error::new_spanned(
                rec,
                "#[architect::rpc] methods must take `&self` — `self` and \
                 `mut self` consume the backend and aren't dyn-callable.",
            ));
        }
    } else {
        return Err(syn::Error::new_spanned(
            &method.sig,
            "#[architect::rpc] methods must take `&self` as the first parameter.",
        ));
    }

    if let ReturnType::Type(_, ty) = &method.sig.output {
        if matches!(&**ty, Type::Reference(_)) {
            return Err(syn::Error::new_spanned(
                ty,
                "#[architect::rpc] methods must return owned values — \
                 borrowed returns cannot cross the bridge or the wire.",
            ));
        }
        if let Type::Path(p) = &**ty
            && p.path.is_ident("Self")
        {
            return Err(syn::Error::new_spanned(
                ty,
                "#[architect::rpc] methods cannot return `Self` — the \
                 trait must be object-safe.",
            ));
        }
    }

    let is_async = method.sig.asyncness.is_some();

    // Walk inputs, build mirror-side inputs (owned) and the per-arg
    // info the bridge body needs.
    let mut mirror_inputs = Vec::new();
    let mut arg_idents = Vec::new();
    let mut arg_was_ref = Vec::new();

    for (i, input) in method.sig.inputs.iter().enumerate() {
        match input {
            FnArg::Receiver(_) => mirror_inputs.push(input.clone()),
            FnArg::Typed(pat_ty) => {
                let ident = match &*pat_ty.pat {
                    Pat::Ident(PatIdent { ident, .. }) => ident.clone(),
                    _ => format_ident!("__arg{i}"),
                };
                let (owned_ty, was_ref) = owned_form(&pat_ty.ty);
                let mut mirror_pat = pat_ty.clone();
                mirror_pat.ty = Box::new(owned_ty);
                mirror_pat.pat = Box::new(parse_quote! { #ident });
                mirror_inputs.push(FnArg::Typed(mirror_pat));
                arg_idents.push(ident);
                arg_was_ref.push(was_ref);
            }
        }
    }

    let ops_skip = has_ops_skip_attr(method);
    let mut decl = method.clone();
    strip_ops_attrs(&mut decl);

    Ok(Method {
        decl,
        is_async,
        mirror_inputs,
        arg_idents,
        arg_was_ref,
        return_ty: method.sig.output.clone(),
        ops_skip,
    })
}

/// Convert a parameter type to its owned `'static` form, returning
/// whether a reference was unwrapped (so the bridge knows to pass
/// `&owned` when calling the sync trait).
fn owned_form(ty: &Type) -> (Type, bool) {
    if let Type::Reference(ty_ref) = ty {
        let inner = &*ty_ref.elem;
        // `&str` → `String`
        if let Type::Path(p) = inner
            && p.path.is_ident("str")
        {
            return (parse_quote! { ::std::string::String }, true);
        }
        // `&[T]` → `Vec<T>`
        if let Type::Slice(slice) = inner {
            let elem = &slice.elem;
            return (parse_quote! { ::std::vec::Vec<#elem> }, true);
        }
        // Other `&T` — clone the inner type. Caller must impl Clone.
        return (inner.clone(), true);
    }
    (ty.clone(), false)
}

// ── Trait shape classification ─────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
enum Shape {
    Empty,
    AllSync,
    AllAsync,
    Mixed,
}

fn classify_shape(methods: &[Method]) -> Shape {
    if methods.is_empty() {
        return Shape::Empty;
    }
    let async_count = methods.iter().filter(|m| m.is_async).count();
    let sync_count = methods.len() - async_count;
    match (sync_count, async_count) {
        (0, _) => Shape::AllAsync,
        (_, 0) => Shape::AllSync,
        _ => Shape::Mixed,
    }
}

// ── Emission ───────────────────────────────────────────────────────────

/// Re-emit the user's trait, rewriting any `async fn` to
/// `fn -> impl Future<Output = R> + Send` so backends promise
/// `Send` futures. The bridge calls `self.inner.method(...).await`
/// in a context that requires Send (the mirror's `async fn`
/// expands via vox::service to a Send-bounded future), so the
/// inner trait must guarantee Send too.
///
/// Sync methods are re-emitted verbatim. `Send + Sync + 'static`
/// bounds live on the bridge's where-clauses (not the trait
/// itself) so the trait remains impl-able by borrowed-view types.
fn emit_user_trait(trait_item: &ItemTrait, shape: Shape, ctx: Option<&Type>) -> TokenStream2 {
    // AllAsync: vox::service is applied to the user trait directly
    // (no hidden mirror), so leave `async fn` intact — vox's parser
    // rewrites it to `fn -> impl Future + MaybeSend` itself. Sync/
    // Mixed shapes go through the architect bridge, where we rewrite
    // async signatures to `fn -> impl Future + Send` so the bridge's
    // `.await` on the inner method satisfies the mirror's Send bound.
    let apply_vox = matches!(shape, Shape::AllAsync);
    let mut out = trait_item.clone();
    // Drop `#[subscribe]` declarations — they exist only to drive the
    // stream-sibling emission, never as callable trait methods.
    out.items.retain(|item| match item {
        TraitItem::Fn(f) => !has_subscribe_attr(f),
        _ => true,
    });
    for item in &mut out.items {
        if let TraitItem::Fn(f) = item {
            strip_ops_attrs(f);
        }
    }
    // Ambient context: declared once on the attribute, threaded here
    // into every method — sync methods borrow it (`ctx: &T`, direct
    // callers pay nothing), async methods own it (they're the wire
    // face for AllAsync, and futures can't borrow from the bridge).
    if let Some(ctx_ty) = ctx {
        for item in &mut out.items {
            if let TraitItem::Fn(f) = item {
                let arg: FnArg = if f.sig.asyncness.is_some() || apply_vox {
                    parse_quote! { ctx: #ctx_ty }
                } else {
                    parse_quote! { ctx: &#ctx_ty }
                };
                f.sig.inputs.insert(1, arg);
            }
        }
    }
    if !apply_vox {
        for item in &mut out.items {
            if let syn::TraitItem::Fn(f) = item
                && f.sig.asyncness.is_some()
            {
                f.sig.asyncness = None;
                let ret_ty: Type = match &f.sig.output {
                    ReturnType::Default => parse_quote! { () },
                    ReturnType::Type(_, ty) => (**ty).clone(),
                };
                f.sig.output = parse_quote! {
                    -> impl ::core::future::Future<Output = #ret_ty> + ::architect::MaybeSend
                };
            }
        }
    }
    if apply_vox {
        quote! {
            #[cfg_attr(feature = "vox", ::architect::vox::service)]
            #out
        }
    } else {
        quote! { #out }
    }
}

/// Hidden mirror trait — the vox-served async surface. Each user
/// method becomes an `async fn` with owned arguments.
fn emit_mirror_trait(
    rpc_trait_name: &syn::Ident,
    vis: &syn::Visibility,
    methods: &[Method],
    ctx: Option<&Type>,
) -> TokenStream2 {
    let ctx_param = ctx.map(|t| quote! { ctx: #t, });
    let methods_iter = methods.iter().map(|m| {
        let name = &m.decl.sig.ident;
        let mut inputs = m.mirror_inputs.iter();
        let receiver = inputs.next();
        let inputs: Vec<_> = inputs.collect();
        let output = &m.return_ty;
        // Strip doc comments to keep the hidden trait clean. The
        // ambient context travels as the first wire argument.
        quote! {
            async fn #name(#receiver, #ctx_param #(#inputs),*) #output;
        }
    });

    // No supertraits on the mirror — `#[vox::service]` is picky about
    // the trait shape it accepts (no `:` bounds in its parser). Auto
    // trait bounds on the generated client/dispatcher come from vox's
    // own emission; the bridge's impl block carries the Send/Sync
    // bounds where they're actually needed.
    quote! {
        #[doc(hidden)]
        #[cfg_attr(feature = "vox", ::architect::vox::service)]
        #vis trait #rpc_trait_name {
            #(#methods_iter)*
        }
    }
}

/// Bridge struct for sync / mixed traits.
fn emit_bridge_host(
    trait_name: &syn::Ident,
    host_name: &syn::Ident,
    vis: &syn::Visibility,
) -> TokenStream2 {
    quote! {
        /// Internal bridge: holds a backend + dispatcher and implements
        /// the auto-emitted `<T>Rpc` async mirror by marshaling sync
        /// methods through the dispatcher. Hidden from documentation
        /// and users — the public mount surface is the `serve`
        /// function emitted alongside.
        #[doc(hidden)]
        #vis struct #host_name<S, D>
        where
            S: #trait_name + ::architect::MaybeSendSync + 'static,
            D: ::architect::dispatch::Dispatcher,
        {
            inner: ::std::sync::Arc<S>,
            dispatcher: ::std::sync::Arc<D>,
        }

        impl<S, D> #host_name<S, D>
        where
            S: #trait_name + ::architect::MaybeSendSync + 'static,
            D: ::architect::dispatch::Dispatcher,
        {
            /// Wrap a backend impl with a dispatcher. Both are kept
            /// behind `Arc` so the host is cheap to clone.
            pub fn new(inner: S, dispatcher: D) -> Self {
                Self {
                    inner: ::std::sync::Arc::new(inner),
                    dispatcher: ::std::sync::Arc::new(dispatcher),
                }
            }

            /// Wrap pre-arc'd dependencies — useful when the backend or
            /// dispatcher is already shared across multiple hosts.
            pub fn from_arc(
                inner: ::std::sync::Arc<S>,
                dispatcher: ::std::sync::Arc<D>,
            ) -> Self {
                Self { inner, dispatcher }
            }
        }

        impl<S, D> ::core::clone::Clone for #host_name<S, D>
        where
            S: #trait_name + ::architect::MaybeSendSync + 'static,
            D: ::architect::dispatch::Dispatcher,
        {
            fn clone(&self) -> Self {
                Self {
                    inner: ::std::sync::Arc::clone(&self.inner),
                    dispatcher: ::std::sync::Arc::clone(&self.dispatcher),
                }
            }
        }
    }
}

/// Bridge impl: implement the mirror trait on the host by marshaling
/// each sync method through `Dispatcher::dispatch` and passing async
/// methods through directly.
fn emit_bridge_impl(
    trait_name: &syn::Ident,
    host_name: &syn::Ident,
    rpc_trait_name: &syn::Ident,
    methods: &[Method],
    _all_sync: bool,
    ctx: Option<&Type>,
) -> TokenStream2 {
    let ctx_param = ctx.map(|t| quote! { ctx: #t, });
    // Sync methods receive the ambient context by reference; the owned
    // wire value is captured into the dispatcher closure.
    let ctx_sync_arg = ctx.map(|_| quote! { &ctx, });
    let ctx_async_arg = ctx.map(|_| quote! { ctx, });
    let method_impls = methods.iter().map(|m| {
        let name = &m.decl.sig.ident;
        let mut mirror_iter = m.mirror_inputs.iter();
        let receiver = mirror_iter.next();
        let mirror_inputs: Vec<_> = mirror_iter.collect();
        let output = &m.return_ty;
        let arg_idents = &m.arg_idents;
        let arg_was_ref = &m.arg_was_ref;

        // Build the call expression that invokes the user trait method
        // from inside the dispatcher closure / async block, taking
        // `&owned` for any arg that was originally a reference.
        let call_args = arg_idents
            .iter()
            .zip(arg_was_ref.iter())
            .map(|(id, &was_ref)| {
                if was_ref {
                    quote! { &#id }
                } else {
                    quote! { #id }
                }
            });
        let call_args2 = call_args.clone();

        if m.is_async {
            // Pass-through: backend's async method is awaited directly.
            quote! {
                async fn #name(#receiver, #ctx_param #(#mirror_inputs),*) #output {
                    self.inner.#name(#ctx_async_arg #(#call_args),*).await
                }
            }
        } else {
            // Marshal: clone the inner Arc, capture args by move, run
            // through the dispatcher.
            quote! {
                async fn #name(#receiver, #ctx_param #(#mirror_inputs),*) #output {
                    let __inner = ::std::sync::Arc::clone(&self.inner);
                    let __disp = ::std::sync::Arc::clone(&self.dispatcher);
                    ::architect::dispatch::run(
                        &*__disp,
                        move || __inner.#name(#ctx_sync_arg #(#call_args2),*),
                    )
                    .await
                    .expect("#[architect::rpc] bridge: dispatcher failed")
                }
            }
        }
    });

    quote! {
        impl<S, D> #rpc_trait_name for #host_name<S, D>
        where
            S: #trait_name + ::architect::MaybeSendSync + 'static,
            D: ::architect::dispatch::Dispatcher,
        {
            #(#method_impls)*
        }
    }
}

/// Pass-through host for all-async traits. The user trait *is* its own
/// async surface, so the host is a thin newtype wrapping a backend
/// `Arc<S>`. Construction takes only the backend — no dispatcher.
fn emit_passthrough_host(
    trait_name: &syn::Ident,
    host_name: &syn::Ident,
    vis: &syn::Visibility,
) -> TokenStream2 {
    quote! {
        /// Internal newtype wrapper for an all-async backend. Hidden
        /// from documentation; user code mounts via `serve`.
        #[doc(hidden)]
        #vis struct #host_name<S>
        where
            S: #trait_name + ::architect::MaybeSendSync + 'static,
        {
            inner: ::std::sync::Arc<S>,
        }

        impl<S> #host_name<S>
        where
            S: #trait_name + ::architect::MaybeSendSync + 'static,
        {
            pub fn new(inner: S) -> Self {
                Self { inner: ::std::sync::Arc::new(inner) }
            }

            pub fn from_arc(inner: ::std::sync::Arc<S>) -> Self {
                Self { inner }
            }
        }

        impl<S> ::core::clone::Clone for #host_name<S>
        where
            S: #trait_name + ::architect::MaybeSendSync + 'static,
        {
            fn clone(&self) -> Self {
                Self { inner: ::std::sync::Arc::clone(&self.inner) }
            }
        }
    }
}

/// Pass-through impl: each method calls through to the inner backend.
/// All methods are async by definition for this shape.
fn emit_passthrough_impl(
    trait_name: &syn::Ident,
    host_name: &syn::Ident,
    methods: &[Method],
    ctx: Option<&Type>,
) -> TokenStream2 {
    let ctx_param = ctx.map(|t| quote! { ctx: #t, });
    let ctx_arg = ctx.map(|_| quote! { ctx, });
    let method_impls = methods.iter().map(|m| {
        let name = &m.decl.sig.ident;
        let mut mirror_iter = m.mirror_inputs.iter();
        let receiver = mirror_iter.next();
        let mirror_inputs: Vec<_> = mirror_iter.collect();
        let output = &m.return_ty;
        let arg_idents = &m.arg_idents;
        let arg_was_ref = &m.arg_was_ref;

        let call_args = arg_idents
            .iter()
            .zip(arg_was_ref.iter())
            .map(|(id, &was_ref)| {
                if was_ref {
                    quote! { &#id }
                } else {
                    quote! { #id }
                }
            });

        quote! {
            async fn #name(#receiver, #ctx_param #(#mirror_inputs),*) #output {
                self.inner.#name(#ctx_arg #(#call_args),*).await
            }
        }
    });

    quote! {
        impl<S> #trait_name for #host_name<S>
        where
            S: #trait_name + ::architect::MaybeSendSync + 'static,
        {
            #(#method_impls)*
        }
    }
}
