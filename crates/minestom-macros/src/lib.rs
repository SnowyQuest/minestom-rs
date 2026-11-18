//! Procedural macros for `minestom-rs`.
//!
//! Provides optional derive macros to reduce boilerplate when defining
//! Minecraft protocol packets and typed server events.

use proc_macro::TokenStream;
use quote::quote;
use syn::{DeriveInput, parse_macro_input};

/// Derives compile-time packet metadata helpers for a protocol struct.
#[proc_macro_derive(PacketMeta)]
pub fn derive_packet_meta(input: TokenStream) -> TokenStream {
    let parsed = parse_macro_input!(input as DeriveInput);
    let ident = &parsed.ident;
    let expanded = quote! {
        impl #ident {
            /// Returns the Rust struct type name of this packet.
            pub const fn packet_type_name() -> &'static str {
                stringify!(#ident)
            }
        }
    };
    TokenStream::from(expanded)
}
