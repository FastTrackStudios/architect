//! `#[derive(Settings)]` (`docs/content/architecture/settings-page.md`):
//! a facet struct as a settings section.
//!
//! ```ignore
//! /// Appearance
//! #[derive(Facet, Settings, Default, Clone, PartialEq)]
//! #[settings(icon = "palette", scope = Synced)]
//! pub struct Appearance {
//!     /// Text size
//!     /// How large text is everywhere.
//!     #[setting(control = slider(80..=140, step = 5), unit = "%")]
//!     pub text_size: u32,
//! }
//! ```
//!
//! Emits `impl architect_settings::Settings` whose `spec()` is the
//! attributes, as data (`architect_settings::derive::SectionSpec`). The
//! words, the types and the defaults are not read here: they come from
//! the struct's facet shape and its `Default` at run time
//! (`architect_settings::derive::section_of`), the way
//! `architect::schema` reads them, so the two cannot disagree.
//!
//! On the struct, `#[settings(..)]`: `id = ".."` (the section's id and
//! its keys' prefix; the struct's name in snake case when left out),
//! `section = ".."` (its title; the doc comment's first line when left
//! out), `icon`, `group`, `scope = Device | Synced | Project`.
//!
//! On a field, `#[setting(..)]`: `control = switch | select | segmented |
//! cards | stepper | slider | text | shortcut`, or with its range
//! `slider(0..=100, step = 5)` / `stepper(1..=16)`; `range = a..=b`,
//! `step`, `unit = ".."`, `choices = [64, 128, 256]`, `aliases = [".."]`,
//! `placeholder = ".."`, `skip`.

use proc_macro::TokenStream;
use proc_macro2::{Literal, Span, TokenStream as TokenStream2};
use quote::quote;
use syn::{
    Attribute, Data, DeriveInput, Expr, ExprLit, ExprRange, Fields, Lit, LitStr, RangeLimits, UnOp,
    parse_macro_input, spanned::Spanned,
};

#[proc_macro_derive(Settings, attributes(settings, setting))]
pub fn derive_settings(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand(&input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// The struct's `#[settings(..)]`.
#[derive(Default)]
struct SectionArgs {
    id: Option<String>,
    title: Option<String>,
    icon: Option<String>,
    group: Option<String>,
    scope: Option<syn::Ident>,
}

/// A field's `#[setting(..)]`.
#[derive(Default)]
struct FieldArgs {
    control: Option<syn::Ident>,
    min: Option<f64>,
    max: Option<f64>,
    step: Option<f64>,
    unit: Option<String>,
    choices: Vec<ChoiceArg>,
    aliases: Vec<String>,
    placeholder: Option<String>,
    skip: bool,
}

enum ChoiceArg {
    Int(i64),
    Float(f64),
    Text(String),
}

const CONTROLS: &[&str] = &[
    "switch",
    "select",
    "segmented",
    "cards",
    "stepper",
    "slider",
    "text",
    "shortcut",
];

fn expand(input: &DeriveInput) -> syn::Result<TokenStream2> {
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new(
            input.generics.span(),
            "#[derive(Settings)] takes a struct without generics",
        ));
    }
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new(
            input.ident.span(),
            "#[derive(Settings)] is for a struct of named fields",
        ));
    };
    let Fields::Named(named) = &data.fields else {
        return Err(syn::Error::new(
            input.ident.span(),
            "#[derive(Settings)] is for a struct of named fields",
        ));
    };
    let section = section_args(&input.attrs)?;
    let ident = &input.ident;
    let id = section
        .id
        .clone()
        .unwrap_or_else(|| snake(&ident.to_string()));
    let title = opt_str(section.title.as_deref());
    let icon = opt_str(section.icon.as_deref());
    let group = opt_str(section.group.as_deref());
    let scope = match &section.scope {
        None => quote!(::architect_settings::Scope::Device),
        Some(s) if ["Device", "Synced", "Project"].contains(&s.to_string().as_str()) => {
            quote!(::architect_settings::Scope::#s)
        }
        Some(s) => {
            return Err(syn::Error::new(
                s.span(),
                "scope is one of Device, Synced, Project",
            ));
        }
    };
    let mut fields = Vec::new();
    for field in &named.named {
        let Some(name) = &field.ident else { continue };
        let args = field_args(&field.attrs)?;
        fields.push(field_spec(&name.to_string(), &args));
    }
    Ok(quote! {
        impl ::architect_settings::Settings for #ident {
            fn spec() -> &'static ::architect_settings::derive::SectionSpec {
                static SPEC: ::architect_settings::derive::SectionSpec =
                    ::architect_settings::derive::SectionSpec {
                        id: #id,
                        title: #title,
                        icon: #icon,
                        group: #group,
                        scope: #scope,
                        fields: &[#(#fields),*],
                    };
                &SPEC
            }
        }
    })
}

