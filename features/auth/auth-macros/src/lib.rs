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

use heck::ToSnakeCase;
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{Expr, Fields, ItemStruct, LitStr, Result, parse_macro_input};

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
        return Err(syn::Error::new_spanned(&ident, "an entity needs named fields"));
    };
    let mut fields = Vec::new();
    for f in named.named.iter_mut() {
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
        fields.push((f.ident.clone().expect("named"), f.ty.clone(), opts, docs));
    }
    if !fields.iter().any(|(_, _, o, _)| o.primary_key) {
        return Err(syn::Error::new_spanned(
            &ident,
            "an entity needs a #[entity(primary_key)] field",
        ));
    }
    let table = table
        .map(|t| t.value())
        .unwrap_or_else(|| ident.to_string().to_snake_case());

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
    let update_doc = format!("What a caller may change of a [`{ident}`]; `None` leaves a field as it is.");
    let list_doc = format!("A page of [`{ident}`] rows.");

    Ok(quote! {
        #[derive(::facet::Facet, ::core::clone::Clone, ::core::fmt::Debug, ::core::cmp::PartialEq)]
        #[cfg_attr(feature = "fake", derive(::fake::Dummy))]
        #item

        #[doc = #create_doc]
        #[derive(::facet::Facet, ::core::clone::Clone, ::core::fmt::Debug, ::core::cmp::PartialEq)]
        #[cfg_attr(feature = "fake", derive(::fake::Dummy))]
        #vis struct #create { #(#create_fields,)* }

        #[doc = #update_doc]
        #[derive(::facet::Facet, ::core::clone::Clone, ::core::fmt::Debug, ::core::cmp::PartialEq, ::core::default::Default)]
        #[cfg_attr(feature = "fake", derive(::fake::Dummy))]
        #vis struct #update { #(#update_fields,)* }

        #[doc = #list_doc]
        #[derive(::facet::Facet, ::core::clone::Clone, ::core::fmt::Debug, ::core::cmp::PartialEq)]
        #[cfg_attr(feature = "fake", derive(::fake::Dummy))]
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
