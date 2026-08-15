//! Proc-macros for `architect-story`.
//!
//! ```ignore
//! use architect_story_runtime::story;
//! use dioxus::prelude::*;
//!
//! /// Standard CTA button.
//! #[story(
//!     category = "Buttons",
//!     knobs(
//!         label = "Click me",
//!         disabled = false,
//!     ),
//! )]
//! pub fn button(label: &str, disabled: bool) -> Element {
//!     rsx! { Button { disabled, "{label}" } }
//! }
//! ```
//!
//! Knob defaults are listed in the outer `#[story(...)]` attribute
//! instead of per-arg attributes — stable rustc forbids non-builtin
//! attributes on fn parameters, so the lookbook-style inner-attr
//! syntax can't be used.
//!
//! Each entry inside `knobs(...)` is `<arg_name> = <default_expr>`.
//! The macro matches each entry to a fn arg by identifier name and
//! uses the arg's declared type to pick a `KnobKind`.
//!
//! Knob types currently introspected (with editable widgets in the
//! shell): `bool`, signed/unsigned ints up to 64 bits, `f32`/`f64`,
//! `&str`, `String`. Anything else renders with the supplied default
//! expression and shows as opaque (read-only) in the shell.

use heck::ToShoutySnakeCase;
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{
    parse_macro_input, spanned::Spanned, Attribute, Expr, ExprLit, FnArg, ItemFn, Lit, Meta, Pat,
    Type,
};

#[proc_macro_attribute]
pub fn story(args: TokenStream, input: TokenStream) -> TokenStream {
    let item_fn = parse_macro_input!(input as ItemFn);
    story_impl(args.into(), item_fn).into()
}

#[proc_macro_attribute]
pub fn story_test(_args: TokenStream, input: TokenStream) -> TokenStream {
    // TODO: parse a returned `InteractionScript` and register via
    // `#[distributed_slice(INTERACTION_SCRIPTS)]`.
    input
}

#[proc_macro_attribute]
pub fn states(_args: TokenStream, input: TokenStream) -> TokenStream {
    // TODO: emit a `&'static [StateAssignment]` and wire it into the
    // sibling story's `auto_states` field.
    input
}

// ── Implementation ──────────────────────────────────────────────────────────

struct StoryArgs {
    category: Option<String>,
    name_override: Option<String>,
    /// Map from arg name to user-supplied default expression.
    knob_defaults: Vec<(syn::Ident, Expr)>,
}

fn parse_story_args(args: TokenStream2) -> Result<StoryArgs, syn::Error> {
    let mut out = StoryArgs {
        category: None,
        name_override: None,
        knob_defaults: Vec::new(),
    };
    if args.is_empty() {
        return Ok(out);
    }
    let parser = syn::meta::parser(|meta| {
        if meta.path.is_ident("category") {
            let v: syn::LitStr = meta.value()?.parse()?;
            out.category = Some(v.value());
            Ok(())
        } else if meta.path.is_ident("name") {
            let v: syn::LitStr = meta.value()?.parse()?;
            out.name_override = Some(v.value());
            Ok(())
        } else if meta.path.is_ident("knobs") {
            // `knobs(arg1 = <expr>, arg2 = <expr>, ...)` — each entry
            // names a fn arg and supplies its default value.
            meta.parse_nested_meta(|inner| {
                let ident = inner
                    .path
                    .get_ident()
                    .ok_or_else(|| inner.error("knob name must be a bare identifier"))?
                    .clone();
                let value = inner.value()?;
                let expr: Expr = value.parse()?;
                out.knob_defaults.push((ident, expr));
                Ok(())
            })
        } else {
            Err(meta.error("supported keys: `category`, `name`, `knobs(arg = expr, ...)`"))
        }
    });
    syn::parse::Parser::parse2(parser, args)?;
    Ok(out)
}

#[derive(Clone, Copy)]
enum InferredKind {
    Bool,
    Int,
    Float,
    Str,
    Opaque,
}

struct ParsedKnob {
    name: syn::Ident,
    ty: Type,
    /// Empty for now — per-knob doc strings need a different syntax
    /// since rustc disallows `#[doc]` inside fn-param positions for
    /// non-builtin attribute macros. Phase 3 will likely accept
    /// `knobs(label = "..." doc "Visible label", ...)` or similar.
    doc: String,
    default: Expr,
    kind: InferredKind,
}

