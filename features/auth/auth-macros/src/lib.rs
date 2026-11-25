//! `#[auth_macros::entity(table_name = "…")]`: one of auth's table rows.
//!
//! On a struct with named fields, it writes:
//! - the row itself, deriving `Facet`, `Clone`, `Debug`, `PartialEq`
//!   (and `fake::Dummy` under the crate's `fake` feature);
//! - `<Row>Create`: the fields a caller supplies (every field except those
//!   marked `exclude(create)` or given an `on_create` value);
//! - `<Row>Update`: each field a caller may change, as an `Option`
//!   (every field except the primary key and those marked
//!   `exclude(update)` or given an `on_update` value);
//! - `<Row>List`: a page of rows (`items`, `total`, `page: crate::Page`);
//! - under the crate's `server` feature, `__<row>_storage`: the row's
//!   sea-orm entity (`Model`, `ActiveModel`, `Column`, `Entity`,
//!   `PrimaryKey`, `Relation`) and `From<Model> for <Row>`.
//!
//! Field options, in `#[entity(…)]`: `primary_key`,
//! `auto_increment = <bool>` (default `false`), `on_create = <expr>`,
//! `on_update = <expr>`, `exclude(create, update)`, `json` (a JSON
//! column). Tables, indexes and migrations are written by hand
//! (`auth-db`); the attribute never creates them.
//!
//! `#[auth_macros::service(path = "…")]`: one of auth's services, on a
//! trait of `async fn`s each returning `Result<T, E>`. It writes:
//! - the trait under `#[vox::service]` (the crate's `vox` feature): vox's
//!   `<Trait>Client`, `<Trait>Dispatcher` and
//!   `<trait>_service_descriptor`, with vox's own method ids;
//! - `<trait>_http`: `PATHS`, one `<Method>Args` per method, and (the
//!   crate's `http` and `vox` features) `mount` / `router`, every method
//!   as `POST /<path>/<tail>` (`#[http(path = "…")]`, or the method name
//!   in kebab-case), an argument called `token` also taken from
//!   `Authorization: Bearer`;
//! - `<Trait>HttpClient` (the crate's `http-client` feature), and the
//!   trait implemented for it and for the vox client, so a caller that
//!   takes `impl Trait` never knows which wire is underneath;
//! - `prelude`: what a crate root re-exports.
//!
//! Everything it writes names the crate's own `crate::http` runtime.

use heck::ToSnakeCase;
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{
    Expr, Fields, FnArg, ItemStruct, ItemTrait, LitStr, Pat, Result, ReturnType, TraitItem, Type,
    parse_macro_input,
};

#[derive(Default)]
struct FieldOpts {
    primary_key: bool,
    auto_increment: bool,
    on_create: Option<Expr>,
    on_update: Option<Expr>,
    exclude_create: bool,
    exclude_update: bool,
    json: bool,
}

fn field_opts(attrs: &[syn::Attribute]) -> Result<FieldOpts> {
    let mut o = FieldOpts::default();
    for a in attrs.iter().filter(|a| a.path().is_ident("entity")) {
        a.parse_nested_meta(|m| {
            if m.path.is_ident("primary_key") {
                o.primary_key = true;
            } else if m.path.is_ident("auto_increment") {
                o.auto_increment = m.value()?.parse::<syn::LitBool>()?.value;
            } else if m.path.is_ident("on_create") {
                o.on_create = Some(m.value()?.parse()?);
            } else if m.path.is_ident("on_update") {
                o.on_update = Some(m.value()?.parse()?);
            } else if m.path.is_ident("json") {
                o.json = true;
            } else if m.path.is_ident("exclude") {
                m.parse_nested_meta(|e| {
                    if e.path.is_ident("create") {
                        o.exclude_create = true;
                    } else if e.path.is_ident("update") {
                        o.exclude_update = true;
                    } else {
                        return Err(e.error("exclude(create) or exclude(update)"));
                    }
                    Ok(())
                })?;
            } else {
                return Err(m.error(
                    "unknown option: primary_key, auto_increment, on_create, on_update, exclude(..) or json",
                ));
            }
            Ok(())
        })?;
    }
    Ok(o)
}