fn field_spec(name: &str, a: &FieldArgs) -> TokenStream2 {
    let control = a.control.as_ref().map_or_else(
        || quote!(::core::option::Option::None),
        |c| {
            let variant = syn::Ident::new(&camel(&c.to_string()), c.span());
            quote!(::core::option::Option::Some(::architect_settings::derive::ControlSpec::#variant))
        },
    );
    let min = opt_f64(a.min);
    let max = opt_f64(a.max);
    let step = opt_f64(a.step);
    let unit = a.unit.clone().unwrap_or_default();
    let placeholder = opt_str(a.placeholder.as_deref());
    let aliases = &a.aliases;
    let choices = a.choices.iter().map(|c| match c {
        ChoiceArg::Int(i) => quote!(::architect_settings::derive::ChoiceSpec::Int(#i)),
        ChoiceArg::Float(f) => {
            let f = Literal::f64_suffixed(*f);
            quote!(::architect_settings::derive::ChoiceSpec::Float(#f))
        }
        ChoiceArg::Text(t) => quote!(::architect_settings::derive::ChoiceSpec::Text(#t)),
    });
    let skip = a.skip;
    quote! {
        ::architect_settings::derive::FieldSpec {
            name: #name,
            control: #control,
            min: #min,
            max: #max,
            step: #step,
            unit: #unit,
            choices: &[#(#choices),*],
            aliases: &[#(#aliases),*],
            placeholder: #placeholder,
            skip: #skip,
        }
    }
}

fn opt_str(s: Option<&str>) -> TokenStream2 {
    s.map_or_else(
        || quote!(::core::option::Option::None),
        |s| quote!(::core::option::Option::Some(#s)),
    )
}

fn opt_f64(v: Option<f64>) -> TokenStream2 {
    v.map_or_else(
        || quote!(::core::option::Option::None),
        |v| {
            let v = Literal::f64_suffixed(v);
            quote!(::core::option::Option::Some(#v))
        },
    )
}

fn section_args(attrs: &[Attribute]) -> syn::Result<SectionArgs> {
    let mut out = SectionArgs::default();
    for attr in attrs.iter().filter(|a| a.path().is_ident("settings")) {
        attr.parse_nested_meta(|meta| {
            let key = meta
                .path
                .get_ident()
                .map(ToString::to_string)
                .unwrap_or_default();
            match key.as_str() {
                "id" => out.id = Some(meta.value()?.parse::<LitStr>()?.value()),
                "section" | "title" => out.title = Some(meta.value()?.parse::<LitStr>()?.value()),
                "icon" => out.icon = Some(meta.value()?.parse::<LitStr>()?.value()),
                "group" => out.group = Some(meta.value()?.parse::<LitStr>()?.value()),
                "scope" => out.scope = Some(meta.value()?.parse::<syn::Ident>()?),
                _ => {
                    return Err(meta.error("#[settings(..)] takes id, section, icon, group, scope"));
                }
            }
            Ok(())
        })?;
    }
    Ok(out)
}

fn field_args(attrs: &[Attribute]) -> syn::Result<FieldArgs> {
    let mut out = FieldArgs::default();
    for attr in attrs.iter().filter(|a| a.path().is_ident("setting")) {
        attr.parse_nested_meta(|meta| {
            let key = meta
                .path
                .get_ident()
                .map(ToString::to_string)
                .unwrap_or_default();
            match key.as_str() {
                "skip" => out.skip = true,
                "control" => control(&meta.value()?.parse::<Expr>()?, &mut out)?,
                "range" => {
                    let (min, max) = range(&meta.value()?.parse::<Expr>()?)?;
                    out.min = Some(min);
                    out.max = Some(max);
                }
                "step" => out.step = Some(number(&meta.value()?.parse::<Expr>()?)?),
                "unit" => out.unit = Some(meta.value()?.parse::<LitStr>()?.value()),
                "placeholder" => {
                    out.placeholder = Some(meta.value()?.parse::<LitStr>()?.value());
                }
                "aliases" => {
                    let Expr::Array(list) = meta.value()?.parse::<Expr>()? else {
                        return Err(meta.error("aliases = [\"..\", ..]"));
                    };
                    for e in &list.elems {
                        let Expr::Lit(ExprLit {
                            lit: Lit::Str(s), ..
                        }) = e
                        else {
                            return Err(syn::Error::new(e.span(), "an alias is a string"));
                        };
                        out.aliases.push(s.value());
                    }
                }
                "choices" => {
                    let Expr::Array(list) = meta.value()?.parse::<Expr>()? else {
                        return Err(meta.error("choices = [a, b, ..]"));
                    };
                    for e in &list.elems {
                        out.choices.push(choice(e)?);
                    }
                }
                _ => {
                    return Err(meta.error(
                        "#[setting(..)] takes control, range, step, unit, choices, aliases, placeholder, skip",
                    ));
                }
            }
            Ok(())
        })?;
    }
    Ok(out)
}

/// `control = slider` or `control = slider(0..=100, step = 5)`.
fn control(expr: &Expr, out: &mut FieldArgs) -> syn::Result<()> {
    let (name, args) = match expr {
        Expr::Path(p) => (p.path.get_ident().cloned(), None),
        Expr::Call(call) => match &*call.func {
            Expr::Path(p) => (p.path.get_ident().cloned(), Some(&call.args)),
            _ => (None, None),
        },
        _ => (None, None),
    };
    let Some(name) = name.filter(|n| CONTROLS.contains(&n.to_string().as_str())) else {
        return Err(syn::Error::new(
            expr.span(),
            "control is one of switch, select, segmented, cards, stepper, slider, text, shortcut",
        ));
    };
    for arg in args.into_iter().flatten() {
        match arg {
            Expr::Range(_) => {
                let (min, max) = range(arg)?;
                out.min = Some(min);
                out.max = Some(max);
            }
            Expr::Assign(a) if matches!(&*a.left, Expr::Path(p) if p.path.is_ident("step")) => {
                out.step = Some(number(&a.right)?);
            }
            other => {
                return Err(syn::Error::new(
                    other.span(),
                    "a control takes a range (`0..=100`) and `step = n`",
                ));
            }
        }
    }
    out.control = Some(name);
    Ok(())
}

fn range(expr: &Expr) -> syn::Result<(f64, f64)> {
    let Expr::Range(ExprRange {
        start: Some(start),
        end: Some(end),
        limits: RangeLimits::Closed(_),
        ..
    }) = expr
    else {
        return Err(syn::Error::new(expr.span(), "a range is `min..=max`"));
    };
    Ok((number(start)?, number(end)?))
}

fn number(expr: &Expr) -> syn::Result<f64> {
    match expr {
        Expr::Lit(ExprLit {
            lit: Lit::Int(i), ..
        }) => i.base10_parse::<f64>(),
        Expr::Lit(ExprLit {
            lit: Lit::Float(f), ..
        }) => f.base10_parse::<f64>(),
        Expr::Unary(u) if matches!(u.op, UnOp::Neg(_)) => number(&u.expr).map(|n| -n),
        Expr::Paren(p) => number(&p.expr),
        _ => Err(syn::Error::new(expr.span(), "a number literal")),
    }
}

fn choice(expr: &Expr) -> syn::Result<ChoiceArg> {
    match expr {
        Expr::Lit(ExprLit {
            lit: Lit::Int(i), ..
        }) => Ok(ChoiceArg::Int(i.base10_parse::<i64>()?)),
        Expr::Lit(ExprLit {
            lit: Lit::Float(f), ..
        }) => Ok(ChoiceArg::Float(f.base10_parse::<f64>()?)),
        Expr::Lit(ExprLit {
            lit: Lit::Str(s), ..
        }) => Ok(ChoiceArg::Text(s.value())),
        Expr::Unary(u) if matches!(u.op, UnOp::Neg(_)) => match choice(&u.expr)? {
            ChoiceArg::Int(i) => Ok(ChoiceArg::Int(i.saturating_neg())),
            ChoiceArg::Float(f) => Ok(ChoiceArg::Float(-f)),
            ChoiceArg::Text(_) => Err(syn::Error::new(expr.span(), "a number or a string")),
        },
        _ => Err(syn::Error::new(
            Span::call_site(),
            "a choice is a number or a string literal",
        )),
    }
}

/// `TextSize` → `text_size`.
fn snake(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// `segmented` → `Segmented`.
fn camel(name: &str) -> String {
    let mut chars = name.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}