fn parse_knob(arg: &FnArg, defaults: &[(syn::Ident, Expr)]) -> Result<ParsedKnob, syn::Error> {
    let pat_ty = match arg {
        FnArg::Typed(p) => p,
        FnArg::Receiver(r) => {
            return Err(syn::Error::new(
                r.span(),
                "`#[story]` does not support `self`",
            ));
        }
    };

    let name = match &*pat_ty.pat {
        Pat::Ident(p) => p.ident.clone(),
        other => {
            return Err(syn::Error::new(
                other.span(),
                "`#[story]` arguments must be plain identifiers",
            ));
        }
    };

    let default = defaults
        .iter()
        .find(|(n, _)| n == &name)
        .map(|(_, e)| e.clone())
        .ok_or_else(|| {
            syn::Error::new(
                name.span(),
                format!(
                    "missing default for arg `{name}`; add `knobs({name} = <expr>)` \
                     to the `#[story(...)]` attribute"
                ),
            )
        })?;

    let kind = infer_kind(&pat_ty.ty);

    Ok(ParsedKnob {
        name,
        ty: (*pat_ty.ty).clone(),
        doc: String::new(),
        default,
        kind,
    })
}

fn doc_string(attr: &Attribute) -> Option<String> {
    if let Meta::NameValue(nv) = &attr.meta {
        if let Expr::Lit(ExprLit {
            lit: Lit::Str(s), ..
        }) = &nv.value
        {
            return Some(s.value());
        }
    }
    None
}

fn infer_kind(ty: &Type) -> InferredKind {
    fn last_segment(ty: &Type) -> Option<String> {
        if let Type::Path(p) = ty {
            return p.path.segments.last().map(|s| s.ident.to_string());
        }
        None
    }
    if let Type::Reference(r) = ty {
        if let Type::Path(p) = &*r.elem {
            if let Some(seg) = p.path.segments.last() {
                if seg.ident == "str" {
                    return InferredKind::Str;
                }
            }
        }
    }
    match last_segment(ty).as_deref() {
        Some("bool") => InferredKind::Bool,
        Some("i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" | "usize" | "isize") => {
            InferredKind::Int
        }
        Some("f32" | "f64") => InferredKind::Float,
        Some("String") => InferredKind::Str,
        _ => InferredKind::Opaque,
    }
}