#[proc_macro_attribute]
pub fn entity(args: TokenStream, input: TokenStream) -> TokenStream {
    let item = parse_macro_input!(input as ItemStruct);
    let mut table: Option<LitStr> = None;
    let parser = syn::meta::parser(|m| {
        if m.path.is_ident("table_name") {
            table = Some(m.value()?.parse()?);
            Ok(())
        } else {
            Err(m.error("expected table_name = \"…\""))
        }
    });
    parse_macro_input!(args with parser);
    match expand(item, table) {
        Ok(t) => t.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn expand(mut item: ItemStruct, table: Option<LitStr>) -> Result<TokenStream2> {
    let ident = item.ident.clone();
    let vis = item.vis.clone();
    let Fields::Named(named) = &mut item.fields else {
        return Err(syn::Error::new_spanned(
            &ident,
            "an entity needs named fields",
        ));
    };
    let mut fields = Vec::new();
    for f in &mut named.named {
        let opts = field_opts(&f.attrs)?;
        f.attrs.retain(|a| !a.path().is_ident("entity"));
        // What goes onto the derived structs: its docs, nothing that
        // needs a derive they do not have.
        let docs: Vec<syn::Attribute> = f
            .attrs
            .iter()
            .filter(|a| a.path().is_ident("doc"))
            .cloned()
            .collect();
        let Some(ident) = f.ident.clone() else {
            return Err(syn::Error::new_spanned(&*f, "an entity needs named fields"));
        };
        fields.push((ident, f.ty.clone(), opts, docs));
    }
    if !fields.iter().any(|(_, _, o, _)| o.primary_key) {
        return Err(syn::Error::new_spanned(
            &ident,
            "an entity needs a #[entity(primary_key)] field",
        ));
    }
    let table = table.map_or_else(|| ident.to_string().to_snake_case(), |t| t.value());

    let create = format_ident!("{ident}Create");
    let update = format_ident!("{ident}Update");
    let list = format_ident!("{ident}List");
    let storage = format_ident!("__{}_storage", ident.to_string().to_snake_case());

    let create_fields = fields
        .iter()
        .filter(|(_, _, o, _)| !o.exclude_create && o.on_create.is_none())
        .map(|(id, ty, _, docs)| quote! { #(#docs)* pub #id: #ty });
    let update_fields = fields
        .iter()
        .filter(|(_, _, o, _)| !o.exclude_update && !o.primary_key && o.on_update.is_none())
        .map(|(id, ty, _, docs)| quote! { #(#docs)* pub #id: ::core::option::Option<#ty> });
    let model_fields = fields.iter().map(|(id, ty, o, docs)| {
        let mut sea = Vec::new();
        if o.primary_key {
            if o.auto_increment {
                sea.push(quote! { primary_key });
            } else {
                sea.push(quote! { primary_key, auto_increment = false });
            }
        }
        if o.json {
            sea.push(quote! { column_type = "Json" });
        }
        let sea = if sea.is_empty() {
            quote! {}
        } else {
            quote! { #[sea_orm(#(#sea),*)] }
        };
        quote! { #(#docs)* #sea pub #id: #ty }
    });
    let from_model = fields.iter().map(|(id, ..)| quote! { #id: m.#id });
    let create_doc = format!("What a caller supplies to make a [`{ident}`].");
    let update_doc =
        format!("What a caller may change of a [`{ident}`]; `None` leaves a field as it is.");
    let list_doc = format!("A page of [`{ident}`] rows.");

    Ok(quote! {
        #[derive(::facet::Facet, ::core::clone::Clone, ::core::fmt::Debug, ::core::cmp::PartialEq)]
        #[cfg_attr(feature = "fake", derive(::fake::Dummy))]
        #item

        #[doc = #create_doc]
        #[derive(::facet::Facet, ::core::clone::Clone, ::core::fmt::Debug, ::core::cmp::PartialEq)]
        #[cfg_attr(feature = "fake", derive(::fake::Dummy))]
        #[allow(clippy::derive_partial_eq_without_eq)]
        #vis struct #create { #(#create_fields,)* }

        #[doc = #update_doc]
        #[derive(::facet::Facet, ::core::clone::Clone, ::core::fmt::Debug, ::core::cmp::PartialEq, ::core::default::Default)]
        #[cfg_attr(feature = "fake", derive(::fake::Dummy))]
        #[allow(clippy::derive_partial_eq_without_eq)]
        #vis struct #update { #(#update_fields,)* }

        #[doc = #list_doc]
        #[derive(::facet::Facet, ::core::clone::Clone, ::core::fmt::Debug, ::core::cmp::PartialEq)]
        #[cfg_attr(feature = "fake", derive(::fake::Dummy))]
        #[allow(clippy::derive_partial_eq_without_eq)]
        #vis struct #list {
            pub items: ::std::vec::Vec<#ident>,
            pub total: u32,
            pub page: crate::Page,
        }

        #[cfg(feature = "server")]
        #[doc(hidden)]
        #vis mod #storage {
            #[allow(unused_imports)]
            use super::*;
            // What sea-orm's derives name without a path.
            #[allow(unused_imports)]
            use ::sea_orm::{
                ActiveModelBehavior, ActiveModelTrait, DeriveEntityModel, DerivePrimaryKey,
                DeriveRelation, EntityTrait, EnumIter, IntoActiveModel, PrimaryKeyTrait,
            };

            #[derive(::core::clone::Clone, ::core::fmt::Debug, ::core::cmp::PartialEq, ::sea_orm::DeriveEntityModel)]
            #[sea_orm(table_name = #table)]
            #[allow(clippy::derive_partial_eq_without_eq)]
            pub struct Model { #(#model_fields,)* }

            #[derive(::core::marker::Copy, ::core::clone::Clone, ::core::fmt::Debug, ::sea_orm::EnumIter, ::sea_orm::DeriveRelation)]
            pub enum Relation {}

            impl ::sea_orm::ActiveModelBehavior for ActiveModel {}

            impl ::core::convert::From<Model> for super::#ident {
                fn from(m: Model) -> Self {
                    super::#ident { #(#from_model,)* }
                }
            }
        }
    })
}

// ── #[auth_macros::service] ─────────────────────────────────────────────

/// `sign_in_email` → `SignInEmail`.
fn pascal(snake: &str) -> String {
    snake
        .split('_')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_uppercase().chain(c).collect::<String>())
                .unwrap_or_default()
        })
        .collect()
}

/// The default prefix: the trait name in kebab-case, minus `Service`.
fn http_prefix(trait_name: &str) -> String {
    let base = match trait_name.strip_suffix("Service") {
        Some(s) if !s.is_empty() => s,
        _ => trait_name,
    };
    base.to_snake_case().replace('_', "-")
}

/// `Result<T, E>` → `(T, E)`.
fn result_parts(ret: &ReturnType) -> Option<(Type, Type)> {
    let ReturnType::Type(_, ty) = ret else {
        return None;
    };
    let Type::Path(p) = &**ty else { return None };
    let last = p.path.segments.last()?;
    if last.ident != "Result" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(args) = &last.arguments else {
        return None;
    };
    let mut tys = args.args.iter().filter_map(|a| match a {
        syn::GenericArgument::Type(t) => Some(t.clone()),
        _ => None,
    });
    Some((tys.next()?, tys.next()?))
}

struct Method {
    ident: syn::Ident,
    args: Vec<(syn::Ident, Type)>,
    ok: Type,
    err: Type,
    path: String,
    docs: Vec<syn::Attribute>,
}

#[proc_macro_attribute]
pub fn service(args: TokenStream, input: TokenStream) -> TokenStream {
    let mut item = parse_macro_input!(input as ItemTrait);
    let mut prefix: Option<LitStr> = None;
    let parser = syn::meta::parser(|m| {
        if m.path.is_ident("path") {
            prefix = Some(m.value()?.parse()?);
            Ok(())
        } else {
            Err(m.error("expected path = \"…\""))
        }
    });
    parse_macro_input!(args with parser);
    match expand_service(&mut item, prefix) {
        Ok(t) => t.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn expand_service(item: &mut ItemTrait, prefix: Option<LitStr>) -> Result<TokenStream2> {
    let name = item.ident.clone();
    let vis = item.vis.clone();
    let prefix = prefix.map_or_else(|| http_prefix(&name.to_string()), |p| p.value());
    let mut methods = Vec::new();
    for ti in &mut item.items {
        let TraitItem::Fn(f) = ti else { continue };
        let mut tail: Option<String> = None;
        let mut kept = Vec::new();
        for a in f.attrs.drain(..) {
            if a.path().is_ident("http") {
                a.parse_nested_meta(|m| {
                    if m.path.is_ident("path") {
                        tail = Some(m.value()?.parse::<LitStr>()?.value());
                        Ok(())
                    } else {
                        Err(m.error("expected path = \"…\""))
                    }
                })?;
            } else {
                kept.push(a);
            }
        }
        f.attrs = kept;
        if f.sig.asyncness.is_none() {
            return Err(syn::Error::new_spanned(
                &f.sig,
                "a service method is an async fn",
            ));
        }
        let (ok, err) = result_parts(&f.sig.output).ok_or_else(|| {
            syn::Error::new_spanned(&f.sig.output, "a service method returns Result<T, E>")
        })?;
        let mut args = Vec::new();
        for input in &f.sig.inputs {
            match input {
                FnArg::Receiver(_) => {}
                FnArg::Typed(t) => {
                    let Pat::Ident(id) = &*t.pat else {
                        return Err(syn::Error::new_spanned(&t.pat, "name each argument"));
                    };
                    args.push((id.ident.clone(), (*t.ty).clone()));
                }
            }
        }
        let ident = f.sig.ident.clone();
        let tail = tail.unwrap_or_else(|| ident.to_string().replace('_', "-"));
        let docs = f
            .attrs
            .iter()
            .filter(|a| a.path().is_ident("doc"))
            .cloned()
            .collect();
        methods.push(Method {
            ident,
            args,
            ok,
            err,
            path: format!("/{prefix}/{tail}"),
            docs,
        });
    }

    let snake = name.to_string().to_snake_case();
    let http_mod = format_ident!("{snake}_http");
    let client = format_ident!("{name}Client");
    let dispatcher = format_ident!("{name}Dispatcher");
    let descriptor = format_ident!("{snake}_service_descriptor");
    let http_client = format_ident!("{name}HttpClient");

    let paths = methods.iter().map(|m| {
        let n = m.ident.to_string();
        let p = &m.path;
        quote! { (#n, #p) }
    });
    let args_structs = methods.iter().map(|m| {
        let a = format_ident!("{}Args", pascal(&m.ident.to_string()));
        // A `token` may be left out of the body when it rides
        // `Authorization: Bearer`.
        let fields = m.args.iter().map(|(id, ty)| {
            if id == "token" {
                quote! { #[facet(default)] pub #id: #ty }
            } else {
                quote! { pub #id: #ty }
            }
        });
        let doc = format!("`{}`'s body: one field per argument.", m.path);
        quote! {
            #[doc = #doc]
            #[derive(::facet::Facet, ::core::fmt::Debug, ::core::clone::Clone)]
            pub struct #a { #(#fields,)* }
        }
    });
    let handlers = methods.iter().map(|m| {
        let h = format_ident!("__http_{}", m.ident);
        let a = format_ident!("{}Args", pascal(&m.ident.to_string()));
        let ident = &m.ident;
        let names: Vec<_> = m.args.iter().map(|(id, _)| id).collect();
        let fold = if m.args.iter().any(|(id, _)| id == "token") {
            quote! { args.token = crate::http::token_or_bearer(args.token, bearer); }
        } else {
            quote! { let _ = bearer; }
        };
        quote! {
            async fn #h<S>(
                crate::http::axum::extract::State(svc): crate::http::axum::extract::State<::std::sync::Arc<S>>,
                call: crate::http::Call<#a>,
            ) -> crate::http::axum::response::Response
            where
                S: #name + ::core::marker::Send + ::core::marker::Sync + 'static,
            {
                let crate::http::Call { mut args, bearer } = call;
                #fold
                let _ = &mut args;
                crate::http::respond(svc.#ident(#(args.#names),*).await)
            }
        }
    });
    let routes = methods.iter().map(|m| {
        let h = format_ident!("__http_{}", m.ident);
        let p = &m.path;
        quote! {
            let router = router.route(
                #p,
                crate::http::axum::routing::post(#h::<S>).with_state(svc.clone()),
            );
        }
    });
    let client_methods = methods.iter().map(|m| {
        let ident = &m.ident;
        let a = format_ident!("{}Args", pascal(&ident.to_string()));
        let params = m.args.iter().map(|(id, ty)| quote! { #id: #ty });
        let names: Vec<_> = m.args.iter().map(|(id, _)| id).collect();
        let (ok, err, p, docs) = (&m.ok, &m.err, &m.path, &m.docs);
        quote! {
            #(#docs)*
            pub async fn #ident(&self, #(#params),*)
                -> ::core::result::Result<#ok, crate::http::ClientError<#err>>
            {
                let args = #http_mod::#a { #(#names),* };
                self.inner.call(#p, &args).await
            }
        }
    });
    let trait_impl = |target: &syn::Ident, fold: TokenStream2| {
        let fns = methods.iter().map(|m| {
            let ident = &m.ident;
            let params = m.args.iter().map(|(id, ty)| quote! { #id: #ty });
            let names: Vec<_> = m.args.iter().map(|(id, _)| id).collect();
            let (ok, err) = (&m.ok, &m.err);
            quote! {
                async fn #ident(&self, #(#params),*) -> ::core::result::Result<#ok, #err> {
                    #target::#ident(self, #(#names),*).await.map_err(#fold)
                }
            }
        });
        quote! { impl #name for #target { #(#fns)* } }
    };
    let http_impl = trait_impl(&http_client, quote! { crate::http::ClientError::into_app });
    let vox_impl = trait_impl(&client, quote! { crate::http::vox_into_app });
    let http_client_doc = format!("[`{name}`] over HTTP: one method per trait method.");
    let mod_doc = format!("[`{name}`]'s HTTP face: its paths, bodies and routes.");

    Ok(quote! {
        #[cfg_attr(feature = "vox", ::vox::service)]
        #item

        #[doc = #mod_doc]
        #vis mod #http_mod {
            #[allow(unused_imports)]
            use super::*;

            /// `(method, path)` for every route.
            pub const PATHS: &[(&str, &str)] = &[#(#paths),*];

            #(#args_structs)*

            #[cfg(all(feature = "http", feature = "vox"))]
            mod handlers {
                #[allow(unused_imports)]
                use super::*;
                #(#handlers)*

                pub fn mount<S>(
                    svc: ::std::sync::Arc<S>,
                    router: crate::http::axum::Router,
                ) -> crate::http::axum::Router
                where
                    S: #name + ::core::marker::Send + ::core::marker::Sync + 'static,
                {
                    #(#routes)*
                    router
                }
            }

            /// Add every method's route to `router`, answered by `svc`.
            #[cfg(all(feature = "http", feature = "vox"))]
            pub fn mount<S>(
                svc: ::std::sync::Arc<S>,
                router: crate::http::axum::Router,
            ) -> crate::http::axum::Router
            where
                S: #name + ::core::marker::Send + ::core::marker::Sync + 'static,
            {
                handlers::mount(svc, router)
            }

            /// Every method as a route, answered by `svc`.
            #[cfg(all(feature = "http", feature = "vox"))]
            pub fn router<S>(svc: S) -> crate::http::axum::Router
            where
                S: #name + ::core::marker::Send + ::core::marker::Sync + 'static,
            {
                handlers::mount(::std::sync::Arc::new(svc), crate::http::axum::Router::new())
            }
        }

        #[doc = #http_client_doc]
        #[cfg(feature = "http-client")]
        #[derive(::core::clone::Clone, ::core::fmt::Debug)]
        #vis struct #http_client {
            inner: crate::http::HttpClient,
        }

        #[cfg(feature = "http-client")]
        impl #http_client {
            /// Over a shared transport (base URL, bearer, pool).
            pub fn new(inner: crate::http::HttpClient) -> Self {
                Self { inner }
            }
            /// A fresh transport rooted at `base_url`.
            pub fn at(base_url: impl ::core::convert::Into<::std::string::String>) -> Self {
                Self::new(crate::http::HttpClient::new(base_url))
            }
            /// Present `token` as `Authorization: Bearer` on every call.
            pub fn with_bearer(
                mut self,
                token: impl ::core::convert::Into<::std::string::String>,
            ) -> Self {
                self.inner = self.inner.with_bearer(token);
                self
            }
            /// The underlying transport.
            pub fn transport(&self) -> &crate::http::HttpClient {
                &self.inner
            }
            #(#client_methods)*
        }

        #[cfg(feature = "http-client")]
        #http_impl

        #[cfg(feature = "vox")]
        #vox_impl

        /// What a crate root re-exports.
        #vis mod prelude {
            pub use super::#name;
            pub use super::#http_mod;
            #[cfg(feature = "vox")]
            pub use super::{#client, #dispatcher, #descriptor};
            #[cfg(feature = "http-client")]
            pub use super::#http_client;
        }
    })
}

// ── #[derive(auth_macros::Config)] ──────────────────────────────────────

/// `#[derive(auth_macros::Config)]`: a configuration struct that reads
/// itself from the environment, `Self::from_env()`.
///
/// Each field reads the variable `<PREFIX>_<FIELD>` (container
/// `#[config(prefix = "AUTH")]`), or the one `#[config(env = "…")]`
/// names. Field options: `default = <expr>` (used when unset),
/// `secret` (also read from the file `<VAR>_FILE` names), `nested` (a
/// struct of its own, `<Ty>::from_env()`), `skip` (`Default`). Values
/// parse through the crate's `crate::env::FromEnvValue`: scalars,
/// `bool` (`1/true/yes/on`, `0/false/no/off`), `Option<T>` and `Vec<T>`
/// (comma-separated).
#[proc_macro_derive(Config, attributes(config))]
pub fn derive_config(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as syn::DeriveInput);
    match expand_config(&input) {
        Ok(t) => t.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn expand_config(input: &syn::DeriveInput) -> Result<TokenStream2> {
    let ident = &input.ident;
    let mut prefix: Option<String> = None;
    for attr in input.attrs.iter().filter(|a| a.path().is_ident("config")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("prefix") {
                let s: LitStr = meta.value()?.parse()?;
                prefix = Some(s.value().trim_end_matches('_').to_owned());
                Ok(())
            } else {
                Err(meta.error("expected prefix = \"…\""))
            }
        })?;
    }
    let syn::Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            ident,
            "Config derives on structs only",
        ));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new_spanned(ident, "Config needs named fields"));
    };
    let mut reads = Vec::new();
    for field in &fields.named {
        let Some(name) = field.ident.as_ref() else {
            return Err(syn::Error::new_spanned(field, "Config needs named fields"));
        };
        let ty = &field.ty;
        let mut env: Option<String> = None;
        let mut default: Option<Expr> = None;
        let (mut secret, mut nested, mut skip) = (false, false, false);
        for attr in field.attrs.iter().filter(|a| a.path().is_ident("config")) {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("env") {
                    env = Some(meta.value()?.parse::<LitStr>()?.value());
                } else if meta.path.is_ident("default") {
                    default = Some(meta.value()?.parse()?);
                } else if meta.path.is_ident("secret") {
                    secret = true;
                } else if meta.path.is_ident("nested") {
                    nested = true;
                } else if meta.path.is_ident("skip") {
                    skip = true;
                } else {
                    return Err(meta.error("expected env, default, secret, nested or skip"));
                }
                Ok(())
            })?;
        }
        if skip {
            reads.push(quote! { #name: ::core::default::Default::default() });
            continue;
        }
        if nested {
            reads.push(quote! { #name: <#ty>::from_env()? });
            continue;
        }
        let var = env.unwrap_or_else(|| {
            let upper = name.to_string().to_uppercase();
            match &prefix {
                Some(p) => format!("{p}_{upper}"),
                None => upper,
            }
        });
        let raw = if secret {
            quote! { crate::env::secret(#var)? }
        } else {
            quote! { crate::env::var(#var) }
        };
        let read = default.map_or_else(
            || quote! { <#ty as crate::env::FromEnvValue>::from_env_value(#var, #raw)? },
            |expr| {
                quote! {
                    match #raw {
                        ::core::option::Option::Some(raw) => {
                            <#ty as crate::env::FromEnvValue>::from_env_value(
                                #var,
                                ::core::option::Option::Some(raw),
                            )?
                        }
                        ::core::option::Option::None => ::core::convert::Into::into(#expr),
                    }
                }
            },
        );
        reads.push(quote! { #name: #read });
    }
    Ok(quote! {
        impl #ident {
            /// Read every field from the environment.
            pub fn from_env() -> ::core::result::Result<Self, crate::env::ConfigError> {
                ::core::result::Result::Ok(Self { #(#reads),* })
            }
        }
    })
}