fn story_impl(args: TokenStream2, item_fn: ItemFn) -> TokenStream2 {
    let args = match parse_story_args(args) {
        Ok(a) => a,
        Err(e) => return e.to_compile_error(),
    };

    let mut description = String::new();
    for attr in &item_fn.attrs {
        if attr.path().is_ident("doc") {
            if let Some(line) = doc_string(attr) {
                if !description.is_empty() {
                    description.push(' ');
                }
                description.push_str(line.trim());
            }
        }
    }

    let mut knobs: Vec<ParsedKnob> = Vec::with_capacity(item_fn.sig.inputs.len());
    for arg in item_fn.sig.inputs.iter() {
        match parse_knob(arg, &args.knob_defaults) {
            Ok(k) => knobs.push(k),
            Err(e) => return e.to_compile_error(),
        }
    }

    let user_fn_ident = item_fn.sig.ident.clone();
    let story_name = args
        .name_override
        .clone()
        .unwrap_or_else(|| user_fn_ident.to_string());
    let upper = user_fn_ident.to_string().to_shouty_snake_case();
    let knobs_static = format_ident!("{upper}_STORY_KNOBS");
    let story_static = format_ident!("{upper}_STORY");
    let thunk_ident = format_ident!("__{upper}_STORY_THUNK");
    let reg_ident = format_ident!("__{upper}_STORY_REG");

    let category_expr = match args.category {
        Some(s) => quote! { ::core::option::Option::Some(#s) },
        None => quote! { ::core::option::Option::None },
    };

    let knob_specs = knobs.iter().map(|k| {
        let name = k.name.to_string();
        let doc = k.doc.clone();
        let kind_expr = match k.kind {
            InferredKind::Bool => quote! { ::architect_story_runtime::KnobKind::Bool },
            InferredKind::Int | InferredKind::Float => quote! {
                ::architect_story_runtime::KnobKind::Number {
                    min: ::core::option::Option::None,
                    max: ::core::option::Option::None,
                    step: ::core::option::Option::None,
                }
            },
            InferredKind::Str => quote! {
                ::architect_story_runtime::KnobKind::String { multiline: false }
            },
            InferredKind::Opaque => quote! { ::architect_story_runtime::KnobKind::Opaque },
        };
        let default_for_spec = match k.kind {
            InferredKind::Bool => {
                let d = &k.default;
                quote! { ::core::option::Option::Some(::architect_story_runtime::KnobValue::Bool(#d)) }
            }
            InferredKind::Int => {
                let d = &k.default;
                quote! { ::core::option::Option::Some(::architect_story_runtime::KnobValue::Int((#d) as i64)) }
            }
            InferredKind::Float => {
                let d = &k.default;
                quote! { ::core::option::Option::Some(::architect_story_runtime::KnobValue::Float((#d) as f64)) }
            }
            InferredKind::Str => {
                let d = &k.default;
                quote! { ::core::option::Option::Some(::architect_story_runtime::KnobValue::Str(#d)) }
            }
            InferredKind::Opaque => quote! { ::core::option::Option::None },
        };
        quote! {
            ::architect_story_runtime::KnobSpec {
                name: #name,
                doc: #doc,
                kind: #kind_expr,
                default: #default_for_spec,
            }
        }
    });

    let thunk_lets = knobs.iter().map(|k| {
        let name_str = k.name.to_string();
        let ident = &k.name;
        let default = &k.default;
        match k.kind {
            InferredKind::Bool => quote! {
                let #ident = match __src.get(#name_str) {
                    ::core::option::Option::Some(::architect_story_runtime::KnobValue::Bool(__v)) => *__v,
                    _ => (#default),
                };
            },
            InferredKind::Int => quote! {
                let #ident = match __src.get(#name_str) {
                    ::core::option::Option::Some(::architect_story_runtime::KnobValue::Int(__v)) => *__v as _,
                    _ => (#default),
                };
            },
            InferredKind::Float => quote! {
                let #ident = match __src.get(#name_str) {
                    ::core::option::Option::Some(::architect_story_runtime::KnobValue::Float(__v)) => *__v as _,
                    _ => (#default),
                };
            },
            InferredKind::Str => {
                if matches!(&k.ty, Type::Reference(_)) {
                    quote! {
                        let #ident: &str = match __src.get(#name_str) {
                            ::core::option::Option::Some(::architect_story_runtime::KnobValue::Str(__v)) => *__v,
                            _ => (#default),
                        };
                    }
                } else {
                    quote! {
                        let #ident: ::std::string::String = match __src.get(#name_str) {
                            ::core::option::Option::Some(::architect_story_runtime::KnobValue::Str(__v)) => (*__v).to_string(),
                            _ => (#default).to_string(),
                        };
                    }
                }
            }
            InferredKind::Opaque => quote! {
                let #ident = (#default);
            },
        }
    });

    let arg_idents: Vec<_> = knobs.iter().map(|k| &k.name).collect();
    let user_fn = &item_fn;

    quote! {
        #user_fn

        #[allow(non_upper_case_globals)]
        static #knobs_static: &'static [::architect_story_runtime::KnobSpec] = &[ #(#knob_specs),* ];

        #[allow(non_snake_case)]
        fn #thunk_ident(__src: &dyn ::architect_story_runtime::KnobSource)
            -> ::dioxus::prelude::Element
        {
            #(#thunk_lets)*
            #user_fn_ident( #(#arg_idents),* )
        }

        pub static #story_static: ::architect_story_runtime::Story =
            ::architect_story_runtime::const_story(::architect_story_runtime::StoryDef {
                name: #story_name,
                category: #category_expr,
                description: #description,
                source: concat!(file!(), ":", line!()),
                render: #thunk_ident,
                knobs: #knobs_static,
                ..::architect_story_runtime::StoryDef::DEFAULT
            });

        // linkme 0.3 doesn't compile on wasm32-unknown-unknown. Skip
        // the registration there — STORIES is an empty stub on wasm32
        // (see architect-story-core). The const-evaluated story_static
        // above still exists and can be referenced directly.
        #[cfg(not(target_arch = "wasm32"))]
        #[::linkme::distributed_slice(::architect_story_runtime::STORIES)]
        #[allow(non_upper_case_globals)]
        static #reg_ident: &::architect_story_runtime::Story = &#story_static;
    }
}
